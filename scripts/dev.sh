#!/usr/bin/env bash
# dev.sh — build the UI bundle, then run cutd serving it (server contract).
# Usage: scripts/dev.sh [--headless] [extra cutd serve args…]
#   --headless  skip the UI build + serve API-only.
# cutd binds 127.0.0.1:6161; UI at /, verbs at POST /api/verb/{name},
# registry at GET /api/verbs, events WS at GET /api/events.
# Callers: humans + agents starting a dev loop; e2e.sh starts its own cutd
# on a separate port (E2E_ADDR, default :6166) so this server is never hit.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

command -v cargo >/dev/null || { echo "dev.sh: cargo not found — install Rust (rustup)" >&2; exit 1; }

if [[ "${1:-}" == "--headless" ]]; then
  shift
  echo "[dev] headless — API only at http://127.0.0.1:6161/ (POST /api/verb/{name})"
  exec cargo run --manifest-path "$ROOT/app/Cargo.toml" -p server -- serve --headless "$@"
fi

# Build the UI bundle first so cutd serves fresh assets (vite build is fast;
# use `npm run dev` in ui/ for HMR against a separately-running cutd).
command -v npm >/dev/null || { echo "dev.sh: npm not found — install Node (or use --headless for API-only)" >&2; exit 1; }
if [[ ! -d "$ROOT/ui/node_modules" ]]; then
  echo "[dev] ui/node_modules missing — running npm ci first"
  ( cd "$ROOT/ui" && npm ci )
fi
( cd "$ROOT/ui" && npm run build )

echo "[dev] open http://127.0.0.1:6161/ once cutd reports listening"
exec cargo run --manifest-path "$ROOT/app/Cargo.toml" -p server -- serve "$@"
