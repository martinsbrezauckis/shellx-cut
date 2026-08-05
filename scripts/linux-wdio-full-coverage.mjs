#!/usr/bin/env node
import { spawn, spawnSync } from 'node:child_process'
import { homedir } from 'node:os'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildSshEnvPayload, readEnvFirstLine, SSH_KEEPALIVE_ARGS } from './lib/ssh-stdin-env.mjs'
import { FULL_AGENT_FIXTURE_SHELL } from './lib/native-full-agent-fixture-env.mjs'
const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
function arg(name, fallback = '') {
  const index = process.argv.indexOf(name)
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback
}
const flag = (name) => process.argv.includes(name)
function run(command, args, { input, cwd = REPO_ROOT } = {}) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(command, args, {
      cwd,
      env: process.env,
      stdio: input ? ['pipe', 'inherit', 'inherit'] : 'inherit',
    })
    child.on('error', reject)
    child.on('exit', (code, signal) => {
      if (code === 0) resolveRun()
      else reject(new Error(
        `${command} ${args.join(' ')} failed: code=${code} signal=${signal || 'none'}`,
      ))
    })
    if (input) child.stdin.end(input)
  })
}
function usage() {
  console.log(`Usage: node scripts/linux-wdio-full-coverage.mjs [--suite full-coverage|drop-to-create|adapter-smoke] [--drop-case video|image|both] --host <ssh-host> [--remote-dir ~/shellx-cut] [--scene-clip <path>] [--image <path>] [--speech-clip <path>] [--face-clip <path>] [--speakers-clip <path>] [--second-clip <path>] [--out <evidence-dir>] [--section <list>] [--only <text>] [--trace] [--full] [--strict-candidate-actions] [--real-screen-record] [--installed-final] [--tauri-driver <path>] [--webkit-driver <path>] [--skip-sync] [--clean-after]
Default mode runs the instrumented native candidate under Xvfb.
--installed-final drives the exact shipping .deb; --clean-after preserves evidence.
When ANTHROPIC_API_KEY is set locally, it is forwarded over SSH stdin to the test process only.`)
}
async function main() {
  if (flag('--help') || flag('-h')) return usage()
  const host = arg('--host', process.env.SHELLX_CUT_LINUX_HOST || '')
  if (!host) throw new Error('configure the Linux SSH host with --host or SHELLX_CUT_LINUX_HOST')
  const remoteDir = arg('--remote-dir', process.env.SHELLX_CUT_LINUX_REMOTE_DIR || '~/shellx-cut')
  const sceneClip = arg('--scene-clip',
    arg('--clip', process.env.SHELLX_CUT_WDIO_SCENE_CLIP ||
      process.env.SHELLX_CUT_WDIO_CLIP || 'testdata/talking_head.mp4'))
  const speechClip = arg('--speech-clip',
    arg('--library-clip', process.env.SHELLX_CUT_WDIO_SPEECH_CLIP ||
      process.env.SHELLX_CUT_WDIO_LIBRARY_CLIP || sceneClip))
  const image = arg('--image', process.env.SHELLX_CUT_WDIO_IMAGE || 'testdata/real/intro.png')
  const faceClip = arg('--face-clip', process.env.SHELLX_CUT_WDIO_FACE_CLIP || '')
  const speakersClip = arg('--speakers-clip', process.env.SHELLX_CUT_WDIO_SPEAKERS_CLIP || '')
  const secondClip = arg('--second-clip', process.env.SHELLX_CUT_WDIO_SECOND_CLIP || '')
  const stamp = new Date().toISOString().replace(/[:.]/g, '-')
  const suite = arg('--suite', flag('--adapter-smoke') ? 'adapter-smoke' : 'full-coverage')
  if (!['full-coverage', 'drop-to-create', 'adapter-smoke'].includes(suite)) throw new Error(`unsupported suite: ${suite}`)
  const dropCase = arg('--drop-case', 'both')
  if (!['video', 'image', 'both'].includes(dropCase)) throw new Error(`unsupported drop case: ${dropCase}`)
  const outDir = arg(
    '--out',
    process.env.SHELLX_CUT_WDIO_OUT ||
      `~/.shellx-scratch/shellx-cut/linux-wdio-${suite}-${stamp}`,
  )
  const installedFinal = flag('--installed-final')
  const sourceHead = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: REPO_ROOT, encoding: 'utf8' }).stdout.trim()
  const sourceDirty = spawnSync('git', ['status', '--porcelain'], { cwd: REPO_ROOT, encoding: 'utf8' }).stdout.trim()
  if (installedFinal && sourceDirty) throw new Error('installed-final requires a clean source worktree')
  const cleanAfter = flag('--clean-after')
  const tauriDriver = arg('--tauri-driver', process.env.SHELLX_CUT_TAURI_DRIVER || '')
  const webkitDriver = arg('--webkit-driver', process.env.SHELLX_CUT_WEBKIT_DRIVER || '')
  const anthropicKey = readEnvFirstLine('ANTHROPIC_API_KEY')
  const section = arg('--section', process.env.FCV_SECTION || '')
  const only = arg('--only', process.env.FCV_ONLY || '')
  const strictCandidateActions = flag('--strict-candidate-actions')
  const realScreenRecord = flag('--real-screen-record')
  const full = flag('--full') || strictCandidateActions || installedFinal
  if (strictCandidateActions && (section || only || suite !== 'full-coverage')) throw new Error('--strict-candidate-actions forbids --section, --only and --adapter-smoke')
  if (installedFinal && (section || only || suite !== 'full-coverage' || strictCandidateActions)) throw new Error('--installed-final requires --suite full-coverage and forbids --section, --only, and --strict-candidate-actions')
  const remotePath = (path) => !path || path.startsWith('/') || path.startsWith('~')
    ? path
    : `${remoteDir}/${path}`
  const remoteSceneClip = remotePath(sceneClip)
  const remoteSpeechClip = remotePath(speechClip)
  const remoteImage = remotePath(image)
  const remoteFaceClip = remotePath(faceClip)
  const remoteSpeakersClip = remotePath(speakersClip)
  const remoteSecondClip = remotePath(secondClip)
  const remoteTauriDriver = remotePath(tauriDriver)
  const remoteWebkitDriver = remotePath(webkitDriver)
  const include = [
    '.gitignore', 'AGENTS.md', 'LICENSE', 'NOTICE', 'README.md', 'START_HERE_FOR_AGENT.txt',
    'app/***', 'docs/***', 'schema/***', 'scripts/***', 'skill/***', 'testdata/***', 'ui/***',
  ]
  const exclude = [
    '.git/***',
    'app/target/***',
    'app/desktop/src-tauri/target/***',
    'ui/dist/***',
    'ui/node_modules/***',
  ]
  const rsyncArgs = [
    '-az',
    '--delete',
    ...exclude.flatMap((pattern) => ['--exclude', pattern]),
    ...include.flatMap((pattern) => ['--include', pattern]),
    '--exclude', '*',
    `${REPO_ROOT}/`,
    `${host}:${remoteDir}/`,
  ]
  console.log(`Linux host: ${host}`)
  console.log(`Remote dir: ${remoteDir}`)
  console.log(`Evidence dir: ${outDir}`)
  console.log(`Suite: ${suite}`)
  console.log(`Mode: ${installedFinal ? 'installed shipping package (external WebDriver)' : 'instrumented candidate (embedded WebDriver)'}`)
  if (!flag('--skip-sync')) await run('rsync', rsyncArgs)
  const remoteScript = String.raw`
set -euo pipefail
expand_remote_path() {
  case "$1" in
    "~") printf '%s\n' "$HOME" ;;
    "~/"*) printf '%s/%s\n' "$HOME" "$(printf '%s\n' "$1" | sed 's#^~/##')" ;;
    *) printf '%s\n' "$1" ;;
  esac
}
REMOTE_DIR_RESOLVED="$(expand_remote_path "$REMOTE_DIR")"
resolve_optional_path() {
  if [ -n "$1" ]; then expand_remote_path "$1"; fi
}
WDIO_SCENE_CLIP_RESOLVED="$(expand_remote_path "$WDIO_SCENE_CLIP")"
WDIO_SPEECH_CLIP_RESOLVED="$(expand_remote_path "$WDIO_SPEECH_CLIP")"
WDIO_IMAGE_RESOLVED="$(expand_remote_path "$WDIO_IMAGE")"
WDIO_FACE_CLIP_RESOLVED="$(resolve_optional_path "$WDIO_FACE_CLIP")"
WDIO_SPEAKERS_CLIP_RESOLVED="$(resolve_optional_path "$WDIO_SPEAKERS_CLIP")"
WDIO_SECOND_CLIP_RESOLVED="$(resolve_optional_path "$WDIO_SECOND_CLIP")"
WDIO_OUT_RESOLVED="$(expand_remote_path "$WDIO_OUT")"
if [ "$INSTALLED_FINAL" = "1" ]; then FCV_INSTALLED_RUNTIME_RECEIPT_VALUE="$WDIO_OUT_RESOLVED/installed-runtime-receipt.json"; else FCV_INSTALLED_RUNTIME_RECEIPT_VALUE=""; fi
WDIO_TAURI_DRIVER_RESOLVED="$(resolve_optional_path "$WDIO_TAURI_DRIVER")"
WDIO_WEBKIT_DRIVER_RESOLVED="$(resolve_optional_path "$WDIO_WEBKIT_DRIVER")"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin"
cd "$REMOTE_DIR_RESOLVED"
${FULL_AGENT_FIXTURE_SHELL}
mkdir -p "$WDIO_OUT_RESOLVED"
SOURCE_CONTENT_MANIFEST_SHA256="$(node scripts/source-content-manifest.mjs --out "$WDIO_OUT_RESOLVED/source-content-manifest.json" --sha256)"
runtime_dir="$WDIO_OUT_RESOLVED/runtime"
mkdir -p "$runtime_dir"
chmod 700 "$runtime_dir"
app_cwd="$runtime_dir/app-cwd"
mkdir -p "$app_cwd"
chmod 700 "$app_cwd"
command -v Xvfb >/dev/null || {
  echo "Xvfb is required for the native Linux action gate" >&2
  exit 1
}
if ss -ltn 2>/dev/null | grep -Eq '127[.]0[.]0[.]1:6161[[:space:]]'; then
  echo "127.0.0.1:6161 is already occupied; refusing to reuse another Cut session" >&2
  exit 1
fi
test -f "$WDIO_SCENE_CLIP_RESOLVED" || {
  echo "scene clip not found: $WDIO_SCENE_CLIP_RESOLVED" >&2
  exit 1
}
test -f "$WDIO_SPEECH_CLIP_RESOLVED" || {
  echo "speech clip not found: $WDIO_SPEECH_CLIP_RESOLVED" >&2
  exit 1
}
test -f "$WDIO_IMAGE_RESOLVED" || {
  echo "still image not found: $WDIO_IMAGE_RESOLVED" >&2
  exit 1
}
for role_path in "$WDIO_FACE_CLIP_RESOLVED" "$WDIO_SPEAKERS_CLIP_RESOLVED" "$WDIO_SECOND_CLIP_RESOLVED"; do
  if [ -n "$role_path" ] && [ ! -f "$role_path" ]; then
    echo "explicit role clip not found: $role_path" >&2
    exit 1
  fi
done
. scripts/lib/linux-native-run-cleanup.sh
npm --prefix ui install --no-audit --no-fund
if [ "$INSTALLED_FINAL" = "1" ]; then
  if [ -n "$WDIO_TAURI_DRIVER_RESOLVED" ]; then
    test -x "$WDIO_TAURI_DRIVER_RESOLVED" || {
      echo "tauri-driver is not executable: $WDIO_TAURI_DRIVER_RESOLVED" >&2
      exit 1
    }
  else
    WDIO_TAURI_DRIVER_RESOLVED="$(command -v tauri-driver || true)"
  fi
  test -n "$WDIO_TAURI_DRIVER_RESOLVED" || {
    echo "official tauri-driver is required for --installed-final" >&2
    exit 1
  }
  if [ -n "$WDIO_WEBKIT_DRIVER_RESOLVED" ]; then
    test -x "$WDIO_WEBKIT_DRIVER_RESOLVED" || {
      echo "WebKitWebDriver is not executable: $WDIO_WEBKIT_DRIVER_RESOLVED" >&2
      exit 1
    }
    export PATH="$(dirname "$WDIO_WEBKIT_DRIVER_RESOLVED"):$PATH"
  fi
  command -v WebKitWebDriver >/dev/null || {
    echo "WebKitWebDriver is required for --installed-final; pass --webkit-driver when it is unpacked outside PATH" >&2
    exit 1
  }
  TAURI_FEATURES="" scripts/build-linux.sh release
  bundle_root="app/desktop/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle"
  artifact_root="$WDIO_OUT_RESOLVED/artifacts"
  retained_deb="$(bash scripts/lib/retain-linux-shipping-packages.sh "$bundle_root" "$artifact_root")"
  package_root="$WDIO_OUT_RESOLVED/package-root"
  test ! -e "$package_root" || {
    echo "isolated package root already exists: $package_root" >&2
    exit 1
  }
  mkdir -p "$package_root"
  dpkg-deb -x "$retained_deb" "$package_root"
  APP_BIN="$(find "$package_root" -type f -name shellx-cut -perm -u+x -print -quit)"
  test -n "$APP_BIN" && test -x "$APP_BIN" || {
    echo "the extracted shipping package has no shellx-cut executable" >&2
    exit 1
  }
  sha256sum "$artifact_root"/* "$APP_BIN" >"$WDIO_OUT_RESOLVED/shipping-package-sha256.txt"
  node scripts/linux-installed-walkthrough-receipt.mjs --start --source-commit "$SOURCE_COMMIT" --source-content-manifest "$SOURCE_CONTENT_MANIFEST_SHA256" --artifact "$APP_BIN" --package "$retained_deb" --pre-out "$WDIO_OUT_RESOLVED/installed-walkthrough-pre.json"
  WDIO_PROVIDER=external
  INSTALLED_APP=1
  NATIVE_PROVIDER=external
else
  npm --prefix ui run build
  (cd app && cargo build -p server --bin cutd)
  rust_target="$(rustc -vV | sed -n 's/^host: //p')"
  test -n "$rust_target"
  mkdir -p app/desktop/src-tauri/binaries
  cp app/target/debug/cutd "app/desktop/src-tauri/binaries/cutd-$rust_target"
  (cd app/desktop && cargo tauri build --debug --features webdriver-test --no-bundle --config '{"bundle":{"createUpdaterArtifacts":false}}')
  APP_BIN="$REMOTE_DIR_RESOLVED/app/desktop/src-tauri/target/debug/shellx-cut"
  test -x "$APP_BIN" || {
    echo "test app binary not found: $APP_BIN" >&2
    exit 1
  }
  WDIO_PROVIDER=embedded
  INSTALLED_APP=0
  NATIVE_PROVIDER=embedded
fi
XDG_RUNTIME_DIR="$runtime_dir" setsid xvfb-run -a -s '-screen 0 1600x900x24' dbus-run-session -- env \
  SHELLX_CUT_WDIO_APP="$REMOTE_DIR_RESOLVED/scripts/lib/run-isolated-native-app.sh" \
  SHELLX_CUT_WDIO_REAL_APP="$APP_BIN" \
  SHELLX_CUT_WDIO_APP_CWD="$app_cwd" \
  SHELLX_CUT_WDIO_PROVIDER="$WDIO_PROVIDER" \
  SHELLX_CUT_TAURI_DRIVER="$WDIO_TAURI_DRIVER_RESOLVED" \
  SHELLX_CUT_WDIO_CLIP="$WDIO_SCENE_CLIP_RESOLVED" \
  SHELLX_CUT_WDIO_LIBRARY_CLIP="$WDIO_SPEECH_CLIP_RESOLVED" \
  SHELLX_CUT_WDIO_IMAGE="$WDIO_IMAGE_RESOLVED" \
  SHELLX_CUT_WDIO_DROP_CASE=${JSON.stringify(dropCase)} \
  SHELLX_CUT_WDIO_OUT="$WDIO_OUT_RESOLVED" \
  SHELLX_CUT_WDIO_PORT=4445 \
  SHELLX_CUT_HOME="$WDIO_OUT_RESOLVED/app-home" \
  SHELLX_CUT_PROJECTS_DIR="$WDIO_OUT_RESOLVED/projects" \
  FCV_UI_DRIVER=tauri-wdio \
  FCV_NATIVE_PROVIDER="$NATIVE_PROVIDER" \
  FCV_NATIVE_ACTION_CONTROLLER="$REMOTE_DIR_RESOLVED/scripts/release/native-os-action-controller.mjs" \
  FCV_NATIVE_ACTION_PLATFORM=linux \
  SWEEP_CUTD=http://127.0.0.1:6161 \
  FCV_SECTION="$FCV_SECTION_VALUE" \
  FCV_ONLY="$FCV_ONLY_VALUE" \
  FCV_NO_AGENT="$FCV_NO_AGENT_VALUE" \
  FCV_REQUIRE_FULL="$FCV_REQUIRE_FULL_VALUE" \
  FCV_FINAL_ALL_ACTIONS="$FCV_STRICT_ACTIONS_VALUE" \
  FCV_REAL_SCREEN_RECORD="$FCV_REAL_SCREEN_RECORD_VALUE" \
  FCV_AGENT_FIXTURES="$FCV_AGENT_FIXTURES_VALUE" \
  FCV_ACTION_MANIFEST="$REMOTE_DIR_RESOLVED/ui/public-tests/full-ui-action-manifest.json" \
  FCV_SCREENS="$WDIO_OUT_RESOLVED/screens" \
  FCV_RESULT_RECEIPT="$WDIO_OUT_RESOLVED/full-coverage-receipt.json" \
  FCV_TARGET_SURFACE=linux-control \
  FCV_INSTALLED_APP="$INSTALLED_APP" \
  FCV_SOURCE_GIT_COMMIT="$SOURCE_COMMIT" \
  FCV_SOURCE_CONTENT_MANIFEST_SHA256="$SOURCE_CONTENT_MANIFEST_SHA256" \
  FCV_INSTALLED_RUNTIME_RECEIPT="$FCV_INSTALLED_RUNTIME_RECEIPT_VALUE" \
  FCV_TRACE="$FCV_TRACE_VALUE" \
  CUT_TEST_MEDIA_DIR="$(dirname "$WDIO_SCENE_CLIP_RESOLVED")" \
  RELEASE_CLIP="$WDIO_SCENE_CLIP_RESOLVED" \
  RELEASE_CLIP_SPEECH="$WDIO_SPEECH_CLIP_RESOLVED" \
  RELEASE_CLIP_FACE="$WDIO_FACE_CLIP_RESOLVED" \
  RELEASE_CLIP_SPEAKERS="$WDIO_SPEAKERS_CLIP_RESOLVED" \
  RELEASE_CLIP2="$WDIO_SECOND_CLIP_RESOLVED" \
  NODE_OPTIONS=--max-old-space-size=8192 WDIO_LOG_LEVEL=silent \
npm --prefix ui run "$WDIO_SCRIPT" &
gate_pid=$!
set +e; wait "$gate_pid"; gate_status=$?; set -e
if [ "$gate_status" = "0" ] && [ "$INSTALLED_FINAL" = "1" ]; then
  node scripts/linux-installed-walkthrough-receipt.mjs --finish --source-commit "$SOURCE_COMMIT" --source-content-manifest "$SOURCE_CONTENT_MANIFEST_SHA256" --pre "$WDIO_OUT_RESOLVED/installed-walkthrough-pre.json" --runtime "$WDIO_OUT_RESOLVED/installed-runtime-receipt.json" --full-coverage "$WDIO_OUT_RESOLVED/full-coverage-receipt.json" --integrity-out "$WDIO_OUT_RESOLVED/installed-artifact-integrity.json" --out "$WDIO_OUT_RESOLVED/installed-walkthrough-receipt.json"
fi
printf '%s\n' "$gate_status" >"$WDIO_OUT_RESOLVED/.wdio-exit-code"; exit "$gate_status"
`
  const envPrefix = [
    `REMOTE_DIR=${JSON.stringify(remoteDir)}`,
    `WDIO_SCENE_CLIP=${JSON.stringify(remoteSceneClip)}`,
    `WDIO_SPEECH_CLIP=${JSON.stringify(remoteSpeechClip)}`,
    `WDIO_IMAGE=${JSON.stringify(remoteImage)}`,
    `WDIO_FACE_CLIP=${JSON.stringify(remoteFaceClip)}`,
    `WDIO_SPEAKERS_CLIP=${JSON.stringify(remoteSpeakersClip)}`,
    `WDIO_SECOND_CLIP=${JSON.stringify(remoteSecondClip)}`,
    `WDIO_TAURI_DRIVER=${JSON.stringify(remoteTauriDriver)}`,
    `WDIO_WEBKIT_DRIVER=${JSON.stringify(remoteWebkitDriver)}`,
    `WDIO_OUT=${JSON.stringify(outDir)}`,
    `INSTALLED_FINAL=${installedFinal ? '1' : '0'}`,
    `SOURCE_COMMIT=${JSON.stringify(sourceHead)}`,
    `CLEAN_AFTER=${cleanAfter ? '1' : '0'}`,
    `WDIO_SCRIPT=${JSON.stringify(
      suite === 'adapter-smoke'
        ? 'wdio:native-adapter-smoke'
        : suite === 'drop-to-create'
          ? 'wdio:native-drop-to-create'
          : 'wdio:native-full-coverage',
    )}`,
    `FCV_SECTION_VALUE=${JSON.stringify(section)}`,
    `FCV_ONLY_VALUE=${JSON.stringify(only)}`,
    `FCV_TRACE_VALUE=${flag('--trace') ? '1' : '0'}`,
    `FCV_NO_AGENT_VALUE=${full ? '0' : '1'}`,
    `FCV_REQUIRE_FULL_VALUE=${full ? '1' : '0'}`,
    `FCV_STRICT_ACTIONS_VALUE=${strictCandidateActions || installedFinal ? '1' : '0'}`,
    `FCV_REAL_SCREEN_RECORD_VALUE=${installedFinal || realScreenRecord ? '1' : '0'}`,
    `FCV_AGENT_FIXTURES_VALUE=${full ? '1' : '0'}`,
  ].join(' ')
  const sshPayload = buildSshEnvPayload(anthropicKey, 'ANTHROPIC_API_KEY', envPrefix, remoteScript)
  await run('ssh', [...SSH_KEEPALIVE_ARGS, host, sshPayload.command], {
    cwd: homedir(),
    input: sshPayload.input,
  })
}

main().catch((error) => {
  console.error(error?.stack || error?.message || String(error))
  process.exit(1)
})
