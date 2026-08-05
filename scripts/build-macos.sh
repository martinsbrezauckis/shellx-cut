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
#   app/target/aarch64-apple-darwin/<mode>/cutd                         (engine)
#   app/desktop/src-tauri/target/aarch64-apple-darwin/<mode>/bundle/
#     macos/ShellX Cut.app
#     dmg/ShellX Cut_<version>_aarch64.dmg
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

FEATURES_STR="${TAURI_FEATURES:-}"
if printf '%s\n' "$FEATURES_STR" | tr ', ' '\n\n' | grep -qx 'webdriver-test'; then
  echo "FAIL: webdriver-test feature is test-only and must not be enabled for shipping macOS builds" >&2
  exit 1
fi

started=$(date +%s)
sha() { shasum -a 256 "$1"; }
mtime() { stat -f %m "$1"; }
agent_doc_paths=$(node scripts/lib/agent-docs.mjs --paths)
agent_doc_count=$(printf '%s\n' "$agent_doc_paths" | wc -l | tr -d ' ')
while IFS= read -r rel; do
  [ -f "$rel" ] || { echo "FAIL: bundled agent doc missing from source: $rel" >&2; exit 1; }
done <<<"$agent_doc_paths"
echo "[build-macos] agent-doc source manifest present ($agent_doc_count files)"
bundle_root="app/desktop/src-tauri/target/$TARGET/$MODE/bundle"
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
cutd_log=$( cd app && cargo build ${CARGO_FLAG[@]+"${CARGO_FLAG[@]}"} -p server --bin cutd --target "$TARGET" 2>&1 ) \
  || { echo "$cutd_log"; echo "FAIL: cargo build (cutd) failed" >&2; exit 1; }
echo "$cutd_log"
cutd_bin="app/target/$TARGET/$MODE/cutd"
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

# ── 3b. Updater signing. tauri.conf `createUpdaterArtifacts:true` makes the
#       bundle emit a signed `.sig` + feed entry — which needs the minisign key.
#       RELEASE: caller provides the Tauri updater signing key through the build
#       environment. This script does not persist the key.
#       DEV (no key): build WITHOUT updater artifacts so a plain build still works.
UPDATER_CFG=()
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
  echo "[build-macos] updater artifacts WILL be signed (TAURI_SIGNING_PRIVATE_KEY set)"
else
  echo "[build-macos] WARN: no TAURI_SIGNING_PRIVATE_KEY — building WITHOUT signed updater artifacts (dev build; a RELEASE must export the key)" >&2
  UPDATER_CFG=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
fi

# ── 3c. Developer ID signing guard (RELEASE only). tauri.conf.json bundle.macOS
#       is empty, so Tauri does Developer-ID signing ONLY when APPLE_SIGNING_IDENTITY
#       is exported. Without it the build is only ad-hoc signed and cannot satisfy
#       Developer ID notarization. Fail closed when a release lacks both the
#       environment variable and a configured identity.
if [ "$MODE" = "release" ] && [ -z "${APPLE_SIGNING_IDENTITY:-}" ] \
   && ! grep -q '"signingIdentity"' app/desktop/src-tauri/tauri.conf.json 2>/dev/null; then
  echo "FAIL: release build but APPLE_SIGNING_IDENTITY is unset and tauri.conf.json has no signingIdentity." >&2
  echo "      → the build would AD-HOC sign and notarization returns Invalid (no Developer ID, no hardened runtime)." >&2
  echo "      Export the full Developer ID Application certificate identity before building." >&2
  exit 1
fi

# ── 4. Shell + bundle (separate cargo workspace at app/desktop/src-tauri)
echo "[build-macos] cargo tauri build $MODE → $TARGET"
shell_log=$( cd app/desktop && cargo tauri build ${TAURI_FLAG[@]+"${TAURI_FLAG[@]}"} ${UPDATER_CFG[@]+"${UPDATER_CFG[@]}"} --target "$TARGET" 2>&1 ) \
  || { echo "$shell_log"; echo "FAIL: cargo tauri build failed" >&2; exit 1; }
echo "$shell_log"
if ! printf '%s\n' "$shell_log" | grep -Eq 'Built application at: .*/shellx-cut$'; then
  echo "FAIL: Tauri selected a non-shell helper as the macOS app executable" >&2
  exit 1
fi

out="app/desktop/src-tauri/target/$TARGET/$MODE"
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
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  updater_archive="$app_bundle.tar.gz"
  updater_sig="$updater_archive.sig"
  [ -s "$updater_archive" ] || { echo "FAIL: signed updater build did not produce $updater_archive" >&2; exit 1; }
  [ -s "$updater_sig" ] || { echo "FAIL: signed updater build did not produce $updater_sig" >&2; exit 1; }
  [ "$(mtime "$updater_archive")" -ge "$started" ] || { echo "FAIL: $updater_archive is STALE (mtime predates build start)" >&2; exit 1; }
  [ "$(mtime "$updater_sig")" -ge "$started" ] || { echo "FAIL: $updater_sig is STALE (mtime predates build start)" >&2; exit 1; }
  echo "[updater-archive]"; ls -lh "$updater_archive" "$updater_sig"
  sha "$updater_archive"; sha "$updater_sig"
fi

# DMG (version comes from tauri.conf.json — BSD grep has no -P, so sed-extract).
version=$(grep '"version"' app/desktop/src-tauri/tauri.conf.json | head -1 \
          | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')
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
  # DMG notarization (signed builds only): Tauri notarizes + staples the .app but
  # NOT the .dmg CONTAINER, so a DOWNLOADED dmg still trips Gatekeeper on mount
  # ("Unnotarized Developer ID"). When the App Store Connect API creds are in the
  # env (same ones Tauri used to notarize the app), notarize + staple the dmg too
  # → the whole download is clean (spctl: "Notarized Developer ID"). NON-FATAL: a
  # notary hiccup warns but never loses the build (the .app inside stays valid).
  if [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    echo "[build-macos] notarizing + stapling the DMG container…"
    nout=$(xcrun notarytool submit "$dmg" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" --wait 2>&1) || true
    echo "$nout" | tail -4
    if echo "$nout" | grep -q "status: Accepted"; then
      if xcrun stapler staple "$dmg"; then echo "[build-macos] DMG notarized + stapled"; sha "$dmg"; else echo "[warn] DMG staple failed (notarization Accepted)" >&2; fi
    else
      echo "[warn] DMG notarization NOT Accepted — the .app inside is still notarized+stapled; staple the dmg manually before distribution" >&2
    fi
  else
    echo "[build-macos] DMG container notarization skipped (no APPLE_API_* env — dev/unsigned build)"
  fi
else
  # A missing DMG is non-fatal (the .app is the primary artifact; DMG needs
  # hdiutil which can be flaky on headless sessions) — warn loudly, don't fail.
  echo "[warn] no DMG produced for version $version (hdiutil unavailable on a headless SSH session?) — the .app bundle is valid" >&2
fi
echo "[build-macos] OK ($MODE) in $(( $(date +%s) - started ))s"
