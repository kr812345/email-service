//! Optional HTTP transport for running on a VPS.
//! Enabled when MCP_HTTP_ADDR is set; stdio remains the default.
//! Exposes both:
//! - `/mcp`: MCP over streamable HTTP
//! - `POST /send` and `GET /senders`: direct REST endpoints for backend services

use std::net::SocketAddr;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::json;
use tower_http::validate_request::ValidateRequestHeaderLayer;

use crate::server::{EmailService, SendEmailParams, SendFailure};

/// Landing page, embedded at compile time — the binary stays self-contained.
const LANDING_HTML: &str = include_str!("../web/index.html");

/// Direct REST `POST /send` handler.
async fn handle_rest_send(
    State(service): State<EmailService>,
    Json(params): Json<SendEmailParams>,
) -> impl IntoResponse {
    match service.send_direct(params).await {
        Ok(outcome) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "id": outcome.id,
                "from": outcome.from,
            })),
        ),
        Err(err) => {
            let status = match &err {
                SendFailure::UnknownSender(_) | SendFailure::Validation(_) => {
                    StatusCode::BAD_REQUEST
                }
                SendFailure::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
                SendFailure::Upstream(_) => StatusCode::BAD_GATEWAY,
            };
            (
                status,
                Json(json!({
                    "ok": false,
                    "error": err.message(),
                })),
            )
        }
    }
}

/// Direct REST `GET /senders` handler.
async fn handle_rest_senders(State(service): State<EmailService>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "senders": service.senders(),
        })),
    )
}

/// Serve MCP over streamable HTTP at `/mcp` and direct REST at `POST /send` + `GET /senders`,
/// protected by a bearer token. `/healthz` is intentionally unauthenticated for uptime checks.
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
    let mcp_service_clone = service.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(mcp_service_clone.clone()),
        LocalSessionManager::default().into(),
        config,
    );

    let auth_layer = ValidateRequestHeaderLayer::has_header_value(
        "authorization",
        &format!("Bearer {auth_token}"),
    )
    .expect("`authorization` is a valid header name");

    let authed_router = Router::new()
        .nest_service("/mcp", mcp_service)
        .route("/send", post(handle_rest_send))
        .route("/senders", get(handle_rest_senders))
        .with_state(service)
        .layer(auth_layer);

    // Landing page + health check are public; /mcp, /send, and /senders require the token.
    let app = Router::new()
        .route("/", get(|| async { Html(LANDING_HTML) }))
        .route("/healthz", get(|| async { "ok" }))
        .merge(authed_router);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    tracing::info!(%addr, "HTTP server listening on /mcp and POST /send");
    axum::serve(listener, app)
        .await
        .context("HTTP server terminated with an error")
}
