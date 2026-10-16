# email_service — MCP & REST Email Server for Resend

A fast, secure [Model Context Protocol](https://modelcontextprotocol.io) (MCP) and HTTP REST email server, written in Rust, that lets AI agents and backend services send emails through [Resend](https://resend.com). Works with Cursor, Claude Desktop, any MCP-compatible client, or a plain HTTP `POST /send` request.

- **Two ways to use it** — MCP (`stdio` or streamable HTTP at `/mcp`) for AI agents, or direct REST (`POST /send`, `GET /senders`) for backend apps and scripts
- **Three MCP tools, zero bloat** — `list_senders`, `send_email`, `get_email_status`
- **Configurable sender registry** — your verified addresses live in a JSON file, never in code; pick one by `id` or email address
- **Secure by default** — sender allowlist, strict input validation, header-injection and path-injection guards, rate limiting, HTTPS-only with timeouts
- **Single static binary** — ~8 MB, no runtime dependencies, starts in milliseconds

## Paste this to your AI agent

The fastest way to start: copy one of these blocks into your AI agent (Cursor, Claude, …) — it does the setup for you.

### Option 1 — Hosted MCP (free tier: 5 emails/month)

```text
Add this MCP server to my client config:

{
  "mcpServers": {
    "email": {
      "url": "https://email.itskrishna.live/mcp",
      "headers": { "Authorization": "Bearer YOUR_TOKEN" }
    }
  }
}

Then send emails for me when I ask.
```

No token yet? [Request one](https://github.com/kr812345/email-service/issues) — free tier, no card.

### Option 2 — Direct HTTP POST (no MCP client required)

Any backend service, webhook, or script can send emails with a single `POST /send` request:

```bash
curl -X POST https://email.itskrishna.live/send \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "from": "founder-upcheck",
    "to": "user@example.com",
    "subject": "Welcome to Upcheck",
    "text": "You are on the early access list!"
  }'
```

### Option 3 — Self-hosted (free forever)

```text
Set up this open-source MCP email server for me:

  git clone https://github.com/kr812345/email-service
  cd email-service
  cp senders.example.json senders.json   # add my sender addresses
  cargo build --release

Register it as an MCP server (stdio) with my Resend API key
(RESEND_API_KEY in .env). Guide me through it.
```

Your keys, your domains, your box.

## How it works

```
AI agent (MCP over stdio / HTTP /mcp)     Backend app / script (POST /send)
                   \                       /
                    ▼                     ▼
                 email_service  ── validates input, checks sender allowlist, rate-limits
                        │  HTTPS (rustls)
                        ▼
                     Resend API  ── delivers the email, tracks status
```

The service is completely stateless — no database. Send history and delivery analytics live in your Resend dashboard.

## Manual setup

**Prerequisites:** Rust ≥ 1.85, and a [Resend API key](https://resend.com/api-keys) with at least one [verified domain](https://resend.com/domains).

```bash
git clone https://github.com/kr812345/email-service && cd email-service
cp senders.example.json senders.json   # then edit: your verified addresses
cargo build --release
```

Register the server with your MCP client:

**Cursor** (`~/.cursor/mcp.json`) or **Claude Desktop** (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "email": {
      "command": "/absolute/path/to/email_service/target/release/email_service",
      "env": {
        "RESEND_API_KEY": "re_xxxxxxxxxxxx",
        "SENDERS_FILE": "/absolute/path/to/email_service/senders.json"
      }
    }
  }
}
```

Restart the client, then ask your agent: *"Send an email to someone@example.com saying the deployment is done."*

## Configuration

### Environment variables

| Variable          | Required  | Default          | Description                                                             |
| ----------------- | --------- | ---------------- | ----------------------------------------------------------------------- |
| `RESEND_API_KEY`  | yes       | —                | API key from resend.com                                                 |
| `SENDERS_FILE`    | no        | `./senders.json` | Path to the sender registry                                             |
| `MCP_HTTP_ADDR`   | HTTP mode | —                | Bind address, e.g. `127.0.0.1:8787` — serves `/mcp`, `/send`, `/senders`|
| `MCP_AUTH_TOKEN`  | HTTP mode | —                | Bearer token clients must send (min 16 chars; `openssl rand -hex 32`)   |
| `RESEND_API_BASE` | no        | Resend API       | Override for testing/self-hosted proxy (HTTPS only\*)                   |

\* Plain HTTP is accepted only for loopback addresses (test mocks).

A `.env` file in the working directory is loaded automatically (see `.env.example`).

### Sender registry (`senders.json`)

```json
[
  { "id": "me",      "email": "you@yourdomain.com",     "display_name": "Your Name" },
  { "id": "support", "email": "support@yourdomain.com", "display_name": "Your App Support" }
]
```

- `id` — short handle used as `from` (lowercase, digits, `-`; 2–40 chars)
- `email` — must belong to a **verified domain** in your Resend account
- `display_name` — appears as `Display Name <email>` in the From header

The file is validated at startup (unique ids/emails, valid addresses); the server refuses to start on a bad config.

## Direct HTTP REST API

When running in HTTP mode (`MCP_HTTP_ADDR` set), `email_service` exposes direct REST endpoints alongside `/mcp` so backend services can send emails without speaking MCP. All REST endpoints require `Authorization: Bearer <MCP_AUTH_TOKEN>`.

### `POST /send`

Sends an email using the same sender allowlist, validation rules, and rate limiter as MCP.

**Request body (`application/json`):**

```json
{
  "from": "founder-upcheck",
  "to": "user@example.com",
  "subject": "You're on the early access list",
  "text": "Plain-text fallback body",
  "html": "<p>HTML body</p>",
  "reply_to": ["founder@upcheck.in.net"]
}
```

- `from` accepts either the sender `id` (e.g. `"founder-upcheck"`) or the full verified email address (e.g. `"founder@upcheck.in.net"`).
- `to` accepts either a single email string (`"user@example.com"`) or an array of strings (`["user@example.com"]`).

**Success response (`200 OK`):**

```json
{
  "ok": true,
  "id": "49a3999c-0ce1-4ea6-ab68-afcd6dc2e794",
  "from": "founder@upcheck.in.net"
}
```

**Error response (`400` / `429` / `502`):**

```json
{
  "ok": false,
  "error": "Validation failed: ..."
}
```

### `GET /senders`

Returns the configured sender registry as JSON:

```json
{
  "ok": true,
  "senders": [
    {
      "id": "me",
      "email": "you@yourdomain.com",
      "display_name": "Your Name"
    }
  ]
}
```

## MCP Tools

### `list_senders`

Lists the configured senders. Takes no arguments.

### `send_email`

Sends an email via Resend.

| Parameter  | Type                 | Required | Notes                                            |
| ---------- | -------------------- | -------- | ------------------------------------------------ |
| `from`     | string               | yes      | Sender `id` or full email from the registry      |
| `to`       | string \| string[]   | yes      | Bare address(es) (`user@example.com`)            |
| `subject`  | string               | yes      | Single line, ≤ 998 chars                         |
| `text`     | string               | one of   | Plain-text body (≤ 1 MB)                         |
| `html`     | string               | one of   | HTML body (≤ 1 MB)                               |
| `cc`       | string[]             | no       |                                                  |
| `bcc`      | string[]             | no       |                                                  |
| `reply_to` | string[]             | no       |                                                  |

Limits: ≤ 50 total recipients (Resend's cap), ≤ 20 emails/minute (built-in guardrail).

Returns the Resend email id on success.

### `get_email_status`

| Parameter | Type   | Required | Notes                          |
| --------- | ------ | -------- | ------------------------------ |
| `id`      | string | yes      | Email id from `send_email`     |

Returns Resend's JSON for the email, including `last_event` (`sent`, `delivered`, `delivery_delayed`, `bounced`, `complained`, …).

## Deploying on a VPS

Stdio mode only works when the agent and the server are on the same machine. To let remote agents or backend apps use the server, run it in **HTTP mode** behind a process manager:

```bash
# on the VPS, from the repo root
./deploy.sh
```

The script checks prerequisites (Rust, pm2), validates `.env` and `senders.json`, builds the release binary, and starts it under [pm2](https://pm2.keymetrics.io) (auto-restart, logs, boot persistence via `pm2 startup`).

Required in `.env` for HTTP mode:

```bash
MCP_HTTP_ADDR=127.0.0.1:8787
MCP_AUTH_TOKEN=<openssl rand -hex 32>
```

The server then serves:
- `/mcp` — MCP over [streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports) (bearer-token protected)
- `POST /send` & `GET /senders` — direct JSON REST endpoints (bearer-token protected)
- `/healthz` — public health check (`200 OK`)

**TLS:** never expose plain HTTP beyond localhost. Put a reverse proxy in front — with Caddy it's two lines (automatic HTTPS):

```
mcp.yourdomain.com
reverse_proxy 127.0.0.1:8787
```

Then point your MCP client or backend service at the remote server:

```json
{
  "mcpServers": {
    "email": {
      "url": "https://mcp.yourdomain.com/mcp",
      "headers": { "Authorization": "Bearer <MCP_AUTH_TOKEN>" }
    }
  }
}
```

## Security

- **Sender allowlist** — callers can only send from addresses in `senders.json`
- **Input validation** — addresses, subject, bodies, and ids are validated before any network call; invalid requests never reach Resend
- **Injection guards** — CR/LF rejected in subjects; status ids restricted to a URL-safe charset
- **Rate limiting** — 20 sends/minute shared across MCP and REST, so a runaway loop can't drain your quota
- **HTTP mode auth** — bearer token required on `/mcp`, `/send`, and `/senders`; missing/short token refuses to start
- **Network hardening** — HTTPS-only Resend client (rustls), 10s connect / 30s request timeouts
- **Secret hygiene** — the API key comes from the environment, never appears in logs or error output, and `.env`/`senders.json` are gitignored
- `#![forbid(unsafe_code)]`, no database, no shell-outs

## Development

```bash
cargo test                                # unit + end-to-end tests (mock Resend, no real emails)
cargo clippy --all-targets -- -D warnings # lint
cargo fmt                                 # format
```

The e2e suite spawns the real binary, tests both stdio MCP and HTTP (`/mcp`, `POST /send`, `GET /senders`), and asserts against a mock Resend API — including that invalid or malicious calls never touch the network.

## FAQ

**What is an MCP server?**
A process that exposes tools to AI agents via the Model Context Protocol. MCP clients (Cursor, Claude Desktop, …) spawn it or connect over HTTP and let the agent call its tools.

**Do I have to use MCP to send emails?**
No. In HTTP mode, you can send a regular `POST /send` JSON request with `Authorization: Bearer <MCP_AUTH_TOKEN>` from any backend or script.

**Which email providers are supported?**
Only [Resend](https://resend.com), deliberately — one well-tested integration over many shallow ones.

**Do I need a database?**
No. The service is stateless; delivery status is fetched live from Resend.

**Can I send from any address?**
Only addresses you put in `senders.json`, and the domain must be verified in Resend.

**How do I add a new sender?**
Add an entry to `senders.json` and restart the server.

**Does it support attachments / templates / scheduled sends?**
Not yet — see the issue tracker before opening a PR.

## Contributing

Issues and PRs welcome. Run `cargo test` and `cargo clippy --all-targets -- -D warnings` before submitting; CI enforces both plus `cargo fmt --check`.

## License

[MIT](LICENSE)
