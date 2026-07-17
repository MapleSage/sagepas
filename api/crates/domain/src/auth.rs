use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Consumer,
    Broker,
    Agent,
    Insurer,
    Regulator,
}

impl std::str::FromStr for UserRole {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "consumer" => Ok(Self::Consumer),
            "broker" => Ok(Self::Broker),
            "agent" => Ok(Self::Agent),
            "insurer" => Ok(Self::Insurer),
            "regulator" => Ok(Self::Regulator),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Consumer => "consumer",
            Self::Broker => "broker",
            Self::Agent => "agent",
            Self::Insurer => "insurer",
            Self::Regulator => "regulator",
        };
        f.write_str(s)
    }
}

/// JWT payload — issued by the auth service, verified by the JWT middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — the user's UUID.
    pub sub: Uuid,
    pub role: UserRole,
    /// Issued-at (Unix seconds).
    pub iat: i64,
    /// Expiry (Unix seconds).
    pub exp: i64,
}

/// PAS application roles. These are the only roles the frontend and API
/// authorize against — always derived from a validated token claim
/// (Entra app role / group, or the dev-local `UserRole` mapping below),
/// never from client-supplied or self-asserted input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasRole {
    Admin,
    Agent,
    Underwriter,
    Customer,
}

impl PasRole {
    /// Maps a raw Entra `roles`/`groups` claim value to a PAS role.
    /// Unrecognized claim values map to `None` and are dropped rather than
    /// granting any default access.
    pub fn from_claim(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "admin" | "pas.admin" | "administrator" => Some(Self::Admin),
            "agent" | "pas.agent" => Some(Self::Agent),
            "underwriter" | "pas.underwriter" => Some(Self::Underwriter),
            "customer" | "pas.customer" | "consumer" => Some(Self::Customer),
            _ => None,
        }
    }

    /// Maps the legacy dev-local `UserRole` to a PAS role, for the
    /// HS256 development auth path only (see `AuthSource::DevLocal`).
    pub fn from_user_role(role: UserRole) -> Option<Self> {
        match role {
            UserRole::Consumer => Some(Self::Customer),
            UserRole::Agent | UserRole::Broker => Some(Self::Agent),
            UserRole::Insurer => Some(Self::Underwriter),
            UserRole::Regulator => Some(Self::Admin),
        }
    }
}

/// Where a request's identity was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    /// Cryptographically validated Microsoft Entra ID (Azure AD) token.
    Entra,
    /// Local HS256 dev-only token — only ever issued/accepted when
    /// `dev_local_auth_enabled` is explicitly set.
    DevLocal,
}

/// The authenticated identity attached to a request by `require_auth`.
/// Always the product of cryptographic token validation — never
/// constructed from unverified client input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    /// Entra `oid` (object id) claim, or the dev-local user's UUID string.
    pub oid: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub tenant_id: Option<String>,
    /// Raw role/group claim strings as presented by the token, for audit.
    pub raw_roles: Vec<String>,
    /// Roles mapped and validated against the PAS role set.
    pub pas_roles: Vec<PasRole>,
    pub source: AuthSource,
}

impl AuthenticatedUser {
    pub fn has_any_role(&self, allowed: &[PasRole]) -> bool {
        self.pas_roles.iter().any(|role| allowed.contains(role))
    }
}

#[cfg(test)]
mod pas_role_tests {
    use super::*;

    #[test]
    fn from_claim_maps_known_roles_case_insensitively() {
        assert_eq!(PasRole::from_claim("Admin"), Some(PasRole::Admin));
        assert_eq!(PasRole::from_claim("AGENT"), Some(PasRole::Agent));
        assert_eq!(
            PasRole::from_claim("underwriter"),
            Some(PasRole::Underwriter)
        );
        assert_eq!(PasRole::from_claim("Customer"), Some(PasRole::Customer));
        assert_eq!(PasRole::from_claim("pas.admin"), Some(PasRole::Admin));
    }

    #[test]
    fn from_claim_rejects_unknown_values_instead_of_defaulting() {
        assert_eq!(PasRole::from_claim("superuser"), None);
        assert_eq!(PasRole::from_claim(""), None);
        assert_eq!(PasRole::from_claim("global admin"), None);
    }

    #[test]
    fn from_user_role_maps_every_legacy_role() {
        assert_eq!(
            PasRole::from_user_role(UserRole::Consumer),
            Some(PasRole::Customer)
        );
        assert_eq!(
            PasRole::from_user_role(UserRole::Agent),
            Some(PasRole::Agent)
        );
        assert_eq!(
            PasRole::from_user_role(UserRole::Broker),
            Some(PasRole::Agent)
        );
        assert_eq!(
            PasRole::from_user_role(UserRole::Insurer),
            Some(PasRole::Underwriter)
        );
        assert_eq!(
            PasRole::from_user_role(UserRole::Regulator),
            Some(PasRole::Admin)
        );
    }

    #[test]
    fn has_any_role_checks_membership() {
        let user = AuthenticatedUser {
            oid: "test-oid".to_string(),
            email: None,
            name: None,
            tenant_id: None,
            raw_roles: vec!["agent".to_string()],
            pas_roles: vec![PasRole::Agent],
            source: AuthSource::Entra,
        };
        assert!(user.has_any_role(&[PasRole::Agent, PasRole::Admin]));
        assert!(!user.has_any_role(&[PasRole::Customer]));
    }
}
