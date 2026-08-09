#!/usr/bin/env bash
# verbargs-sync.sh — assert the UI's VerbArgs map (ui/src/lib/client.ts) types
# EVERY verb in the schema (schema/verbs.json). The drawers call callVerb<'verb'>,
# which is keyed on `keyof VerbArgs`; a verb missing from VerbArgs fails the TS
# build. A missing entry means a UI caller can compile against a stale contract.
#
# Run this inexpensive guard in CI and before builds so the
# schema and the typed client never drift again. Exit 0 = in sync.
set -uo pipefail
cd "$(dirname "$0")/.."

schema_verbs=$(jq -r '.verbs[].name' schema/verbs.json | sort -u)
# Verb keys are string-literal property keys with two or more dotted segments.
# The repeated group matters for connector lifecycle verbs such as
# `motion.link.refresh`; matching only `domain.verb` falsely reports them absent.
client_verbs=$(grep -oE "'[a-z_]+(\.[a-z_]+)+':" ui/src/lib/client.ts | tr -d "':" | sort -u)

missing=$(comm -23 <(echo "$schema_verbs") <(echo "$client_verbs"))
if [ -n "$missing" ]; then
  echo "FAIL: schema verbs missing from ui/src/lib/client.ts VerbArgs:" >&2
  echo "$missing" | sed 's/^/  - /' >&2
  echo "Add each as a typed entry so callVerb('<verb>', …) type-checks." >&2
  exit 1
fi

n=$(echo "$schema_verbs" | grep -c .)
echo "OK: VerbArgs covers all $n schema verbs"
