#!/usr/bin/env node
// full-coverage-gate.mjs — the stack-launching WRAPPER that turns the exhaustive
// UI gate (ui/public-tests/full-coverage-verify.mjs) into a one-command, per-surface
// release gate. full-coverage-verify.mjs walks every user-facing control with a
// 4-part PRESENT/RENDER/CLICK/
// RESULT verdict — see its header); this wrapper cold-starts the stack it needs.
//
// ROLE
//   1. Cold-start an ISOLATED release-binary cutd that serves the built ui/dist
//      AND the verb API on ONE free loopback port (the production topology —
//      vite.config.ts: "Prod: npm run build → ui/dist, served BY cutd itself at
//      /; relative /api paths just work"; client.ts API_BASE='' = same origin).
//      So SWEEP_CUTD === SWEEP_APP === http://127.0.0.1:<port>. No vite needed.
//   2. OR drive an EXTERNAL already-running stack when SWEEP_CUTD is set in env
//      (a rig with the installed app / a shared dev engine) — then it does NOT
//      cold-start and does NOT tear anything down.
//   3. Export the env the suite needs (SHELLX_CUT_SIDECAR_DIR, CUT_TEST_MEDIA_DIR,
//      SHELLX_CUT_PYTHON pass-through) and PRE-FLIGHT the resolved ffmpeg for the
//      `zscale` (libzimg) filter — a zscale-less ffmpeg silently breaks every
//      rec2020 / color_space render+export (the colour-managed output path).
//   3b. FULL-VERIFY GATE (the zero-N/A standard). As a RELEASE gate this wrapper
//      defaults to FCV_REQUIRE_FULL=1 — exported into the suite's env so the suite
//      hard-fails on a missing dependency / unverifiable control instead of
//      emitting a quiet N/A — and runs a COMPLETE-ENV PRE-FLIGHT *before* launching
//      the suite that FAILS (non-zero, with an itemised list) if any of the things
//      a full run needs is absent: `claude` on PATH (agent.chat / translate /
//      judge), the perception venv importing the full sidecar battery (cv2,
//      torch/torchvision/torchaudio, silero-vad, PySceneDetect,
//      supervision, RapidOCR for redact / auto-zoom / silence / scenes / beats /
//      OCR / transcribe), the local RVM matte runtime (Python deps + runner +
//      model), the diarize (:9002) + dub (:9001) endpoints responding, a
//      zscale-enabled ffmpeg first on PATH, and the real
//      high-quality SCENE/SPEECH/SPEAKERS clips in CUT_TEST_MEDIA_DIR. So "run the
//      gate" means "run it FULLY, or fail" — never pass on a degraded subset.
//      Dev opt-out: FCV_REQUIRE_FULL=0 (caller-set only) demotes the misses to
//      warnings and runs a DEGRADED scope that is NOT a release verdict.
//   4. Run `node public-tests/full-coverage-verify.mjs`, surface its pass/fail summary,
//      write a per-surface receipt, and tear the cold-started stack down BY EXACT
//      PID (process-group kill — never `pkill -f`, which would also kill our own
//      ssh/node on a rig).
//   5. Exit non-zero on any real FAIL (the suite's exit code is propagated).
//
// HEAVY — COLD-START MODE REFUSES WSL BY DEFAULT. The suite spawns torch/ffmpeg
// per check, which can exhaust a constrained WSL environment. Set FCV_ALLOW_WSL=1
// only when the host is provisioned for the full workload. Run platform evidence
// directly on macOS, Windows (real WebView2), and native Linux.
// EXTERNAL-stack mode (SWEEP_CUTD set) is allowed even from WSL, because then the
// heavy cutd runs on the external host, not here — the wrapper is only the driver.
//
// USAGE
//   Cold-start (on a rig — builds ui/dist if missing, picks a free port):
//     node scripts/release/full-coverage-gate.mjs
//   External stack (drive an already-running cutd+UI; e.g. installed app on :6161):
//     # Start cutd with the release fixture PATH/adapters first, then acknowledge
//     # that inherited environment to the driver wrapper:
//     FCV_EXTERNAL_FIXTURES_READY=1 SWEEP_CUTD=http://127.0.0.1:6161 \
//       node scripts/release/full-coverage-gate.mjs
//     SWEEP_CUTD=http://127.0.0.1:6171 SWEEP_APP=http://localhost:5173 \
//       node scripts/release/full-coverage-gate.mjs   # split origin (vite UI)
//   Flags / env:
//     --build / FCV_BUILD=1        force `npm run build` even if ui/dist exists
//     --no-build / FCV_SKIP_BUILD=1  never build (cold-start fails if dist absent)
//     FCV_ALLOW_WSL=1              override the WSL cold-start refusal (not advised)
//     FCV_EXTERNAL_FIXTURES_READY=1 assert that an external cutd was launched with
//                                  the fixture PATH and all CUTD_*_ADAPTER vars;
//                                  required when external + fixture mode are active
//     FCV_SURFACE=<macos-installed|windows-installed|linux-control>
//                                  label an external cross-host run
//                                  with the target surface instead of the driver's OS;
//                                  rejected for cold starts and on unknown values
//     FCV_REQUIRE_FULL=0           DEV OPT-OUT — demote the complete-env pre-flight
//                                  misses to warnings + drop the suite out of full
//                                  mode (default is 1: full verification REQUIRED,
//                                  any missing dependency FAILS the gate)
//     --final-all-actions / FCV_FINAL_ALL_ACTIONS=1
//                                  FINAL PRE-RELEASE mode: refuses filters and
//                                  requires every receipt row to pass all four
//                                  PRESENT/RENDER/CLICK/RESULT dimensions
//     FCV_INSTALLED_APP=1          records that the target is the installed app;
//                                  required by final-all-actions mode
//     FCV_UI_DRIVER=<id>           native installed driver identity; required by
//                                  final-all-actions mode and may not be
//                                  playwright-chromium
//     FCV_CDP_URL=<url>             actual installed WebView2 CDP endpoint when
//                                  FCV_UI_DRIVER=webview2-cdp.
//     FCV_NATIVE_PROVIDER=external accepted with FCV_UI_DRIVER=tauri-wdio on
//                                  native Linux/Windows, where official
//                                  tauri-driver attaches to the shipping binary.
//     FCV_ACTION_MANIFEST=<path>   committed source-action manifest; final mode
//                                  fails when source, native sweep, or manifest drift
//     FCV_REQUIRE_ZSCALE=1         turn the zscale pre-flight WARN into a hard FAIL
//                                  (note: under the FCV_REQUIRE_FULL=1 default the
//                                  complete-env pre-flight already fails on a
//                                  zscale-less ffmpeg)
//     CUT_DIARIZE_ENDPOINT, CUT_DUB_ENDPOINT  diarize/dub services the pre-flight
//                                  health-probes (defaults :9002 / :9001)
//     RELEASE_CLIP, RELEASE_CLIP_SPEECH, RELEASE_CLIP_SPEAKERS  override the real
//                                  SCENE/SPEECH/SPEAKERS clip paths the pre-flight
//                                  (and the suite) require
//     CUTD_BIN=<path>              cutd binary (default app/target/release/cutd,
//                                  falls back to app/target/debug/cutd)
//     FCV_RECEIPT=<path>           where to write the JSON receipt (default
//                                  <repo>/.shellx-scratch/full-coverage/receipt-*.json)
//     SHELLX_CUT_PYTHON, SHELLX_CUT_SIDECAR_DIR, CUT_TEST_MEDIA_DIR, FCV_SECTION,
//     FCV_ONLY, FCV_NO_AGENT, RELEASE_CLIP* — passed straight through to the suite.
//
// Callers: automated per-surface qualification and developers running the gate.
// Dependencies: node 18+ (global fetch), the release cutd binary, a built ui/dist
// (or --build), Playwright (installed under ui/), ffmpeg on PATH.

import { spawn, spawnSync } from 'node:child_process'
import { createServer } from 'node:net'
import { accessSync, constants, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { delimiter, dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { resolveDriverPath } from '../lib/cross-host-media.mjs'
import { resolveTestProjectsIsolation } from '../lib/test-project-isolation.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '..', '..') // scripts/release → repo root
const UI_DIR = join(REPO, 'ui')
const UI_DIST = join(UI_DIR, 'dist')
const SIDECAR_DEFAULT = join(REPO, 'app', 'perception', 'py') // dir that DIRECTLY holds instruments.py
const FIXTURE_DIR = join(REPO, 'scripts', 'release', 'fixtures')
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const log = (...m) => console.log('[fcv-gate]', ...m)
const warn = (...m) => console.warn('[fcv-gate] ⚠', ...m)

// ── surface label (for the receipt) ──────────────────────────────────────────
// darwin → macos, win32 → windows, linux → linux-control. External cross-host
// drivers may provide an explicit target label; cold starts cannot override the
// local platform because that would make the receipt lie about where work ran.
// Kept public-safe (no host names / IPs) so the receipt can be shared.
const SURFACE_LABELS = new Set(['macos-installed', 'windows-installed', 'linux-control'])
export function surfaceLabel({
  platform = process.platform,
  override = process.env.FCV_SURFACE || '',
  external = Boolean(process.env.SWEEP_CUTD),
} = {}) {
  const explicit = String(override).trim()
  if (explicit) {
    if (!external) throw new Error('FCV_SURFACE is only valid when SWEEP_CUTD selects an external stack')
    if (!SURFACE_LABELS.has(explicit)) {
      throw new Error(`FCV_SURFACE must be one of: ${[...SURFACE_LABELS].join(', ')}`)
    }
    return explicit
  }
  if (platform === 'darwin') return 'macos-installed'
  if (platform === 'win32') return 'windows-installed'
  return 'linux-control'
}

export function assessFinalAllActionsConfig({
  finalAllActions,
  requireFull,
  filtered,
  installedApp,
  uiDriver,
  cdpUrl,
  nativeProvider,
  platform = process.platform,
} = {}) {
  if (!finalAllActions) return { ok: true, missing: [] }
  const missing = []
  if (!requireFull) missing.push('FCV_REQUIRE_FULL must remain 1')
  if (filtered) missing.push('FCV_SECTION, FCV_ONLY and FCV_NO_AGENT are forbidden')
  if (!installedApp) missing.push('FCV_INSTALLED_APP=1 is required')
  if (uiDriver === 'webview2-cdp') {
    if (!String(cdpUrl || '').trim()) {
      missing.push('FCV_CDP_URL must point at the running installed WebView2 CDP endpoint')
    }
  } else if (
    uiDriver === 'tauri-wdio'
    && nativeProvider === 'external'
    && (platform === 'linux' || platform === 'win32')
  ) {
    // Official external tauri-driver attaches to the uninstrumented shipping
    // binary on Linux/Windows. macOS has no official external WKWebView backend.
  } else {
    missing.push(
      'use installed WebView2 CDP, or FCV_UI_DRIVER=tauri-wdio with ' +
      'FCV_NATIVE_PROVIDER=external on native Linux/Windows',
    )
  }
  return { ok: missing.length === 0, missing }
}

// ── WSL host detection ────────────────────────────────────────────────────────
// True only on a Microsoft WSL kernel. A native Linux release rig does NOT match,
// so a cold-start there is never wrongly refused. Signals: the WSL kernel marker
// in /proc/version + the WSL interop runtime. Either is sufficient.
function isWslHost() {
  if (process.platform !== 'linux') return false
  try {
    if (/microsoft|wsl/i.test(readFileSync('/proc/version', 'utf8'))) return true
  } catch { /* /proc unreadable → fall through to the interop check */ }
  return Boolean(process.env.WSL_INTEROP) || existsSync('/run/WSL')
}

// ── resolve the cutd binary (release preferred) ───────────────────────────────
function resolveCutd() {
  if (process.env.CUTD_BIN) return process.env.CUTD_BIN
  const rel = join(REPO, 'app', 'target', 'release', 'cutd')
  const dbg = join(REPO, 'app', 'target', 'debug', 'cutd')
  if (existsSync(rel)) return rel
  if (existsSync(dbg)) {
    warn(`release cutd missing — falling back to the DEBUG binary (${dbg}). A release gate should run the release binary; build it with: cd app && cargo build --release -p server`)
    return dbg
  }
  return rel // report the expected release path in the not-found error
}

// ── pick a free loopback port (ephemeral bind → read → close) ─────────────────
function freePort() {
  return new Promise((res, rej) => {
    const srv = createServer()
    srv.once('error', rej)
    srv.listen(0, '127.0.0.1', () => {
      const { port } = srv.address()
      srv.close(() => res(port))
    })
  })
}

// ── ffmpeg zscale (libzimg) pre-flight ────────────────────────────────────────
// Colour-managed renders/exports (rec2020 working space, color_space output)
// route through ffmpeg's `zscale` filter. A zscale-less ffmpeg does NOT error —
// it silently drops the colour conversion, so the colour output is wrong with no
// signal. The app auto-fetches BtbN's GPL ffmpeg (which DOES include zscale), but
// a system ffmpeg earlier on PATH may not — so we check the ffmpeg that will
// actually be used. WARN by default (the suite's own colour RESULT checks will
// fail on real breakage); FCV_REQUIRE_ZSCALE=1 makes it a hard pre-flight FAIL.
function checkZscale() {
  const ff = process.env.FFMPEG_BIN || 'ffmpeg'
  const r = spawnSync(ff, ['-hide_banner', '-filters'], { encoding: 'utf8' })
  if (r.error || r.status !== 0) {
    const msg = `could not run '${ff} -filters' to verify zscale (${r.error ? r.error.message : `exit ${r.status}`}); colour-managed renders/exports may be unverifiable`
    if (process.env.FCV_REQUIRE_ZSCALE === '1') { console.error(`[fcv-gate] ✗ ${msg}`); process.exit(3) }
    warn(msg)
    return { ok: false, ffmpeg: ff, reason: 'ffmpeg-unavailable' }
  }
  const has = /\bzscale\b/.test(r.stdout)
  if (has) { log(`zscale pre-flight: ffmpeg '${ff}' HAS zscale (libzimg) — colour-managed output supported`); return { ok: true, ffmpeg: ff } }
  const msg = `ffmpeg '${ff}' has NO zscale (libzimg) filter — ALL rec2020 / color_space renders+exports will be silently wrong. Install a zscale-enabled ffmpeg (BtbN GPL build) and put it first on PATH.`
  if (process.env.FCV_REQUIRE_ZSCALE === '1') { console.error(`[fcv-gate] ✗ ${msg}`); process.exit(3) }
  warn(msg)
  return { ok: false, ffmpeg: ff, reason: 'no-zscale' }
}

const PERCEPTION_FULL_IMPORTS = [
  'cv2',
  'torch',
  'torchvision',
  'torchaudio',
  'silero_vad',
  'scenedetect',
  'supervision',
  'rapidocr_onnxruntime',
]

// ── COMPLETE-ENV PRE-FLIGHT helpers (the full-verify, zero-N/A standard) ───────
// Each probe is a small, side-effect-free check that returns { ok, ... }. They
// feed assessCompleteEnv() (a PURE verdict, exported + unit-testable) so the gate
// logic can be proven without the heavy deps physically present.

// Is `cmd` an executable on PATH? Cross-platform (PATHEXT on Windows) and does NOT
// execute the program (so probing `claude` never spends a turn or hits auth).
function environmentValue(env, name) {
  const key = Object.keys(env).find((candidate) => candidate.toLowerCase() === name.toLowerCase())
  return key ? env[key] : ''
}

function onPath(cmd, env = process.env) {
  const dirs = environmentValue(env, 'PATH').split(delimiter).filter(Boolean)
  const exts = process.platform === 'win32'
    ? (environmentValue(env, 'PATHEXT') || '.EXE;.CMD;.BAT;.COM').split(';')
    : ['']
  const mode = process.platform === 'win32' ? constants.F_OK : constants.X_OK
  for (const dir of dirs) {
    for (const ext of exts) {
      const full = join(dir, cmd + ext)
      try { accessSync(full, mode); return full } catch { /* keep looking */ }
    }
  }
  return null
}

export function prependEnvPath(env, entry, { platform = process.platform } = {}) {
  const pathKeys = Object.keys(env).filter((key) => key.toLowerCase() === 'path')
  const key = pathKeys[0] || (platform === 'win32' ? 'Path' : 'PATH')
  const previous = pathKeys.map((candidate) => env[candidate]).find(Boolean) || ''
  for (const duplicate of pathKeys) {
    if (duplicate !== key) delete env[duplicate]
  }
  const separator = platform === 'win32' ? ';' : delimiter
  env[key] = previous ? `${entry}${separator}${previous}` : entry
  return key
}

// claude CLI — agent.chat / audio.dub-translate / LLM-judge all shell out to it.
function probeClaude(env = process.env) {
  const found = onPath('claude', env)
  return found ? { ok: true, path: found } : { ok: false, detail: 'not on PATH' }
}

// Perception venv must import the full sidecar modules used by the release gate:
// CV/detector deps (redact / auto-zoom / director sheets), silence/scenes/beats
// deps, OCR redaction, plus Canary timestamp alignment. STT readiness is proven
// by the suite through system.doctor and transcribe controls.
// SHELLX_CUT_PYTHON must point at that venv's python.
function probePerception(env = process.env) {
  const py = environmentValue(env, 'SHELLX_CUT_PYTHON')
  if (!py) return { ok: false, detail: 'SHELLX_CUT_PYTHON is not set — point it at the perception venv python' }
  const driverPy = resolveDriverPath(py)
  const r = spawnSync(driverPy, ['-c', `import ${PERCEPTION_FULL_IMPORTS.join(', ')}`], { encoding: 'utf8' })
  if (r.error) return { ok: false, python: py, driverPython: driverPy, detail: `cannot run '${driverPy}': ${r.error.message}` }
  if (r.status !== 0) {
    const why = (r.stderr || '').trim().split('\n').pop() || `exit ${r.status}`
    return { ok: false, python: py, driverPython: driverPy, detail: why }
  }
  return { ok: true, python: py, driverPython: driverPy, modules: PERCEPTION_FULL_IMPORTS }
}

function matteDataDir(env, platform = process.platform) {
  if (platform === 'win32') {
    const local = environmentValue(env, 'LOCALAPPDATA')
    return local ? join(local, 'ShellX Cut', 'matte') : null
  }
  const home = environmentValue(env, 'HOME')
  if (platform === 'darwin') {
    return home ? join(home, 'Library', 'Application Support', 'ShellX Cut', 'matte') : null
  }
  const dataHome = environmentValue(env, 'XDG_DATA_HOME') || (home ? join(home, '.local', 'share') : '')
  return dataHome ? join(dataHome, 'shellx-cut', 'matte') : null
}

function configuredMatteModel(env) {
  const explicit = environmentValue(env, 'MATTE_MODEL')
  if (explicit) return explicit
  const dir = matteDataDir(env)
  if (!dir) return ''
  try {
    const settings = JSON.parse(readFileSync(join(dir, 'settings.json'), 'utf8'))
    if (typeof settings?.model === 'string' && settings.model.trim()) return settings.model
  } catch { /* absent or invalid settings fall back to the managed model */ }
  return join(dir, 'rvm.onnx')
}

// Mirror system.doctor's `matte` card before spending the full browser sweep:
// the configured perception Python must carry the RVM dependencies, and the
// shippable runner plus a managed/explicit model must exist.
function probeMatte(env = process.env) {
  const python = environmentValue(env, 'MATTE_RUNNER_PY') || environmentValue(env, 'SHELLX_CUT_PYTHON')
  const sidecarDir = environmentValue(env, 'SHELLX_CUT_SIDECAR_DIR') || SIDECAR_DEFAULT
  const script = environmentValue(env, 'MATTE_RUNNER_SCRIPT') || join(sidecarDir, 'matte_runner.py')
  const model = configuredMatteModel(env)
  if (!python) return { ok: false, detail: 'MATTE_RUNNER_PY/SHELLX_CUT_PYTHON is not set' }
  if (!model) return { ok: false, python, script, detail: 'cannot resolve the managed matte model directory' }

  const driverPython = resolveDriverPath(python)
  const driverScript = resolveDriverPath(script)
  const driverModel = resolveDriverPath(model)
  const missing = []
  if (!existsSync(driverScript)) missing.push(`runner missing at ${script}`)
  if (!existsSync(driverModel)) missing.push(`RVM model missing at ${model}`)
  const imports = spawnSync(driverPython, ['-c', 'import onnxruntime, numpy, PIL'], { encoding: 'utf8' })
  if (imports.error || imports.status !== 0) {
    const why = imports.error?.message || (imports.stderr || '').trim().split('\n').pop() || `exit ${imports.status}`
    missing.push(`Python cannot import onnxruntime/numpy/PIL: ${why}`)
  }
  return {
    ok: missing.length === 0,
    python,
    driverPython,
    script,
    model,
    ...(missing.length ? { detail: missing.join('; ') } : {}),
  }
}

// Loopback sidecar health-probe. These model services have a real `/health`
// contract, so accepted TCP or an arbitrary HTTP response is not enough. Stale SSH
// tunnels can accept and then reset; loading/misrouted services can return non-2xx.
// Release preflight requires a successful health response before the heavy sweep.
async function probeEndpoint(label, base) {
  try {
    const r = await fetch(`${base}/health`, { signal: AbortSignal.timeout(3000) })
    const status = r.status
    return {
      ok: status >= 200 && status < 300,
      label,
      endpoint: base,
      ...(status >= 200 && status < 300 ? {} : { detail: `/health returned HTTP ${status}` }),
    }
  } catch (e) {
    return { ok: false, label, endpoint: base, detail: e.message }
  }
}

// The real high-quality clip ROLES the suite resolves from CUT_TEST_MEDIA_DIR
// (default names mirror full-coverage-verify.mjs's media() exactly, incl. the
// RELEASE_CLIP* overrides) — a fixture fallback means the 4K/perf class is NOT
// exercised, which the full-verify standard forbids.
function probeClips(mediaDir, env = process.env) {
  const roles = [
    { role: 'SCENE', envVar: 'RELEASE_CLIP', name: '20260618_172347.mp4' },
    { role: 'SPEECH', envVar: 'RELEASE_CLIP_SPEECH', name: 'talkinghead_hq.mp4' },
    { role: 'FACE', envVar: 'RELEASE_CLIP_FACE', name: 'face_hq.mp4' },
    { role: 'SPEAKERS', envVar: 'RELEASE_CLIP_SPEAKERS', name: 'podcast_2speakers.mp4' },
  ]
  return roles.map(({ role, envVar, name }) => {
    const override = env[envVar]
    const path = override || join(mediaDir, name)
    return { role, envVar, path, ok: existsSync(path) }
  })
}

// External-agent seams are intentionally expensive/non-deterministic: agent.chat and
// translate shell out to a subscription CLI, assets.generate may not expose image
// tools in the current session, and comment.draft depends on a language model choosing
// an actionable edit. Release coverage still needs real app effects, so the gate
// defaults to deterministic local fixtures in full mode. They exercise the same
// product paths: the "claude" fixture translates text and drives agent.chat edits via
// POST /api/verb, assets.generate spawns a "codex" executable that writes a valid
// media file, comment.apply executes a drafted edit returned by CUTD_DRAFT_ADAPTER,
// and verify.judge runs the normal async job/receipt path through CUTD_JUDGE_ADAPTER.
// Set FCV_AGENT_FIXTURES=0 to exercise the live external agents instead.
function prepareAgentFixtures(env, { requireFull }) {
  const mode = env.FCV_AGENT_FIXTURES ?? (requireFull ? '1' : '0')
  env.FCV_AGENT_FIXTURES = mode
  if (mode !== '1') return { active: false }
  const claudeFixture = join(FIXTURE_DIR, process.platform === 'win32' ? 'claude.cmd' : 'claude')
  const draftAdapter = env.CUTD_DRAFT_ADAPTER || join(FIXTURE_DIR, 'comment-draft-adapter.py')
  const judgeAdapter = env.CUTD_JUDGE_ADAPTER || join(FIXTURE_DIR, 'judge-adapter.py')
  prependEnvPath(env, FIXTURE_DIR)
  env.CUTD_DRAFT_ADAPTER = draftAdapter
  env.CUTD_JUDGE_ADAPTER = judgeAdapter
  // Keep the deterministic provider alive long enough for the native Generate
  // surface to expose and exercise jobs.cancel before the fixture writes its
  // image. This adds no network/provider work and is bounded to fixture mode.
  env.CUTD_GENERATE_FIXTURE_DELAY_MS ??= '1200'
  return { active: true, dir: FIXTURE_DIR, claudeFixture, draftAdapter, judgeAdapter }
}

// PURE verdict over the probe results. requireFull=true (the RELEASE default) =>
// any miss FAILS (returned in `missing`); requireFull=false (explicit dev opt-out)
// => the same misses are reported but `ok` is forced true (degraded, non-release).
// Exported so the fail/pass gate logic is unit-testable with injected inputs.
export function assessCompleteEnv(checks, { requireFull } = {}) {
  const missing = []
  if (!checks.claude?.ok) missing.push(`claude CLI ${checks.claude?.detail || 'missing'} — agent.chat / translate / LLM-judge can't run (install the Claude Code CLI or add it to PATH)`)
  if (!checks.perception?.ok) missing.push(`perception venv can't import full sidecar modules (${PERCEPTION_FULL_IMPORTS.join(', ')}) for redact / auto-zoom / director sheets / silence / scenes / beats / OCR / transcribe: ${checks.perception?.detail || 'unavailable'}`)
  if (!checks.matte?.ok) missing.push(`matte runtime is incomplete for edit.matte (requires onnxruntime/numpy/Pillow, the bundled matte runner, and an RVM model): ${checks.matte?.detail || 'unavailable'} — run system.setup_matte or set MATTE_MODEL`)
  if (!checks.diarize?.ok) missing.push(`diarize service did not respond at ${checks.diarize?.endpoint}/health (speaker diarization) — start the Sortformer service or set/tunnel CUT_DIARIZE_ENDPOINT`)
  if (!checks.dub?.ok) missing.push(`dub service did not respond at ${checks.dub?.endpoint}/health (audio.dub / re-voice) — start the OmniVoice service or set/tunnel CUT_DUB_ENDPOINT`)
  if (!checks.zscale?.ok) missing.push(`ffmpeg first on PATH ('${checks.zscale?.ffmpeg}') has no zscale (libzimg) filter — colour-managed rec2020 / color_space render+export would be silently wrong; put a zscale-enabled (BtbN GPL) ffmpeg first on PATH`)
  for (const clip of checks.clips || []) {
    if (!clip.ok) missing.push(`real ${clip.role} clip missing at ${clip.path} — the full gate must run on real high-quality footage, not a testdata fixture (set ${clip.envVar} or place the clip in CUT_TEST_MEDIA_DIR)`)
  }
  return { ok: requireFull ? missing.length === 0 : true, enforced: Boolean(requireFull), missing }
}

// An external cutd was launched before this wrapper, so mutating `env` below can
// configure only the Playwright driver. It cannot inject fixture PATH/adapters
// into that already-running engine. Require the rig launcher to acknowledge that
// it started cutd with the matching fixture environment instead of accepting a
// locally-probed fixture as proof of the remote engine's configuration.
export function assessExternalFixtureContract({ external, fixtureActive, acknowledged }) {
  if (!external || !fixtureActive || acknowledged) return { ok: true, missing: [] }
  return {
    ok: false,
    missing: [
      'external cutd fixture environment is unconfirmed — launch the external engine with the release-fixture PATH plus CUTD_DRAFT_ADAPTER, CUTD_JUDGE_ADAPTER, CUTD_GENERATE_PROMPT_ADAPTER, and CUTD_GENERATE_STORYBOARD_ADAPTER, then set FCV_EXTERNAL_FIXTURES_READY=1 for this wrapper run',
    ],
  }
}

// ── build ui/dist when needed ─────────────────────────────────────────────────
function ensureUiDist() {
  const haveDist = existsSync(join(UI_DIST, 'index.html'))
  const wantBuild = flag('--build') || process.env.FCV_BUILD === '1'
  const skipBuild = flag('--no-build') || process.env.FCV_SKIP_BUILD === '1'
  if (haveDist && !wantBuild) { log(`ui/dist present (${UI_DIST}) — using it (pass --build to rebuild)`); return }
  if (skipBuild) {
    if (!haveDist) { console.error(`[fcv-gate] ✗ ui/dist missing and --no-build set — build it first: cd ui && npm run build`); process.exit(3) }
    return
  }
  log(`building ui/dist (cd ui && npm run build)…`)
  const r = spawnSync('npm', ['run', 'build'], { cwd: UI_DIR, stdio: 'inherit' })
  if (r.status !== 0) { console.error('[fcv-gate] ✗ ui build failed — cannot serve a stale/absent bundle'); process.exit(3) }
  if (!existsSync(join(UI_DIST, 'index.html'))) { console.error('[fcv-gate] ✗ ui build reported success but dist/index.html is missing'); process.exit(3) }
}

// ── poll cutd until it answers GET /api/verbs ─────────────────────────────────
async function waitForCutd(base, deadlineMs, child) {
  const deadline = Date.now() + deadlineMs
  while (Date.now() < deadline) {
    if (child && child.exitCode !== null) throw new Error(`cutd exited during startup (code ${child.exitCode}) — see the cutd log`)
    try {
      const r = await fetch(`${base}/api/verbs`, { signal: AbortSignal.timeout(2000) })
      if (r.ok) return true
    } catch { /* not up yet */ }
    await sleep(400)
  }
  return false
}

// ── receipt ───────────────────────────────────────────────────────────────────
function writeReceipt(receipt) {
  const dir = join(REPO, '.shellx-scratch', 'full-coverage')
  const out = process.env.FCV_RECEIPT || join(dir, `receipt-${receipt.surface}-${receipt.startedAt.replace(/[:.]/g, '-')}.json`)
  mkdirSync(dirname(out), { recursive: true })
  writeFileSync(out, JSON.stringify(receipt, null, 2), 'utf8')
  log(`receipt → ${out}`)
}

async function main() {
  const surface = surfaceLabel()
  const startedAt = new Date().toISOString()
  const receiptStem = `${surface}-${startedAt.replace(/[:.]/g, '-')}`
  const receiptDir = join(REPO, '.shellx-scratch', 'full-coverage')
  const resultReceipt = process.env.FCV_RESULT_RECEIPT || join(receiptDir, `results-${receiptStem}.json`)
  const external = Boolean(process.env.SWEEP_CUTD)
  const mode = external ? 'external-stack' : 'cold-start'
  // RELEASE-GATE DEFAULT: full verification REQUIRED. Only an explicit caller
  // opt-out (FCV_REQUIRE_FULL=0) drops to the degraded dev scope.
  const requireFull = process.env.FCV_REQUIRE_FULL !== '0'
  const finalAllActions = flag('--final-all-actions') || process.env.FCV_FINAL_ALL_ACTIONS === '1'
  const installedApp = process.env.FCV_INSTALLED_APP === '1'
  const uiDriver = String(process.env.FCV_UI_DRIVER || 'playwright-chromium').trim()
  const nativeProvider = String(process.env.FCV_NATIVE_PROVIDER || '').trim()
  const finalConfig = assessFinalAllActionsConfig({
    finalAllActions,
    requireFull,
    filtered: Boolean(process.env.FCV_SECTION || process.env.FCV_ONLY || process.env.FCV_NO_AGENT === '1'),
    installedApp,
    uiDriver,
    cdpUrl: process.env.FCV_CDP_URL,
    nativeProvider,
    platform: process.platform,
  })
  if (!finalConfig.ok) {
    console.error('[fcv-gate] ✗ FINAL ALL-ACTIONS PREFLIGHT FAILED.')
    for (const miss of finalConfig.missing) console.error(`           • ${miss}`)
    console.error('           macOS remains release-red until an installed WKWebView action adapter is selected.')
    process.exit(3)
  }
  log(`surface=${surface} mode=${mode} requireFull=${requireFull ? '1 (release default)' : '0 (dev opt-out)'} finalAllActions=${finalAllActions ? '1' : '0'} installedApp=${installedApp ? '1' : '0'} driver=${uiDriver} repo=${REPO}`)

  // WSL refusal — cold-start only. External mode drives a remote engine, so the
  // heavy work isn't local; that stays allowed even from WSL.
  if (!external && isWslHost() && process.env.FCV_ALLOW_WSL !== '1') {
    console.error('[fcv-gate] ✗ REFUSING to cold-start the heavy full-coverage suite on WSL.')
    console.error('           Cut is the most feature-rich ShellX app → the biggest suite, and cutd spawns')
    console.error('           torch/ffmpeg per check; a constrained WSL host may exhaust memory.')
    console.error('           Run the per-surface gate on macOS, Windows (WebView2), or native')
    console.error('           Linux compute. To drive an EXTERNAL stack from here instead, set SWEEP_CUTD.')
    console.error('           (Override — not advised — with FCV_ALLOW_WSL=1.)')
    process.exit(2)
  }

  // Env the suite reads — set defaults without clobbering an explicit override.
  const env = { ...process.env }
  const projectsIsolation = resolveTestProjectsIsolation({
    external,
    configuredDir: env.SHELLX_CUT_PROJECTS_DIR,
    repoDir: REPO,
    receiptStem,
  })
  if (!projectsIsolation.ok) {
    console.error(`[fcv-gate] ✗ PROJECT ISOLATION FAILED: ${projectsIsolation.error}`)
    process.exit(3)
  }
  env.SHELLX_CUT_PROJECTS_DIR = projectsIsolation.dir
  if (projectsIsolation.ownedByRun) mkdirSync(projectsIsolation.dir, { recursive: true })
  // Export the full-verify flag INTO the suite's env so the harness hard-fails on
  // a missing dependency / unverifiable control rather than emitting a quiet N/A.
  env.FCV_REQUIRE_FULL = requireFull ? '1' : '0'
  env.FCV_FINAL_ALL_ACTIONS = finalAllActions ? '1' : '0'
  env.FCV_INSTALLED_APP = installedApp ? '1' : '0'
  env.FCV_UI_DRIVER = uiDriver
  env.FCV_TARGET_SURFACE = surface
  env.FCV_RESULT_RECEIPT = resultReceipt
  if (!env.SHELLX_CUT_SIDECAR_DIR) {
    env.SHELLX_CUT_SIDECAR_DIR = SIDECAR_DEFAULT
    if (!existsSync(join(SIDECAR_DEFAULT, 'instruments.py'))) {
      warn(`default SHELLX_CUT_SIDECAR_DIR=${SIDECAR_DEFAULT} but instruments.py is not there — perception (transcribe/diarize) will fail; set SHELLX_CUT_SIDECAR_DIR to the dir that DIRECTLY holds instruments.py`)
    }
  }
  if (!env.CUT_TEST_MEDIA_DIR) env.CUT_TEST_MEDIA_DIR = join(homedir(), 'Downloads')
  if (!env.CUTD_GENERATE_PROMPT_ADAPTER) {
    env.CUTD_GENERATE_PROMPT_ADAPTER = join(REPO, 'ui', 'tests', 'fixtures', 'generate-prompt-adapter.py')
  }
  if (!env.CUTD_GENERATE_STORYBOARD_ADAPTER) {
    env.CUTD_GENERATE_STORYBOARD_ADAPTER = join(REPO, 'ui', 'tests', 'fixtures', 'generate-storyboard-adapter.py')
  }
  if (!env.SHELLX_CUT_PYTHON) warn('SHELLX_CUT_PYTHON is not set — cutd will fall back to its default python resolution; set it to the perception venv python for the speech features')
  const agentFixtures = prepareAgentFixtures(env, { requireFull })
  if (agentFixtures.active) {
    log(`agent fixtures active (FCV_AGENT_FIXTURES=1): PATH += ${agentFixtures.dir}; claude=${agentFixtures.claudeFixture}; CUTD_DRAFT_ADAPTER=${agentFixtures.draftAdapter}; CUTD_JUDGE_ADAPTER=${agentFixtures.judgeAdapter}`)
  }
  const externalFixtureContract = assessExternalFixtureContract({
    external,
    fixtureActive: agentFixtures.active,
    acknowledged: env.FCV_EXTERNAL_FIXTURES_READY === '1',
  })
  if (!externalFixtureContract.ok) {
    console.error('[fcv-gate] ✗ EXTERNAL FIXTURE CONTRACT FAILED:')
    for (const miss of externalFixtureContract.missing) console.error(`           • ${miss}`)
    process.exit(3)
  }

  const zscale = checkZscale()

  // ── COMPLETE-ENV PRE-FLIGHT ─────────────────────────────────────────────────
  // Build the probe results, then a SINGLE aggregated verdict, BEFORE the heavy
  // suite launches. Under the FCV_REQUIRE_FULL=1 release default any missing
  // precondition FAILS the gate here (exit 3) with an itemised list — so "run the
  // gate" can never mean "run a degraded subset and pass". FCV_REQUIRE_FULL=0
  // (explicit dev opt-out) demotes the misses to warnings. Runs in BOTH cold-start
  // and external mode (the suite exercises these deps on whichever stack it drives).
  const diarizeBase = env.CUT_DIARIZE_ENDPOINT || 'http://127.0.0.1:9002'
  const dubBase = env.CUT_DUB_ENDPOINT || 'http://127.0.0.1:9001'
  const preflightChecks = {
    claude: probeClaude(env),
    perception: probePerception(env),
    matte: probeMatte(env),
    diarize: await probeEndpoint('diarize', diarizeBase),
    dub: await probeEndpoint('dub', dubBase),
    zscale,
    clips: probeClips(env.CUT_TEST_MEDIA_DIR, env),
  }
  const preflight = assessCompleteEnv(preflightChecks, { requireFull })
  if (preflight.missing.length) {
    if (requireFull) {
      console.error('[fcv-gate] ✗ COMPLETE-ENV PRE-FLIGHT FAILED — full verification is REQUIRED (FCV_REQUIRE_FULL=1, the release default).')
      console.error('           The full-coverage gate must run on the COMPLETE env or fail; it must never pass with an N/A or a missing dependency.')
      for (const m of preflight.missing) console.error(`           ✗ ${m}`)
      console.error('           (Dev opt-out — a DEGRADED scope that is NOT a release verdict — only with FCV_REQUIRE_FULL=0.)')
      process.exit(3)
    }
    warn('FCV_REQUIRE_FULL=0 (dev opt-out) — DEGRADED scope, NOT a release verdict. Missing for a full run:')
    for (const m of preflight.missing) warn(`  – ${m}`)
  } else {
    log(`complete-env pre-flight OK — claude + perception(full sidecar: ${PERCEPTION_FULL_IMPORTS.join('/')}) + matte(RVM) + diarize(${diarizeBase}) + dub(${dubBase}) + zscale-ffmpeg + real SCENE/SPEECH/FACE/SPEAKERS clips all present`)
  }

  let child = null
  let cutdGroupPid = null
  let base
  const cutdBin = resolveCutd()

  // cleanup — kill the cold-started cutd (and its ffmpeg/python children) by the
  // EXACT process-group pid. detached:true made the child a group leader, so a
  // negative-pid kill reaps the whole tree without `pkill -f` (which would also
  // kill this node/ssh). No-op in external mode.
  let cleaned = false
  const cleanup = () => {
    if (cleaned) return
    cleaned = true
    if (cutdGroupPid) {
      try { process.kill(-cutdGroupPid, 'SIGTERM') } catch { /* already gone */ }
      const t0 = Date.now()
      let groupGone = false
      // brief grace, then SIGKILL the group if still alive
      while (Date.now() - t0 < 4000) {
        try { process.kill(-cutdGroupPid, 0) } catch { groupGone = true; break }
        const s = spawnSync('sleep', ['0.2'])
        if (s.error) break
      }
      if (!groupGone) {
        try { process.kill(-cutdGroupPid, 'SIGKILL') } catch { /* gone */ }
      }
    }
    if (projectsIsolation.ownedByRun) {
      rmSync(projectsIsolation.dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 })
    }
  }
  process.on('exit', cleanup)
  for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    process.on(sig, () => { cleanup(); process.exit(130) })
  }

  if (external) {
    base = process.env.SWEEP_CUTD
    env.SWEEP_CUTD = process.env.SWEEP_CUTD
    env.SWEEP_APP = process.env.SWEEP_APP || process.env.SWEEP_CUTD
    log(`external stack: SWEEP_CUTD=${env.SWEEP_CUTD} SWEEP_APP=${env.SWEEP_APP}`)
    const up = await waitForCutd(env.SWEEP_CUTD, 15000, null)
    if (!up) { console.error(`[fcv-gate] ✗ external stack ${env.SWEEP_CUTD} did not answer GET /api/verbs within 15s`); process.exit(3) }
  } else {
    if (!existsSync(cutdBin)) {
      console.error(`[fcv-gate] ✗ cutd binary not found at ${cutdBin} — build it: cd app && cargo build --release -p server (or set CUTD_BIN)`)
      process.exit(3)
    }
    ensureUiDist()
    const port = await freePort()
    base = `http://127.0.0.1:${port}`
    const logDir = join(REPO, '.shellx-scratch', 'full-coverage')
    mkdirSync(logDir, { recursive: true })
    const cutdLog = join(logDir, `cutd-${port}.log`)
    log(`cold-starting cutd: ${cutdBin} serve --addr 127.0.0.1:${port} --ui-dist ${UI_DIST}  (log → ${cutdLog})`)
    const { openSync } = await import('node:fs')
    const fd = openSync(cutdLog, 'a')
    // detached:true → child is its own process-group leader so cleanup() can
    // negative-pid-kill the whole tree (cutd + ffmpeg/python children).
    child = spawn(cutdBin, ['serve', '--addr', `127.0.0.1:${port}`, '--ui-dist', UI_DIST], {
      cwd: REPO, detached: true, stdio: ['ignore', fd, fd], env,
    })
    cutdGroupPid = child.pid
    child.on('error', (e) => { console.error(`[fcv-gate] ✗ failed to spawn cutd: ${e.message}`); process.exit(3) })
    log(`cutd pid=${child.pid} — waiting for it to serve…`)
    const up = await waitForCutd(base, 60000, child)
    if (!up) {
      console.error(`[fcv-gate] ✗ cutd did not answer GET /api/verbs on ${base} within 60s — log tail:`)
      try { console.error(readFileSync(cutdLog, 'utf8').split('\n').slice(-20).join('\n')) } catch { /* no log */ }
      process.exit(3)
    }
    env.SWEEP_CUTD = base
    env.SWEEP_APP = base
    log(`cutd serving — SWEEP_CUTD=SWEEP_APP=${base} (single-origin: UI + API on one port)`)
  }

  // Run the exhaustive suite. stdio inherited so its full per-control report and
  // the FAILURES section stream live. Its exit code is the gate verdict.
  log('running public-tests/full-coverage-verify.mjs …')
  const run = spawnSync('node', ['public-tests/full-coverage-verify.mjs'], { cwd: UI_DIR, stdio: 'inherit', env })
  const exitCode = run.status === null ? 2 : run.status
  const status = exitCode === 0 ? 'pass' : 'fail'

  cleanup() // tear the cold-started stack down before we write the receipt + exit

  writeReceipt({
    schema: 'shellx-cut/full-coverage-gate@1',
    surface, mode, status, exitCode,
    requireFull,
    finalAllActions,
    installedApp,
    uiDriver,
    projectsIsolated: true,
    projectsRootOwnedByRun: projectsIsolation.ownedByRun,
    preflight: { ok: preflight.ok, enforced: preflight.enforced, missing: preflight.missing },
    cutd: external ? base : cutdBin,
    stackBase: base,
    resultReceipt,
    zscale,
    sidecarDir: env.SHELLX_CUT_SIDECAR_DIR,
    mediaDir: env.CUT_TEST_MEDIA_DIR,
    python: env.SHELLX_CUT_PYTHON || null,
    startedAt, endedAt: new Date().toISOString(),
  })

  console.log(`[fcv-gate] ${status.toUpperCase()} (exit ${exitCode}) — surface=${surface} mode=${mode}`)
  process.exit(exitCode)
}

// Run only when invoked as the CLI — importing the module (e.g. to unit-test
// assessCompleteEnv) must NOT launch a stack.
if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => { console.error('[fcv-gate] ✗ wrapper error:', e); process.exit(2) })
}
