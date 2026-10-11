//! End-to-end tests: spawn the real binary, speak MCP (NDJSON over stdio),
//! and run it against a mock Resend API.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

const TEST_KEY: &str = "re_test_key_123";
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Fictional sender registry used by every test.
const TEST_SENDERS: &str = r#"[
    {"id": "me", "email": "alice@example.com", "display_name": "Alice"},
    {"id": "support", "email": "support@example.org", "display_name": "Example Support"}
]"#;

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Kept alive so the senders file isn't deleted while the child runs.
    _senders_file: tempfile::NamedTempFile,
}

impl McpProcess {
    async fn spawn(api_base: &str) -> Self {
        let senders_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(senders_file.path(), TEST_SENDERS).unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_email_service"))
            .env("RESEND_API_KEY", TEST_KEY)
            .env("RESEND_API_BASE", api_base)
            .env("SENDERS_FILE", senders_file.path())
            // Force stdio mode even if a local .env configures HTTP.
            .env_remove("MCP_HTTP_ADDR")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("failed to spawn email_service");

        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut proc = Self {
            child,
            stdin,
            stdout,
            _senders_file: senders_file,
        };
        proc.handshake().await;
        proc
    }

    async fn send_line(&mut self, v: &Value) {
        let mut line = serde_json::to_string(v).unwrap();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn read_response(&mut self, id: i64) -> Value {
        let mut line = String::new();
        timeout(IO_TIMEOUT, self.stdout.read_line(&mut line))
            .await
            .expect("timed out waiting for server response")
            .unwrap();
        let v: Value = serde_json::from_str(line.trim()).expect("server emitted non-JSON");
        assert_eq!(v["id"], id, "response id mismatch: {v}");
        v
    }

    async fn handshake(&mut self) {
        self.send_line(&json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "e2e", "version": "0.0.0"}
            }
        }))
        .await;
        let resp = self.read_response(0).await;
        assert_eq!(resp["result"]["serverInfo"]["name"], "email-service");
        self.send_line(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
    }

    async fn call_tool(&mut self, id: i64, name: &str, args: Value) -> Value {
        self.send_line(&json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": args}
        }))
        .await;
        self.read_response(id).await
    }

    async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

/// Extract the text of a successful/failed tool result.
fn tool_text(resp: &Value) -> &str {
    resp["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result should contain text content")
}

fn send_args() -> Value {
    json!({
        "from": "me",
        "to": ["someone@example.com"],
        "subject": "e2e test",
        "text": "hello from e2e"
    })
}

#[tokio::test]
async fn tools_list_exposes_three_tools() {
    let mock = MockServer::start().await;
    let mut mcp = McpProcess::spawn(&mock.uri()).await;

    mcp.send_line(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
        .await;
    let resp = mcp.read_response(1).await;
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["get_email_status", "list_senders", "send_email"]);

    mcp.kill().await;
}

#[tokio::test]
async fn list_senders_returns_configured_registry() {
    let mock = MockServer::start().await;
    let mut mcp = McpProcess::spawn(&mock.uri()).await;

    let resp = mcp.call_tool(1, "list_senders", json!({})).await;
    assert_eq!(resp["result"]["isError"], false);
    let text = tool_text(&resp);
    assert!(
        text.contains("alice@example.com"),
        "missing sender:\n{text}"
    );
    assert!(
        text.contains("support@example.org"),
        "missing sender:\n{text}"
    );

    mcp.kill().await;
}

#[tokio::test]
async fn send_email_happy_path() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(header("authorization", format!("Bearer {TEST_KEY}")))
        .and(body_json(json!({
            "from": "Alice <alice@example.com>",
            "to": ["someone@example.com"],
            "subject": "e2e test",
            "text": "hello from e2e"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "email-id-123"})))
        .expect(1)
        .mount(&mock)
        .await;

    let mut mcp = McpProcess::spawn(&mock.uri()).await;
    let resp = mcp.call_tool(1, "send_email", send_args()).await;

    assert_eq!(resp["result"]["isError"], false);
    let text = tool_text(&resp);
    assert!(text.contains("email-id-123"), "unexpected: {text}");
    assert!(text.contains("alice@example.com"), "unexpected: {text}");

    mcp.kill().await;
    mock.verify().await;
}

#[tokio::test]
async fn send_email_rejects_invalid_input_without_calling_resend() {
    let mock = MockServer::start().await;
    let mut mcp = McpProcess::spawn(&mock.uri()).await;

    let cases = [
        // unknown sender
        json!({"from": "nobody", "to": ["a@b.com"], "subject": "s", "text": "x"}),
        // malformed recipient
        json!({"from": "me", "to": ["not-an-email"], "subject": "s", "text": "x"}),
        // header injection attempt
        json!({"from": "me", "to": ["a@b.com"], "subject": "hi\nBcc: evil@x.com", "text": "x"}),
        // no body at all
        json!({"from": "me", "to": ["a@b.com"], "subject": "s"}),
        // empty recipient list
        json!({"from": "me", "to": [], "subject": "s", "text": "x"}),
        // over the recipient cap
        json!({"from": "me", "to": (0..51).map(|i| format!("u{i}@x.com")).collect::<Vec<_>>(), "subject": "s", "text": "x"}),
    ];

    for (i, args) in cases.iter().enumerate() {
        let resp = mcp
            .call_tool(i as i64 + 1, "send_email", args.clone())
            .await;
        assert_eq!(
            resp["result"]["isError"],
            true,
            "case {i} should fail: {}",
            tool_text(&resp)
        );
    }

    mcp.kill().await;
    assert!(
        mock.received_requests().await.unwrap().is_empty(),
        "invalid calls must never reach Resend"
    );
}

#[tokio::test]
async fn send_email_surfaces_resend_errors() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "statusCode": 422, "name": "validation_error", "message": "domain not verified"
        })))
        .mount(&mock)
        .await;

    let mut mcp = McpProcess::spawn(&mock.uri()).await;
    let resp = mcp.call_tool(1, "send_email", send_args()).await;

    assert_eq!(resp["result"]["isError"], true);
    let text = tool_text(&resp);
    assert!(text.contains("422"), "should include status: {text}");
    assert!(
        text.contains("domain not verified"),
        "should include Resend message: {text}"
    );
    // The API key must never leak into error output.
    assert!(!text.contains(TEST_KEY), "API key leaked: {text}");

    mcp.kill().await;
}

#[tokio::test]
async fn rate_limiter_blocks_after_20_sends() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x"})))
        .expect(20)
        .mount(&mock)
        .await;

    let mut mcp = McpProcess::spawn(&mock.uri()).await;
    for i in 0..20 {
        let resp = mcp.call_tool(i, "send_email", send_args()).await;
        assert_eq!(resp["result"]["isError"], false, "send {i} should succeed");
    }
    let resp = mcp.call_tool(21, "send_email", send_args()).await;
    assert_eq!(resp["result"]["isError"], true);
    assert!(
        tool_text(&resp).contains("Rate limit"),
        "unexpected: {}",
        tool_text(&resp)
    );

    mcp.kill().await;
    mock.verify().await;
}

#[tokio::test]
async fn get_email_status_happy_path() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/email-id-123"))
        .and(header("authorization", format!("Bearer {TEST_KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "email", "id": "email-id-123",
            "from": "Alice <alice@example.com>",
            "to": ["someone@example.com"],
            "subject": "e2e test", "last_event": "delivered"
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let mut mcp = McpProcess::spawn(&mock.uri()).await;
    let resp = mcp
        .call_tool(1, "get_email_status", json!({"id": "email-id-123"}))
        .await;

    assert_eq!(resp["result"]["isError"], false);
    let text = tool_text(&resp);
    assert!(
        text.contains("\"last_event\": \"delivered\""),
        "unexpected: {text}"
    );

    mcp.kill().await;
    mock.verify().await;
}

#[tokio::test]
async fn get_email_status_rejects_path_injection() {
    let mock = MockServer::start().await;
    let mut mcp = McpProcess::spawn(&mock.uri()).await;

    for bad_id in ["../domains", "id/../../x", "id?key=1", "id with spaces"] {
        let resp = mcp
            .call_tool(1, "get_email_status", json!({"id": bad_id}))
            .await;
        assert_eq!(resp["result"]["isError"], true, "should reject {bad_id}");
    }

    mcp.kill().await;
    assert!(
        mock.received_requests().await.unwrap().is_empty(),
        "malicious ids must never reach Resend"
    );
}
