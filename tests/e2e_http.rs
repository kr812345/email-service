//! E2E for HTTP mode: spawn the binary with MCP_HTTP_ADDR, then drive the
//! streamable-HTTP transport with a plain HTTP client — auth included.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::{Child, Command};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const TEST_KEY: &str = "re_test_key_123";
const AUTH_TOKEN: &str = "test-token-0123456789abcdef";
const TEST_SENDERS: &str =
    r#"[{"id": "me", "email": "alice@example.com", "display_name": "Alice"}]"#;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct HttpServer {
    child: Child,
    base: String,
    _senders_file: tempfile::NamedTempFile,
}

impl HttpServer {
    async fn spawn(mock_uri: &str) -> Self {
        let port = free_port();
        let senders_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(senders_file.path(), TEST_SENDERS).unwrap();

        let child = Command::new(env!("CARGO_BIN_EXE_email_service"))
            .env("RESEND_API_KEY", TEST_KEY)
            .env("RESEND_API_BASE", mock_uri)
            .env("SENDERS_FILE", senders_file.path())
            .env("MCP_HTTP_ADDR", format!("127.0.0.1:{port}"))
            .env("MCP_AUTH_TOKEN", AUTH_TOKEN)
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("failed to spawn email_service");

        let server = Self {
            child,
            base: format!("http://127.0.0.1:{port}"),
            _senders_file: senders_file,
        };

        // Wait until the listener is up.
        let client = reqwest::Client::new();
        for _ in 0..50 {
            if let Ok(r) = client.get(format!("{}/healthz", server.base)).send().await
                && r.status().is_success()
            {
                return server;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("server did not become ready");
    }

    async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

/// Parse a streamable-HTTP response body: plain JSON, or the first SSE `data:` payload.
async fn parse_mcp_response(resp: reqwest::Response) -> Value {
    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));
    let body = resp.text().await.unwrap();
    if is_sse {
        // The stream may open with priming events carrying empty data lines —
        // take the first non-empty payload.
        let data = body
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(str::trim)
            .find(|d| !d.is_empty())
            .expect("SSE body without JSON payload");
        serde_json::from_str(data).unwrap()
    } else {
        serde_json::from_str(&body).unwrap()
    }
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "e2e-http", "version": "0.0.0"}
        }
    })
}

#[tokio::test]
async fn http_mode_full_flow_with_auth() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "http-e2e-id"})))
        .expect(1)
        .mount(&mock)
        .await;

    let mut server = HttpServer::spawn(&mock.uri()).await;
    let client = reqwest::Client::new();
    let mcp = format!("{}/mcp", server.base);
    let accept = "application/json, text/event-stream";

    // healthz is public
    let r = client
        .get(format!("{}/healthz", server.base))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());

    // no auth -> rejected (tower-http answers 403 for a failed header check)
    let r = client
        .post(&mcp)
        .header("accept", accept)
        .json(&initialize_request())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "unauthenticated request must be rejected");

    // wrong token -> rejected
    let r = client
        .post(&mcp)
        .header("accept", accept)
        .bearer_auth("wrong-token")
        .json(&initialize_request())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "wrong token must be rejected");

    // initialize with auth
    let r = client
        .post(&mcp)
        .header("accept", accept)
        .bearer_auth(AUTH_TOKEN)
        .json(&initialize_request())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let session = r
        .headers()
        .get("mcp-session-id")
        .expect("legacy initialize should issue a session id")
        .to_str()
        .unwrap()
        .to_string();
    let body = parse_mcp_response(r).await;
    assert_eq!(body["result"]["serverInfo"]["name"], "email-service");

    let session_headers = |c: reqwest::RequestBuilder| {
        c.header("accept", accept)
            .bearer_auth(AUTH_TOKEN)
            .header("mcp-session-id", &session)
            .header("mcp-protocol-version", "2025-11-25")
    };

    // notifications/initialized
    let r = session_headers(client.post(&mcp))
        .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());

    // tools/call send_email -> hits the mock Resend exactly once
    let r = session_headers(client.post(&mcp))
        .json(&json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {
                "name": "send_email",
                "arguments": {
                    "from": "me",
                    "to": ["someone@example.com"],
                    "subject": "http e2e",
                    "text": "via http transport"
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body = parse_mcp_response(r).await;
    let text = body["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("http-e2e-id"),
        "unexpected tool result: {text}"
    );

    server.kill().await;
    mock.verify().await;
}

#[tokio::test]
async fn http_mode_requires_token() {
    let mock = MockServer::start().await;
    let port = free_port();
    let senders_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(senders_file.path(), TEST_SENDERS).unwrap();

    // MCP_HTTP_ADDR without MCP_AUTH_TOKEN must fail fast, not run open.
    let output = Command::new(env!("CARGO_BIN_EXE_email_service"))
        .env("RESEND_API_KEY", TEST_KEY)
        .env("RESEND_API_BASE", mock.uri())
        .env("SENDERS_FILE", senders_file.path())
        .env("MCP_HTTP_ADDR", format!("127.0.0.1:{port}"))
        .env_remove("MCP_AUTH_TOKEN")
        // Run outside the project dir so dotenvy can't reload a token from .env.
        .current_dir(std::env::temp_dir())
        .output()
        .await
        .expect("failed to spawn email_service");

    assert!(
        !output.status.success(),
        "server must refuse HTTP mode without MCP_AUTH_TOKEN"
    );
}
