#!/bin/sh
set -eu

# Consume the Claude-style stdin prompt, then emulate one MCP-proxied edit using
# the exact routing and actor environment inherited by the real CLI/MCP child.
cat >/dev/null
sleep 0.4
/usr/bin/curl -fsS \
  -X POST \
  -H 'content-type: application/json' \
  -H "x-cut-actor: ${CUTD_PROXY_ACTOR}" \
  --data '{"at_ms":900,"label":"Agent Chat server gate"}' \
  "http://${CUTD_PROXY_ADDR}/api/verb/edit.add_marker" >/dev/null
printf '%s\n' '{"type":"result","is_error":false,"result":"Added the server-gate marker.","total_cost_usd":0}'
