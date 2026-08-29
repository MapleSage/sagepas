//! Microsoft Entra ID (Azure AD) token validation.
//!
//! Fetches the tenant's OIDC discovery document and JWKS, verifies RS256
//! access tokens cryptographically against the signing keys, and enforces
//! issuer / audience / tenant / expiry before any claim is trusted.
//! Nothing here ever accepts a decoded-but-unverified token.
//!
//! sagepas trusts up to two independent Entra tenants (work order Phase 8):
//! - the Workforce tenant (staff: Admin/Agent/Underwriter, via MSAL)
//! - the Entra External ID (CIAM) tenant (Customer self-service)
//! Each gets its own [`EntraValidator`] instance; `AppState` holds both and
//! tries staff-then-consumer per request.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use domain::auth::{AuthSource, AuthenticatedUser, PasRole};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use tokio::sync::RwLock;

const JWKS_TTL: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Deserialize, Clone)]
struct OidcDiscovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Jwk {
    kid: String,
    kty: String,
    n: String,
    e: String,
}

#[derive(Debug, Deserialize)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

/// The subset of an Entra access token's claims this API cares about.
/// Deliberately does not declare `aud`/`iss` — `jsonwebtoken` validates
/// those against the raw token payload independent of this struct.
#[derive(Debug, Deserialize)]
struct RawEntraClaims {
    sub: String,
    oid: Option<String>,
    tid: Option<String>,
    email: Option<String>,
    preferred_username: Option<String>,
    upn: Option<String>,
    name: Option<String>,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    groups: Vec<String>,
}

struct CachedJwks {
    keys: HashMap<String, Jwk>,
    fetched_at: Instant,
}

/// Validates Entra-issued RS256 access tokens for a single configured tenant + audience.
pub struct EntraValidator {
    tenant_id: String,
    audience: String,
    /// Explicit issuer override, if configured. When absent, the issuer
    /// accepted is whatever Microsoft's own discovery document reports —
    /// never hand-constructed. A CIAM tenant's real `iss` claim uses the
    /// tenant-ID form of its ciamlogin.com host
    /// (`https://<tenant-id>.ciamlogin.com/...`), not the friendly-name
    /// form used to reach the discovery endpoint in the first place
    /// (`https://sagesureconsumer.ciamlogin.com/...`) — both are valid
    /// aliases for reaching the tenant, but only the discovery document
    /// tells you which one Microsoft actually stamps into tokens. Ported
    /// from sesure-us's `entra_auth.rs`, which hit this live.
    issuer_override: Option<String>,
    issuer_v1: String,
    discovery_url: String,
    http: reqwest::Client,
    discovered_issuer: RwLock<Option<String>>,
    jwks_uri: RwLock<Option<String>>,
    jwks: RwLock<Option<CachedJwks>>,
}

impl EntraValidator {
    /// `authority_host` is the domain used to *reach* this tenant's OIDC
    /// discovery document. Defaults to `login.microsoftonline.com` (the
    /// Workforce tenant host). Entra External ID (CIAM) tenants are served
    /// from a distinct `<subdomain>.ciamlogin.com` host — pass it explicitly
    /// for a consumer/CIAM validator, or discovery targets the wrong tenant
    /// type and every token from that tenant fails to validate. Any valid
    /// alias host works here since the *issuer actually enforced* comes from
    /// the discovery document's own `issuer` field, not from this parameter.
    pub fn new(
        tenant_id: String,
        audience: String,
        issuer_override: Option<String>,
        authority_host: Option<String>,
    ) -> Self {
        let authority_host = authority_host
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "login.microsoftonline.com".to_string());
        let issuer_override = issuer_override.filter(|s| !s.trim().is_empty());
        let issuer_v1 = format!("https://sts.windows.net/{tenant_id}/");
        let discovery_url = format!(
            "https://{authority_host}/{tenant_id}/v2.0/.well-known/openid-configuration"
        );
        Self {
            tenant_id,
            audience,
            issuer_override,
            issuer_v1,
            discovery_url,
            http: reqwest::Client::new(),
            discovered_issuer: RwLock::new(None),
            jwks_uri: RwLock::new(None),
            jwks: RwLock::new(None),
        }
    }

    /// Fetches and caches the OIDC discovery document (issuer + JWKS URI
    /// together, since both come from the same response). Cached forever
    /// once resolved — like the JWKS themselves, a tenant's discovery
    /// metadata does not change during a process's lifetime.
    async fn resolve_discovery(&self) -> anyhow::Result<(String, String)> {
        if let (Some(issuer), Some(uri)) = (
            self.discovered_issuer.read().await.clone(),
            self.jwks_uri.read().await.clone(),
        ) {
            return Ok((issuer, uri));
        }
        let discovery: OidcDiscovery = self
            .http
            .get(&self.discovery_url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        *self.discovered_issuer.write().await = Some(discovery.issuer.clone());
        *self.jwks_uri.write().await = Some(discovery.jwks_uri.clone());
        Ok((discovery.issuer, discovery.jwks_uri))
    }

    async fn refresh_jwks(&self) -> anyhow::Result<()> {
        let (_, uri) = self.resolve_discovery().await?;
        let doc: JwksDocument = self
            .http
            .get(&uri)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let keys = doc.keys.into_iter().map(|k| (k.kid.clone(), k)).collect();
        let mut guard = self.jwks.write().await;
        *guard = Some(CachedJwks {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }

    /// Looks up a signing key by `kid`, refreshing the JWKS cache on a
    /// cold start, TTL expiry, or an unknown `kid` (covers key rotation).
    async fn key_for(&self, kid: &str) -> anyhow::Result<Jwk> {
        {
            let guard = self.jwks.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < JWKS_TTL {
                    if let Some(key) = cached.keys.get(kid) {
                        return Ok(key.clone());
                    }
                }
            }
        }
        self.refresh_jwks().await?;
        let guard = self.jwks.read().await;
        guard
            .as_ref()
            .and_then(|cached| cached.keys.get(kid).cloned())
            .ok_or_else(|| anyhow::anyhow!("no signing key found for kid {kid}"))
    }

    /// Verifies signature, issuer, audience, expiry, and tenant, then
    /// returns the authenticated identity with mapped PAS roles.
    pub async fn validate(&self, token: &str) -> anyhow::Result<AuthenticatedUser> {
        let header = decode_header(token)?;
        if header.alg != Algorithm::RS256 {
            anyhow::bail!(
                "unsupported token algorithm {:?}; expected RS256",
                header.alg
            );
        }
        let kid = header
            .kid
            .ok_or_else(|| anyhow::anyhow!("token header is missing kid"))?;
        let jwk = self.key_for(&kid).await?;
        if jwk.kty != "RSA" {
            anyhow::bail!("unsupported JWK key type {}", jwk.kty);
        }
        let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)?;

        // key_for() above guarantees discovery has already run at least once,
        // so this is a cheap cache read except on a very first cold call.
        let issuer_v2 = match &self.issuer_override {
            Some(issuer) => issuer.clone(),
            None => {
                let (discovered, _) = self.resolve_discovery().await?;
                discovered
            }
        };

        validate_with_key(
            token,
            &decoding_key,
            &self.tenant_id,
            &self.audience,
            &issuer_v2,
            &self.issuer_v1,
        )
    }
}

/// Pure claim-validation logic, separated from network/JWKS fetching so it
/// can be exercised directly in tests with a locally generated keypair.
fn validate_with_key(
    token: &str,
    decoding_key: &DecodingKey,
    tenant_id: &str,
    audience: &str,
    issuer_v2: &str,
    issuer_v1: &str,
) -> anyhow::Result<AuthenticatedUser> {
    // Microsoft identity platform tokens for a resource may carry either the
    // App ID URI (e.g. `api://<client-id>`) or the bare client ID GUID as
    // `aud`, depending on how the token was requested — both are valid for
    // the same resource and a validator must accept either.
    let bare_client_id = audience.strip_prefix("api://").unwrap_or(audience);
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[audience, bare_client_id]);
    validation.set_issuer(&[issuer_v2, issuer_v1]);
    validation.validate_exp = true;

    let data = decode::<RawEntraClaims>(token, decoding_key, &validation)
        .map_err(|e| anyhow::anyhow!("token validation failed: {e}"))?;
    let claims = data.claims;

    if let Some(tid) = &claims.tid {
        if tid != tenant_id {
            anyhow::bail!("token tenant {tid} does not match configured tenant {tenant_id}");
        }
    }

    let oid = claims
        .oid
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| claims.sub.clone());

    let mut raw_roles = claims.roles.clone();
    raw_roles.extend(claims.groups.clone());

    let mut pas_roles: Vec<PasRole> = raw_roles
        .iter()
        .filter_map(|r| PasRole::from_claim(r))
        .collect();
    pas_roles.sort_by_key(|r| format!("{r:?}"));
    pas_roles.dedup();

    Ok(AuthenticatedUser {
        oid,
        email: claims.email.or(claims.preferred_username).or(claims.upn),
        name: claims.name,
        tenant_id: claims.tid,
        raw_roles,
        pas_roles,
        source: AuthSource::Entra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use rsa::{
        RsaPrivateKey,
        pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
        rand_core::OsRng,
    };
    use serde_json::json;
    use std::sync::OnceLock;

    // Generate an ephemeral test keypair at runtime. No private key material is
    // stored in source control.
    fn test_keypair() -> &'static (String, String) {
        static KEYS: OnceLock<(String, String)> = OnceLock::new();
        KEYS.get_or_init(|| {
            let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("generate test RSA key");
            let public = private.to_public_key();
            (
                private
                    .to_pkcs8_pem(LineEnding::LF)
                    .expect("encode test private key")
                    .to_string(),
                public
                    .to_public_key_pem(LineEnding::LF)
                    .expect("encode test public key"),
            )
        })
    }

    const TENANT_ID: &str = "11111111-1111-1111-1111-111111111111";
    const AUDIENCE: &str = "api://sagesure-pas-test";
    const ISSUER_V2: &str =
        "https://login.microsoftonline.com/11111111-1111-1111-1111-111111111111/v2.0";
    const ISSUER_V1: &str = "https://sts.windows.net/11111111-1111-1111-1111-111111111111/";

    fn decoding_key() -> DecodingKey {
        DecodingKey::from_rsa_pem(test_keypair().1.as_bytes()).unwrap()
    }

    fn sign_test_token(extra_claims: serde_json::Value) -> String {
        let encoding_key = EncodingKey::from_rsa_pem(test_keypair().0.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".to_string());
        encode(&header, &extra_claims, &encoding_key).unwrap()
    }

    fn base_claims() -> serde_json::Value {
        let now = 2_000_000_000i64; // fixed far-future timestamp, no live clock needed
        json!({
            "sub": "user-subject-id",
            "oid": "22222222-2222-2222-2222-222222222222",
            "tid": TENANT_ID,
            "aud": AUDIENCE,
            "iss": ISSUER_V2,
            "exp": now + 3600,
            "iat": now,
            "nbf": now,
            "email": "agent@example.com",
            "name": "Test Agent",
            "roles": ["agent"],
        })
    }

    #[test]
    fn valid_token_produces_authenticated_user_with_mapped_roles() {
        let token = sign_test_token(base_claims());
        let user = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        )
        .expect("valid token should validate");

        assert_eq!(user.oid, "22222222-2222-2222-2222-222222222222");
        assert_eq!(user.email.as_deref(), Some("agent@example.com"));
        assert_eq!(user.pas_roles, vec![PasRole::Agent]);
        assert_eq!(user.source, AuthSource::Entra);
    }

    #[test]
    fn bare_client_id_audience_is_also_accepted() {
        // Confirmed live: `az account get-access-token --resource api://<id>`
        // can return a token with the bare GUID as `aud`, not the App ID URI.
        let mut claims = base_claims();
        claims["aud"] = json!("sagesure-pas-test");
        let token = sign_test_token(claims);
        let user = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        );
        assert!(user.is_ok(), "bare client-id audience must be accepted");
    }

    #[test]
    fn wrong_audience_is_rejected() {
        let mut claims = base_claims();
        claims["aud"] = json!("api://some-other-app");
        let token = sign_test_token(claims);
        let result = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        );
        assert!(
            result.is_err(),
            "token for a different audience must be rejected"
        );
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        let mut claims = base_claims();
        claims["iss"] = json!("https://login.microsoftonline.com/some-other-tenant/v2.0");
        let token = sign_test_token(claims);
        let result = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        );
        assert!(
            result.is_err(),
            "token from an unexpected issuer must be rejected"
        );
    }

    #[test]
    fn mismatched_tenant_claim_is_rejected_even_with_correct_issuer() {
        let mut claims = base_claims();
        claims["tid"] = json!("33333333-3333-3333-3333-333333333333");
        let token = sign_test_token(claims);
        let result = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        );
        assert!(
            result.is_err(),
            "tid claim must match the configured tenant"
        );
    }

    #[test]
    fn expired_token_is_rejected() {
        let mut claims = base_claims();
        claims["exp"] = json!(1_000_000_000i64); // far in the past
        let token = sign_test_token(claims);
        let result = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        );
        assert!(result.is_err(), "expired token must be rejected");
    }

    #[test]
    fn unrecognized_role_claims_do_not_grant_any_pas_role() {
        let mut claims = base_claims();
        claims["roles"] = json!(["some-unrelated-app-role"]);
        let token = sign_test_token(claims);
        let user = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        )
        .expect("token itself is otherwise valid");
        assert!(
            user.pas_roles.is_empty(),
            "unknown role claims must not map to a default PAS role"
        );
    }

    #[test]
    fn groups_claim_also_contributes_pas_roles() {
        let mut claims = base_claims();
        claims["roles"] = json!([]);
        claims["groups"] = json!(["underwriter"]);
        let token = sign_test_token(claims);
        let user = validate_with_key(
            &token,
            &decoding_key(),
            TENANT_ID,
            AUDIENCE,
            ISSUER_V2,
            ISSUER_V1,
        )
        .expect("valid token should validate");
        assert_eq!(user.pas_roles, vec![PasRole::Underwriter]);
    }
}
