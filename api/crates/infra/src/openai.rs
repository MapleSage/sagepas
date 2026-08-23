use reqwest::Client;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenAIError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("API error: {0}")]
    ApiError(String),
}

#[derive(Clone, Debug)]
pub struct OpenAIClient {
    http: Client,
    endpoint: String,
    api_key: String,
    deployment: String,
}

impl OpenAIClient {
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        deployment: impl Into<String>,
    ) -> Self {
        Self {
            http: Client::new(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            deployment: deployment.into(),
        }
    }

    /// Chat completion via Azure AI Foundry's `/openai/v1/responses` API.
    /// If model is empty, uses the configured deployment.
    ///
    /// `temperature` is accepted for backward compatibility with existing callers
    /// but silently dropped: reasoning-model deployments reject it outright with
    /// HTTP 400 ("not supported with this model") -- confirmed against the live
    /// endpoint in sagesure-us before this client was ported here.
    pub async fn chat_completion(
        &self,
        messages: &[Value],
        model: &str,
        _temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<Value, OpenAIError> {
        let deployment_name = if model.trim().is_empty() {
            &self.deployment
        } else {
            model
        };
        let url = format!(
            "{}/openai/v1/responses",
            self.endpoint.trim_end_matches('/')
        );

        let mut body = json!({
            "model": deployment_name,
            "input": messages,
        });

        if let Some(max) = max_tokens {
            body["max_output_tokens"] = json!(max);
        }

        let resp = self
            .http
            .post(&url)
            .header("api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(OpenAIError::ApiError(format!("HTTP {}: {}", status, text)));
        }

        let raw: Value = resp.json().await?;

        if let Some(err) = raw.get("error").filter(|e| !e.is_null()) {
            return Err(OpenAIError::ApiError(err.to_string()));
        }

        let text = raw
            .get("output")
            .and_then(|v| v.as_array())
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("type").and_then(|t| t.as_str()) == Some("message"))
            })
            .and_then(|item| item.get("content"))
            .and_then(|v| v.as_array())
            .and_then(|contents| {
                contents
                    .iter()
                    .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("output_text"))
            })
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .ok_or_else(|| OpenAIError::ApiError(format!("no output_text in response: {raw}")))?;

        Ok(json!({
            "choices": [{ "message": { "role": "assistant", "content": text } }],
            "usage": raw.get("usage").cloned().unwrap_or(Value::Null),
        }))
    }
}
