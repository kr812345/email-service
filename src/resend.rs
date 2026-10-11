//! Minimal Resend API client (https://resend.com/docs/api-reference).

use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

const DEFAULT_API_BASE: &str = "https://api.resend.com";

#[derive(Clone)]
pub struct ResendClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

#[derive(Debug, Serialize)]
pub struct SendEmailRequest {
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct SendEmailResponse {
    pub id: String,
}

impl ResendClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, DEFAULT_API_BASE)
    }

    /// Custom base URL — used by tests (mock server) or a self-hosted proxy.
    /// Plain HTTP is accepted only for loopback addresses (test mocks);
    /// anything else must be HTTPS.
    pub fn with_base_url(api_key: String, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let loopback_http = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
            .iter()
            .any(|p| base_url.starts_with(p));
        let http = reqwest::Client::builder()
            .user_agent(concat!("email-service/", env!("CARGO_PKG_VERSION")))
            // reqwest has NO default timeouts — without these a hung
            // connection would block a tool call forever.
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .https_only(!loopback_http)
            .build()
            .expect("failed to build HTTP client");
        Self {
            http,
            api_key,
            base_url,
        }
    }

    pub async fn send_email(&self, req: &SendEmailRequest) -> anyhow::Result<SendEmailResponse> {
        let resp = self
            .http
            .post(format!("{}/emails", self.base_url))
            .bearer_auth(&self.api_key)
            .json(req)
            .send()
            .await
            .context("failed to reach Resend API")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Resend returned {status}: {body}");
        }
        resp.json::<SendEmailResponse>()
            .await
            .context("failed to parse Resend response")
    }

    /// Fetch a previously sent email. Returns the raw JSON, which includes
    /// `from`, `to`, `subject`, and `last_event` (sent, delivered, bounced, ...).
    pub async fn get_email(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/emails/{id}", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .context("failed to reach Resend API")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Resend returned {status}: {body}");
        }
        resp.json::<serde_json::Value>()
            .await
            .context("failed to parse Resend response")
    }
}
