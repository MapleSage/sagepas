use std::sync::Arc;

use chrono::{DateTime, Utc};
use infra::db::DbPool;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ConversationStore {
    pool: Arc<DbPool>,
}

impl ConversationStore {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> Arc<DbPool> {
        self.pool.clone()
    }

    pub async fn get_or_create_session(
        &self,
        user_id: Uuid,
        surface: &str,
        workflow_id: Option<&str>,
    ) -> Result<ConversationSession, MemoryError> {
        get_or_create_session(&self.pool, user_id, surface, workflow_id).await
    }

    pub async fn append_message(
        &self,
        session_id: Uuid,
        role: &str,
        content: &str,
        surface: &str,
    ) -> Result<(), MemoryError> {
        append_message(&self.pool, session_id, role, content, surface).await
    }

    pub async fn load_recent_messages(
        &self,
        session_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>, MemoryError> {
        load_recent_messages(&self.pool, session_id, limit).await
    }

    pub async fn upsert_fact(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        fact_type: &str,
        fact_key: &str,
        fact_value: &str,
        source_surface: &str,
    ) -> Result<(), MemoryError> {
        upsert_fact(
            &self.pool,
            user_id,
            session_id,
            fact_type,
            fact_key,
            fact_value,
            source_surface,
        )
        .await
    }

    pub async fn load_facts(&self, user_id: Uuid) -> Result<Vec<MemoryFact>, MemoryError> {
        load_facts(&self.pool, user_id).await
    }

    pub async fn build_context(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        current_message: &str,
    ) -> Result<ConversationContext, MemoryError> {
        build_context(&self.pool, user_id, session_id, current_message).await
    }

    pub async fn save_summary(
        &self,
        session_id: Uuid,
        summary_text: &str,
        message_count: usize,
    ) -> Result<(), MemoryError> {
        save_summary(&self.pool, session_id, summary_text, message_count).await
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ConversationSession {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub surface: String,
    pub workflow_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub message_id: Uuid,
    pub session_id: Uuid,
    pub role: String,
    pub content: String,
    pub surface: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct MemoryFact {
    pub fact_id: Uuid,
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    pub fact_type: String,
    pub fact_key: String,
    pub fact_value: String,
    pub confidence: String,
    pub source_surface: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedFact {
    pub fact_type: String,
    pub fact_key: String,
    pub fact_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationContext {
    pub session_id: Uuid,
    pub messages: Vec<ConversationMessage>,
    pub facts: Vec<MemoryFact>,
    pub system_prompt_injection: String,
}

impl ConversationContext {
    pub fn to_system_injection(&self) -> String {
        if self.facts.is_empty() {
            return String::new();
        }

        let mut out = String::from("Known context for this user:\n");
        for fact in &self.facts {
            out.push_str(&format!("- {}: {}\n", fact.fact_type, fact.fact_value));
        }
        out.trim_end().to_string()
    }
}

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("database error: {0}")]
    DatabaseError(String),
    #[error("session not found")]
    SessionNotFound,
    #[error("fact conflict: {0}")]
    FactConflict(String),
}

impl From<sqlx::Error> for MemoryError {
    fn from(value: sqlx::Error) -> Self {
        MemoryError::DatabaseError(value.to_string())
    }
}

pub async fn get_or_create_session(
    pool: &Arc<DbPool>,
    user_id: Uuid,
    surface: &str,
    workflow_id: Option<&str>,
) -> Result<ConversationSession, MemoryError> {
    if let Some(session) = sqlx::query_as::<_, ConversationSession>(
        r#"
        UPDATE conversation_sessions
        SET last_active_at = NOW()
        WHERE session_id = (
            SELECT session_id
            FROM conversation_sessions
            WHERE user_id = $1
              AND surface = $2
              AND (
                ($3::TEXT IS NULL AND workflow_id IS NULL)
                OR workflow_id = $3
              )
              AND last_active_at > NOW() - INTERVAL '30 minutes'
            ORDER BY last_active_at DESC
            LIMIT 1
        )
        RETURNING session_id, user_id, surface, workflow_id, created_at, last_active_at, metadata
        "#,
    )
    .bind(user_id)
    .bind(surface)
    .bind(workflow_id)
    .fetch_optional(&***pool)
    .await?
    {
        return Ok(session);
    }

    let session = sqlx::query_as::<_, ConversationSession>(
        r#"
        INSERT INTO conversation_sessions (user_id, surface, workflow_id)
        VALUES ($1, $2, $3)
        RETURNING session_id, user_id, surface, workflow_id, created_at, last_active_at, metadata
        "#,
    )
    .bind(user_id)
    .bind(surface)
    .bind(workflow_id)
    .fetch_one(&***pool)
    .await?;

    Ok(session)
}

pub async fn append_message(
    pool: &Arc<DbPool>,
    session_id: Uuid,
    role: &str,
    content: &str,
    surface: &str,
) -> Result<(), MemoryError> {
    sqlx::query(
        r#"
        INSERT INTO conversation_messages (session_id, role, content, surface)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(surface)
    .execute(&***pool)
    .await?;

    sqlx::query("UPDATE conversation_sessions SET last_active_at = NOW() WHERE session_id = $1")
        .bind(session_id)
        .execute(&***pool)
        .await?;

    Ok(())
}

pub async fn load_recent_messages(
    pool: &Arc<DbPool>,
    session_id: Uuid,
    limit: usize,
) -> Result<Vec<ConversationMessage>, MemoryError> {
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let messages = sqlx::query_as::<_, ConversationMessage>(
        r#"
        SELECT * FROM (
            SELECT message_id, session_id, role, content, surface, created_at
            FROM conversation_messages
            WHERE session_id = $1
            ORDER BY created_at DESC
            LIMIT $2
        ) recent
        ORDER BY created_at ASC
        "#,
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(&***pool)
    .await?;

    Ok(messages)
}

pub async fn upsert_fact(
    pool: &Arc<DbPool>,
    user_id: Uuid,
    session_id: Uuid,
    fact_type: &str,
    fact_key: &str,
    fact_value: &str,
    source_surface: &str,
) -> Result<(), MemoryError> {
    sqlx::query(
        r#"
        INSERT INTO conversation_memory_facts
            (user_id, session_id, fact_type, fact_key, fact_value, source_surface)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (user_id, fact_type, fact_key)
        DO UPDATE SET fact_value = EXCLUDED.fact_value, updated_at = NOW()
        "#,
    )
    .bind(user_id)
    .bind(session_id)
    .bind(fact_type)
    .bind(fact_key)
    .bind(fact_value)
    .bind(source_surface)
    .execute(&***pool)
    .await?;

    Ok(())
}

pub async fn load_facts(pool: &Arc<DbPool>, user_id: Uuid) -> Result<Vec<MemoryFact>, MemoryError> {
    let facts = sqlx::query_as::<_, MemoryFact>(
        r#"
        SELECT fact_id, user_id, session_id, fact_type, fact_key, fact_value,
               confidence::TEXT AS confidence, source_surface, created_at, updated_at
        FROM conversation_memory_facts
        WHERE user_id = $1
        ORDER BY updated_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(&***pool)
    .await?;

    Ok(facts)
}

pub async fn build_context(
    pool: &Arc<DbPool>,
    user_id: Uuid,
    session_id: Uuid,
    _current_message: &str,
) -> Result<ConversationContext, MemoryError> {
    let messages = load_recent_messages(pool, session_id, 20).await?;
    let facts = load_facts(pool, user_id).await?;
    let mut context = ConversationContext {
        session_id,
        messages,
        facts,
        system_prompt_injection: String::new(),
    };
    context.system_prompt_injection = context.to_system_injection();
    Ok(context)
}

pub async fn save_summary(
    pool: &Arc<DbPool>,
    session_id: Uuid,
    summary_text: &str,
    message_count: usize,
) -> Result<(), MemoryError> {
    let message_count = i32::try_from(message_count).unwrap_or(i32::MAX);
    sqlx::query(
        r#"
        INSERT INTO conversation_summaries (session_id, summary_text, message_count)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(session_id)
    .bind(summary_text)
    .bind(message_count)
    .execute(&***pool)
    .await?;

    Ok(())
}

pub fn extract_facts_from_turn(user_message: &str, assistant_response: &str) -> Vec<ExtractedFact> {
    let text = format!("{user_message}\n{assistant_response}");
    let mut facts = Vec::new();

    collect_matches(&mut facts, &text, r"\b[A-Z]{2,4}-\d{4,}\b", "policy_number");
    collect_matches(&mut facts, &text, r"\bCLM-[A-Z0-9]+\b", "claim_id");
    collect_matches(
        &mut facts,
        &text,
        r"\b\d{4}-\d{2}-\d{2}\b",
        "date_mentioned",
    );
    collect_matches(&mut facts, &text, r"\$[\d,]+\b", "amount");

    let name_re =
        Regex::new(r"(?i)\b(?:my name is|i am)\s+([A-Z][A-Za-z'\-]*(?:\s+[A-Z][A-Za-z'\-]*){0,3})")
            .unwrap();
    for cap in name_re.captures_iter(&text) {
        if let Some(name) = cap.get(1) {
            let value = name.as_str().trim().trim_end_matches('.').to_string();
            if !value.is_empty() {
                push_unique(&mut facts, "user_name", "user_name", &value);
            }
        }
    }

    facts
}

fn collect_matches(facts: &mut Vec<ExtractedFact>, text: &str, pattern: &str, fact_type: &str) {
    let re = Regex::new(pattern).unwrap();
    for m in re.find_iter(text) {
        let value = m.as_str();
        push_unique(facts, fact_type, value, value);
    }
}

fn push_unique(facts: &mut Vec<ExtractedFact>, fact_type: &str, fact_key: &str, fact_value: &str) {
    if facts
        .iter()
        .any(|f| f.fact_type == fact_type && f.fact_key == fact_key && f.fact_value == fact_value)
    {
        return;
    }

    facts.push(ExtractedFact {
        fact_type: fact_type.to_string(),
        fact_key: fact_key.to_string(),
        fact_value: fact_value.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn memory_fact(fact_type: &str, fact_value: &str) -> MemoryFact {
        let now = Utc::now();
        MemoryFact {
            fact_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            session_id: Some(Uuid::new_v4()),
            fact_type: fact_type.to_string(),
            fact_key: fact_value.to_string(),
            fact_value: fact_value.to_string(),
            confidence: "1.00".to_string(),
            source_surface: "chat".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn test_extract_policy_number() {
        let facts = extract_facts_from_turn("My policy is HO-12345", "I found HO-12345.");
        assert!(
            facts
                .iter()
                .any(|f| f.fact_type == "policy_number" && f.fact_value == "HO-12345")
        );
    }

    #[test]
    fn test_extract_claim_id() {
        let facts = extract_facts_from_turn("Check CLM-ABC123", "Claim CLM-ABC123 is open.");
        assert!(
            facts
                .iter()
                .any(|f| f.fact_type == "claim_id" && f.fact_value == "CLM-ABC123")
        );
    }

    #[test]
    fn test_extract_amount() {
        let facts = extract_facts_from_turn("The damage is $12,500", "Noted $12,500.");
        assert!(
            facts
                .iter()
                .any(|f| f.fact_type == "amount" && f.fact_value == "$12,500")
        );
    }

    #[test]
    fn test_context_injection_format() {
        let context = ConversationContext {
            session_id: Uuid::new_v4(),
            messages: vec![],
            facts: vec![
                memory_fact("policy_number", "HO-12345"),
                memory_fact("claim_id", "CLM-ABC123"),
            ],
            system_prompt_injection: String::new(),
        };

        let injection = context.to_system_injection();
        assert!(injection.starts_with("Known context for this user:\n"));
        assert!(injection.contains("- policy_number: HO-12345"));
        assert!(injection.contains("- claim_id: CLM-ABC123"));
    }

    #[test]
    fn test_no_facts_empty_injection() {
        let context = ConversationContext {
            session_id: Uuid::new_v4(),
            messages: vec![],
            facts: vec![],
            system_prompt_injection: String::new(),
        };

        assert_eq!(context.to_system_injection(), "");
    }
}
