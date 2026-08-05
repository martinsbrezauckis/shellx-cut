#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildSshEnvPayload, readEnvFirstLine, SSH_KEEPALIVE_ARGS } from './lib/ssh-stdin-env.mjs'
import { FULL_AGENT_FIXTURE_SHELL } from './lib/native-full-agent-fixture-env.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(__dirname, '..')

function arg(name, fallback = '') {
  const i = process.argv.indexOf(name)
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback
}

function flag(name) {
  return process.argv.includes(name)
}

function run(cmd, args, options = {}) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(cmd, args, {
      cwd: options.cwd || REPO_ROOT,
      env: { ...process.env, ...(options.env || {}) },
      stdio: options.input ? ['pipe', 'inherit', 'inherit'] : (options.capture ? ['ignore', 'pipe', 'pipe'] : 'inherit'),
    })
    let stdout = ''
    let stderr = ''
    if (child.stdout) child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    if (child.stderr) child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('exit', (code, signal) => {
      if (code === 0) resolveRun({ stdout, stderr })
      else reject(new Error(`${cmd} ${args.join(' ')} failed: code=${code} signal=${signal || 'none'}\n${stdout}${stderr}`))
    })
    if (options.input) {
      child.stdin.end(options.input)
    }
  })
}

function usage() {
  console.log(`Usage: node scripts/macos-wdio-track-controls.mjs [--suite track-controls|media-drag|drop-to-create|composed-playback|adapter-smoke|full-coverage] [--drop-case video|image|both] --host <ssh-host> [--remote-dir ~/Developer/shellx-cut] [--scene-clip <remote media path>] [--image <remote still path>] [--speech-clip <remote media path>] [--face-clip <remote media path>] [--speakers-clip <remote media path>] [--second-clip <remote media path>] [--out <remote evidence dir>] [--section <comma-list>] [--only <substring>] [--trace] [--full] [--strict-candidate-actions] [--real-screen-record] [--embedded-port 4445] [--wdio-log-level silent] [--skip-sync] [--native-input] [--clean-after]

Builds a macOS ShellX Cut debug app with --features webdriver-test and runs the selected WDIO native WKWebView suite on the configured macOS host. Media paths are resolved on the Mac host. This is internal test automation only; shipping build scripts reject webdriver-test.

Legacy --clip and --library-clip remain aliases for --scene-clip and
--speech-clip. --full requires the complete runtime and runs real agent actions.
--strict-candidate-actions also requires every registered UI action to pass, but
the receipt deliberately remains candidate-only (FCV_INSTALLED_APP=0).
When ANTHROPIC_API_KEY is set locally, it is forwarded over SSH stdin to the
test process only; the value is never placed in argv or written remotely.`)
}

async function main() {
  if (flag('--help') || flag('-h')) return usage()

  const suite = arg('--suite', 'track-controls')
  const dropCase = arg('--drop-case', 'both')
  if (!['video', 'image', 'both'].includes(dropCase)) {
    throw new Error(`unsupported drop case: ${dropCase}`)
  }
  const npmScript = suite === 'media-drag'
    ? 'wdio:mac-media-drag'
    : suite === 'drop-to-create'
      ? 'wdio:native-drop-to-create'
    : suite === 'composed-playback'
      ? 'wdio:mac-composed-playback'
      : suite === 'track-controls'
        ? 'wdio:mac-track-controls'
        : suite === 'adapter-smoke'
          ? 'wdio:native-adapter-smoke'
        : suite === 'full-coverage'
          ? 'wdio:native-full-coverage'
        : ''
  if (!npmScript) throw new Error(`unsupported suite: ${suite}`)
  const host = arg('--host', process.env.SHELLX_CUT_MAC_HOST || '')
  if (!host) throw new Error('configure the macOS SSH host with --host or SHELLX_CUT_MAC_HOST')
  const remoteDir = arg('--remote-dir', process.env.SHELLX_CUT_MAC_REMOTE_DIR || '~/Developer/shellx-cut')
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
  const outDir = arg('--out', process.env.SHELLX_CUT_WDIO_OUT || `~/.shellx-scratch/shellx-cut/wdio-${suite}-${stamp}`)
  const embeddedPort = arg('--embedded-port', process.env.WDIO_TAURI_PORT || '4445')
  const wdioLogLevel = arg('--wdio-log-level', process.env.WDIO_LOG_LEVEL || 'silent')
  const section = arg('--section', process.env.FCV_SECTION || '')
  const only = arg('--only', process.env.FCV_ONLY || '')
  const strictCandidateActions = flag('--strict-candidate-actions')
  const anthropicKey = readEnvFirstLine('ANTHROPIC_API_KEY')
  const full = flag('--full') || strictCandidateActions
  if (strictCandidateActions && (section || only || suite !== 'full-coverage')) {
    throw new Error('--strict-candidate-actions requires --suite full-coverage and forbids --section/--only')
  }
  const remotePath = (path) => !path || path.startsWith('/') || path.startsWith('~')
    ? path
    : `${remoteDir}/${path}`
  const remoteSceneClip = remotePath(sceneClip)
  const remoteSpeechClip = remotePath(speechClip)
  const remoteImage = remotePath(image)
  const remoteFaceClip = remotePath(faceClip)
  const remoteSpeakersClip = remotePath(speakersClip)
  const remoteSecondClip = remotePath(secondClip)

  const include = [
    'AGENTS.md',
    '.gitignore',
    'LICENSE',
    'NOTICE',
    'README.md',
    'START_HERE_FOR_AGENT.txt',
    'app/***',
    'docs/***',
    'schema/***',
    'scripts/***',
    'skill/***',
    'testdata/***',
    'ui/***',
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

  console.log(`Mac host: ${host}`)
  console.log(`Suite: ${suite}`)
  console.log(`Remote dir: ${remoteDir}`)
  console.log(`Evidence dir: ${outDir}`)

  if (!flag('--skip-sync')) {
    await run('rsync', rsyncArgs)
  }

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
export PATH="$HOME/.local/share/shellx-cut/test-tools/claude-code/node_modules/.bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$REMOTE_DIR_RESOLVED"
${FULL_AGENT_FIXTURE_SHELL}
if ioreg -n Root -d1 2>/dev/null |
  grep -E '"?CGSSessionScreenIsLocked"?[[:space:]]*=[[:space:]]*Yes' >/dev/null; then
  echo "the Mac console is locked; unlock it before native UI qualification" >&2
  exit 1
fi
mkdir -p "$WDIO_OUT_RESOLVED"
SOURCE_CONTENT_MANIFEST_SHA256="$(node scripts/source-content-manifest.mjs --out "$WDIO_OUT_RESOLVED/source-content-manifest.json" --sha256)"
if pgrep -x "ShellX Cut" >/dev/null 2>&1 || pgrep -x shellx-cut >/dev/null 2>&1 || pgrep -x cutd >/dev/null 2>&1; then
  echo "an existing ShellX Cut/cutd session is active; refusing to terminate or reuse it" >&2
  exit 1
fi
if lsof -nP -iTCP:6161 -sTCP:LISTEN >/dev/null 2>&1; then
  echo "127.0.0.1:6161 is already occupied; refusing to reuse another session" >&2
  exit 1
fi
cleanup_new_processes() {
  for process_name in "ShellX Cut" shellx-cut cutd; do
    for pid in $(pgrep -x "$process_name" 2>/dev/null || true); do
      kill "$pid" >/dev/null 2>&1 || true
    done
  done
  if [ "$CLEAN_AFTER" = "1" ]; then
    rm -rf app/target app/desktop/src-tauri/target ui/node_modules ui/dist \
      "$WDIO_OUT_RESOLVED/app-home" \
      "$WDIO_OUT_RESOLVED/app-cwd" \
      "$WDIO_OUT_RESOLVED/projects"
  fi
}
trap cleanup_new_processes EXIT

test -f "$WDIO_SCENE_CLIP_RESOLVED" || { echo "scene clip not found: $WDIO_SCENE_CLIP_RESOLVED" >&2; exit 1; }
test -f "$WDIO_SPEECH_CLIP_RESOLVED" || { echo "speech clip not found: $WDIO_SPEECH_CLIP_RESOLVED" >&2; exit 1; }
test -f "$WDIO_IMAGE_RESOLVED" || { echo "still image not found: $WDIO_IMAGE_RESOLVED" >&2; exit 1; }
for role_path in "$WDIO_FACE_CLIP_RESOLVED" "$WDIO_SPEAKERS_CLIP_RESOLVED" "$WDIO_SECOND_CLIP_RESOLVED"; do
  if [ -n "$role_path" ] && [ ! -f "$role_path" ]; then
    echo "explicit role clip not found: $role_path" >&2
    exit 1
  fi
done

npm --prefix ui install --no-audit --no-fund
npm --prefix ui run build

(cd app && cargo build -p server --bin cutd --target aarch64-apple-darwin)
mkdir -p app/desktop/src-tauri/binaries
cp app/target/aarch64-apple-darwin/debug/cutd app/desktop/src-tauri/binaries/cutd-aarch64-apple-darwin

(cd app/desktop && cargo tauri build --debug --target aarch64-apple-darwin --features webdriver-test --no-bundle --config '{"bundle":{"createUpdaterArtifacts":false}}')

APP_BIN="$REMOTE_DIR_RESOLVED/app/desktop/src-tauri/target/aarch64-apple-darwin/debug/shellx-cut"
if [ ! -x "$APP_BIN" ]; then
  APP_BIN="$REMOTE_DIR_RESOLVED/app/desktop/src-tauri/target/debug/shellx-cut"
fi
if [ ! -x "$APP_BIN" ]; then
  echo "test app binary not found" >&2
  find "$REMOTE_DIR_RESOLVED/app/desktop/src-tauri/target" -maxdepth 6 -type f -name 'shellx-cut' -print >&2 || true
  exit 1
fi
# Tauri rejects symlinked starting-binary paths on macOS. Exact-source rigs may
# reuse a Cargo target through a symlink, so launch the same binary by its
# physical path and keep resource_dir() pointed at the staged debug resources.
APP_BIN="$(cd "$(dirname "$APP_BIN")" && pwd -P)/$(basename "$APP_BIN")"
app_cwd="$WDIO_OUT_RESOLVED/app-cwd"
mkdir -p "$app_cwd"
chmod 700 "$app_cwd"

SHELLX_CUT_WDIO_APP="$REMOTE_DIR_RESOLVED/scripts/lib/run-isolated-native-app.sh" \
SHELLX_CUT_WDIO_REAL_APP="$APP_BIN" \
SHELLX_CUT_WDIO_APP_CWD="$app_cwd" \
SHELLX_CUT_WDIO_CLIP="$WDIO_SCENE_CLIP_RESOLVED" \
SHELLX_CUT_WDIO_LIBRARY_CLIP="$WDIO_SPEECH_CLIP_RESOLVED" \
SHELLX_CUT_WDIO_IMAGE="$WDIO_IMAGE_RESOLVED" \
SHELLX_CUT_WDIO_DROP_CASE=${JSON.stringify(dropCase)} \
SHELLX_CUT_WDIO_OUT="$WDIO_OUT_RESOLVED" \
SHELLX_CUT_WDIO_NATIVE_INPUT="$NATIVE_INPUT" \
SHELLX_CUT_WDIO_PORT=${JSON.stringify(embeddedPort)} \
SHELLX_CUT_HOME="$WDIO_OUT_RESOLVED/app-home" \
SHELLX_CUT_PROJECTS_DIR="$WDIO_OUT_RESOLVED/projects" \
FCV_UI_DRIVER=tauri-wdio \
FCV_NATIVE_ACTION_CONTROLLER="$REMOTE_DIR_RESOLVED/scripts/release/native-os-action-controller.mjs" \
FCV_NATIVE_ACTION_PLATFORM=macos \
FCV_NATIVE_EXPECTED_PROCESS=shellx-cut \
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
FCV_TARGET_SURFACE=macos-installed \
FCV_INSTALLED_APP=0 \
FCV_SOURCE_CONTENT_MANIFEST_SHA256="$SOURCE_CONTENT_MANIFEST_SHA256" \
FCV_TRACE="$FCV_TRACE_VALUE" \
CUT_TEST_MEDIA_DIR="$(dirname "$WDIO_SCENE_CLIP_RESOLVED")" \
RELEASE_CLIP="$WDIO_SCENE_CLIP_RESOLVED" \
RELEASE_CLIP_SPEECH="$WDIO_SPEECH_CLIP_RESOLVED" \
RELEASE_CLIP_FACE="$WDIO_FACE_CLIP_RESOLVED" \
RELEASE_CLIP_SPEAKERS="$WDIO_SPEAKERS_CLIP_RESOLVED" \
RELEASE_CLIP2="$WDIO_SECOND_CLIP_RESOLVED" \
NODE_OPTIONS=--max-old-space-size=8192 \
WDIO_LOG_LEVEL=${JSON.stringify(wdioLogLevel)} \
/usr/bin/caffeinate -dimsu -t 14400 npm --prefix ui run "$WDIO_SCRIPT"
`

  const envPrefix = [
    `REMOTE_DIR=${JSON.stringify(remoteDir)}`,
    `WDIO_SCENE_CLIP=${JSON.stringify(remoteSceneClip)}`,
    `WDIO_SPEECH_CLIP=${JSON.stringify(remoteSpeechClip)}`,
    `WDIO_IMAGE=${JSON.stringify(remoteImage)}`,
    `WDIO_FACE_CLIP=${JSON.stringify(remoteFaceClip)}`,
    `WDIO_SPEAKERS_CLIP=${JSON.stringify(remoteSpeakersClip)}`,
    `WDIO_SECOND_CLIP=${JSON.stringify(remoteSecondClip)}`,
    `WDIO_OUT=${JSON.stringify(outDir)}`,
    `WDIO_SCRIPT=${JSON.stringify(npmScript)}`,
    `NATIVE_INPUT=${flag('--native-input') ? '1' : '0'}`,
    `CLEAN_AFTER=${flag('--clean-after') ? '1' : '0'}`,
    `FCV_SECTION_VALUE=${JSON.stringify(section)}`,
    `FCV_ONLY_VALUE=${JSON.stringify(only)}`,
    `FCV_TRACE_VALUE=${flag('--trace') ? '1' : '0'}`,
    `FCV_NO_AGENT_VALUE=${full ? '0' : '1'}`,
    `FCV_REQUIRE_FULL_VALUE=${full ? '1' : '0'}`,
    `FCV_STRICT_ACTIONS_VALUE=${strictCandidateActions ? '1' : '0'}`,
    `FCV_REAL_SCREEN_RECORD_VALUE=${flag('--real-screen-record') || strictCandidateActions ? '1' : '0'}`,
    `FCV_AGENT_FIXTURES_VALUE=${full ? '1' : '0'}`,
  ].join(' ')
  const sshPayload = buildSshEnvPayload(anthropicKey, 'ANTHROPIC_API_KEY', envPrefix, remoteScript)
  await run('ssh', [...SSH_KEEPALIVE_ARGS, host, sshPayload.command], {
    input: sshPayload.input,
    capture: false,
    cwd: homedir(),
  }).catch(async (err) => {
    throw err
  })
}

main().catch((err) => {
  console.error(err?.stack || err?.message || String(err))
  process.exit(1)
})
