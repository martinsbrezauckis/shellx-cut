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
  exec "$helper" "$artifact"
fi

if [ ! -e "$artifact" ]; then
  echo "windows-artifact-sign: artifact not found: $artifact" >&2
  exit 1
fi

echo "windows-artifact-sign: unsigned public-source build"
