//! Optional streamable-HTTP transport for running on a VPS.
//! Enabled when MCP_HTTP_ADDR is set; stdio remains the default.

use std::net::SocketAddr;

use anyhow::Context;
use axum::{Router, response::Html, routing::get};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tower_http::validate_request::ValidateRequestHeaderLayer;

use crate::server::EmailService;

/// Landing page, embedded at compile time — the binary stays self-contained.
const LANDING_HTML: &str = include_str!("../web/index.html");

/// Serve MCP over streamable HTTP at `/mcp`, protected by a bearer token.
/// `/healthz` is intentionally unauthenticated for uptime checks.
pub async fn serve(
    addr: SocketAddr,
    service: EmailService,
    auth_token: String,
) -> anyhow::Result<()> {
    // JSON responses for simple request/response tools (SSE only when a
    // handler streams). Legacy session mode stays enabled for compatibility
    // with 2025-era MCP clients.
    let config = StreamableHttpServerConfig::default().with_json_response(true);

    // The factory runs per request/session — clones share state (rate
    // limiter, HTTP client) through the same Arc-backed handles.
    let mcp_service = StreamableHttpService::new(
        move || Ok(service.clone()),
        LocalSessionManager::default().into(),
        config,
    );

    let auth_layer = ValidateRequestHeaderLayer::has_header_value(
        "authorization",
        &format!("Bearer {auth_token}"),
    )
    .expect("`authorization` is a valid header name");

    let mcp_router = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(auth_layer);

    // Landing page + health check are public; only /mcp requires the token.
    let app = Router::new()
        .route("/", get(|| async { Html(LANDING_HTML) }))
        .route("/healthz", get(|| async { "ok" }))
        .merge(mcp_router);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    tracing::info!(%addr, "MCP streamable HTTP server listening on /mcp");
    axum::serve(listener, app)
        .await
        .context("HTTP server terminated with an error")
}
