#![forbid(unsafe_code)]

mod http;
mod ratelimit;
mod resend;
mod senders;
mod server;
mod validation;

use anyhow::Context;
use rmcp::{ServiceExt, transport::stdio};
use server::EmailService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // stdio transport owns stdout — all logging must go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let api_key = std::env::var("RESEND_API_KEY")
        .context("RESEND_API_KEY is not set (put it in .env or in your MCP client's env config)")?;

    // Sender registry lives outside the binary — nothing is hardcoded.
    let senders_path = std::env::var("SENDERS_FILE").unwrap_or_else(|_| "senders.json".to_string());
    let senders = senders::load(std::path::Path::new(&senders_path)).context(
        "could not load senders (copy senders.example.json to senders.json and edit it)",
    )?;
    tracing::info!(count = senders.len(), path = %senders_path, "loaded senders");

    // Optional override for tests / self-hosted proxy. Plain HTTP is only
    // accepted for loopback addresses (enforced in ResendClient).
    let resend = match std::env::var("RESEND_API_BASE") {
        Ok(base) => resend::ResendClient::with_base_url(api_key, base),
        Err(_) => resend::ResendClient::new(api_key),
    };

    // HTTP mode (VPS/remote agents) when MCP_HTTP_ADDR is set; stdio (local
    // agent spawns this binary) otherwise.
    if let Ok(addr) = std::env::var("MCP_HTTP_ADDR") {
        let addr: std::net::SocketAddr = addr
            .parse()
            .context("MCP_HTTP_ADDR must be host:port, e.g. 127.0.0.1:8787")?;
        let token =
            std::env::var("MCP_AUTH_TOKEN").context("MCP_AUTH_TOKEN is required in HTTP mode")?;
        anyhow::ensure!(
            token.len() >= 16,
            "MCP_AUTH_TOKEN too short — use at least 16 chars (openssl rand -hex 32)"
        );
        http::serve(addr, EmailService::new(resend, senders), token).await?;
    } else {
        let service = EmailService::new(resend, senders).serve(stdio()).await?;
        service.waiting().await?;
    }
    Ok(())
}
