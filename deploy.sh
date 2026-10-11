#!/usr/bin/env bash
# Deploy email_service as an MCP-over-HTTP server managed by pm2.
# Run ON the VPS, from the repo root:  ./deploy.sh
# Re-running pulls the latest code and redeploys.
set -euo pipefail
cd "$(dirname "$0")"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mWARN:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# --- 1. prerequisites -------------------------------------------------------
command -v cargo >/dev/null || die "Rust toolchain not found.
  Install:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
command -v pm2 >/dev/null || die "pm2 not found.
  Install Node.js (https://nodejs.org), then:  npm install -g pm2"

# Building Rust deps needs memory; warn on tiny VPSes (or build locally + rsync).
MEM_KB=$(awk '/MemTotal/ {print $2}' /proc/meminfo 2>/dev/null || echo 0)
[ "$MEM_KB" -gt 0 ] && [ "$MEM_KB" -lt 1500000 ] \
    && warn "Less than ~1.5GB RAM — the build may OOM. Consider: build locally, rsync target/release/email_service to the VPS."

# --- 2. pull latest code (if this is a git checkout) -------------------------
if git rev-parse --is-inside-work-tree >/dev/null 2>&1 && git remote get-url origin >/dev/null 2>&1; then
    if [ -z "$(git status --porcelain -- . ':!.env' ':!senders.json')" ]; then
        info "Pulling latest code"
        git pull --ff-only || warn "git pull failed — deploying current local code"
    else
        warn "Local changes present — skipping git pull, deploying as-is"
    fi
fi

# --- 3. configuration -------------------------------------------------------
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

ADDR=$(grep '^MCP_HTTP_ADDR=' .env | cut -d= -f2)
case "$ADDR" in
    0.0.0.0:*|\[::\]:*) warn "MCP_HTTP_ADDR=$ADDR binds all interfaces — only do this behind a TLS reverse proxy (Caddy/nginx), never directly exposed." ;;
esac

# --- 4. build ---------------------------------------------------------------
# The landing page (web/index.html) is compiled into the binary — no static
# files to ship; a rebuild is all it takes to update it.
info "Building release binary"
cargo build --release --locked

# --- 5. (re)start under pm2 -------------------------------------------------
info "Starting/reloading with pm2"
pm2 startOrReload ecosystem.config.cjs
pm2 save

# --- 6. post-deploy health check --------------------------------------------
HEALTH_ADDR=${ADDR/0.0.0.0/127.0.0.1}
if command -v curl >/dev/null; then
    info "Checking http://$HEALTH_ADDR/healthz"
    ok=false
    for _ in $(seq 1 10); do
        if curl -fs "http://$HEALTH_ADDR/healthz" >/dev/null 2>&1; then ok=true; break; fi
        sleep 1
    done
    if $ok; then
        curl -fs "http://$HEALTH_ADDR/" | grep -q "email-service" \
            && info "Landing page OK — http://$HEALTH_ADDR/" \
            || warn "/healthz OK but landing page didn't respond as expected"
    else
        pm2 logs email-service --lines 30 --nostream || true
        die "Health check failed — see logs above"
    fi
fi

info "Deployed. Point your reverse proxy at $ADDR, e.g. Caddy:"
echo "    email.itskrishna.live"
echo "    reverse_proxy $ADDR"
echo "  → landing page at https://<domain>/  ·  MCP endpoint at https://<domain>/mcp"
echo
info "First time on this VPS? Also run:  pm2 startup   # survive reboots"
