#!/usr/bin/env bash
# Sign a Windows artifact from WSL using Azure Trusted (Artifact) Signing + SignTool.
#
# Tauri calls this per artifact via bundle.windows.signCommand (%1 = the file).
# It lives next to src-tauri (app/desktop/scripts) so the signCommand relative
# path `../scripts/...` resolves from the Tauri project directory. All signing
# tool and Azure Artifact Signing inputs are supplied explicitly by the caller.
#
# OPT-IN: signing is OFF unless
# SHELLX_WINDOWS_SIGNING_REQUIRED=1 — so dev/iteration builds stay UNSIGNED even
# when signing inputs are configured. For a SIGNED
# release: `SHELLX_WINDOWS_SIGNING_REQUIRED=1 bash scripts/build-windows.sh release`
# (requires the caller to configure provider authentication on Windows).
set -euo pipefail

artifact="${1:-}"
if [ -z "$artifact" ]; then
  echo "windows-artifact-sign: missing artifact path" >&2
  exit 2
fi

# NSIS helper plugins are not real artifacts — never sign them (Tauri calls the
# signCommand on them too). Skip both WSL and Windows path shapes.
case "$artifact" in
  *"/nsis/"*"Plugins/"*|*'\nsis\'*'\Plugins\'*)
    echo "windows-artifact-sign: skip NSIS helper plugin"
    exit 0
    ;;
esac

# Gate 1 — signing must be explicitly requested. Default OFF (unsigned dev build).
required="${SHELLX_WINDOWS_SIGNING_REQUIRED:-0}"
if [ "$required" != "1" ]; then
  echo "windows-artifact-sign: SHELLX_WINDOWS_SIGNING_REQUIRED!=1 — unsigned build (set =1 for a signed release)"
  exit 0
fi

# From here, signing IS requested → every prerequisite is mandatory (fail loud).
metadata="${SHELLX_WINDOWS_SIGNING_METADATA_PATH:-}"
signtool="${SHELLX_WINDOWS_SIGNTOOL:-}"
dlib="${SHELLX_WINDOWS_DLIB:-}"

if [ ! -s "$metadata" ]; then
  echo "windows-artifact-sign: signing REQUIRED but metadata not found — set SHELLX_WINDOWS_SIGNING_METADATA_PATH" >&2
  exit 1
fi
if [ -z "$signtool" ]; then
  echo "windows-artifact-sign: signing REQUIRED but SHELLX_WINDOWS_SIGNTOOL is unset" >&2
  exit 1
fi
if [ -z "$dlib" ]; then
  echo "windows-artifact-sign: signing REQUIRED but SHELLX_WINDOWS_DLIB is unset" >&2
  exit 1
fi
if ! command -v powershell.exe >/dev/null; then
  echo "windows-artifact-sign: signing REQUIRED but powershell.exe unavailable (run on Windows / WSL with interop)" >&2
  exit 1
fi
if [ ! -e "$artifact" ]; then
  echo "windows-artifact-sign: artifact not found: $artifact" >&2
  exit 1
fi

# Convert WSL paths → Windows paths for SignTool, then sign + verify in one
# PowerShell call. The Azure CodeSigning Dlib and metadata file drive the
# configured Artifact Signing provider.
artifact_win="$(wslpath -w "$(realpath -m "$artifact")")"
metadata_win="$(wslpath -w "$(realpath -m "$metadata")")"
sign_wsl_env="SHELLX_SIGNTOOL:SHELLX_DLIB:SHELLX_METADATA:SHELLX_ARTIFACT"
if [ -n "${WSLENV:-}" ]; then
  sign_wsl_env="${sign_wsl_env}:${WSLENV}"
fi

WSLENV="$sign_wsl_env" \
SHELLX_SIGNTOOL="$signtool" \
SHELLX_DLIB="$dlib" \
SHELLX_METADATA="$metadata_win" \
SHELLX_ARTIFACT="$artifact_win" \
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command '
  $ErrorActionPreference = "Stop"
  $signtool = $env:SHELLX_SIGNTOOL
  $dlib = $env:SHELLX_DLIB
  $metadata = $env:SHELLX_METADATA
  $artifact = $env:SHELLX_ARTIFACT
  if (-not (Test-Path -LiteralPath $signtool)) { throw "SignTool not found: $signtool" }
  if (-not (Test-Path -LiteralPath $dlib)) { throw "Azure signing dlib not found: $dlib (install Microsoft.ArtifactSigning.Client)" }
  if (-not (Test-Path -LiteralPath $metadata)) { throw "Signing metadata not found: $metadata" }
  if (-not (Test-Path -LiteralPath $artifact)) { throw "Artifact not found: $artifact" }
  & $signtool sign /fd SHA256 /tr "http://timestamp.acs.microsoft.com" /td SHA256 /dlib $dlib /dmdf $metadata $artifact
  if ($LASTEXITCODE -ne 0) { throw "SignTool sign failed with exit $LASTEXITCODE" }
  & $signtool verify /pa /v $artifact
  if ($LASTEXITCODE -ne 0) { throw "SignTool verify failed with exit $LASTEXITCODE" }
'

echo "windows-artifact-sign: signed + verified $(basename "$artifact")"
