//! Native Meta WhatsApp Cloud API client — ported from `sagesure-us`'s
//! `whatsapp` crate (same source, same payload shape, same phone
//! normalization: strip everything but digits, drop a leading "00").
//!
//! Native-only: unlike sesure-us, sagepas has no separate Python
//! `notifications` service to fall back to, so this is the only WhatsApp
//! send path here.

use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WhatsAppError {
    #[error("WhatsApp credentials not configured")]
    NotConfigured,
    #[error("invalid destination phone format: {0}")]
    InvalidPhone(String),
    #[error("WhatsApp request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("WhatsApp API returned {status}: {body}")]
    Api { status: u16, body: String },
}

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    pub access_token: String,
    pub phone_id: String,
}

impl WhatsAppConfig {
    pub fn is_configured(&self) -> bool {
        !self.access_token.is_empty() && !self.phone_id.is_empty()
    }
}

pub struct WhatsAppClient {
    http: reqwest::Client,
    config: WhatsAppConfig,
}

/// Meta Cloud API expects the destination in international format, digits only,
/// no leading '+' — same normalization as the Python client.
fn normalize_phone(phone: &str) -> Option<String> {
    let mut digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.starts_with("00") {
        digits = digits[2..].to_string();
    }
    if digits.is_empty() { None } else { Some(digits) }
}

impl WhatsAppClient {
    pub fn new(config: WhatsAppConfig) -> Self {
        Self { http: reqwest::Client::new(), config }
    }

    /// Sends a plain text WhatsApp message via the Meta Cloud API.
    /// Returns Ok(()) on a successful send; every failure mode is a distinct
    /// error variant so callers can log/handle without string-matching.
    pub async fn send_text(&self, phone: &str, message: &str) -> Result<(), WhatsAppError> {
        if !self.config.is_configured() {
            tracing::warn!(phone, "WhatsApp Meta credentials missing");
            return Err(WhatsAppError::NotConfigured);
        }

        let normalized = normalize_phone(phone).ok_or_else(|| WhatsAppError::InvalidPhone(phone.to_string()))?;

        let url = format!("https://graph.facebook.com/v18.0/{}/messages", self.config.phone_id);
        let payload = json!({
            "messaging_product": "whatsapp",
            "to": normalized,
            "type": "text",
            "text": { "body": message }
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.access_token)
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            tracing::info!(phone, "WhatsApp message sent");
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(200).collect();
            tracing::error!(phone, status, body = %truncated, "WhatsApp send failed");
            Err(WhatsAppError::Api { status, body: truncated })
        }
    }

    /// Sends a WhatsApp Flow message (an interactive, multi-screen form) via
    /// the Meta Cloud API. `first_screen` must match a screen `id` in the
    /// published flow's JSON — the flow opens navigating straight to it.
    /// `flow_token` identifies this specific flow session; callers that need
    /// to correlate the eventual flow-completion webhook back to a record
    /// (e.g. a claim draft) should generate and track their own token rather
    /// than relying on the random default.
    pub async fn send_flow(
        &self,
        phone: &str,
        flow_id: &str,
        first_screen: &str,
        flow_cta: &str,
        body_text: &str,
    ) -> Result<(), WhatsAppError> {
        if !self.config.is_configured() {
            tracing::warn!(phone, "WhatsApp Meta credentials missing");
            return Err(WhatsAppError::NotConfigured);
        }

        let normalized = normalize_phone(phone).ok_or_else(|| WhatsAppError::InvalidPhone(phone.to_string()))?;
        let flow_token = uuid::Uuid::new_v4().to_string();

        let url = format!("https://graph.facebook.com/v18.0/{}/messages", self.config.phone_id);
        let payload = json!({
            "messaging_product": "whatsapp",
            "to": normalized,
            "type": "interactive",
            "interactive": {
                "type": "flow",
                "body": { "text": body_text },
                "action": {
                    "name": "flow",
                    "parameters": {
                        "flow_message_version": "3",
                        "flow_token": flow_token,
                        "flow_id": flow_id,
                        "flow_cta": flow_cta,
                        "flow_action": "navigate",
                        "flow_action_payload": { "screen": first_screen }
                    }
                }
            }
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.access_token)
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            tracing::info!(phone, flow_id, flow_token, "WhatsApp flow sent");
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(200).collect();
            tracing::error!(phone, status, body = %truncated, "WhatsApp flow send failed");
            Err(WhatsAppError::Api { status, body: truncated })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_plain_digits() {
        assert_eq!(normalize_phone("+1 (415) 555-0100"), Some("14155550100".to_string()));
    }

    #[test]
    fn strips_leading_00() {
        assert_eq!(normalize_phone("0044123456789"), Some("44123456789".to_string()));
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(normalize_phone("+++"), None);
    }
}
