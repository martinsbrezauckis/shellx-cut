#!/usr/bin/env bash

coverage_audit_request() {
  local addr="${1:?audit address is required}"
  local verb="${2:?coverage verb is required}"
  local payload="${3:?coverage payload is required}"
  local timeout_seconds="${4:?coverage timeout is required}"
  curl -sS --max-time "$timeout_seconds" -H 'content-type: application/json' \
    -X POST "http://$addr/api/verb/$verb" -d "$payload" -w $'\n%{http_code}'
}

# The structural audit sends deliberately invalid JSON. Any accepted handler or
# non-validation envelope is evidence that registry validation changed and is
# therefore a failing coverage result, not a successful route check.
coverage_audit_response_verdict() {
  local body="${1:?coverage response is required}"
  if printf '%s' "$body" | jq -e '.ok == false and (.error | type == "object") and .error.code == "invalid_args"' >/dev/null 2>&1; then
    printf 'validation\n'
  elif printf '%s' "$body" | jq -e '.ok == true' >/dev/null 2>&1; then
    printf 'accepted\n'
  else
    printf 'unexpected\n'
  fi
}
