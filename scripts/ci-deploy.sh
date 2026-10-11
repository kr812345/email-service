#!/usr/bin/env bash
# CI deploy step, runs ON the VPS (invoked over SSH by GitHub Actions).
# Expects the pre-built binary at target/release/email_service.new.
# Swaps it in, reloads pm2, health-checks, rolls back on failure.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/release/email_service

[ -f "$BIN.new" ] || { echo "ERROR: $BIN.new was not uploaded"; exit 1; }
[ -f .env ] || { echo "ERROR: .env missing on server"; exit 1; }

ADDR=$(grep '^MCP_HTTP_ADDR=' .env | cut -d= -f2 || true)
[ -n "$ADDR" ] || { echo "ERROR: MCP_HTTP_ADDR not set in .env"; exit 1; }
HEALTH="http://${ADDR/0.0.0.0/127.0.0.1}/healthz"

[ -f "$BIN" ] && cp "$BIN" "$BIN.bak"
mv "$BIN.new" "$BIN"

pm2 startOrReload ecosystem.config.cjs >/dev/null
pm2 save >/dev/null

sleep 2
if curl -fs "$HEALTH" >/dev/null 2>&1; then
    echo "Deploy OK — $HEALTH responds"
    exit 0
fi

echo "ERROR: health check failed at $HEALTH"
if [ -f "$BIN.bak" ]; then
    echo "Rolling back to previous binary"
    mv "$BIN.bak" "$BIN"
    pm2 reload ecosystem.config.cjs >/dev/null
fi
pm2 logs email-service --lines 30 --nostream || true
exit 1
