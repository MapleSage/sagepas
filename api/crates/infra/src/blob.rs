use anyhow::{Context, anyhow};
use azure_core::auth::{AccessToken, TokenCredential};
use azure_storage::StorageCredentials;
use azure_storage_blobs::prelude::{BlobServiceClient, ContainerClient};
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tracing::error;

/// Workload-identity credential that re-reads `AZURE_FEDERATED_TOKEN_FILE`
/// on every token request instead of once at construction.
///
/// `azure_identity::WorkloadIdentityCredential` reads the federated JWT once
/// when created and holds it for the credential's lifetime. AKS rotates that
/// projected token file periodically (kubelet refreshes it well before its
/// own expiry). A `BlobClient` built once at process startup and held for
/// the pod's life (as this one is, via `main.rs`) would keep exchanging an
/// assertion captured at boot — correct for the first stretch after a
/// deploy, then failing with a 401 once that original assertion expires,
/// even though the on-disk file has long since been rotated to a valid one.
/// This type fixes that by treating the file as the source of truth on
/// every call, not a one-time read.
#[derive(Debug)]
struct FreshFederatedCredential {
    http_client: Arc<dyn azure_core::HttpClient>,
    authority_host: azure_core::Url,
    tenant_id: String,
    client_id: String,
    token_file: String,
}

impl FreshFederatedCredential {
    fn create() -> anyhow::Result<Self> {
        let tenant_id = std::env::var("AZURE_TENANT_ID")
            .context("workload identity credential requires AZURE_TENANT_ID")?;
        let client_id = std::env::var("AZURE_CLIENT_ID")
            .context("workload identity credential requires AZURE_CLIENT_ID")?;
        let token_file = std::env::var("AZURE_FEDERATED_TOKEN_FILE")
            .context("workload identity credential requires AZURE_FEDERATED_TOKEN_FILE")?;
        let authority_host = std::env::var("AZURE_AUTHORITY_HOST")
            .unwrap_or_else(|_| "https://login.microsoftonline.com/".to_string());
        Ok(Self {
            http_client: azure_core::new_http_client(),
            authority_host: azure_core::Url::parse(&authority_host)
                .context("AZURE_AUTHORITY_HOST is not a valid URL")?,
            tenant_id,
            client_id,
            token_file,
        })
    }
}

#[async_trait::async_trait]
impl TokenCredential for FreshFederatedCredential {
    async fn get_token(&self, scopes: &[&str]) -> azure_core::Result<AccessToken> {
        use azure_core::error::{ErrorKind, ResultExt};

        let assertion = ResultExt::with_context(
            std::fs::read_to_string(&self.token_file),
            ErrorKind::Credential,
            || format!("failed to read federated token from file {}", self.token_file),
        )?;
        let login = ResultExt::with_context(
            azure_identity::federated_credentials_flow::perform(
                self.http_client.clone(),
                &self.client_id,
                &assertion,
                scopes,
                &self.tenant_id,
                &self.authority_host,
            )
            .await,
            ErrorKind::Credential,
            || "request token error",
        )?;
        Ok(AccessToken::new(
            login.access_token().clone(),
            OffsetDateTime::now_utc() + Duration::from_secs(login.expires_in),
        ))
    }

    async fn clear_cache(&self) -> azure_core::Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct BlobClient {
    container: ContainerClient,
}

impl BlobClient {
    /// Create a client using access key (from AZURE_STORAGE_ACCESS_KEY env) or managed identity fallback.
    pub fn from_managed_identity(account: &str, container: &str) -> anyhow::Result<Self> {
        // Try access key first (for clusters without workload identity webhook)
        if let Ok(access_key) = std::env::var("AZURE_STORAGE_ACCESS_KEY") {
            tracing::info!(account = %account, container = %container, "using access key auth for blob client");
            return Self::from_access_key(account, container, &access_key);
        }

        // azure_identity 0.20's DefaultAzureCredential chain is Environment -> AppService
        // -> VirtualMachine (IMDS) -> AzureCli; it does not try WorkloadIdentityCredential,
        // so on an AKS pod using workload identity federation (AZURE_FEDERATED_TOKEN_FILE)
        // every source in that chain fails and DefaultAzureCredential never authenticates.
        // Try WorkloadIdentityCredential explicitly first when its env vars are present.
        if std::env::var("AZURE_FEDERATED_TOKEN_FILE").is_ok() {
            tracing::info!(account = %account, container = %container, "using FreshFederatedCredential (re-reads token file per request) for blob client");
            let cred = FreshFederatedCredential::create()
                .context("failed to create workload identity credential for blob managed identity")?;
            return Ok(Self::new(account, container, Arc::new(cred)));
        }

        // Fallback to DefaultAzureCredential (VM IMDS / Azure CLI / App Service)
        tracing::info!(account = %account, container = %container, "using DefaultAzureCredential for blob client");
        let cred = azure_identity::create_default_credential()
            .context("failed to create Azure default credential for blob managed identity")?;
        Ok(Self::new(account, container, cred))
    }

    pub fn from_access_key(
        account: &str,
        container: &str,
        access_key: &str,
    ) -> anyhow::Result<Self> {
        let creds = StorageCredentials::access_key(account.to_string(), access_key.to_string());
        let service = BlobServiceClient::new(account, creds);
        Ok(Self {
            container: service.container_client(container),
        })
    }

    pub fn new(account: &str, container: &str, credential: Arc<dyn TokenCredential>) -> Self {
        let creds = StorageCredentials::token_credential(credential);
        let service = BlobServiceClient::new(account, creds);
        Self {
            container: service.container_client(container),
        }
    }

    pub async fn upload(
        &self,
        name: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> anyhow::Result<()> {
        let ct = content_type.to_owned();
        match self
            .container
            .blob_client(name)
            .put_block_blob(data)
            .content_type(ct)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                let error_code = azure_error_code_hint(&e).unwrap_or_else(|| "unknown".to_string());
                let err = anyhow!(e).context(format!(
                    "Azure blob upload failed for blob '{name}' (azure_error_code={error_code})"
                ));
                let error_chain = format!("{err:#}");

                error!(
                    blob_name = %name,
                    azure_error_code = %error_code,
                    error_chain = %error_chain,
                    "blob upload failed"
                );

                Err(err)
            }
        }
    }

    pub async fn download(&self, name: &str) -> anyhow::Result<Vec<u8>> {
        let data = self
            .container
            .blob_client(name)
            .get_content()
            .await
            .with_context(|| format!("Azure blob download failed for blob '{name}'"))?;
        Ok(data)
    }

    pub async fn delete(&self, name: &str) -> anyhow::Result<()> {
        self.container
            .blob_client(name)
            .delete()
            .await
            .with_context(|| format!("Azure blob delete failed for blob '{name}'"))?;
        Ok(())
    }
}

fn azure_error_code_hint<E: std::fmt::Debug + std::fmt::Display>(err: &E) -> Option<String> {
    let haystack = format!("{err} {err:?}");
    for code in [
        "AuthorizationPermissionMismatch",
        "AuthorizationFailure",
        "AuthenticationFailed",
        "ResourceNotFound",
        "ContainerNotFound",
        "BlobNotFound",
        "PublicAccessNotPermitted",
        "AccountIsDisabled",
        "InsufficientAccountPermissions",
    ] {
        if haystack.contains(code) {
            return Some(code.to_string());
        }
    }
    None
}

/// Extract the blob path (name within the container) from a full Azure blob URL.
///
/// URL format: `https://{account}.blob.core.windows.net/{container}/{path}`
pub fn blob_path_from_url(url: &str, account: &str, container: &str) -> String {
    let prefix = format!("https://{account}.blob.core.windows.net/{container}/");
    url.strip_prefix(&prefix).unwrap_or(url).to_string()
}
