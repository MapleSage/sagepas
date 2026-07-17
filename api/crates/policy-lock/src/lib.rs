use std::sync::Arc;

use axum::{
    Json,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use domain::ids::PolicyId;
use redis::Script;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_LOCK_TTL_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct PolicyLockManager {
    client: Option<redis::Client>,
    lock_ttl_secs: u64,
}

impl PolicyLockManager {
    pub fn new(redis_url: &str) -> Result<Self, PolicyLockError> {
        if redis_url.trim().is_empty() {
            return Ok(Self::new_disabled());
        }

        let client = redis::Client::open(redis_url)
            .map_err(|err| PolicyLockError::RedisError(err.to_string()))?;

        Ok(Self {
            client: Some(client),
            lock_ttl_secs: DEFAULT_LOCK_TTL_SECS,
        })
    }

    pub fn new_disabled() -> Self {
        Self {
            client: None,
            lock_ttl_secs: DEFAULT_LOCK_TTL_SECS,
        }
    }

    pub fn lock_key(policy_id: PolicyId) -> String {
        format!("policy_lock:{policy_id}")
    }

    pub async fn acquire(
        self: &Arc<Self>,
        policy_id: PolicyId,
    ) -> Result<PolicyLock, PolicyLockError> {
        let Some(client) = &self.client else {
            return Ok(PolicyLock::disabled(policy_id, Arc::clone(self)));
        };

        let owner_token = Uuid::new_v4().to_string();
        let key = Self::lock_key(policy_id);
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| PolicyLockError::RedisError(err.to_string()))?;

        let lock_result: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(&owner_token)
            .arg("NX")
            .arg("EX")
            .arg(self.lock_ttl_secs)
            .query_async(&mut conn)
            .await
            .map_err(|err| PolicyLockError::RedisError(err.to_string()))?;

        if lock_result.is_none() {
            return Err(PolicyLockError::AlreadyLocked);
        }

        Ok(PolicyLock {
            policy_id,
            owner_token,
            manager: Arc::clone(self),
            disabled: false,
            released: false,
        })
    }

    pub async fn release(&self, mut lock: PolicyLock) -> Result<(), PolicyLockError> {
        self.release_inner(lock.policy_id, &lock.owner_token, lock.disabled)
            .await?;
        lock.released = true;
        Ok(())
    }

    async fn release_inner(
        &self,
        policy_id: PolicyId,
        owner_token: &str,
        disabled: bool,
    ) -> Result<(), PolicyLockError> {
        if disabled {
            return Ok(());
        }

        let Some(client) = &self.client else {
            return Ok(());
        };

        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| PolicyLockError::RedisError(err.to_string()))?;

        let script = Script::new(
            "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end",
        );
        let _: i32 = script
            .key(Self::lock_key(policy_id))
            .arg(owner_token)
            .invoke_async(&mut conn)
            .await
            .map_err(|err| PolicyLockError::RedisError(err.to_string()))?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct PolicyLock {
    policy_id: PolicyId,
    owner_token: String,
    manager: Arc<PolicyLockManager>,
    disabled: bool,
    released: bool,
}

impl PolicyLock {
    fn disabled(policy_id: PolicyId, manager: Arc<PolicyLockManager>) -> Self {
        Self {
            policy_id,
            owner_token: String::new(),
            manager,
            disabled: true,
            released: false,
        }
    }

    pub fn policy_id(&self) -> PolicyId {
        self.policy_id
    }

    pub fn owner_token(&self) -> &str {
        &self.owner_token
    }
}

impl Drop for PolicyLock {
    fn drop(&mut self) {
        if self.released || self.disabled {
            return;
        }

        let manager = Arc::clone(&self.manager);
        let policy_id = self.policy_id;
        let owner_token = self.owner_token.clone();
        tokio::spawn(async move {
            let _ = manager.release_inner(policy_id, &owner_token, false).await;
        });
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PolicyLockError {
    #[error("policy locked")]
    AlreadyLocked,
    #[error("redis error: {0}")]
    RedisError(String),
    #[error("policy locking disabled")]
    Disabled,
}

#[derive(Serialize)]
struct LockedBody {
    error: &'static str,
    retry_after_secs: u64,
}

pub trait HasPolicyLock {
    fn policy_lock(&self) -> Arc<PolicyLockManager>;
}

pub async fn require_policy_lock<S>(
    State(state): State<S>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response
where
    S: HasPolicyLock + Clone + Send + Sync + 'static,
{
    let policy_id = match policy_id_from_request(&mut request).await {
        Ok(policy_id) => policy_id,
        Err(response) => return response,
    };

    let manager = state.policy_lock();
    let lock = match manager.acquire(policy_id).await {
        Ok(lock) => lock,
        Err(PolicyLockError::AlreadyLocked) => {
            return (
                StatusCode::LOCKED,
                Json(LockedBody {
                    error: "policy locked",
                    retry_after_secs: 5,
                }),
            )
                .into_response();
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    let response = next.run(request).await;
    let _ = manager.release(lock).await;
    response
}

async fn policy_id_from_request(
    request: &mut Request<axum::body::Body>,
) -> Result<PolicyId, Response> {
    let body = std::mem::replace(request.body_mut(), Body::empty());
    let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid request body: {err}") })),
            )
                .into_response());
        }
    };

    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(err) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid json body: {err}") })),
            )
                .into_response());
        }
    };

    let Some(policy_id) = value.get("policy_id").and_then(|v| v.as_str()) else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing policy_id" })),
        )
            .into_response());
    };

    let policy_id = policy_id.parse::<PolicyId>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid policy_id" })),
        )
            .into_response()
    })?;

    *request.body_mut() = axum::body::Body::from(bytes);
    Ok(policy_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_disabled_mode_always_succeeds() {
        let manager = Arc::new(PolicyLockManager::new_disabled());
        let policy_id = PolicyId::new();

        let lock1 = manager.acquire(policy_id).await.expect("first lock");
        let lock2 = manager.acquire(policy_id).await.expect("second lock");

        assert_eq!(lock1.policy_id(), policy_id);
        assert_eq!(lock2.policy_id(), policy_id);
    }

    #[test]
    fn test_lock_key_format() {
        let policy_id: PolicyId = "11111111-1111-4111-8111-111111111111"
            .parse()
            .expect("valid uuid");

        assert_eq!(
            PolicyLockManager::lock_key(policy_id),
            "policy_lock:11111111-1111-4111-8111-111111111111"
        );
    }
}
