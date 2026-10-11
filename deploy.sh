#!/usr/bin/env bash
# Deploy email_service as an MCP-over-HTTP server managed by pm2.
# Run ON the VPS, from the repo root:  ./deploy.sh
set -euo pipefail
cd "$(dirname "$0")"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# --- 1. prerequisites -------------------------------------------------------
command -v cargo >/dev/null || die "Rust toolchain not found.
  Install:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
command -v pm2 >/dev/null || die "pm2 not found.
  Install Node.js (https://nodejs.org), then:  npm install -g pm2"

# --- 2. configuration -------------------------------------------------------
if [ ! -f .env ]; then
    cp .env.example .env && chmod 600 .env
    die "Created .env — fill in RESEND_API_KEY, MCP_HTTP_ADDR, MCP_AUTH_TOKEN, then re-run ./deploy.sh"
fi
if [ ! -f senders.json ]; then
    cp senders.example.json senders.json
    die "Created senders.json — add your verified sender addresses, then re-run ./deploy.sh"
fi
grep -q '^RESEND_API_KEY=re_' .env \
    || die ".env: RESEND_API_KEY missing or still a placeholder"
grep -q '^MCP_HTTP_ADDR=' .env \
    || die ".env: set MCP_HTTP_ADDR (e.g. MCP_HTTP_ADDR=127.0.0.1:8787)"
grep -q '^MCP_AUTH_TOKEN=.\{16,\}' .env \
    || die ".env: MCP_AUTH_TOKEN missing/short — generate one:  openssl rand -hex 32"

# --- 3. build ---------------------------------------------------------------
info "Building release binary"
cargo build --release --locked

# --- 4. (re)start under pm2 -------------------------------------------------
info "Starting/reloading with pm2"
pm2 startOrReload ecosystem.config.cjs
pm2 save

ADDR=$(grep '^MCP_HTTP_ADDR=' .env | cut -d= -f2)
info "Deployed. Verify with:"
echo "  pm2 status"
echo "  pm2 logs email-service"
echo "  curl http://$ADDR/healthz"
echo
info "First time on this VPS? Also run:  pm2 startup   # survive reboots"
info "Expose https://your-domain/mcp via Caddy/nginx -> $ADDR (see README#deploying-on-a-vps)"
