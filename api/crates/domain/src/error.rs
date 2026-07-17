use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    // ── Auth ─────────────────────────────────────────────────────────────────
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("account locked until {until}")]
    AccountLocked {
        until: chrono::DateTime<chrono::Utc>,
    },
    #[error("token expired")]
    TokenExpired,
    #[error("token invalid")]
    TokenInvalid,
    #[error("insufficient role: need {required}, have {actual}")]
    InsufficientRole { required: String, actual: String },

    // ── ScamShield ───────────────────────────────────────────────────────────
    #[error("pattern engine error: {0}")]
    PatternEngine(String),
    #[error("phone verification failed: {0}")]
    PhoneVerification(String),

    // ── Policy Pulse ─────────────────────────────────────────────────────────
    #[error("pdf extraction failed: {0}")]
    PdfExtraction(String),
    #[error("translation failed: {0}")]
    Translation(String),

    // ── Claims Defender ──────────────────────────────────────────────────────
    #[error("claim not found: {0}")]
    ClaimNotFound(String),

    // ── Sovereign Vault ──────────────────────────────────────────────────────
    #[error("encryption error")]
    Encryption,
    #[error("document not found: {0}")]
    DocumentNotFound(String),

    // ── Infrastructure (non-I/O, surface-level) ───────────────────────────────
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation: {0}")]
    Validation(String),
}
