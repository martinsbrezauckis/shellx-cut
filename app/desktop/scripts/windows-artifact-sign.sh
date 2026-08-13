#!/usr/bin/env bash
# Public-source Tauri signing hook.
#
# Contributor and CI packages are deliberately unsigned. Official release
# signing is performed by maintainer-owned packaging infrastructure outside
# this repository. Keeping this hook makes the checked-in Tauri configuration
# portable without exposing credential-loading or provider configuration.
set -euo pipefail

artifact="${1:-}"
if [ -z "$artifact" ]; then
  echo "windows-artifact-sign: missing artifact path" >&2
  exit 2
fi

case "$artifact" in
  *"/nsis/"*"Plugins/"*|*'\nsis\'*'\Plugins\'*)
    exit 0
    ;;
esac

record_signed_artifact() {
  local event_log="${SHELLX_WINDOWS_ARTIFACT_SIGNING_EVENT_LOG:-}"
  [ -n "$event_log" ] || return 0
  [ -f "$event_log" ] || {
    echo "windows-artifact-sign: signing event log does not exist: $event_log" >&2
    return 1
  }
  node - "$event_log" "$artifact" <<'NODE'
const { appendFileSync, readFileSync } = require('node:fs')
const { createHash } = require('node:crypto')
const { basename, resolve } = require('node:path')

const [eventLog, artifact] = process.argv.slice(2)
const bytes = readFileSync(artifact)
appendFileSync(eventLog, `${JSON.stringify({
  artifactPath: resolve(artifact),
  name: basename(artifact),
  sha256: createHash('sha256').update(bytes).digest('hex'),
  signatureStatus: 'Valid',
})}\n`, { encoding: 'utf8' })
NODE
}

if [ "${SHELLX_WINDOWS_SIGNING_REQUIRED:-0}" = "1" ]; then
  helper="${SHELLX_WINDOWS_SIGNING_HELPER:-}"
  if [ -z "$helper" ] || [ ! -x "$helper" ]; then
    echo "windows-artifact-sign: official signing requires an executable SHELLX_WINDOWS_SIGNING_HELPER" >&2
    exit 1
  fi
  if [ "$(realpath -m "$helper")" = "$(realpath -m "$0")" ]; then
    echo "windows-artifact-sign: refusing a recursive signing helper" >&2
    exit 1
  fi
  "$helper" "$artifact"
  record_signed_artifact
  exit 0
fi

if [ ! -e "$artifact" ]; then
  echo "windows-artifact-sign: artifact not found: $artifact" >&2
  exit 1
fi

echo "windows-artifact-sign: unsigned public-source build"
