//! MCP service exposing the email tools.

use std::time::Duration;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_handler, tool_router,
};

use crate::{
    ratelimit::RateLimiter,
    resend::{ResendClient, SendEmailRequest},
    senders::{self, Sender},
    validation,
};

/// Guardrail against an agent loop blasting emails: max sends per minute.
const MAX_SENDS_PER_MINUTE: u32 = 20;

#[derive(Clone)]
pub struct EmailService {
    resend: ResendClient,
    senders: Vec<Sender>,
    send_limiter: RateLimiter,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendEmailParams {
    /// Who to send from: a sender id from `list_senders` (e.g. "me",
    /// "support") or a full sender email address.
    pub from: String,
    /// Recipient email addresses.
    pub to: Vec<String>,
    /// Subject line.
    pub subject: String,
    /// Plain-text body. At least one of text/html is required.
    pub text: Option<String>,
    /// HTML body. At least one of text/html is required.
    pub html: Option<String>,
    /// CC addresses.
    pub cc: Option<Vec<String>>,
    /// BCC addresses.
    pub bcc: Option<Vec<String>>,
    /// Reply-To addresses.
    pub reply_to: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetEmailStatusParams {
    /// The Resend email id returned by send_email.
    pub id: String,
}

#[tool_router]
impl EmailService {
    pub fn new(resend: ResendClient, senders: Vec<Sender>) -> Self {
        Self {
            resend,
            senders,
            send_limiter: RateLimiter::new(Duration::from_secs(60), MAX_SENDS_PER_MINUTE),
        }
    }

    #[tool(
        description = "List the pre-configured senders (verified domains) you can send \
        email from. Use a sender's `id` or `email` as the `from` argument of send_email."
    )]
    fn list_senders(&self) -> String {
        self.senders
            .iter()
            .map(|s| {
                format!(
                    "- id: {}\n  email: {}\n  sends as: {}",
                    s.id,
                    s.email,
                    s.as_from_header()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tool(
        description = "Send an email via Resend from one of the pre-configured senders. \
        Call list_senders first if unsure which senders are available."
    )]
    async fn send_email(
        &self,
        Parameters(params): Parameters<SendEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(sender) = senders::resolve(&self.senders, &params.from) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Unknown sender '{}'. Call list_senders to see available senders.",
                params.from
            ))]));
        };
        if let Err(msg) = validation::validate_send(&params) {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Validation failed: {msg}"
            ))]));
        }
        if let Err(retry_in) = self.send_limiter.check().await {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Rate limit exceeded: max {MAX_SENDS_PER_MINUTE} emails/minute, retry in {retry_in}s."
            ))]));
        }

        let req = SendEmailRequest {
            from: sender.as_from_header(),
            to: params.to,
            subject: params.subject,
            text: params.text,
            html: params.html,
            cc: params.cc,
            bcc: params.bcc,
            reply_to: params.reply_to,
        };

        match self.resend.send_email(&req).await {
            Ok(resp) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "Email sent from {}.\nResend id: {}\nUse get_email_status with this id to check delivery.",
                sender.email, resp.id
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Resend API error: {e:#}"
            ))])),
        }
    }

    #[tool(
        description = "Check a previously sent email by its Resend id. Returns JSON with \
        from/to/subject and last_event (sent, delivered, delivery_delayed, bounced, complained, ...)."
    )]
    async fn get_email_status(
        &self,
        Parameters(params): Parameters<GetEmailStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        if !validation::is_valid_email_id(&params.id) {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "Invalid email id: expected the id returned by send_email \
                 (alphanumeric, '-' and '_' only).",
            )]));
        }
        match self.resend.get_email(&params.id).await {
            Ok(v) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Resend API error: {e:#}"
            ))])),
        }
    }
}

#[tool_handler(
    name = "email-service",
    version = "0.1.0",
    instructions = "Send email via Resend from a pre-configured registry of verified \
        senders. Workflow: list_senders -> pick a sender -> send_email -> optionally get_email_status."
)]
impl ServerHandler for EmailService {}
