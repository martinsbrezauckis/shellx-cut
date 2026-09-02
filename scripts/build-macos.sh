#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# build-macos.sh — build the ShellX Cut macOS app bundle NATIVELY on a Mac.
#
# Unlike build-windows.sh (which cross-compiles from WSL via cargo-xwin), the
# macOS bundle MUST be built on macOS: the Tauri shell links against the system
# WebKit (WKWebView) and the bundler invokes Apple tooling (actool/codesign/
# hdiutil) that only exists on macOS. Therefore this script must run directly
# on an Apple Silicon macOS host.
#
# Pipeline (engine-first, mirrors build-windows.sh):
#   1. build ui/dist (Vite)                    — bundled as a Tauri resource
#   2. cargo build cutd (native arm64)         — the ENGINE, identical to the
#                                                headless `cutd serve` binary
#   3. stage cutd as the Tauri externalBin     — binaries/cutd-aarch64-apple-darwin
#   4. cargo tauri build (native)              → .app + .dmg
#
# Stranger-ready packaging: the perception sidecar SCRIPT (instruments.py +
# requirements.txt + face model) is staged into the bundle as a Tauri resource
# mapped to `perception/` (tauri.conf.json bundle.resources), so a cold install
# always finds the script; only the heavy venv + ffmpeg are fetched on first
# use. On macOS ffmpeg has NO auto-fetcher (fetch.rs downloads BtbN builds only
# on Windows/Linux) — the resolver expects ffmpeg on PATH (Homebrew) or in the
# application support tools directory. See tools.rs
# bootstrap_hint() for the user-facing message.
#
# Produces:
#   <candidate-target>/engine/aarch64-apple-darwin/<mode>/cutd          (engine)
#   <candidate-target>/desktop-tauri/aarch64-apple-darwin/<mode>/bundle/
#     macos/ShellX Cut.app
#     dmg/ShellX Cut_<version>_aarch64.dmg
#
# When supplied, `CARGO_TARGET_DIR` is the candidate-target identity. A release
# build must receive that explicit absolute path from Release Studio; local
# debug builds without one retain the normal in-tree developer targets.
#
# PREREQS on the Mac:
#   rustup target aarch64-apple-darwin (default on Apple Silicon), cargo-tauri
#   (tauri-cli 2.x), node/npm (UI build), Xcode Command Line Tools.
#   ffmpeg/ffprobe are RUNTIME deps (not bundled) — needed only to exercise the
#   media verbs, not to build.
#
# USAGE:  scripts/build-macos.sh [debug|release]   (default: release)
#
# VERIFY-AFTER-BUILD (mirrors the Windows guard): assert every produced
# artifact's mtime is fresh and print sizes + sha256, so a silent stale-binary
# build (cargo "Finished" but link skipped) can never pass unnoticed. Uses BSD
# stat (-f %m) + shasum (-a 256) — this script is macOS-only.
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root
repo_root="$(pwd -P)"
source scripts/lib/tauri-updater-signing.sh
source scripts/lib/macos-release-notarization.sh
source scripts/lib/macos-build-target-layout.sh

MODE="${1:-release}"
TARGET="aarch64-apple-darwin"
case "$MODE" in
  # NOTE: `cargo tauri build` is RELEASE by default and takes `--debug` (NOT
  # `--release`, unlike plain `cargo build`). So TAURI_FLAG carries --debug only
  # in debug mode; CARGO_FLAG (for the engine `cargo build`) uses --release.
  debug)   TAURI_FLAG=(--debug); CARGO_FLAG=() ;;
  release) TAURI_FLAG=();        CARGO_FLAG=(--release) ;;
  *) echo "usage: $0 [debug|release]" >&2; exit 2 ;;
esac
configure_macos_build_target_layout
if [ "$MODE" = "release" ]; then
  require_macos_release_candidate_target_layout
fi

# The registered Release Studio release command supplies the immutable
# candidate inputs. A release build cannot infer or default any of them.
release_build_input_verifier=""
require_release_build_input_contract() {
  local variable
  for variable in \
    SHELLX_RELEASE_STUDIO_ROOT \
    SHELLX_CUT_PUBLIC_EXPORT_DIR \
    SHELLX_CUT_CANDIDATE_FREEZE \
    SHELLX_CUT_CANDIDATE_FREEZE_MANIFEST_SHA256; do
    [ -n "${!variable:-}" ] || { echo "FAIL: release build requires $variable" >&2; return 1; }
  done
  release_build_input_verifier="$SHELLX_RELEASE_STUDIO_ROOT/tools/verify-release-build-input.mjs"
  [ -f "$release_build_input_verifier" ] || { echo "FAIL: trusted Release Studio build-input verifier is missing: $release_build_input_verifier" >&2; return 1; }
}

verify_release_build_input() {
  [ "$MODE" = "release" ] || return 0
  node "$release_build_input_verifier" \
    --project shellx-cut \
    --checkout "$repo_root" \
    --public-export "$SHELLX_CUT_PUBLIC_EXPORT_DIR" \
    --freeze "$SHELLX_CUT_CANDIDATE_FREEZE" \
    --freeze-manifest-sha256 "$SHELLX_CUT_CANDIDATE_FREEZE_MANIFEST_SHA256"
}

if [ "$MODE" = "release" ]; then
  require_release_build_input_contract
fi

FEATURES_STR="${TAURI_FEATURES:-}"
if printf '%s\n' "$FEATURES_STR" | tr ', ' '\n\n' | grep -qx 'webdriver-test'; then
  echo "FAIL: webdriver-test feature is test-only and must not be enabled for shipping macOS builds" >&2
  exit 1
fi

started=$(date +%s)
sha() { shasum -a 256 "$1"; }
mtime() { stat -f %m "$1"; }

# Release admission comes before any cleanup or build work. It is intentionally
# read-only: it validates only caller-provided identity/updater inputs and the
# local toolchain; private updater-key material remains confined to the later
# cargo-tauri subprocess.
UPDATER_CFG=()
prepare_tauri_updater_signing
if [ "$TAURI_UPDATER_ARTIFACTS_SIGNED" = "1" ]; then
  echo "[build-macos] updater artifacts WILL be signed"
else
  if [ "$MODE" = "release" ]; then
    echo "[build-macos] release has no signed updater key; the release admission will fail" >&2
  else
    echo "[build-macos] WARN: no Tauri updater key — building WITHOUT signed updater artifacts (debug build only)" >&2
    UPDATER_CFG=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
  fi
fi
if [ "$MODE" = "release" ]; then
  require_macos_release_admission
else
  report_macos_dev_notary_status "$MODE"
fi

agent_doc_paths=$(node scripts/lib/agent-docs.mjs --paths)
agent_doc_count=$(printf '%s\n' "$agent_doc_paths" | wc -l | tr -d ' ')
while IFS= read -r rel; do
  [ -f "$rel" ] || { echo "FAIL: bundled agent doc missing from source: $rel" >&2; exit 1; }
done <<<"$agent_doc_paths"
echo "[build-macos] agent-doc source manifest present ($agent_doc_count files)"
echo "[build-macos] Cargo target roots: identity=${MACOS_CANDIDATE_TARGET_ROOT:-developer-default} engine=$CUTD_TARGET_ROOT tauri=$TAURI_TARGET_ROOT"
bundle_root="$TAURI_TARGET_ROOT/$TARGET/$MODE/bundle"
dmg_dir="$bundle_root/dmg"
macos_bundle_dir="$bundle_root/macos"
if [ -d "$dmg_dir" ]; then
  echo "[build-macos] cleaning previous ShellX Cut DMGs from $dmg_dir"
  find "$dmg_dir" -maxdepth 1 -type f \( -name 'ShellX Cut_*.dmg' -o -name 'ShellX Cut_*.dmg.sig' \) -print -delete
fi
if [ -d "$macos_bundle_dir" ]; then
  echo "[build-macos] cleaning previous ShellX Cut app archives from $macos_bundle_dir"
  rm -rf "$macos_bundle_dir/ShellX Cut.app" "$macos_bundle_dir/ShellX Cut.app.tar.gz" "$macos_bundle_dir/ShellX Cut.app.tar.gz.sig"
fi

# ── 1. UI bundle (gitignored — always rebuild so the app ships the current UI)
echo "[build-macos] building ui/dist"
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
# Mirror build-windows.sh's full payload list: all 10 files are
# bundled via tauri.conf.json resources, so a missing one should fail the Mac build LOUD
# here, not slip past a 3-file guard and only surface at runtime.
for f in app/perception/py/instruments.py app/perception/py/requirements.txt \
         app/perception/py/requirements-full.txt \
         app/perception/py/requirements-full.linux-x86_64.lock \
         app/perception/py/requirements-full.windows-x86_64.lock \
         app/perception/py/requirements-full.macos-aarch64.lock \
         app/perception/py/safe_numbers.py \
         app/perception/py/blaze_face_short_range.tflite \
         app/perception/py/matte_runner.py app/perception/py/matanyone_runner.py \
         app/perception/py/siglip_index.py app/perception/py/track_runner.py \
         app/perception/py/ocr_runner.py app/perception/py/translate_runner.py \
         app/perception/py/dub_runner.py app/perception/py/diarize_runner.py \
         app/perception/py/face_runner.py \
         app/perception/py/face_detection_yunet_2023mar.onnx; do
  [ -f "$f" ] || { echo "FAIL: sidecar payload missing: $f (resources in tauri.conf.json)" >&2; exit 1; }
done
echo "[build-macos] sidecar payload present (perception + matte/track/ocr/face/translate/dub/diarize runners + models)"

# ── 2. Engine: native build of cutd for arm64 (the engine workspace, untouched)
echo "[build-macos] cargo build $MODE cutd → $TARGET  (started $(date +%H:%M:%S))"
# (workspace package name is `server`, binary name `cutd` — see app/server/Cargo.toml)
# No crt-static here: that flag is a Windows-MSVC concern (VCRUNTIME140). On macOS
# the binary links the system libSystem/dyld — the standard, expected linkage.
cutd_log=$( cd app && env "${MACOS_ENGINE_CARGO_ENV[@]}" cargo build ${CARGO_FLAG[@]+"${CARGO_FLAG[@]}"} -p server --bin cutd --target "$TARGET" 2>&1 ) \
  || { echo "$cutd_log"; echo "FAIL: cargo build (cutd) failed" >&2; exit 1; }
echo "$cutd_log"
cutd_bin="$CUTD_TARGET_ROOT/$TARGET/$MODE/cutd"
[ -f "$cutd_bin" ] || { echo "FAIL: $cutd_bin was not produced" >&2; exit 1; }
# Freshness guard ONLY when cargo actually COMPILED the engine (UI-only changes
# legitimately leave the engine cached; the bundle guard below is the backstop).
if echo "$cutd_log" | grep -q "Compiling "; then
  [ "$(mtime "$cutd_bin")" -ge "$started" ] || { echo "FAIL: $cutd_bin is STALE (cargo compiled but the binary predates build start)" >&2; exit 1; }
  echo "[verify] cutd rebuilt + fresh:"
else
  echo "[verify] cutd unchanged (engine cached — UI-only build); using the existing valid binary:"
fi
ls -lh "$cutd_bin"; sha "$cutd_bin"
# Sanity: the engine binary actually runs on this Mac (catches a broken arch / link).
"$cutd_bin" --version || { echo "FAIL: cutd --version did not run on this Mac" >&2; exit 1; }

# ── 3. Stage the engine as the Tauri external binary (target-triple suffix is
#      the externalBin naming convention; the bundler strips it on install).
mkdir -p app/desktop/src-tauri/binaries
cp "$cutd_bin" "app/desktop/src-tauri/binaries/cutd-$TARGET"
chmod +x "app/desktop/src-tauri/binaries/cutd-$TARGET"

# ── 4. Shell + bundle (separate cargo workspace at app/desktop/src-tauri)
# The engine command above used its own root. From here through the updater
# verifier and identity signer, every Cargo consumer is desktop-side.
if [ "${#MACOS_TAURI_CARGO_ENV[@]}" -gt 0 ]; then
  export "${MACOS_TAURI_CARGO_ENV[@]}"
fi
verify_release_build_input
echo "[build-macos] cargo tauri build $MODE → $TARGET"
shell_log=$( (
  cd app/desktop
  materialize_tauri_updater_signing_key
  cargo tauri build ${TAURI_FLAG[@]+"${TAURI_FLAG[@]}"} ${UPDATER_CFG[@]+"${UPDATER_CFG[@]}"} --target "$TARGET"
) 2>&1 ) \
  || { echo "$shell_log"; echo "FAIL: cargo tauri build failed" >&2; exit 1; }
echo "$shell_log"
reject_tauri_updater_key_mismatch "$shell_log"
if ! printf '%s\n' "$shell_log" | grep -Eq 'Built application at: .*/shellx-cut$'; then
  echo "FAIL: Tauri selected a non-shell helper as the macOS app executable" >&2
  exit 1
fi

out="$TAURI_TARGET_ROOT/$TARGET/$MODE"
app_bundle="$out/bundle/macos/ShellX Cut.app"
[ -d "$app_bundle" ] || { echo "FAIL: $app_bundle was not produced" >&2; exit 1; }
app_exe="$app_bundle/Contents/MacOS/shellx-cut"
[ -f "$app_exe" ] || { echo "FAIL: app executable missing inside the bundle" >&2; exit 1; }
[ ! -e "$app_bundle/Contents/MacOS/verify-updater-signature" ] || {
  echo "FAIL: .app selected the updater verifier helper as a shipping executable" >&2
  exit 1
}
if echo "$shell_log" | grep -q "Compiling "; then
  [ "$(mtime "$app_exe")" -ge "$started" ] || { echo "FAIL: app exe is STALE (recompiled but predates build start)" >&2; exit 1; }
  echo "[verify] app bundle rebuilt + fresh:"
else
  echo "[verify] app wrapper cached (UI-only build):"
fi
ls -lh "$app_exe"; sha "$app_exe"
agent_docs_dir="$app_bundle/Contents/Resources/agent-docs"
while IFS= read -r rel; do
  packaged="$agent_docs_dir/$rel"
  [ -f "$packaged" ] || { echo "FAIL: .app is missing agent-docs/$rel" >&2; exit 1; }
  cmp -s "$rel" "$packaged" || { echo "FAIL: .app agent-docs/$rel differs from source" >&2; exit 1; }
done <<<"$agent_doc_paths"
echo "[verify] .app bundles all agent docs byte-for-byte ($agent_doc_count files)"
# Bundle and updater identity names use the configured release version.
version=$(grep '"version"' app/desktop/src-tauri/tauri.conf.json | head -1 \
          | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')
if [ "$TAURI_UPDATER_ARTIFACTS_SIGNED" = "1" ]; then
  updater_archive="$app_bundle.tar.gz"
  updater_sig="$updater_archive.sig"
  [ -s "$updater_archive" ] || { echo "FAIL: signed updater build did not produce $updater_archive" >&2; exit 1; }
  [ -s "$updater_sig" ] || { echo "FAIL: signed updater build did not produce $updater_sig" >&2; exit 1; }
  [ "$(mtime "$updater_archive")" -ge "$started" ] || { echo "FAIL: $updater_archive is STALE (mtime predates build start)" >&2; exit 1; }
  [ "$(mtime "$updater_sig")" -ge "$started" ] || { echo "FAIL: $updater_sig is STALE (mtime predates build start)" >&2; exit 1; }
  echo "[updater-archive]"; ls -lh "$updater_archive" "$updater_sig"
  sha "$updater_archive"; sha "$updater_sig"
  verify_tauri_updater_artifact "$updater_archive" "$updater_sig"
  write_tauri_updater_artifact_identity "$updater_archive" "darwin-aarch64" "$version"
fi
if [ "$MODE" = "release" ]; then
  verify_macos_release_app_bundle "$app_bundle"
fi

# DMG (version was read above with BSD-compatible sed).
dmg_dir="$out/bundle/dmg"
dmg=""
if [ -d "$dmg_dir" ]; then
  shopt -s nullglob
  dmgs=("$dmg_dir"/*"$version"*.dmg)
  shopt -u nullglob
  [ "${#dmgs[@]}" -gt 0 ] && dmg="${dmgs[0]}"
fi
if [ -n "$dmg" ]; then
  [ "$(mtime "$dmg")" -ge "$started" ] || { echo "FAIL: $dmg is STALE (mtime predates build start)" >&2; exit 1; }
  echo "[dmg]"; ls -lh "$dmg"; sha "$dmg"
  if [ "$MODE" = "release" ]; then
    notarize_release_dmg "$dmg"
    verify_macos_release_dmg_app_identity "$dmg" "$app_bundle"
    # Stapling changes the container bytes. Hash only after all release
    # qualification checks have passed, so the recorded DMG identity is final.
    sha "$dmg"
  else
    echo "[build-macos] debug DMG retained for local use only; it is not release-qualified"
  fi
else
  if [ "$MODE" = "release" ]; then
    echo "FAIL: release build requires a DMG for notarization; none was produced for version $version" >&2
    exit 1
  fi
  echo "[build-macos] NOT RELEASE-QUALIFIED: no debug DMG was produced for version $version" >&2
fi
echo "[build-macos] OK ($MODE) in $(( $(date +%s) - started ))s"
