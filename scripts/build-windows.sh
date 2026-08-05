#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# build-windows.sh — cross-compile the ShellX Cut Windows installer FROM WSL.
#
# Pipeline (cargo-xwin, engine-first):
#   1. build ui/dist (Vite)                — bundled as a Tauri resource
#   2. cargo-xwin build cutd.exe (msvc)    — the ENGINE, byte-identical to the
#                                            headless `cutd serve` binary
#   3. stage cutd.exe as Tauri externalBin — app/desktop/src-tauri/binaries/
#   4. cargo tauri build --runner cargo-xwin → NSIS setup exe
#
# Stranger-ready packaging: the perception sidecar SCRIPT (instruments.py +
# requirements.txt) is staged into the installer as a Tauri resource mapped to
# `perception/` beside the exe (tauri.conf.json bundle.resources), so the cold
# Windows install always finds the script — only the heavy venv + ffmpeg are
# fetched on first run/use (see docs/public/BUILDING.md). This script
# asserts that payload exists in the source tree before building so a missing
# instruments.py can never silently ship.
# Produces:
#   app/desktop/src-tauri/target/x86_64-pc-windows-msvc/<mode>/shellx-cut.exe
#   app/desktop/src-tauri/target/x86_64-pc-windows-msvc/<mode>/bundle/nsis/
#     ShellX Cut_<version>_x64-setup.exe
#
# PREREQUISITES for the supported WSL cross-build path:
#   cargo-xwin, `rustup target add x86_64-pc-windows-msvc`, makensis,
#   cargo-tauri (tauri-cli 2.11.2), node/npm (UI build).
#
# USAGE:  scripts/build-windows.sh [debug|release]   (default: release)
#
# VERIFY AFTER BUILD:
# cargo-xwin can print "Finished" from the host phase while the Windows LINK
# silently fails and the bundler reships a STALE exe. This script asserts every
# produced artifact's mtime is fresh and prints sizes + hashes, so a silent
# stale-binary build can never pass unnoticed.
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root

MODE="${1:-release}"
TARGET="x86_64-pc-windows-msvc"
case "$MODE" in
  debug)   TAURI_FLAG=(--debug); CARGO_FLAG=() ;;
  release) TAURI_FLAG=();        CARGO_FLAG=(--release) ;;
  *) echo "usage: $0 [debug|release]" >&2; exit 2 ;;
esac

FEATURES_STR="${TAURI_FEATURES:-}"
if printf '%s\n' "$FEATURES_STR" | tr ', ' '\n\n' | grep -qx 'webdriver-test'; then
  echo "FAIL: webdriver-test feature is test-only and must not be enabled for shipping Windows builds" >&2
  exit 1
fi

# Tauri can discover portable NSIS plugins through NSIS_PATH, but the Linux
# makensis binary still loads its built-in stubs/includes from the prefix it was
# compiled with (normally /usr/share/nsis). Prove that complete toolset before
# spending minutes cross-compiling Rust. A relocatable Debian payload commonly
# leaves makensis executable while its compiled prefix is absent.
if ! command -v makensis >/dev/null 2>&1; then
  echo "FAIL: makensis is required for the Windows NSIS installer" >&2
  exit 1
fi
if ! nsis_probe=$(makensis -HDRINFO 2>&1); then
  printf '%s\n' "$nsis_probe" >&2
  makensis_real=$(readlink -f "$(command -v makensis)")
  portable_nsis_root="$(dirname "$(dirname "$makensis_real")")/share/nsis"
  echo "FAIL: makensis cannot load its NSIS stubs/includes; fix the compiled toolset prefix before building" >&2
  if [ -d "$portable_nsis_root/Stubs" ] && [ ! -e /usr/share/nsis ]; then
    echo "Portable NSIS detected at $portable_nsis_root" >&2
    echo "Operator command: sudo ln -s '$portable_nsis_root' /usr/share/nsis" >&2
  fi
  exit 1
fi

started=$(date +%s)
agent_doc_paths=$(node scripts/lib/agent-docs.mjs --paths)
agent_doc_count=$(printf '%s\n' "$agent_doc_paths" | wc -l | tr -d ' ')
while IFS= read -r rel; do
  [ -f "$rel" ] || { echo "FAIL: bundled agent doc missing from source: $rel" >&2; exit 1; }
done <<<"$agent_doc_paths"
echo "[build-windows] agent-doc source manifest present ($agent_doc_count files)"
bundle_dir="app/desktop/src-tauri/target/$TARGET/$MODE/bundle/nsis"
if [ -d "$bundle_dir" ]; then
  echo "[build-windows] cleaning previous ShellX Cut package files from $bundle_dir"
  find "$bundle_dir" -maxdepth 1 -type f \
    \( -name 'ShellX Cut_*.exe' -o -name 'ShellX Cut_*.exe.sig' -o -name 'ShellX Cut_*.msi' -o -name 'ShellX Cut_*.msi.sig' \) \
    -print -delete
fi

# ── 1. UI bundle (gitignored — always rebuild so the installer ships current UI)
echo "[build-windows] building ui/dist"
( cd ui
  [ -d node_modules ] || npm install --no-fund --no-audit
  npm run build >/dev/null
)
[ -f ui/dist/index.html ] || { echo "FAIL: ui/dist/index.html missing after build" >&2; exit 1; }
fallback_dir="app/desktop/fallback"
rm -rf "$fallback_dir/assets"
[ -f "$fallback_dir/index.html" ] || { echo "FAIL: $fallback_dir/index.html missing" >&2; exit 1; }
grep -q "engine_status" "$fallback_dir/index.html" || { echo "FAIL: $fallback_dir/index.html must remain the desktop engine-status airlock" >&2; exit 1; }

# ── 1b. Assert the perception sidecar payload exists before bundling it.
#       tauri.conf.json maps these into the installer as `perception/` beside
#       the exe; a missing instruments.py would silently disable transcription
#       on the cold install with no warning, so we fail loud here instead.
for f in app/perception/py/instruments.py app/perception/py/requirements.txt \
         app/perception/py/requirements-full.txt \
         app/perception/py/safe_numbers.py \
         app/perception/py/blaze_face_short_range.tflite \
         app/perception/py/matte_runner.py app/perception/py/matanyone_runner.py \
         app/perception/py/siglip_index.py \
         app/perception/py/track_runner.py app/perception/py/ocr_runner.py \
         app/perception/py/translate_runner.py \
         app/perception/py/dub_runner.py app/perception/py/diarize_runner.py \
         app/perception/py/face_runner.py app/perception/py/face_detection_yunet_2023mar.onnx; do
  [ -f "$f" ] || { echo "FAIL: sidecar payload missing: $f (resources in tauri.conf.json)" >&2; exit 1; }
done
echo "[build-windows] sidecar payload present (instruments.py + requirements.txt + face/yunet model + matte/track/ocr/face/translate/dub/diarize runners)"

# ── 2. Engine: cross-compile cutd for Windows (the engine workspace, untouched)
echo "[build-windows] cargo-xwin $MODE cutd → $TARGET  (started $(date +%H:%M:%S))"
# (workspace package name is `server`, binary name `cutd` — see app/server/Cargo.toml)
# crt-static: statically link the MSVC C runtime into cutd.exe so it runs on a
# CLEAN Windows install with NO Visual C++ Redistributable. Without it cutd.exe imports
# VCRUNTIME140.dll (a redist DLL, absent on fresh VMs) and dies at launch with "VCRUNTIME140.dll
# was not found" → the app shows "engine unavailable" on a bare Windows install.
# SCOPED TO cutd ONLY via RUSTFLAGS on THIS command: the Tauri app exe already needs only the
# OS-provided UCRT (no VCRUNTIME140), AND its C++ webview deps fail to link under crt-static
# (lld-link: undefined calloc/malloc) — so the app build (step 4) stays dynamic. A repo-wide
# .cargo/config.toml would wrongly hit the app build, so we do NOT use one.
# Verify: objdump -p cutd.exe shows no VCRUNTIME140 / api-ms-win-crt imports.
cutd_log=$( cd app && RUSTFLAGS="-C target-feature=+crt-static" cargo xwin build "${CARGO_FLAG[@]}" -p server --bin cutd --target "$TARGET" 2>&1 ) \
  || { echo "$cutd_log"; echo "FAIL: cargo xwin build (cutd) failed" >&2; exit 1; }
echo "$cutd_log"
cutd_exe="app/target/$TARGET/$MODE/cutd.exe"
[ -f "$cutd_exe" ] || { echo "FAIL: $cutd_exe was not produced" >&2; exit 1; }
# Freshness guard ONLY when cargo actually COMPILED the engine. A UI-only change
# leaves the engine unchanged → cargo skips it (cached) → the exe legitimately
# predates the build start; that is NOT a silent link failure. When cargo DID
# compile, the exe must be fresh (catches the cargo-xwin "Finished from the host
# phase but the MSVC link silently failed → stale exe" case the guard exists for).
# The installer guard below is the real backstop — it must always be freshly bundled.
if echo "$cutd_log" | grep -q "Compiling "; then
  mtime=$(stat -c %Y "$cutd_exe")
  [ "$mtime" -ge "$started" ] || { echo "FAIL: $cutd_exe is STALE (cargo compiled but the exe predates build start) — Windows link likely failed silently" >&2; exit 1; }
  echo "[verify] cutd.exe rebuilt + fresh:"
else
  echo "[verify] cutd.exe unchanged (engine cached — UI-only build); using the existing valid binary:"
fi
ls -lh "$cutd_exe"; sha256sum "$cutd_exe"

# ── 3. Stage the engine as the Tauri external binary (target-triple suffix
#      is the externalBin naming convention; bundler strips it on install).
mkdir -p app/desktop/src-tauri/binaries
cp "$cutd_exe" "app/desktop/src-tauri/binaries/cutd-$TARGET.exe"

# ── 3b. Updater signing. tauri.conf `createUpdaterArtifacts:true` makes the
#       bundle emit a signed `.sig` beside the installer. The release feed is
#       assembled later by scripts/release/generate-updater-manifest.mjs.
#       RELEASE/CI: provide the Tauri updater signing key through the build
#       environment. DEV builds without a key omit updater artifacts.
UPDATER_CFG=()
DISABLE_UPDATER_ARTIFACTS="${SHELLX_DISABLE_UPDATER_ARTIFACTS:-0}"
case "$DISABLE_UPDATER_ARTIFACTS" in
  0|1) ;;
  *) echo "FAIL: SHELLX_DISABLE_UPDATER_ARTIFACTS must be 0 or 1" >&2; exit 2 ;;
esac
if [ "$DISABLE_UPDATER_ARTIFACTS" = "1" ]; then
  unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  echo "[build-windows] updater artifacts explicitly disabled for this candidate"
  UPDATER_CFG=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
else
  if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
    echo "[build-windows] updater artifacts WILL be signed (TAURI_SIGNING_PRIVATE_KEY set)"
  else
    echo "[build-windows] WARN: no TAURI_SIGNING_PRIVATE_KEY — building WITHOUT signed updater artifacts (dev build)" >&2
    UPDATER_CFG=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
  fi
fi

# ── 3c. AUTHENTICODE signing (Azure Trusted Signing) — SEPARATE from the updater
#       minisign above. tauri.conf bundle.windows.signCommand calls
#       app/desktop/scripts/windows-artifact-sign.sh per artifact.
#       OPT-IN so incomplete candidates cannot be signed: the .exe/installer are signed ONLY when
#       SHELLX_WINDOWS_SIGNING_REQUIRED=1 — else the helper exits 0 leaving an
#       UNSIGNED dev build. SIGNED RELEASE:
#         SHELLX_WINDOWS_SIGNING_REQUIRED=1 bash scripts/build-windows.sh release
#       then verify: `signtool verify /pa /v <installer.exe>`.
if [ "${SHELLX_WINDOWS_SIGNING_REQUIRED:-0}" = "1" ]; then
  echo "[build-windows] Authenticode signing REQUIRED — artifacts will be Azure-signed (signCommand)"
else
  echo "[build-windows] Authenticode signing OFF (set SHELLX_WINDOWS_SIGNING_REQUIRED=1 for a signed release)"
fi

# ── 4. Shell + bundle (separate cargo workspace at app/desktop/src-tauri)
echo "[build-windows] cargo tauri build $MODE → $TARGET"
shell_log=$( cd app/desktop && cargo tauri build "${TAURI_FLAG[@]}" "${UPDATER_CFG[@]}" --runner cargo-xwin --target "$TARGET" 2>&1 ) \
  || { echo "$shell_log"; echo "FAIL: cargo tauri build failed" >&2; exit 1; }
echo "$shell_log"
if ! printf '%s\n' "$shell_log" | grep -Eq 'Built application at: .*/shellx-cut[.]exe$'; then
  echo "FAIL: Tauri selected a non-shell helper as the Windows app executable" >&2
  exit 1
fi

out="app/desktop/src-tauri/target/$TARGET/$MODE"
shell_exe="$out/shellx-cut.exe"
[ -f "$shell_exe" ] || { echo "FAIL: $shell_exe was not produced" >&2; exit 1; }
# Conditional freshness, same reasoning: the Tauri wrapper is cached on a UI-only
# change (the fresh UI ships as the ui-dist RESOURCE, not embedded in the exe), so
# require a fresh shell exe only when the wrapper actually recompiled. The installer
# below must still be freshly bundled (it carries the current ui-dist) — that guard
# stays strict.
if echo "$shell_log" | grep -q "Compiling "; then
  mtime=$(stat -c %Y "$shell_exe")
  [ "$mtime" -ge "$started" ] || { echo "FAIL: $shell_exe is STALE (recompiled but the exe predates build start)" >&2; exit 1; }
  echo "[verify] shell exe rebuilt + fresh:"
else
  echo "[verify] shell exe unchanged (wrapper cached — UI-only build):"
fi
ls -lh "$shell_exe"; sha256sum "$shell_exe"

# Installer: productName "ShellX Cut" + version from tauri.conf.json.
version=$(grep -oP '"version":\s*"\K[^"]+' app/desktop/src-tauri/tauri.conf.json | head -1)
bundle_dir="$out/bundle/nsis"
installer=""
if [ -d "$bundle_dir" ]; then
  shopt -s nullglob
  installers=("$bundle_dir"/*"$version"*setup.exe)
  shopt -u nullglob
  [ "${#installers[@]}" -gt 0 ] && installer="${installers[0]}"
fi
if [ -n "$installer" ]; then
  inst_mtime=$(stat -c %Y "$installer")
  [ "$inst_mtime" -ge "$started" ] || { echo "FAIL: $installer is STALE (mtime predates build start)" >&2; exit 1; }
  echo "[installer]"; ls -lh "$installer"; sha256sum "$installer"
  if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ "$DISABLE_UPDATER_ARTIFACTS" = "0" ]; then
    updater_sig="$installer.sig"
    [ -s "$updater_sig" ] || { echo "FAIL: signed updater build did not produce $updater_sig" >&2; exit 1; }
    sig_mtime=$(stat -c %Y "$updater_sig")
    [ "$sig_mtime" -ge "$started" ] || { echo "FAIL: $updater_sig is STALE (mtime predates build start)" >&2; exit 1; }
    echo "[updater-signature]"; ls -lh "$updater_sig"; sha256sum "$updater_sig"
  fi
else
  echo "FAIL: no NSIS installer produced for version $version (makensis missing or bundle failed?)" >&2
  exit 1
fi
echo "[build-windows] OK ($MODE) in $(( $(date +%s) - started ))s"
