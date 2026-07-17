use anyhow::{Context, anyhow};
use azure_core::auth::TokenCredential;
use azure_storage::StorageCredentials;
use azure_storage_blobs::prelude::{BlobServiceClient, ContainerClient};
use std::sync::Arc;
use tracing::error;

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

        // Fallback to DefaultAzureCredential (workload identity)
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
