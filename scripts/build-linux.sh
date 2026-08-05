#!/usr/bin/env bash
# Build native x86_64 Linux packages. Run this on a real Linux host, not WSL.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

MODE="${1:-release}"
TARGET="x86_64-unknown-linux-gnu"
case "$MODE" in
  debug) TAURI_FLAG=(--debug); CARGO_FLAG=() ;;
  release) TAURI_FLAG=(); CARGO_FLAG=(--release) ;;
  *) echo "usage: $0 [debug|release]" >&2; exit 2 ;;
esac

if grep -qi microsoft /proc/version 2>/dev/null; then
  echo "FAIL: Linux packages must be built on a native Linux host, not WSL" >&2
  exit 1
fi
if printf '%s\n' "${TAURI_FEATURES:-}" | tr ', ' '\n\n' | grep -qx webdriver-test; then
  echo "FAIL: webdriver-test must not be enabled for shipping Linux builds" >&2
  exit 1
fi

started=$(date +%s)
mtime() { stat -c %Y "$1"; }
agent_doc_paths=$(node scripts/lib/agent-docs.mjs --paths)
agent_doc_count=$(printf '%s\n' "$agent_doc_paths" | wc -l | tr -d ' ')
while IFS= read -r rel; do
  [ -f "$rel" ] || { echo "FAIL: bundled agent doc missing from source: $rel" >&2; exit 1; }
done <<<"$agent_doc_paths"
echo "[build-linux] agent-doc source manifest present ($agent_doc_count files)"

echo "[build-linux] building ui/dist"
(
  cd ui
  [ -d node_modules ] || npm install --no-fund --no-audit
  npm run build >/dev/null
)
[ -f ui/dist/index.html ] || { echo "FAIL: ui/dist/index.html missing" >&2; exit 1; }
rm -rf app/desktop/fallback/assets
grep -q engine_status app/desktop/fallback/index.html || {
  echo "FAIL: desktop fallback must remain the engine-status airlock" >&2
  exit 1
}

for file in app/perception/py/instruments.py app/perception/py/requirements.txt \
  app/perception/py/requirements-full.txt app/perception/py/safe_numbers.py \
  app/perception/py/blaze_face_short_range.tflite app/perception/py/matte_runner.py \
  app/perception/py/matanyone_runner.py app/perception/py/siglip_index.py \
  app/perception/py/track_runner.py app/perception/py/ocr_runner.py \
  app/perception/py/translate_runner.py app/perception/py/dub_runner.py \
  app/perception/py/diarize_runner.py app/perception/py/face_runner.py \
  app/perception/py/face_detection_yunet_2023mar.onnx; do
  [ -f "$file" ] || { echo "FAIL: bundled sidecar payload missing: $file" >&2; exit 1; }
done

echo "[build-linux] cargo build $MODE cutd -> $TARGET"
cutd_log=$(cd app && cargo build "${CARGO_FLAG[@]}" -p server --bin cutd --target "$TARGET" 2>&1) || {
  echo "$cutd_log"
  echo "FAIL: cargo build (cutd) failed" >&2
  exit 1
}
echo "$cutd_log"
cutd_bin="app/target/$TARGET/$MODE/cutd"
[ -x "$cutd_bin" ] || { echo "FAIL: $cutd_bin was not produced" >&2; exit 1; }
if echo "$cutd_log" | grep -q 'Compiling '; then
  [ "$(mtime "$cutd_bin")" -ge "$started" ] || { echo "FAIL: fresh cutd build is stale" >&2; exit 1; }
fi
"$cutd_bin" --version
sha256sum "$cutd_bin"

mkdir -p app/desktop/src-tauri/binaries
cp "$cutd_bin" "app/desktop/src-tauri/binaries/cutd-$TARGET"
chmod +x "app/desktop/src-tauri/binaries/cutd-$TARGET"

if cargo tauri --version >/dev/null 2>&1; then
  TAURI=(cargo tauri)
elif [ -x "$ROOT/ui/node_modules/.bin/tauri" ]; then
  TAURI=("$ROOT/ui/node_modules/.bin/tauri")
else
  echo "FAIL: install cargo-tauri or the UI development dependencies" >&2
  exit 1
fi

bundle_root="app/desktop/src-tauri/target/$TARGET/$MODE/bundle"
rm -rf "$bundle_root/deb" "$bundle_root/rpm"
echo "[build-linux] Tauri $MODE packages -> deb,rpm"
# Tauri's updater artifact on Linux is AppImage-based. These native package
# formats are user/install handoffs, so disable the unsupported updater sidecar.
tauri_log=$(cd app/desktop && "${TAURI[@]}" build "${TAURI_FLAG[@]}" \
  --config '{"bundle":{"createUpdaterArtifacts":false}}' \
  --bundles deb,rpm --target "$TARGET" 2>&1) || {
  echo "$tauri_log"
  echo "FAIL: Tauri Linux package build failed" >&2
  exit 1
}
echo "$tauri_log"
if ! printf '%s\n' "$tauri_log" | grep -Eq 'Built application at: .*/shellx-cut$'; then
  echo "FAIL: Tauri selected a non-shell helper as the Linux app executable" >&2
  exit 1
fi

shopt -s nullglob
debs=("$bundle_root"/deb/*.deb)
rpms=("$bundle_root"/rpm/*.rpm)
shopt -u nullglob
[ "${#debs[@]}" -eq 1 ] || { echo "FAIL: expected exactly one .deb" >&2; exit 1; }
[ "${#rpms[@]}" -eq 1 ] || { echo "FAIL: expected exactly one .rpm" >&2; exit 1; }
for package in "${debs[@]}" "${rpms[@]}"; do
  [ "$(mtime "$package")" -ge "$started" ] || { echo "FAIL: stale package: $package" >&2; exit 1; }
  ls -lh "$package"
  sha256sum "$package"
done

manifest_dir=$(mktemp -d)
trap 'rm -rf "$manifest_dir"' EXIT
dpkg-deb -c "${debs[0]}" >"$manifest_dir/deb-files.txt"
dpkg-deb -x "${debs[0]}" "$manifest_dir/deb-root"
deb_shell="$manifest_dir/deb-root/usr/bin/shellx-cut"
[ -x "$deb_shell" ] || { echo "FAIL: .deb is missing the shellx-cut app executable" >&2; exit 1; }
[ ! -e "$manifest_dir/deb-root/usr/bin/verify-updater-signature" ] || {
  echo "FAIL: .deb selected the updater verifier helper as a shipping executable" >&2
  exit 1
}
while IFS= read -r rel; do
  packaged=$(find "$manifest_dir/deb-root" -type f -path "*/agent-docs/$rel" -print -quit)
  [ -n "$packaged" ] || { echo "FAIL: .deb is missing agent-docs/$rel" >&2; exit 1; }
  cmp -s "$rel" "$packaged" || { echo "FAIL: .deb agent-docs/$rel differs from source" >&2; exit 1; }
done <<<"$agent_doc_paths"
command -v rpm2cpio >/dev/null || {
  echo "FAIL: rpm2cpio is required to verify bundled RPM agent docs" >&2
  exit 1
}
command -v cpio >/dev/null || {
  echo "FAIL: cpio is required to verify bundled RPM agent docs" >&2
  exit 1
}
rpm_artifact=$(realpath "${rpms[0]}")
mkdir -p "$manifest_dir/rpm-root"
rpm -K --nosignature "$rpm_artifact" | grep -q 'digests OK' || {
  echo "FAIL: RPM digest verification failed" >&2
  exit 1
}
rpm_cpio="$manifest_dir/package.cpio"
# Ubuntu's rpm2cpio can return 1 for a valid Tauri RPM while still writing the
# complete archive. Verify the RPM digest above, then trust only a non-empty
# archive that cpio can extract and whose bundled docs compare byte-for-byte.
rpm2cpio "$rpm_artifact" >"$rpm_cpio" || true
[ -s "$rpm_cpio" ] || { echo "FAIL: rpm2cpio produced no archive" >&2; exit 1; }
(
  cd "$manifest_dir/rpm-root"
  cpio --quiet -idmu <"$rpm_cpio"
)
rpm_shell="$manifest_dir/rpm-root/usr/bin/shellx-cut"
[ -x "$rpm_shell" ] || { echo "FAIL: .rpm is missing the shellx-cut app executable" >&2; exit 1; }
[ ! -e "$manifest_dir/rpm-root/usr/bin/verify-updater-signature" ] || {
  echo "FAIL: .rpm selected the updater verifier helper as a shipping executable" >&2
  exit 1
}
while IFS= read -r rel; do
  packaged=$(find "$manifest_dir/rpm-root" -type f -path "*/agent-docs/$rel" -print -quit)
  [ -n "$packaged" ] || { echo "FAIL: .rpm is missing agent-docs/$rel" >&2; exit 1; }
  cmp -s "$rel" "$packaged" || { echo "FAIL: .rpm agent-docs/$rel differs from source" >&2; exit 1; }
done <<<"$agent_doc_paths"

echo "[build-linux] OK ($MODE) in $(( $(date +%s) - started ))s"
