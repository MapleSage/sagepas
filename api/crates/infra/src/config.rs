use serde::Deserialize;

/// Configuration required by the standalone SagePAS API.
#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub database_url: String,

    /// Optional Redis endpoint for distributed policy locks.
    #[serde(default)]
    pub redis_url: String,

    pub storage_account_name: String,
    #[serde(default = "default_storage_container")]
    pub storage_container_name: String,

    /// Private key used only by explicitly enabled local-development auth.
    #[serde(default)]
    pub jwt_private_key: String,

    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_env")]
    pub node_env: String,

    /// Azure AI Search supplies configurable PAS pricing factors.
    #[serde(default = "default_search_endpoint")]
    pub search_endpoint: String,
    /// Optional API-key fallback when managed identity is unavailable.
    #[serde(default)]
    pub search_key: String,

    /// Microsoft Entra ID token-validation boundary.
    #[serde(default)]
    pub azure_entra_tenant_id: String,
    #[serde(default)]
    pub azure_entra_audience: String,
    #[serde(default)]
    pub azure_entra_issuer: String,

    /// Explicit development-only opt-in for locally issued HS256 tokens.
    /// It defaults to false, leaving production authentication Entra-only.
    #[serde(default)]
    pub dev_local_auth_enabled: bool,
}

fn default_port() -> u16 {
    3000
}

fn default_env() -> String {
    "production".into()
}

fn default_search_endpoint() -> String {
    "https://sageinsure-search.search.windows.net".into()
}

fn default_storage_container() -> String {
    "policy-documents".into()
}

impl AppConfig {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::from_env()
    }

    pub fn is_production(&self) -> bool {
        self.node_env == "production"
    }

    pub fn entra_configured(&self) -> bool {
        !self.azure_entra_tenant_id.trim().is_empty()
            && !self.azure_entra_audience.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    fn base_config() -> serde_json::Value {
        serde_json::json!({
            "database_url": "postgres://localhost/test",
            "storage_account_name": "acct"
        })
    }

    #[test]
    fn secure_auth_defaults_are_fail_closed() {
        let cfg: AppConfig = serde_json::from_value(base_config()).expect("config should load");
        assert!(!cfg.dev_local_auth_enabled);
        assert!(!cfg.entra_configured());
        assert_eq!(cfg.storage_container_name, "policy-documents");
    }

    #[test]
    fn entra_requires_tenant_and_audience() {
        let mut partial = base_config();
        partial["azure_entra_tenant_id"] = serde_json::json!("tenant-guid");
        let cfg: AppConfig = serde_json::from_value(partial).expect("partial config should load");
        assert!(!cfg.entra_configured());

        let mut complete = base_config();
        complete["azure_entra_tenant_id"] = serde_json::json!("tenant-guid");
        complete["azure_entra_audience"] = serde_json::json!("api://sagepas");
        let cfg: AppConfig = serde_json::from_value(complete).expect("complete config should load");
        assert!(cfg.entra_configured());
    }
}
