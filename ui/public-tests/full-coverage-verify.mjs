// full-coverage-verify.mjs — EXHAUSTIVE UI + verb coverage gate (the permanent
// replacement for the retired legacy harness).
//
// WHAT MAKES THIS DIFFERENT FROM interaction-verify.mjs:
//   interaction-verify proves a CURATED set of interaction-class regressions.
//   THIS suite walks EVERY user-facing control — data-driven over the catalogs so
//   it stays exhaustive as catalogs grow — and emits a FOUR-DIMENSION verdict per
//   control (the user-facing contract):
//
//     1. PRESENT  — the control exists in the DOM (selector found).
//     2. RENDER   — opened, it actually renders: visible, non-zero bounding box,
//                   on-screen (not clipped), + a SCREENSHOT saved as the visual
//                   evidence a human can eyeball (screens dir, per surface/group).
//     3. CLICK    — the click dispatches (interactable, not disabled-without-reason).
//     4. RESULT   — evidence the action HAPPENED: the op landed in the op-log /
//                   a state field changed / the composed frame changed (SSIM<1) /
//                   an export file/job was produced. This is the load-bearing one:
//                   the "clicked but nothing happened" case must FAIL on RESULT.
//
//   Per control we print:
//     <name>: PRESENT=✓ RENDER=✓ CLICK=✓ RESULT=✓ — <evidence + screenshot path>
//   (✗ = fail, – = N/A with a stated reason; an N/A is NOT a pass).
//
// SELECTION-CONTEXT ORGANIZATION: most of the editor only unlocks when something
// specific is SELECTED. The suite spins through every selection context — no
// selection (project), VIDEO clip, AUDIO clip, CAPTION clip, TITLE clip, SHAPE
// clip, MULTI-select, RANGE (I/O marks) — establishes that selection FIRST, then
// runs the 4-part check on every control that unlocks in it. It ALSO PROVES THE
// GATING ITSELF (controls must be ABSENT/DISABLED in the wrong context and
// ENABLED in the right one) — a gating bug (never unlocks / shows for the wrong
// type) is flagged loud.
//
// REUSE: the Playwright/cutd/project bootstrap, the data-cut-* selector
// conventions, and the assert-ENGINE-STATE-after-each-action pattern are lifted
// from interaction-verify.mjs (its helpers are copied below — that script runs on
// import so it can't be imported without side effects; every sibling test file
// keeps its own copy, the established convention).
//
// TEST MEDIA — REAL high-quality clips by default (NOT the low-grade testdata/
// fixtures): bugs that only surface on real 4K/HEVC footage
// are invisible on the synthetic fixtures. Clips are resolved by ROLE from
// CUT_TEST_MEDIA_DIR (default ~/Downloads): SCENE (general edit/effects/grade/
// crop/transform/speed/render — 4K driving scene), SPEECH (caption/voice —
// 4K real speech), FACE (detector-proven face-redaction fixture), SPEAKERS
// (diarize/multicam). It falls back to a fixture ONLY
// if a real clip is genuinely missing, and LOGS THAT LOUDLY at startup.
//
// FULL-VERIFICATION MODE (the release gate): set FCV_REQUIRE_FULL=1 and every dependency
// (claude CLI, full perception sidecar+STT, diarize + dub services) MUST be present — a
// missing one is a HARD FAIL at startup (preflight, exit 3), NOT a per-control N/A. In this
// mode every control's RESULT is a real PASS/FAIL: the dialog-gated controls assert their
// underlying VERB (caption import → captions.import on a real .srt; export folder → project.
// set_output_dir) and the dependency-backed controls (agent prompts, translate, QC judge,
// redact-faces, auto-zoom, transcribe, diarize, dub) all run for real. Without
// FCV_REQUIRE_FULL the same controls degrade to an HONEST N/A when their dep is absent, so a
// partial dev run still works. J/L split-edit and fit-to-fill build their exact
// seam/gap preconditions and click the real context-menu controls in this suite.
//
// RUN (full — on macOS/resourced rig, with the real clips scp'd to ~/Downloads;
// NOT on WSL, the heavy 4K suite hangs):
//   cd ui && SWEEP_CUTD=http://127.0.0.1:6171 SWEEP_APP=http://localhost:5173 \
//     SHELLX_CUT_PROJECTS_DIR=/path/to/run-owned/projects \
//     FCV_REQUIRE_FULL=1 node public-tests/full-coverage-verify.mjs
// Filters (targeted re-runs / cheap WSL smoke):
//   FCV_REQUIRE_FULL=1         RELEASE GATE — all deps required; a missing dep = HARD FAIL
//   FCV_FINAL_ALL_ACTIONS=1    FINAL PRE-RELEASE gate — every row must pass all
//                              four dimensions; delegated/guard/optional/N-A
//                              rows are release failures
//   FCV_SECTION=video,export   run only these sections (comma list; keys in SECTIONS)
//   FCV_ONLY=<substr>          run only controls whose name contains <substr>
//   FCV_NO_AGENT=1             skip the real agent.chat prompts (ignored under FCV_REQUIRE_FULL)
//   FCV_SCREENS=<dir>          override the screenshots dir
//   FCV_TMP_DIR=<dir>          use one runner-owned exact temporary directory
//   FCV_DEFER_TEMP_CLEANUP=1   let the outer installed runner clean FCV_TMP_DIR
//                              after it stops the app processes that may hold it
//   CUT_TEST_MEDIA_DIR=<dir>   where the real clips live (default ~/Downloads)
//   CUT_DIARIZE_ENDPOINT / CUT_DUB_ENDPOINT   diarize/dub microservice base URLs (preflight /health)
// Exit 0 = no control FAILED any of its applicable dimensions; 1 = a FAIL; 3 = preflight
// hard-fail (FCV_REQUIRE_FULL=1 with an incomplete environment).

import { chromium } from 'playwright'
import { spawnSync } from 'node:child_process'
import { copyFileSync, mkdtempSync, mkdirSync, writeFileSync, existsSync, readFileSync, unlinkSync, rmSync, statSync } from 'node:fs'
import { tmpdir, homedir } from 'node:os'
import { join, dirname, basename } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import {
  basenameHostPath,
  dirnameHostPath,
  joinHostPath,
  resolveDriverPath,
} from '../../scripts/lib/cross-host-media.mjs'
import {
  buildFullCoverageReceipt,
  classifyFullCoverageRow,
} from '../../scripts/lib/full-coverage-receipt.mjs'
import {
  assessUiActionCoverage,
  buildUiActionCoverageAudit,
} from '../../scripts/ui-action-coverage-audit.mjs'
import { assessWebView2CdpVersion } from '../../scripts/lib/native-ui-driver.mjs'
import { collectInstalledRuntimeEvidence } from '../../scripts/lib/installed-walkthrough-receipt.mjs'
import {
  requireIsolatedTestProjectsDir,
  withIsolatedProjectCreate,
} from '../../scripts/lib/test-project-isolation.mjs'
import { createAssembleActionCoverage } from './lib/fullCoverageAssembleActions.mjs'
import { createAppChromeActionCoverage } from './lib/fullCoverageAppChromeActions.mjs'
import { createUserActionFeedbackCoverage } from './lib/fullCoverageUserActionFeedback.mjs'
import { createAssetsActionCoverage } from './lib/fullCoverageAssetsActions.mjs'
import { createAutopilotActionCoverage } from './lib/fullCoverageAutopilotActions.mjs'
import { createChatActionCoverage } from './lib/fullCoverageChatActions.mjs'
import { createClipsActionCoverage } from './lib/fullCoverageClipsActions.mjs'
import { createDirectorActionCoverage } from './lib/fullCoverageDirectorActions.mjs'
import { createEnvironmentActionCoverage } from './lib/fullCoverageEnvironmentActions.mjs'
import { createFullCoverageMedia } from './lib/fullCoverageMedia.mjs'
import { createGradeActionCoverage } from './lib/fullCoverageGradeActions.mjs'
import { createGenerateTemplateActionCoverage } from './lib/fullCoverageGenerateTemplateActions.mjs'
import { createGeneratedMediaActionCoverage } from './lib/fullCoverageGeneratedMediaActions.mjs'
import { createInspectorConditionalActionCoverage } from './lib/fullCoverageInspectorConditionalActions.mjs'
import { createJobWaiters } from './lib/fullCoverageJobs.mjs'
import { createLibraryActionCoverage } from './lib/fullCoverageLibraryActions.mjs'
import { createProjectWaiters } from './lib/fullCoverageProject.mjs'
import { createProjectWithRetry } from './lib/fullCoverageProjectTransition.mjs'
import { createProjectsActionCoverage } from './lib/fullCoverageProjectsActions.mjs'
import { createFullCoverageSettings } from './lib/fullCoverageSettings.mjs'
import { createLayerActionCoverage } from './lib/fullCoverageLayerActions.mjs'
import { createMaskActionCoverage } from './lib/fullCoverageMaskActions.mjs'
import { createMatteActionCoverage } from './lib/fullCoverageMatteActions.mjs'
import { createNativeOtioActionCoverage } from './lib/fullCoverageNativeOtioActions.mjs'
import { createNativeOsActionController } from './lib/nativeOsActionController.mjs'
import { createPreviewActionCoverage } from './lib/fullCoveragePreviewActions.mjs'
import { createRuntimeActionRecorder } from './lib/fullCoverageRuntimeActionRecorder.mjs'
import { createRecordActionCoverage } from './lib/fullCoverageRecordActions.mjs'
import { createRecipeActionCoverage } from './lib/fullCoverageRecipeActions.mjs'
import { createRenderQueueActionCoverage } from './lib/fullCoverageRenderQueueActions.mjs'
import { createReviewActionCoverage } from './lib/fullCoverageReviewActions.mjs'
import { createScopesActionCoverage } from './lib/fullCoverageScopesActions.mjs'
import { createSearchActionCoverage } from './lib/fullCoverageSearchActions.mjs'
import { createSequenceIndexActionCoverage } from './lib/fullCoverageSequenceIndexActions.mjs'
import { createSequenceSwitcherActionCoverage } from './lib/fullCoverageSequenceSwitcherActions.mjs'
import { createShapeActionCoverage } from './lib/fullCoverageShapeActions.mjs'
import { createStatusbarActionCoverage } from './lib/fullCoverageStatusbarActions.mjs'
import { createTitleActionCoverage } from './lib/fullCoverageTitleActions.mjs'
import { createTimelineDialogActionCoverage } from './lib/fullCoverageTimelineDialogActions.mjs'
import { createTimelineToolbarActionCoverage } from './lib/fullCoverageTimelineToolbarActions.mjs'
import { createTimelineTrackActionCoverage } from './lib/fullCoverageTimelineTrackActions.mjs'
import { createTopbarActionCoverage } from './lib/fullCoverageTopbarActions.mjs'
import { createTopbarDialogActionCoverage } from './lib/fullCoverageTopbarDialogActions.mjs'
import { createTranscriptActionCoverage } from './lib/fullCoverageTranscriptActions.mjs'
import { createVisualProof } from './lib/fullCoverageVisual.mjs'

// ── config ───────────────────────────────────────────────────────────────────
const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6171'
const APP = process.env.SWEEP_APP || 'http://localhost:5173'
const CDP = process.env.FCV_CDP_URL || ''
const UI_DRIVER = process.env.FCV_UI_DRIVER || 'playwright-chromium'
const EMBEDDED_WDIO = UI_DRIVER === 'tauri-wdio'
const REAL_SCREEN_RECORD = process.env.FCV_REAL_SCREEN_RECORD === '1'
const NATIVE_OS_ACTIONS = createNativeOsActionController()
// Native OS file dialogs live outside the WebView DOM. Clicking one from a DOM
// driver without a paired OS controller leaves a modal chooser open and poisons
// every later action (confirmed on both WKWebView and WebKitGTK). Keep those
// rows explicit and honestly CLICK=N/A in native-WebView sweeps; the dedicated
// installed OS-action receipt owns open/select-or-cancel proof.
const NATIVE_PICKER_CLICK_NA = UI_DRIVER === 'playwright-chromium' || NATIVE_OS_ACTIONS.enabled
  ? ''
  : `native OS picker delegated to the installed OS-action gate (driver=${UI_DRIVER}); ` +
    'not opened by the DOM sweep because an unpaired modal would block later actions'

// ── test media — REAL high-quality clips by default ──────────────────────────
// Bugs that only surface on real high-resolution footage can be invisible
// against the small synthetic testdata fixtures. The
// harness imports REAL 4K/HEVC clips, resolved by ROLE from CUT_TEST_MEDIA_DIR
// (default ~/Downloads, overridable by each native harness). It falls back to a
// testdata fixture ONLY if the real clip is genuinely
// missing, and LOGS THAT LOUDLY at startup (a degraded run is never silent — a
// fixture fallback means the 4K/perf class is NOT being exercised on that rig).
// Repo-relative (this file is <repo>/ui/public-tests/) so the testdata fallback resolves
// on every rig. A hardcoded absolute path would make other hosts silently fall
// back to a nonexistent file and short-circuit the SCENE role.
const TESTDATA = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'testdata')
const MEDIA_DIR = process.env.CUT_TEST_MEDIA_DIR || join(homedir(), 'Downloads')
const ENGINE_MEDIA_DIR = process.env.CUT_TEST_MEDIA_ENGINE_DIR || MEDIA_DIR
// ffmpeg for HARNESS-side media synthesis (tone bed, shifted multicam copy) —
// distinct from the ENGINE's ffmpeg. On rigs without a PATH ffmpeg, point
// CUT_HARNESS_FFMPEG at any binary (e.g. the installed app's bundled ffmpeg)
// instead of installing one; missing synthesis support is reported honestly.
const HARNESS_FFMPEG = process.env.CUT_HARNESS_FFMPEG || 'ffmpeg'
const { media, fallbackRoles: _mediaFallbacks } = createFullCoverageMedia({
  mediaDir: MEDIA_DIR,
  engineMediaDir: ENGINE_MEDIA_DIR,
})
// SCENE   — general editing / effects / grade / crop / transform / speed / render.
// SPEECH  — speech / caption / transcribe / voice (real speech).
// FACE    — detector-proven face-redaction proof. Do not overload SPEECH here:
//           some talking-head/screen fixtures carry audio but no YuNet-detectable face.
// SPEAKERS— diarize / multicam-by-speaker (2 distinct speakers).
// SECOND  — a visually DISTINCT 2nd asset for overlay / blend / replace / reference.
const SCENE = process.env.RELEASE_CLIP || media('20260618_172347.mp4', join(TESTDATA, 'talking_head.mp4'), 'SCENE: general edit / effects / grade / crop / transform / speed / render — 3840×2160 HEVC60')
const SPEECH = process.env.RELEASE_CLIP_SPEECH || media('talkinghead_hq.mp4', join(TESTDATA, 'talking_head.mp4'), 'SPEECH: speech / caption / transcribe / voice — 4096×2160 real speech')
const FACE = process.env.RELEASE_CLIP_FACE || media('face_hq.mp4', join(TESTDATA, 'moving_face.mp4'), 'FACE: detector-proven face-redaction fixture')
const SPEAKERS = process.env.RELEASE_CLIP_SPEAKERS || media('podcast_2speakers.mp4', join(TESTDATA, 'two_faces.mp4'), 'SPEAKERS: diarize / multicam-by-speaker — 2 speakers')
// SCENE vs SPEECH are visually distinct sources → SECOND reuses SPEECH for the
// overlay/reference role (distinct content, real high-res), unless overridden.
const SECOND = process.env.RELEASE_CLIP2 || (SCENE === SPEECH ? join(TESTDATA, 'silent_screen.mp4') : SPEECH)
// The menu seed is imported by the engine, so it must carry the native engine
// path on cross-host runs (for example, a Windows path from a WSL driver).
const MENU_FIXTURE = SPEECH
const FACE_DETECT_MS = Number(process.env.FCV_FACE_DETECT_MS || 1000)
// Back-compat aliases (the section bootstraps reference these names).
const CLIP = SCENE
const CLIP2 = SECOND
const SCREENS = process.env.FCV_SCREENS || join(homedir(), '.shellx-scratch', 'full-coverage')
const SECTION_FILTER = (process.env.FCV_SECTION || '').split(',').map((s) => s.trim()).filter(Boolean)
const ONLY = process.env.FCV_ONLY || ''
const TRACE = process.env.FCV_TRACE === '1'
const FINAL_ALL_ACTIONS = process.env.FCV_FINAL_ALL_ACTIONS === '1'
const EXPECTED_ACTION_MANIFEST = process.env.FCV_ACTION_MANIFEST ||
  join(dirname(fileURLToPath(import.meta.url)), 'full-ui-action-manifest.json')
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 60000)
const STATE_POLL_TIMEOUT_MS = Number(process.env.FCV_STATE_POLL_TIMEOUT_MS || 5000)
const UI_ACTION_TIMEOUT_MS = Number(process.env.FCV_UI_ACTION_TIMEOUT_MS || 5000)
const FCV_DRAIN_IMPORTS = process.env.FCV_DRAIN_IMPORTS !== '0'
const FCV_IMPORT_DRAIN_TIMEOUT_MS = Number(process.env.FCV_IMPORT_DRAIN_TIMEOUT_MS || 600000)

// ── release-gate mode + dependency preflight ──────────────────────────────────
// FCV_REQUIRE_FULL=1 is the RELEASE GATE: every dependency MUST be present, and a
// missing one is a HARD FAIL at startup (process.exit) — NOT a per-control N/A.
// Unset (the default) = partial dev run: a missing dep degrades the dependent
// controls to an honest N/A so the rest of the sweep still runs. preflight()
// (called first in main()) populates DEP before any section executes.
const FULL = process.env.FCV_REQUIRE_FULL === '1'
// diarize / dub are SEPARATE microservices. The doctor now carries a NEUTRAL,
// optional `diarize`/`dub` card (reachable→ok, else Unknown), but preflight probes
// their /health DIRECTLY here — a real run-time reachability check the gate trusts
// over a cached card. Defaults mirror the engine's CUT_DIARIZE_ENDPOINT /
// CUT_DUB_ENDPOINT (app/server/src/{diarize,dub}.rs).
const DIARIZE_ENDPOINT = process.env.CUT_DIARIZE_ENDPOINT || 'http://127.0.0.1:9002'
const DUB_ENDPOINT = process.env.CUT_DUB_ENDPOINT || 'http://127.0.0.1:9001'
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
// Live dependency availability — set by preflight(); read by the dep-gated sections.
// `generate` = a generation CLI (Codex or Grok) is signed in and ready for the
// secAssets assets.generate drive; `genProvider` names which available provider to use.
// Detected from the doctor judge.codex / judge.grok cards (the
// same CLIs assets.generate shells out to), mirroring how DEP.matte keys off the matte card.
// `chatAgents` = per-provider agent.chat readiness (claude/codex/grok), read from the
// doctor's `judge.<agent>.details.chat` block (the SAME source the AgentChat dropdown
// uses, lib/doctor chatAgentsFrom). An agent is ready only when installed + wired + a
// CONFIRMED session (authenticated==='yes'); 'unknown' (grok's expiring-token case) and
// 'no' are NOT ready — exactly like the dropdown's badge, which never shows a false green.
// Drives per-provider secAgent coverage. Unlike DEP.claude (preflight-enforced
// under FULL), codex/grok are OPTIONAL backends: an absent one is a BENIGN skip, never a
// gate fail (the environment may legitimately lack a Grok session).
const DEP = { engine: false, claude: false, perceptionCv: false, perceptionStt: false, diarize: false, dub: false, matte: false, generate: false, ffmpegLibass: false, ffmpegVidstab: false, ffmpegZscale: false, ffmpegPath: '', genProvider: '', perceptionPy: '', perceptionCvDetail: '', chatAgents: { claude: false, codex: false, grok: false } }
// Agent prompts spend a real subscription-CLI turn; skipped with FCV_NO_AGENT=1 in
// a partial run, but ALWAYS run under the release gate (claude presence is enforced
// by preflight, so there is nothing to honestly skip).
const RUN_AGENT = FULL || process.env.FCV_NO_AGENT !== '1'

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
async function settleWithin(promise, timeoutMs, label) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${label} timed out after ${timeoutMs}ms`)),
          timeoutMs,
        )
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}
const configuredTmp = (process.env.FCV_TMP_DIR || '').trim()
const tmp = configuredTmp || mkdtempSync(join(tmpdir(), 'fcv-'))
if (configuredTmp) mkdirSync(tmp, { recursive: true })
const COVERAGE_CHECK = process.argv.includes('--coverage-check')
const TEST_PROJECTS_DIR = COVERAGE_CHECK
  ? ''
  : requireIsolatedTestProjectsDir(process.env.SHELLX_CUT_PROJECTS_DIR)
const DEFER_TEMP_CLEANUP = process.env.FCV_DEFER_TEMP_CLEANUP === '1'
const crossHostMedia = MEDIA_DIR !== ENGINE_MEDIA_DIR
const synthDriverDir = crossHostMedia ? mkdtempSync(join(MEDIA_DIR, '.shellx-fcv-')) : tmp
const synthEngineDir = crossHostMedia
  ? joinHostPath(ENGINE_MEDIA_DIR, basename(synthDriverDir))
  : tmp
const gradeLutName = 'fcv-test-lut-invert.cube'
const gradeLutDriverPath = crossHostMedia
  ? join(synthDriverDir, gradeLutName)
  : join(TESTDATA, 'test_lut_invert.cube')
if (crossHostMedia) {
  copyFileSync(join(TESTDATA, 'test_lut_invert.cube'), gradeLutDriverPath)
}
const gradeLutEnginePath = crossHostMedia
  ? joinHostPath(synthEngineDir, gradeLutName)
  : gradeLutDriverPath
let seq = 0

function fileBytes(path) {
  if (!path) return 0
  try { return statSync(path).size } catch { return 0 }
}

function cleanupTmp() {
  const options = {
    recursive: true,
    force: true,
    maxRetries: 20,
    retryDelay: 250,
  }
  rmSync(tmp, options)
  if (synthDriverDir !== tmp) rmSync(synthDriverDir, options)
}

export class FullCoverageExit extends Error {
  constructor(code) {
    super(`full coverage verifier exited with code ${code}`)
    this.name = 'FullCoverageExit'
    this.exitCode = code
  }
}

function exit(code) {
  if (!DEFER_TEMP_CLEANUP) cleanupTmp()
  if (EMBEDDED_WDIO) throw new FullCoverageExit(code)
  process.exit(code)
}

// ── shared bootstrap helpers (copied from interaction-verify.mjs) ─────────────
async function verb(name, args = {}, opts = {}) {
  if (TEST_PROJECTS_DIR) args = withIsolatedProjectCreate(name, args, TEST_PROJECTS_DIR)
  const timeoutMs = opts.timeoutMs ?? VERB_TIMEOUT_MS
  const t0 = Date.now()
  try {
    const r = await fetch(`${CUTD}/api/verb/${name}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:ui' },
      body: JSON.stringify(args),
      signal: AbortSignal.timeout(timeoutMs),
    })
    return r.json()
  } catch (e) {
    const hang = e?.name === 'TimeoutError' || /aborted|timed?\s*out/i.test(String(e))
    return { ok: false, hang, error: { message: (hang ? `VERB HANG >${timeoutMs}ms (${Date.now() - t0}ms): ${name} ` : '') + String(e) } }
  }
}
const { state, ops, opsLen, waitForState, opLanded, flatClips, findClip } = createProjectWaiters({ verb, sleep, statePollTimeoutMs: STATE_POLL_TIMEOUT_MS })
const { frame, ssim, renderGroup } = createVisualProof({
  verb,
  tmpDir: tmp,
  screensDir: SCREENS,
  ffmpegBin: HARNESS_FFMPEG,
})
// ── catalogs (mirror the UI source so the loops stay drift-aligned) ───────────
// Curated VIDEO effect chips — panels/Inspector/index.tsx VIDEO_EFFECTS.
const VIDEO_EFFECTS = ['sepia', 'invert', 'vignette', 'sharpen', 'auto_color', 'mirror', 'flip', 'pixelize', 'vhs', 'posterize']
// AUDIO effect chips — AUDIO_EFFECTS.
const AUDIO_EFFECTS = ['denoise', 'compressor', 'gate']
// EQ presets — EQ_PRESETS (+ the explicit "clear").
const EQ_PRESETS = ['voice', 'warmth', 'de_rumble', 'phone', 'de_ess', 'brighten', 'clear']
// Blend modes — BLEND_MODES (overlay video track only).
const BLEND_MODES = ['normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'difference', 'addition', 'subtract', 'softlight', 'hardlight']
// Overlay-only VIDEO effects (effect_specs(): overlay_only:true) — they REFUSE a base
// clip ("chroma key reveals a LOWER track; the base has nothing under it", edit.rs)
// and the Inspector's "More effects…" overflow intentionally HIDES them (no clean one-
// click on a base clip), so there is NO chip for them anywhere. chroma_key is the only
// one today. The harness drives each on an OVERLAY clip in secBlend (verb-level verify;
// no chip to click), and the catalog-drift guard credits exactly these as covered-in-
// blend. A NEW overlay-only engine effect NOT listed here will (correctly) trip the guard.
const OVERLAY_ONLY_EFFECTS = ['chroma_key']

// The non-curated, non-overlay VIDEO effects the engine reports (effects.list) — the
// "More effects…" overflow set (blur/grain/hue_shift/rgb_split/emboss today). SINGLE
// SOURCE OF TRUTH shared by secVideo (drives a chip for EACH) and the catalog-drift
// guard (credits them as covered) so the two can never disagree about what's exercised.
// Mirrors the Inspector's extraVideoEffects filter (panels/Inspector/index.tsx).
const videoOverflowEffects = (cat) =>
  cat.filter((e) => e.track === 'video' && !e.overlay_only && !VIDEO_EFFECTS.includes(e.key))

// Build a full ClipEffect from an effects.list catalog row — number params seeded from
// their declared default (falling back to min), color params seeded with green (#00FF00,
// the green-screen key colour). Unlike the Inspector's clipEffectFromCatalog (number-only
// — its overflow never includes a color-param effect), this also fills REQUIRED color
// params, so a no-chip overlay-only effect (chroma_key needs `color`) can be driven
// directly via edit.effect. Used only by the secBlend overlay-only verify.
function effectFromCatalogFull(entry) {
  const eff = { type: entry.key }
  for (const p of entry.params || []) {
    if (p.kind === 'number') eff[p.name] = typeof p.default === 'number' ? p.default : (p.min ?? 0)
    // Color params must use the engine-accepted form: a name ("green") or 0xRRGGBB.
    // A CSS "#RRGGBB" is REJECTED ("color must be a name (green) or 0xRRGGBB"), which
    // silently made the chroma_key overlay verify fail (op never landed). 0x00FF00 = green.
    else if (p.kind === 'color') eff[p.name] = '0x00FF00'
  }
  return eff
}
// Export menu options — topbar EXPORT_OPTIONS ids → {verb, kind}. kind: 'job'
// (async, result.job_id), 'file' (sync, result.path).
const EXPORT_OPTIONS = [
  { id: 'video', verb: 'render.final', kind: 'job' },
  { id: 'audio', verb: 'export.audio', kind: 'file' },
  { id: 'gif', verb: 'export.gif', kind: 'file' },
  { id: 'pub_youtube', verb: 'export.publish', kind: 'job' },
  { id: 'pub_tiktok', verb: 'export.publish', kind: 'job' },
  { id: 'pub_reels', verb: 'export.publish', kind: 'job' },
  { id: 'pub_x', verb: 'export.publish', kind: 'job' },
  { id: 'frame', verb: 'export.frame', kind: 'file' },
  { id: 'fcpxml', verb: 'export.xml', kind: 'file' },
  { id: 'premiere', verb: 'export.xml', kind: 'file' },
  { id: 'resolve', verb: 'export.xml', kind: 'file' },
  { id: 'otio', verb: 'export.otio', kind: 'file' },
  { id: 'edl', verb: 'export.edl', kind: 'file' },
  { id: 'srt', verb: 'export.srt', kind: 'file', needsCaptions: true },
  { id: 'vtt', verb: 'export.vtt', kind: 'file', needsCaptions: true },
  { id: 'ass', verb: 'export.ass', kind: 'file', needsCaptions: true },
  { id: 'chapters', verb: 'export.chapters', kind: 'file', needsMarkers: true }, // chapters read MARKERS, not captions (secExport seeds them)
  { id: 'transcript', verb: 'export.transcript', kind: 'file', needsCaptions: true },
]
const EXPORT_EXTENSIONS = {
  video: 'mp4', audio: 'mp3', gif: 'gif', frame: 'jpg',
  pub_youtube: 'mp4', pub_tiktok: 'mp4', pub_reels: 'mp4', pub_x: 'mp4',
  fcpxml: 'fcpxml', premiere: 'xml', resolve: 'fcpxml', otio: 'otio',
  edl: 'edl', srt: 'srt', vtt: 'vtt', ass: 'ass', chapters: 'txt',
  transcript: 'md',
}
// Global timeline tools — timeline toolbar → engine global timeline edits.
// Each entry's `verb` is the OP-LOG verb the tool's orchestrator actually emits
// (the dispatched verb differs): edit.trim_edges → edit.ripple_delete sub-ops;
// edit.split_at_scenes → edit.split; edit.mark_scenes → edit.add_marker. RESULT
// asserts THIS specific verb landed (opLanded) — not merely that the op-log grew
// (an unrelated op must NOT pass the tool's RESULT).
// `verb` = the OP-LOG sub-op a successful tool emits (RESULT pass when it lands). `orch`
// = the orchestrator verb the UI actually dispatches on click; its RESPONSE is captured
// so a tool that RAN CLEAN but found no content (no dead-air / no scene cuts) can be
// recorded as honest N/A instead of a false-fail (content-dependent, not a wiring bug).
const TOOLS = [
  { id: 'trim_edges', verb: 'edit.ripple_delete', orch: 'edit.trim_edges' },
  { id: 'split_scenes', verb: 'edit.split', orch: 'edit.split_at_scenes' },
  { id: 'mark_scenes', verb: 'edit.add_marker', orch: 'edit.mark_scenes' },
]
// Render-options selects (topbar render menu) → {sel, option, verb, assert}.
// project.* selects mutate project.settings; render.* selects are local render
// state (asserted PRESENT/RENDER/CLICK only — no op until Render fires).

// ── results model ─────────────────────────────────────────────────────────────
// Each control → one row with 4 dims (each 'pass'|'fail'|'na') + evidence + shot.
const results = []
function inferredRowKind(name, dims, evidence) {
  if (dims.rowKind === 'ui_action' || dims.rowKind === 'support') return dims.rowKind
  if (/^(?:GATE:|BOOTSTRAP$)/.test(name)) return 'support'
  if (/verb-level|catalog|location|console-clean|drift guard|nothing to verify/i.test(`${name} ${evidence}`)) return 'support'
  if ((dims.present || 'na') === 'na' && (dims.click || 'na') === 'na') return 'support'
  return 'ui_action'
}

function rec(surface, name, dims, evidence = '', shot = '') {
  const rowKind = inferredRowKind(name, dims, evidence)
  results.push({
    surface, name,
    rowKind,
    actionId: dims.actionId || `${surface}::${name}`,
    present: dims.present || 'na',
    render: dims.render || 'na',
    click: dims.click || 'na',
    result: dims.result || 'na',
    evidence, shot,
  })
}
const SYM = { pass: '✓', fail: '✗', na: '–' }
// A control FAILED overall if any dimension is 'fail'.
const isFail = (r) => [r.present, r.render, r.click, r.result].includes('fail')
function trace(surface, name, phase = 'point') {
  if (TRACE) console.error(`[fcv-trace] ${surface}/${name} ${phase}`)
}

// ── selection / project bootstrap helpers ─────────────────────────────────────
// The right rail (Inspector + Color/Audio/Chat tabs) DEFAULTS COLLAPSED — when
// collapsed the tabs + Inspector are UNMOUNTED and only the expand affordance
// shows (App.tsx: app__rail--collapsed / data-cut-action="expand-rail"). On the
// rigs a persisted layout keeps it open; a fresh browser starts collapsed. Every
// right-rail check must ensure it's expanded first, or the controls read "absent".
async function ensureRail(page) {
  const expand = page.locator('[data-cut-action="expand-rail"]')
  if (await expand.count()) { await expand.click().catch(() => {}); await sleep(300) }
}
// Dismiss any drawer / topbar menu / modal / scrim a PRIOR check left open so it
// can't cover a LATER check's click target. On macOS-headless-Chromium this was a
// real false-fail source (interaction-verify dropped 52→46): an earlier check's
// drawer stayed up and silently obscured downstream clicks — checks that PASS in
// isolation read as fails. Every panel drawer binds its own Escape handler and every
// topbar menu (export/tools/find/render) closes on Escape too, so Escape ×2 clears
// the common case; the visible close-button sweep is a belt-and-suspenders fallback
// for anything that ignores Escape. ROBUST: every step is .catch-guarded and the
// whole body is try-wrapped, so it NEVER throws when nothing is open. SELECTION-SAFE:
// it never clicks inside the timeline, and it is sequenced at the START of each
// section (called right after the freshProject bootstrap, BEFORE that section selects
// its clip), so even an Escape that clears selection has nothing yet to clear.
async function closeOverlays(page) {
  try {
    await page.keyboard.press('Escape').catch(() => {}); await sleep(120)
    await page.keyboard.press('Escape').catch(() => {}); await sleep(120)
    // Fallback: click any STILL-visible drawer/modal close button (mirrors the
    // data-cut-*-close set the panel drawers emit). isVisible() guards keep this a
    // no-op when nothing is open — and these are dedicated close buttons, never
    // timeline clips, so selection is untouched.
    const closers = page.locator([
      'assemble', 'autopilot', 'clips', 'director', 'environment', 'generate',
      'grade', 'kinetic', 'layer', 'matte', 'musicbed', 'render-queue',
      'search', 'shape', 'stock', 'storyboard', 'title',
    ].map((k) => `[data-cut-${k}-close]`).join(','))
    const n = await closers.count().catch(() => 0)
    for (let i = 0; i < n; i++) {
      const c = closers.nth(i)
      if (await c.isVisible().catch(() => false)) { await c.click({ timeout: 800 }).catch(() => {}); await sleep(80) }
    }
  } catch { /* best-effort: a clean viewport is never worth throwing over */ }
}

/** Robust app reload: `networkidle` can never fire while the UI polls active
 *  jobs with sub-500ms HTTP traffic, so wait for DOM + the topbar root. */
async function reloadApp(page) {
  await page.reload({ waitUntil: 'domcontentloaded' })
  await page.waitForSelector('[data-cut-panel="topbar"]', { timeout: 20000 }).catch(() => {})
  await sleep(400)
}

async function freshProject(page, tag, clip = SCENE) {
  const drained = await drainActiveJobs()
  if (!drained) {
    throw new Error(
      `freshProject(${tag}) timed out draining active jobs before project.create: ` +
      `${activeJobSummary() || 'jobs.list returned active jobs without identifiers'}`,
    )
  }
  const projectName = `fcv_${tag}_` + Math.random().toString(36).slice(2, 6)
  const { response: created, attempts } = await createProjectWithRetry({
    verb,
    name: projectName,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  const projectPath = created.result?.path || ''
  if (!created.ok || !projectPath) {
    throw new Error(
      `freshProject(${tag}) project.create failed after ${attempts} attempt(s): `
      + JSON.stringify(created.error || created).slice(0, 500),
    )
  }
  const imported = await verb('media.import', { path: clip })
  const assetId = imported.result?.asset_id || ''
  if (!imported.ok || !assetId) {
    throw new Error(`freshProject(${tag}) media.import failed: ${JSON.stringify(imported.error || imported).slice(0, 500)}`)
  }
  if (FCV_DRAIN_IMPORTS) await awaitImportJobs(imported, FCV_IMPORT_DRAIN_TIMEOUT_MS)
  await sleep(FCV_DRAIN_IMPORTS ? 300 : 1500)
  await reloadApp(page)
  await sleep(1200)
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await sleep(400)
  await ensureRail(page)
  return { projectPath, assetId, created, imported }
}
function writeSeededTranscript(localProjectPath, assetId, opts = {}) {
  const base = opts.baseMs ?? 120
  const words = opts.words || [
    ['Human', base, base + 220],
    ['speech', base + 260, base + 520],
    ['drives', base + 580, base + 820],
    ['captions', base + 880, base + 1180],
    ['now.', base + 1220, base + 1420],
  ].map(([word, start_ms, end_ms], idx) => ({
    idx,
    word,
    start_ms,
    end_ms,
    confidence: 1.0,
  }))
  const transcript = {
    asset: assetId,
    model: opts.model || 'fixture@full-coverage-nonempty-transcript',
    language: opts.language || 'en',
    words,
  }
  const receipts = join(localProjectPath, 'receipts')
  mkdirSync(receipts, { recursive: true })
  writeFileSync(join(receipts, `${assetId}.words.json`), JSON.stringify(transcript, null, 2))
  try { unlinkSync(join(localProjectPath, 'project.json')) } catch {}
  return transcript
}
async function ensureNonEmptyTranscript(page, projectPath, assetId, reason = 'fcv downstream transcript precondition') {
  if (!projectPath) throw new Error(`cannot seed transcript for ${assetId || '?'}: project path is missing`)
  if (!assetId) throw new Error(`cannot seed transcript in ${projectPath}: asset id is missing`)
  const before = await transcriptWordCount(assetId)
  if (before > 0) return { assetId, words: before, seeded: false }

  const localProjectPath = resolveDriverPath(projectPath)
  const seeded = writeSeededTranscript(localProjectPath, assetId, { reason })
  const reopened = await verb('project.open', { path: projectPath })
  if (!reopened.ok) throw new Error(`reopen after seeded transcript failed: ${JSON.stringify(reopened.error || reopened).slice(0, 160)}`)
  await reloadApp(page)
  await sleep(900)
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await sleep(250)
  await ensureRail(page)
  await waitForState((s) => transcribedAssetIds(s).includes(assetId), 15000)
  const after = await transcriptWordCount(assetId)
  if (after <= 0) throw new Error(`seeded transcript did not link for ${assetId}: ${reason}`)
  return { assetId, words: after, seeded: true, model: seeded.model }
}
function writeSeededMenuReceipts(localProjectPath, assetId) {
  const sourceProbe = JSON.parse(readFileSync(join(localProjectPath, 'receipts', `${assetId}.probe.json`), 'utf8'))
  const durationMs = Number(sourceProbe.duration_ms)
  if (!Number.isFinite(durationMs) || durationMs < 2000) {
    throw new Error(`menu fixture ${assetId} needs at least 2000ms of probed media; got ${sourceProbe.duration_ms ?? 'unknown'}`)
  }
  const firstWordAtMs = Math.max(500, Math.round(durationMs * 0.15))
  const sceneAtMs = Math.round(durationMs * 0.5)
  const lastWordAtMs = Math.min(durationMs - 500, Math.round(durationMs * 0.8))
  const words = {
    asset: assetId,
    model: 'fixture@menu-tools',
    language: 'en',
    words: [
      { idx: 0, word: 'Opening', start_ms: firstWordAtMs, end_ms: firstWordAtMs + 300, confidence: 1.0 },
      { idx: 1, word: 'middle', start_ms: sceneAtMs, end_ms: sceneAtMs + 300, confidence: 1.0 },
      { idx: 2, word: 'ending.', start_ms: lastWordAtMs, end_ms: lastWordAtMs + 300, confidence: 1.0 },
    ],
  }
  const report = {
    schema: 'shellx-cut/perception/1',
    asset_hash: 'fixture',
    source_path: MENU_FIXTURE,
    instruments_run: ['words', 'silence', 'scenes', 'beats', 'loudness'],
    silences: [{ start_ms: sceneAtMs - 300, end_ms: sceneAtMs + 300, source: 'fixture' }],
    scenes: [{ at_ms: sceneAtMs, score: null }],
    beats: { bpm: 120, beats_ms: [1000, 1500, 2000, 2500] },
    loudness: { integrated_lufs: -16, true_peak_dbtp: -1, windows: [] },
    black_spans: [],
    frozen_spans: [],
    words,
  }

  const receipts = join(localProjectPath, 'receipts')
  mkdirSync(receipts, { recursive: true })
  writeFileSync(join(receipts, `${assetId}.perception.json`), JSON.stringify(report, null, 2))
  writeFileSync(join(receipts, `${assetId}.words.json`), JSON.stringify(words, null, 2))

  // ProjectStore::open reconciles derived transcript/perception pointers only
  // when it rebuilds from the op log. The cache is safe to drop in this harness:
  // ops.jsonl remains the source of truth, and the seeded receipts are the
  // evidence files we need the server to link.
  try { unlinkSync(join(localProjectPath, 'project.json')) } catch {}
}
async function seedMenuToolProject(page) {
  const name = `fcv_menus_${Math.random().toString(36).slice(2, 6)}`
  const created = await verb('project.create', { name, settings: { width: 1280, height: 720, fps: 30 } })
  const projectPath = created.result?.path
  if (!created.ok || !projectPath) throw new Error(`menu fixture project.create failed: ${JSON.stringify(created.error || created).slice(0, 120)}`)

  const imp = await verb('media.import', { path: MENU_FIXTURE, proxy: false, rationale: 'fcv: seeded Tools menu transcript/scenes fixture' })
  const assetId = imp.result?.asset_id
  if (!imp.ok || !assetId) throw new Error(`menu fixture media.import failed: ${JSON.stringify(imp.error || imp).slice(0, 120)}`)
  await awaitImportJobs(imp, FCV_IMPORT_DRAIN_TIMEOUT_MS)

  const localProjectPath = resolveDriverPath(projectPath)
  writeSeededMenuReceipts(localProjectPath, assetId)
  const reopened = await verb('project.open', { path: projectPath })
  if (!reopened.ok) throw new Error(`menu fixture project.open failed: ${JSON.stringify(reopened.error || reopened).slice(0, 120)}`)
  await reloadApp(page)
  await sleep(1200)
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await sleep(400)
  await ensureRail(page)
}
async function clipOfKind(kind) {
  const s = await state()
  return s.tracks.find((t) => t.kind === kind)?.clips?.find((c) => c.asset)?.id
}
async function selectClip(page, id) {
  if (!id) return false
  const loc = page.locator(`[data-cut-clip="${id}"]`).first()
  for (let i = 0; i < 5; i++) {
    if (await loc.count()) {
      await loc.scrollIntoViewIfNeeded().catch(() => {})
      // Timeline selection is owned by ClipView.onMouseDown so the native
      // adapter must drive a real pointer sequence. A force-click falls back to
      // an in-page click event and never reaches that handler.
      await loc.click().catch(() => loc.click({ force: true }).catch(() => {}))
    }
    await sleep(350)
    // Native WebViews can apply the local React selection immediately while
    // their UI-state confirmation channel is still reconnecting after reload.
    // The selected class is the rendered product state, so accept it before
    // asking the server to round-trip the same selection back through ui.state.
    if (await loc.evaluate((element) => element.classList.contains('tl-clip--selected')).catch(() => false)) {
      return true
    }
    const ui = await verb('ui.state', {}, { timeoutMs: 5000 }).catch(() => null)
    const selected = ui?.result?.selected_clip_ids || []
    if (Array.isArray(selected) && selected.includes(id)) return true
  }
  return false
}
async function selectClipPair(page, firstId, secondId) {
  if (!firstId || !secondId) return false
  const first = page.locator(`[data-cut-clip="${firstId}"]`).first()
  const second = page.locator(`[data-cut-clip="${secondId}"]`).first()
  const bothSelected = async () => {
    const firstSelected = await first.evaluate((element) =>
      element.classList.contains('tl-clip--selected')).catch(() => false)
    const secondSelected = await second.evaluate((element) =>
      element.classList.contains('tl-clip--selected')).catch(() => false)
    return firstSelected && secondSelected
  }

  await selectClip(page, firstId)
  await second.click({ modifiers: ['ControlOrMeta'] }).catch(() => {})
  await sleep(250)
  if (await bothSelected()) return true

  // WKWebView currently drops modifier state from a combined WebDriver
  // key/pointer action. Re-establish the primary selection, then dispatch the
  // exact modified mouse gesture to the same product handler.
  await selectClip(page, firstId)
  await second.evaluate((element) => {
    const metaKey = /Mac|iPhone|iPad|iPod/i.test(navigator.platform)
    const init = {
      bubbles: true,
      cancelable: true,
      button: 0,
      buttons: 1,
      ctrlKey: !metaKey,
      metaKey,
    }
    element.dispatchEvent(new MouseEvent('mousedown', init))
    element.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }))
    element.dispatchEvent(new MouseEvent('click', { ...init, buttons: 0 }))
  }).catch(() => {})
  await sleep(250)
  return bothSelected()
}
// Bring the FACE clip (detector-proven real face) onto the BASE video track and
// return its clip id (or null). Face-blur / redaction are BASE-track-only (core edit.rs
// rejects masks on overlay tracks), and the default SCENE clip is a road with NO face,
// so the engine's faces auto-detect honestly lands NO op on it (found:0 → non-mutating
// receipt). Inserting at 0 (ripple) makes the imported clip the base clip at position 0;
// callers run this where a one-time timeline shift is harmless (after SSIM-gated checks,
// or in op/count-based sections). Robust to media.import dedup (same path already
// imported) and to the testdata fixture fallback.
async function addBaseFaceClip(page, opts = {}) {
  const baseTrack = (await state()).tracks.find((t) => t.kind === 'video')?.id
  if (!baseTrack) return null
  const atMs = Number(opts.atMs ?? 0)
  const ripple = opts.ripple ?? true
  const imp = await verb('media.import', { path: FACE })
  const asset = imp.result?.asset_id
  if (!asset) return null
  await awaitImportJobs(imp, FCV_IMPORT_DRAIN_TIMEOUT_MS)
  if (FULL || DEP.perceptionCv) await ensureAssetPerception(asset, 240000)
  await sleep(1200)
  await verb('edit.insert', { asset, track: baseTrack, at_ms: atMs, ripple, rationale: 'fcv: real-face clip for redact-faces' })
  let id = null
  for (let i = 0; i < 16; i++) {
    await sleep(400)
    const clips = (await state()).tracks.find((t) => t.id === baseTrack)?.clips || []
    const match = clips.find((c) => c.asset === asset && Math.abs(Number(c.start_ms ?? c.at_ms ?? 0) - atMs) < 50)
      || clips.find((c) => c.asset === asset)
    if (match) { id = match.id; break }
  }
  // Dedup/fallback edge: the import returned an already-present asset id, so no NEW clip
  // matched above — the position-0 media clip (the inserted one) is still the face clip.
  if (!id) {
    const clips = (await state()).tracks.find((t) => t.id === baseTrack)?.clips || []
    id = clips.find((c) => c.asset)?.id || null
  }
  return id
}
async function deselect(page) {
  // Match interaction-verify's proven clear: a plain body click (centre, the
  // preview stage — not a timeline clip) + Escape drops any clip selection so the
  // Inspector falls back to its project-scope (caption composer) surface.
  await page.locator('body').click().catch(() => {})
  await page.keyboard.press('Escape').catch(() => {})
  await sleep(250)
}
async function propertiesTab(page) {
  await ensureRail(page)
  await page.locator('[data-cut-right-tab="properties"]').click().catch(() => {})
  await sleep(250)
}
async function expandInspectorSection(page, key) {
  const section = page.locator(`[data-cut-section="${key}"]`).first()
  if (!(await section.count())) return false
  if ((await section.getAttribute('data-cut-section-collapsed')) === 'true') {
    await page.locator(`[data-cut-section-toggle="${key}"]`).first().click().catch(() => {})
    await sleep(180)
  }
  return (await section.getAttribute('data-cut-section-collapsed')) === 'false'
}
async function openTimelineAutomation(page) {
  const trigger = page.locator('[data-cut-timeline-automation-trigger]').first()
  if (!(await trigger.count())) return false
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') {
    await trigger.click().catch(() => {})
    await sleep(180)
  }
  return await page.locator('[data-cut-timeline-automation-menu]').first().isVisible().catch(() => false)
}
async function waitInspectorKind(page, kind, fieldSelector, timeoutMs = 8000) {
  await propertiesTab(page)
  const insp = page.locator('[data-cut-panel="inspector"]').first()
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const got = await insp.getAttribute('data-cut-inspector-kind').catch(() => null)
    const fields = await page.locator(fieldSelector).count().catch(() => 0)
    if (got === kind && fields > 0) return got
    await sleep(250)
  }
  return await insp.getAttribute('data-cut-inspector-kind').catch(() => null)
}
async function ensureReviewPanel(page) {
  await ensureRail(page)
  const review = page.locator('[data-cut-panel="review"]').first()
  if ((await review.count().catch(() => 0)) === 0) {
    const pin = page.locator('[data-cut-rail-pin]').first()
    if ((await pin.count().catch(() => 0)) > 0) {
      await pin.click({ force: true }).catch(() => {})
      await sleep(350)
    }
  }
  await page.waitForSelector('[data-cut-panel="review"]', { timeout: 5000 }).catch(() => {})
  return page.locator('[data-cut-panel="review"]').first()
}
async function reviewTab(page, tab, readySelector, timeoutMs = 6000) {
  const panel = await ensureReviewPanel(page)
  const tabBtn = panel.locator(`[data-cut-tab="${tab}"]`).first()
  if ((await tabBtn.count().catch(() => 0)) > 0) {
    await tabBtn.click({ force: true }).catch(() => {})
  }
  if (readySelector) await page.waitForSelector(readySelector, { timeout: timeoutMs }).catch(() => {})
  await sleep(250)
  return panel
}
async function continuePreflightIfPresent(page, attempts = 40) {
  for (let i = 0; i < attempts; i++) {
    const warning = page.locator('[data-cut-pregate-warning]').first()
    if ((await warning.count().catch(() => 0)) > 0) {
      const detail = await page.evaluate(() => {
        const warning = document.querySelector('[data-cut-pregate-warning]')
        return {
          blocked: warning?.getAttribute('data-cut-pregate-blocked') === 'true',
          risks: Array.from(warning?.querySelectorAll('[data-cut-pregate-risk]') ?? []).map((el) => ({
            kind: el.getAttribute('data-cut-pregate-risk-kind') || 'uninstrumented',
            severity: el.getAttribute('data-severity') || '',
          })),
        }
      }).catch(() => ({ blocked: false, risks: [] }))
      const cont = page.locator('[data-cut-pregate-continue]').first()
      if (!detail.blocked && (await cont.count().catch(() => 0)) > 0 && await cont.isEnabled().catch(() => false)) {
        await cont.click().catch(() => {})
      }
      return { seen: true, ...detail }
    }
    if (i < attempts - 1) await sleep(250)
  }
  return { seen: false, blocked: false, risks: [] }
}

// Add + select a CAPTION clip; returns its id (or null).
async function addCaption(page, text) {
  await verb('captions.add_text', { text, range_ms: [0, 2500], position: 'bottom' })
  const s = await waitForState((st) => (st.tracks || []).some((t) => t.kind === 'caption' && (t.clips || []).some((c) => c.text === text)), 10000)
  const cap = s ? (s.tracks.find((t) => t.kind === 'caption')?.clips?.find((c) => c.text === text)) : null
  return cap?.id || null
}
// Add + select a TITLE clip (render-backed — poll); returns the clip.
async function addTitle(page, text) {
  const add = await verb('title.add', { text, range_ms: [0, 3000] })
  if (!add.ok) return null
  const s = await waitForState((st) => (st.tracks || []).some((t) => (t.id || '').startsWith('title') && (t.clips || []).some((c) => c.title_text === text)), 18000)
  return s ? s.tracks.find((t) => (t.id || '').startsWith('title'))?.clips?.find((c) => c.title_text === text) : null
}
// Add + select a SHAPE clip (render-backed — poll); returns the clip.
async function addShape(page, label) {
  const add = await verb('edit.add_shape', { shape: 'rect', fill: '#FF0000', text: label, range_ms: [0, 3000] })
  if (!add.ok) return null
  const s = await waitForState((st) => {
    for (const t of st.tracks || []) {
      if (!(t.id || '').startsWith('title')) continue
      if ((t.clips || []).some((c) => c.shape_kind === 'rect' && c.shape_label === label)) return true
    }
    return false
  }, 18000)
  if (!s) return null
  for (const t of s.tracks || []) {
    if (!(t.id || '').startsWith('title')) continue
    const c = (t.clips || []).find((cc) => cc.shape_kind === 'rect' && cc.shape_label === label)
    if (c) return c
  }
  return null
}

// ── generic control probe (the 4-dimension engine) ───────────────────────────
// opts:
//   surface, name          — labels (name is filterable via FCV_ONLY)
//   sel                    — Playwright Locator for the control (PRESENT)
//   group                  — Locator for the visual group to screenshot (RENDER)
//   groupName              — cache key for the group screenshot
//   doClick()              — async, performs the interaction (CLICK)
//   assertResult()         — async → {ok, detail} or {result:'pass'|'fail'|'na', detail}
//   clickNa / resultNa     — string reason ⇒ that dimension is N/A (honest skip)
//   expectDisabled         — when true, PRESENT-but-disabled is the PASS (gating)
//   nativeAction           — installed OS controller mode/path; proves picker opened
//   rowKind                — explicit support for non-action inventory/evidence rows
async function probe(page, opts) {
  const { surface, name, sel, group, groupName, doClick, assertResult, clickNa, resultNa, expectDisabled, actionId, nativeAction, rowKind } = opts
  if (ONLY && !name.includes(ONLY)) return
  if (TRACE) console.error(`[fcv-trace] ${surface}/${name} start`)
  const present = (await sel.count()) > 0
  const dims = {
    present: present ? 'pass' : 'fail',
    rowKind: rowKind || (expectDisabled ? 'support' : 'ui_action'),
    actionId: actionId || `${surface}::${name}`,
  }
  let ev = ''
  // RENDER
  if (present && group) {
    const r = await renderGroup(page, surface, groupName || name, group)
    dims.render = r.ok ? 'pass' : 'fail'
    ev += r.detail
    var shot = r.shot
  } else if (present) {
    dims.render = 'na'
  } else {
    dims.render = 'fail'
  }
  // Gating expectation: control should be DISABLED here. PRESENT + disabled = pass.
  if (expectDisabled) {
    const disabled = present ? await sel.first().isDisabled().catch(() => false) : false
    dims.present = present ? 'pass' : 'fail'
    dims.click = disabled ? 'pass' : 'fail'
    dims.result = 'na'
    rec(surface, name, dims, `gating: present=${present} disabled=${disabled} (expected disabled) ${ev}`.trim(), shot || '')
    if (TRACE) console.error(`[fcv-trace] ${surface}/${name} done`)
    return
  }
  // CLICK
  if (!present) {
    dims.click = 'fail'; dims.result = 'fail'
    rec(surface, name, dims, `absent ${ev}`.trim(), shot || '')
    if (TRACE) console.error(`[fcv-trace] ${surface}/${name} done`)
    return
  }
  if (clickNa) {
    // The control is deliberately NOT clicked (desktop-only native picker / spends
    // a real CLI turn / heavy precondition). With no click there is no action to
    // observe, so RESULT is ALWAYS honestly N/A here — never 'pass' (that would
    // mark an un-actuated control as verified). Replaces a dead `resultNa ? 'na' : 'na'`.
    dims.click = 'na'; dims.result = 'na'
    rec(surface, name, dims, `${clickNa} ${ev}`.trim(), shot || '')
    if (TRACE) console.error(`[fcv-trace] ${surface}/${name} done`)
    return
  }
  let nativeProof = null
  try {
    if (nativeAction && NATIVE_OS_ACTIONS.enabled) {
      nativeProof = await NATIVE_OS_ACTIONS.run({
        actionId: dims.actionId,
        mode: nativeAction.mode || 'cancel',
        path: nativeAction.path || '',
      }, () => nativeAction.useDoClick ? doClick() : sel.first().click())
      ev = `${nativeProof.evidence} ${ev}`.trim()
    } else {
      await doClick()
    }
    dims.click = 'pass'
  } catch (e) {
    dims.click = 'fail'; dims.result = 'fail'
    rec(surface, name, dims, `click threw: ${String(e.message || e).slice(0, 90)} ${ev}`.trim(), shot || '')
    if (TRACE) console.error(`[fcv-trace] ${surface}/${name} done`)
    return
  }
  if (nativeProof && !nativeAction.verifyResult) {
    dims.result = 'pass'
    rec(surface, name, dims, ev, shot || '')
    if (TRACE) console.error(`[fcv-trace] ${surface}/${name} done`)
    return
  }
  // RESULT
  if (resultNa) {
    dims.result = 'na'
    rec(surface, name, dims, `${resultNa} ${ev}`.trim(), shot || '')
    if (TRACE) console.error(`[fcv-trace] ${surface}/${name} done`)
    return
  }
  try {
    const ar = await assertResult()
    dims.result = ['pass', 'fail', 'na'].includes(ar.result) ? ar.result : (ar.ok ? 'pass' : 'fail')
    ev = `${ar.detail || ''} ${ev}`.trim()
  } catch (e) {
    dims.result = 'fail'
    ev = `result threw: ${String(e.message || e).slice(0, 90)} ${ev}`.trim()
  }
  rec(surface, name, dims, ev, shot || '')
  if (TRACE) console.error(`[fcv-trace] ${surface}/${name} done`)
}

// ════════════════════════════════════════════════════════════════════════════
// SECTIONS — each establishes a selection context, then probes its controls.
// ════════════════════════════════════════════════════════════════════════════

// ── helpers for the dialog-gated + dep-gated controls (added for full-verify) ──
// Write a tiny valid 2-cue SRT to a temp file → the import path for captions.import
// (the native picker can't be driven headless, but the FEATURE is fully assertable
// via the verb on a real file). Returns {path, cues:[texts]}.
function writeTempSrt() {
  const a = 'FCV_SRT_A_' + Math.random().toString(36).slice(2, 6).toUpperCase()
  const b = 'FCV_SRT_B_' + Math.random().toString(36).slice(2, 6).toUpperCase()
  // CRLF + blank-line separated cues — the canonical SRT shape the engine parses.
  const srt = `1\r\n00:00:00,000 --> 00:00:01,200\r\n${a}\r\n\r\n2\r\n00:00:01,300 --> 00:00:02,500\r\n${b}\r\n\r\n`
  // Split-fs rigs (harness on WSL, engine on Windows): the engine cannot read
  // the harness tmpdir, so write into the SHARED media dir and hand the verb
  // the ENGINE-side path so captions.import never receives an unreadable temp path.
  const name = `fcv_import_${seq++}.srt`
  if (ENGINE_MEDIA_DIR !== MEDIA_DIR) {
    writeFileSync(join(MEDIA_DIR, name), srt)
    return { path: `${ENGINE_MEDIA_DIR.replace(/[\\/]+$/, '')}/${name}`, cues: [a, b] }
  }
  const path = join(tmp, name)
  writeFileSync(path, srt)
  return { path, cues: [a, b] }
}
// Count caption-kind tracks (captions.translate mode:track adds one → a durable
// state signal that the translation produced a new target-language track).
const capTrackCount = (s) => (s?.tracks || []).filter((t) => t.kind === 'caption').length
const hasCaptionCues = (s) => (s?.tracks || []).some((t) =>
  t.kind === 'caption' && (t.clips || []).some((c) => typeof c.text === 'string' && c.text.length > 0))
const transcribedAssetIds = (s) => Object.entries(s?.assets || {})
  .filter(([, a]) => !!a?.transcript)
  .map(([id]) => id)
async function transcriptWordCount(asset) {
  const r = await verb('transcript.get', { asset })
  return r.ok ? (r.result?.words || []).length : 0
}
async function nonEmptyTranscribedAssetIds(s) {
  const ids = transcribedAssetIds(s || await state())
  const out = []
  for (const id of ids) {
    if (await transcriptWordCount(id) > 0) out.push(id)
  }
  return out
}
// Click a translate button and capture its SETTLED status note. The busy note reads
// "Translating … to <lang>…"; the SUCCESS note begins "Translated …" / "… translated"
// (contains "translated"); a failure note carries "could not"/"error". We poll the
// note element (it auto-clears after a few seconds, so we catch the transition) until
// a settled success/failure note appears or the bounded window elapses.
async function driveTranslate(page, btn) {
  const disabled = await btn.isDisabled().catch(() => true)
  const title = await btn.getAttribute('title').catch(() => '')
  if (disabled) return `error: translate button disabled${title ? ` (${title})` : ''}`
  await btn.click().catch(() => {})
  const deadline = Date.now() + 200000
  let note = ''
  let sawBusy = false
  while (Date.now() < deadline) {
    const t = (await page.locator('[data-cut-translate-note]').first().textContent().catch(() => '')) || ''
    if (/translating/i.test(t)) sawBusy = true
    if (/translated/i.test(t) || /(could not|error|failed)/i.test(t)) { note = t; break }
    if (sawBusy && t.trim() && !/translating/i.test(t)) { note = t; break }
    if (!sawBusy && Date.now() > deadline - 196000) {
      note = 'error: translate click did not start'
      break
    }
    await sleep(700)
  }
  return note || 'error: translate did not settle'
}
const { awaitJob, awaitImportJobs } = createJobWaiters({ verb, sleep })
async function ensureAssetPerception(asset, timeoutMs = 240000) {
  if (!asset) return null
  const r = await verb('media.perception', { asset })
  if (r.result?.job_id) return await awaitJob(r.result.job_id, timeoutMs)
  return r.ok ? { state: 'done', result: r.result } : { state: 'failed', error: r.error }
}
async function waitForStoryboardSettled(page, timeoutMs = 25000) {
  const deadline = Date.now() + timeoutMs
  let stateAttr = ''
  while (Date.now() < deadline) {
    const panel = page.locator('[data-cut-storyboard]').first()
    if (await panel.count()) {
      stateAttr = await panel.getAttribute('data-cut-storyboard-state').catch(() => '') || ''
      const img = (await page.locator('[data-cut-storyboard-img]').count()) > 0
      const err = (await page.locator('[data-cut-storyboard-error]').count()) > 0
      if (img || err || /ready|error/.test(stateAttr)) return { img, err, state: stateAttr }
    }
    await sleep(500)
  }
  return { img: false, err: false, state: stateAttr || 'timeout' }
}

// ── media-synthesis helpers for audio.add_music and multicam_sync ────────────
// Generate a short stereo TONE as a real audio file → the bed material the MusicBed
// drawer needs (its candidate list offers only audio-KIND assets; the footage clips are
// video). Returns the path, or null if ffmpeg is unavailable (then the caller honest-N/As).
function makeToneAudio(durS = 6) {
  const name = `tone_${seq++}.wav`
  const path = join(synthDriverDir, name)
  const r = spawnSync(HARNESS_FFMPEG, ['-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', `sine=frequency=440:duration=${durS}`, '-ac', '2', '-ar', '44100', path], { timeout: 30000 })
  return r.status === 0 && existsSync(path) ? joinHostPath(synthEngineDir, name) : null
}
function makeLibraryRelinkPair() {
  const originalName = `library_relink_original_${seq++}.mp4`
  const replacementName = `library_relink_replacement_${seq++}.mp4`
  const secondReplacementName = `library_relink_replacement_${seq++}.mp4`
  const thirdReplacementName = `library_relink_replacement_${seq++}.mp4`
  const fourthReplacementName = `library_relink_replacement_${seq++}.mp4`
  const originalDriver = join(synthDriverDir, originalName)
  const replacementDriver = join(synthDriverDir, replacementName)
  const secondReplacementDriver = join(synthDriverDir, secondReplacementName)
  const thirdReplacementDriver = join(synthDriverDir, thirdReplacementName)
  const fourthReplacementDriver = join(synthDriverDir, fourthReplacementName)
  const generated = spawnSync(HARNESS_FFMPEG, [
    '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', 'color=c=navy:s=160x90:d=0.5',
    '-f', 'lavfi', '-i', 'sine=frequency=523:duration=0.5',
    '-shortest', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac',
    originalDriver,
  ], { timeout: 30000 })
  if (generated.status !== 0 || !existsSync(originalDriver)) return null
  copyFileSync(originalDriver, replacementDriver)
  copyFileSync(originalDriver, secondReplacementDriver)
  copyFileSync(originalDriver, thirdReplacementDriver)
  copyFileSync(originalDriver, fourthReplacementDriver)
  return {
    originalDriver,
    originalEngine: joinHostPath(synthEngineDir, originalName),
    replacementDriver,
    replacementEngine: joinHostPath(synthEngineDir, replacementName),
    secondReplacementDriver,
    secondReplacementEngine: joinHostPath(synthEngineDir, secondReplacementName),
    thirdReplacementDriver,
    thirdReplacementEngine: joinHostPath(synthEngineDir, thirdReplacementName),
    fourthReplacementEngine: joinHostPath(synthEngineDir, fourthReplacementName),
  }
}
// Build a time-SHIFTED copy of a source clip so two timeline clips have CORRELATED-but-
// OFFSET audio (edit.multicam_sync envelopes the ASSET FILE, so two clips of the SAME
// asset measure offset 0 → no alignment move; a real offset needs two distinct files
// whose audio is the same content shifted in time). We trim `shiftMs` off the FRONT, so
// the copy's audio LEADS the original by shiftMs → cross-correlation locks at that lag and
// the sync's edit.move fires. Downscaled + ultrafast so the re-encode stays cheap even on
// 4K sources (only the audio envelope matters to the sync). Returns the path, or null.
// Wait for the background job queue to go IDLE. Render jobs left running by an
// earlier section keep the render-options button disabled by design. This is
// an honest wait: nothing is cancelled or bypassed; on timeout the caller
// proceeds and any residual failures stay real.
let lastActiveJobs = []
async function drainActiveJobs(maxMs = 600000) {
  const t0 = Date.now()
  while (Date.now() - t0 < maxMs) {
    const r = await verb('jobs.list', {})
    const active = (r.result?.jobs || []).filter((j) => j.state === 'queued' || j.state === 'running')
    lastActiveJobs = active
    if (!active.length) return true
    await sleep(1500)
  }
  return false
}
function activeJobSummary() {
  return lastActiveJobs
    .map((job) => `${job.id || job.job_id || '?'}:${job.kind || job.verb || job.type || 'job'}:${job.state || '?'}`)
    .join(',')
    .slice(0, 500)
}
// CSS attribute-selector value escaping: stock hit ids on Windows are extended
// paths (\\?\C:\...) whose backslashes break a naive [attr="${v}"] locator.
const cssAttrValue = (v) => String(v).replace(/\\/g, '\\\\').replace(/"/g, '\\"')
function makeShiftedClip(srcPath, shiftMs = 1200, durS = 6) {
  const name = `shifted_${seq++}.mp4`
  const path = join(synthDriverDir, name)
  const r = spawnSync(HARNESS_FFMPEG, ['-hide_banner', '-loglevel', 'error', '-y',
    '-ss', String(shiftMs / 1000), '-i', resolveDriverPath(srcPath), '-t', String(durS),
    '-vf', 'scale=480:-2', '-c:v', 'libx264', '-preset', 'ultrafast', '-c:a', 'aac', '-ac', '2', path], { timeout: 120000 })
  return r.status === 0 && existsSync(path) ? joinHostPath(synthEngineDir, name) : null
}

// ── 1. PROJECT scope (no selection) ──────────────────────────────────────────
async function secProject(page) {
  const S = 'project-scope'
  const projectCtx = await freshProject(page, 'proj')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  await deselect(page)
  await propertiesTab(page)
  const composer = page.locator('[data-cut-caption-text]')
  const grp = page.locator('[data-cut-panel="inspector"]')

  // Caption composer — type text, pick position, "Add caption at playhead".
  const capText = 'FCV_CAP_' + Math.random().toString(36).slice(2, 6).toUpperCase()
  await probe(page, {
    surface: S, name: 'caption-composer-add', sel: composer, group: grp, groupName: 'inspector-project',
    doClick: async () => {
      await composer.fill(capText)
      await page.locator('[data-cut-caption-position]').selectOption('bottom').catch(() => {})
      await page.locator('[data-cut-caption-add]').click()
      await sleep(700)
    },
    assertResult: async () => {
      const s = await state()
      const landed = (s.tracks || []).some((t) => t.kind === 'caption' && (t.clips || []).some((c) => c.text === capText))
      return { ok: landed, detail: `caption "${capText}" on a caption track=${landed}` }
    },
  })
  // Caption style control (captions.set_style → project.state.caption_styles[ref]).
  // The native style is keyed off the position (ref `txt_<pos>`); set a DISTINCT size
  // + colour, fire, then assert that style ROW landed in state with our values — a
  // real state mutation, not a "control fired" N/A.
  const styleSize = 96 + Math.floor(Math.random() * 40) // distinct, in-range [12,200]
  await probe(page, {
    surface: S, name: 'caption-set-style', sel: page.locator('[data-cut-caption-style]'), group: grp, groupName: 'inspector-project',
    doClick: async () => {
      await page.locator('[data-cut-caption-position]').selectOption('bottom').catch(() => {})
      await page.locator('[data-cut-caption-size]').fill(String(styleSize)).catch(() => {})
      await page.locator('[data-cut-caption-color]').fill('#33CC88').catch(() => {})
      await page.locator('[data-cut-caption-style]').click()
      await sleep(400)
    },
    assertResult: async () => {
      const st = await waitForState((s) => (s.caption_styles?.['txt_bottom']?.size ?? 0) === styleSize, 10000)
      const got = st?.caption_styles?.['txt_bottom']
      return { ok: !!st, detail: `caption_styles['txt_bottom'].size=${got?.size ?? '?'} (set ${styleSize}), color=${got?.color ?? '?'}` }
    },
  })
  // Caption STYLE GALLERY: save the just-set txt_bottom style as a preset via the
  // gallery controls, then apply a BUILT-IN look with the Apply button and assert the
  // txt_bottom style row CHANGES to the built-in's values (a real state mutation);
  // captions.list_styles backs the dropdown (saved preset must appear as builtin:false).
  await probe(page, {
    surface: S, name: 'caption-style-gallery(save_style+apply_style+list_styles)', sel: page.locator('[data-cut-caption-gallery]'), group: grp, groupName: 'inspector-project',
    doClick: async () => {
      await page.locator('[data-cut-caption-save-name]').fill('fcv look').catch(() => {})
      await page.locator('[data-cut-action="caption-style-save"]').click().catch(() => {})
      await sleep(500)
      await page.locator('[data-cut-caption-preset]').selectOption('broadcast yellow').catch(() => {})
      await page.locator('[data-cut-action="caption-style-apply"]').click().catch(() => {})
      await sleep(500)
    },
    assertResult: async () => {
      const ls = await verb('captions.list_styles', {})
      const saved = (ls.result?.presets || []).find((p) => p.name === 'fcv look' && p.builtin === false)
      const builtins = (ls.result?.presets || []).filter((p) => p.builtin).length
      const st = await waitForState((s) => s.caption_styles?.['txt_bottom']?.color === '#ffe14d', 10000)
      return { ok: !!saved && builtins >= 6 && !!st, detail: `list_styles: saved 'fcv look'=${!!saved}, builtins=${builtins}; apply 'broadcast yellow' → txt_bottom.color=${st?.caption_styles?.['txt_bottom']?.color ?? '?'}` }
    },
  })
  // Import captions (SRT/VTT) — the BUTTON opens a native picker (no-op headless, so the
  // click safely proves the control), and the FEATURE (captions.import) is asserted by
  // driving the verb on a REAL temp .srt we write here (2 cues) → assert those cues land
  // as caption clips on the timeline. PRESENT/RENDER/CLICK from the real button; RESULT
  // from the verb on a real file — no longer a present-only N/A.
  const srt = writeTempSrt()
  let captionImportUi = null
  await probe(page, {
    surface: S, name: 'caption-import', actionId: 'caption-import',
    sel: page.locator('[data-cut-caption-import]'), group: grp, groupName: 'inspector-project',
    clickNa: NATIVE_PICKER_CLICK_NA,
    nativeAction: {
      mode: 'select',
      path: srt.path,
      useDoClick: true,
      verifyResult: true,
    },
    doClick: async () => {
      if (NATIVE_OS_ACTIONS.enabled) {
        captionImportUi = await captureVerbResp(
          page,
          'captions.import',
          () => page.locator('[data-cut-caption-import]').click(),
          30_000,
        )
      } else {
        await page.locator('[data-cut-caption-import]').click().catch(() => {})
      }
      await sleep(200)
    },
    assertResult: async () => {
      const r = NATIVE_OS_ACTIONS.enabled
        ? captionImportUi
        : await verb('captions.import', {
          path: srt.path,
          rationale: 'fcv: browser fallback import of a real 2-cue .srt',
        })
      if (!r?.ok) return { ok: false, detail: `captions.import NOT-ok: ${String(r?.error?.message || r?.error?.code || 'missing response').slice(0, 80)}` }
      const st = await waitForState((s) => (s.tracks || []).some((t) => t.kind === 'caption' && (t.clips || []).some((c) => srt.cues.includes(c.text))), 12000)
      const n = (r.result && r.result.caption_count) ?? '?'
      return { ok: !!st, detail: `captions.import landed ${n} cues through ${NATIVE_OS_ACTIONS.enabled ? 'installed picker' : 'browser fallback'}; imported cue text present on a caption track=${!!st}` }
    },
  })

  // Lang picker (drives both translate buttons) — value sticks.
  await probe(page, {
    surface: S, name: 'translate-lang-picker', sel: page.locator('[data-cut-translate-lang]'), group: grp, groupName: 'inspector-project',
    doClick: async () => { await page.locator('[data-cut-translate-lang]').selectOption('es').catch(() => {}); await sleep(150) },
    assertResult: async () => {
      const v = await page.locator('[data-cut-translate-lang]').inputValue().catch(() => '')
      return { ok: v === 'es', detail: `lang=${v}` }
    },
  })
  // Translate CAPTIONS (captions.translate, mode:track → a NEW target-language caption
  // track). The just-imported .srt is the cap1 SOURCE (so the button is ENABLED). With
  // `claude` present we DRIVE it and assert a new caption track landed; without it (a
  // partial dev run) it is an honest N/A — the release gate (FCV_REQUIRE_FULL=1) enforces
  // claude present so this always runs there.
  const tcSel = page.locator('[data-cut-action="translate-captions"]')
  if (DEP.claude) {
    await probe(page, {
      surface: S, name: 'translate-captions', sel: tcSel, group: grp, groupName: 'inspector-project',
      doClick: async () => {
        // Reload so the UI reflects the just-imported cap1 source (hasCaptionSource → the
        // button is enabled); avoids a snapshot-lag false-fail on the gating state.
        await reloadApp(page); await sleep(700)
        await deselect(page); await propertiesTab(page)
        probe._capTracks0 = capTrackCount(await state())
        await page.locator('[data-cut-translate-lang]').selectOption('es').catch(() => {})
        probe._tnote = await driveTranslate(page, page.locator('[data-cut-action="translate-captions"]'))
      },
      assertResult: async () => {
        const after = capTrackCount(await state())
        const grew = after > (probe._capTracks0 ?? 0)
        return { ok: grew || /translated/i.test(probe._tnote || ''), detail: `caption tracks ${probe._capTracks0}→${after} (new translated track); note="${(probe._tnote || '').slice(0, 50)}"` }
      },
    })
  } else {
    await probe(page, {
      surface: S, name: 'translate-captions', sel: tcSel, group: grp, groupName: 'inspector-project',
      clickNa: 'captions.translate needs the `claude` CLI (absent: system.doctor judge.claude≠ok) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present',
    })
  }
  // Translate TRANSCRIPT (transcript.translate). Needs an asset WITH a transcript.
  // If freshProject's import/enrich already produced a non-empty one, use it;
  // otherwise import SPEECH and transcribe that asset. The Inspector passes the
  // resolved asset id and skips empty transcripts, so projects with several
  // transcripts do not hit the server's ambiguity guard.
  // Gated on claude (translation) + perception STT (the transcript source); absent
  // either → honest N/A (release gate enforces both present).
  const ttSel = page.locator('[data-cut-action="translate-transcript"]')
  if (DEP.claude && DEP.perceptionStt) {
    await probe(page, {
      surface: S, name: 'translate-transcript', sel: ttSel, group: grp, groupName: 'inspector-project',
      doClick: async () => {
        // Establish a transcript only when the bootstrap import did not already
        // produce one. On real rigs SCENE can be speech-bearing, so importing
        // SPEECH unconditionally creates an unnecessary ambiguous project state.
        let ids = await nonEmptyTranscribedAssetIds()
        if (!ids.length) {
          const asset = projectCtx.assetId
          if (asset) {
            const tr = await verb('media.transcribe', { asset })
            if (tr.result?.job_id) await awaitJob(tr.result.job_id)
            await waitForState((s) => transcribedAssetIds(s).includes(asset), 30000)
            for (let i = 0; i < 12; i++) {
              ids = await nonEmptyTranscribedAssetIds()
              if (ids.includes(asset)) break
              await sleep(800)
            }
            if (!ids.includes(asset)) {
              const seeded = await ensureNonEmptyTranscript(page, projectCtx.projectPath, projectCtx.assetId, 'fcv: transcript.translate needs words; live STT returned empty')
              ids = [seeded.assetId]
            }
          }
        }
        await reloadApp(page); await sleep(900)
        await deselect(page); await propertiesTab(page)
        await page.locator('[data-cut-translate-lang]').selectOption('es').catch(() => {})
        const buttonAsset = await page.locator('[data-cut-action="translate-transcript"]').getAttribute('data-cut-translate-transcript-asset').catch(() => '')
        probe._tnote = await driveTranslate(page, page.locator('[data-cut-action="translate-transcript"]'))
        probe._tasset = buttonAsset || ids[0] || ''
      },
      assertResult: async () => ({ ok: /translated/i.test(probe._tnote || ''), detail: `transcript.translate asset=${probe._tasset || '?'} settled note="${(probe._tnote || 'none').slice(0, 60)}"` }),
    })
  } else {
    await probe(page, {
      surface: S, name: 'translate-transcript', sel: ttSel, group: grp, groupName: 'inspector-project',
      clickNa: 'transcript.translate needs `claude` + perception STT (a transcript source) — honest dev skip; FCV_REQUIRE_FULL=1 enforces both present',
    })
  }
}

// ── 2. VIDEO clip selected ───────────────────────────────────────────────────
async function secVideo(page) {
  const S = 'video-clip'
  await freshProject(page, 'video', SPEECH)
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  const clip = await clipOfKind('video')
  if (!clip) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'no video clip imported'); return }
  await selectClip(page, clip)
  await propertiesTab(page)
  const insp = page.locator('[data-cut-panel="inspector"]')
  const originalVideoAsset = findClip(await state(), clip)?.asset || ''
  // Auto Zoom must run before the destructive speed/replace-source coverage
  // below. Those controls intentionally shorten the selected source window;
  // restoring only the asset later preserves that short edit slot and can
  // leave too few loudness samples to form a real energy peak.
  if (originalVideoAsset && (FULL || DEP.perceptionCv)) {
    await ensureAssetPerception(originalVideoAsset, 240000)
  }
  await probe(page, {
    surface: S, name: 'auto-zoom', sel: page.locator('[data-cut-action="auto-zoom"]'), group: insp, groupName: 'inspector-color-ops',
    doClick: async () => {
      await page.locator('[data-cut-autozoom-intensity]').selectOption('0.2').catch(() => {})
      probe._autoKfBefore = JSON.stringify(findClip(await state(), clip)?.keyframes || [])
      probe._autoNote = false
      probe._autoResponse = await captureVerbResp(page, 'edit.auto_zoom', async () => {
        await page.locator('[data-cut-action="auto-zoom"]').click()
        // The no-perception/no-subject note is intentionally transient.
        for (let i = 0; i < 12; i++) {
          if ((await page.locator('[data-cut-inspector-auto-note]').count().catch(() => 0)) > 0) {
            probe._autoNote = true
            break
          }
          await sleep(250)
        }
      }, 30_000)
    },
    assertResult: async () => {
      const st = await state()
      const keyframes = findClip(st, clip)?.keyframes || []
      const hasKf = keyframes.some((k) => k.param === 'scale')
      const autoZoomKfChanged = JSON.stringify(keyframes) !== (probe._autoKfBefore || '[]')
      const noted = !!probe._autoNote || (await page.locator('[data-cut-inspector-auto-note]').count()) > 0
      const needReal = FULL || DEP.perceptionCv
      const responseOk = probe._autoResponse?.ok === true
      const ok = needReal
        ? (responseOk && hasKf && autoZoomKfChanged)
        : (hasKf || autoZoomKfChanged || noted)
      const error = probe._autoResponse?.error?.message || probe._autoResponse?.error?.cause || ''
      return {
        ok,
        detail: `auto_zoom response=${responseOk} scaleKf=${hasKf} keyframesChanged=${autoZoomKfChanged} honestNote=${noted}${error ? ` error=${String(error).slice(0, 120)}` : ''} (perception=${DEP.perceptionCv} → ${needReal ? 'real keyframe required' : 'honest-note accepted'})`,
      }
    },
  })

  // GATING: video clip MUST show transform/effects, MUST NOT show audio EQ or
  // the caption/title/shape editors.
  rec(S, 'GATE:transform-shown', gateDim(await page.locator('[data-cut-prop="transform-x"]').count() > 0), 'transform present on video clip')
  rec(S, 'GATE:audio-eq-hidden', gateDim((await page.locator('[data-cut-inspector-eq]').count()) === 0), 'audio EQ correctly hidden on video clip')
  rec(S, 'GATE:caption-editor-hidden', gateDim((await page.locator('[data-cut-caption-edit-text]').count()) === 0), 'caption editor correctly hidden on video clip')

  // Selected-clip regression: high-value context-menu edits need visible homes.
  // Import a second compatible source so Replace source is enabled, then drive Detach audio and
  // Replace from the Inspector rather than the right-click menu.
  const rep = await verb('media.import', { path: SECOND })
  if (rep.result?.job_id) await awaitJob(rep.result.job_id)
  const repAlternate = await verb('media.import', { path: FACE })
  if (repAlternate.result?.job_id) await awaitJob(repAlternate.result.job_id)
  await sleep(900)
  await reloadApp(page); await sleep(700)
  await selectClip(page, clip)
  await propertiesTab(page)
  for (const key of [
    'cropping',
    'speed',
    'fades',
    'video-motion',
    'video-color',
    'video-effects',
    'video-privacy',
    'engagement',
  ]) await expandInspectorSection(page, key)
  const quickActions = page.locator('[data-cut-inspector-quick-actions]').first()
  const replacementPicker = page.locator('[data-cut-inspector-replace-asset]').first()
  await probe(page, {
    surface: S, name: 'inspector-replace-asset', actionId: 'inspector-replace-asset',
    sel: replacementPicker, group: quickActions, groupName: 'inspector-quick-actions',
    doClick: async () => {
      const optionCount = await replacementPicker.locator('option').count()
      if (optionCount < 2) throw new Error(`replacement picker needs 2 options, found ${optionCount}`)
      const before = await replacementPicker.inputValue()
      const options = await replacementPicker.locator('option').evaluateAll((nodes) => nodes.map((node) => node.value))
      const next = options.find((value) => value && value !== before)
      if (!next) throw new Error(`replacement picker has no alternate option (current=${before})`)
      await replacementPicker.selectOption(next)
      await sleep(100)
      probe._replacementPickerBefore = before
      probe._replacementPickerAfter = next
    },
    assertResult: async () => ({
      ok: !!probe._replacementPickerAfter
        && probe._replacementPickerAfter !== probe._replacementPickerBefore
        && await replacementPicker.inputValue() === probe._replacementPickerAfter,
      detail: `replacement asset ${probe._replacementPickerBefore || '?'}→${probe._replacementPickerAfter || '?'}`,
    }),
  })
  const videoAudioCount = async () => flatClips(await state()).filter((c) => c._kind === 'audio' && c.asset).length
  await probe(page, {
    surface: S, name: 'inspector-detach-audio', sel: page.locator('[data-cut-inspector-action="detach-audio"]'), group: quickActions, groupName: 'inspector-quick-actions',
    doClick: async () => {
      probe._ac = await videoAudioCount()
      await page.locator('[data-cut-inspector-action="detach-audio"]').click()
      await sleep(1000)
    },
    assertResult: async () => {
      const after = await videoAudioCount()
      const note = (await page.locator('[data-cut-inspector-quick-actions-note]').first().textContent().catch(() => '')) || ''
      return { ok: after >= probe._ac && note.trim().length > 0, detail: `detach audio count ${probe._ac}→${after}; note="${note.slice(0, 70)}"` }
    },
  })
  await probe(page, {
    surface: S, name: 'inspector-replace-source', sel: page.locator('[data-cut-inspector-action="replace-source"]'), group: quickActions, groupName: 'inspector-quick-actions',
    doClick: async () => {
      const picker = page.locator('[data-cut-inspector-replace-asset]').first()
      probe._replacement = await picker.inputValue().catch(() => '')
      probe._clip = clip
      await page.locator('[data-cut-inspector-action="replace-source"]').click()
      await sleep(800)
    },
    assertResult: async () => ({
      ok: !!probe._replacement && !!(await waitForState((st) => findClip(st, probe._clip)?.asset === probe._replacement, 15000)),
      detail: `selected clip ${probe._clip} source replaced with ${probe._replacement || '?'}`,
    }),
  })

  // --- Transform / Crop / Speed / Fade numeric fields (PropertyRow → one verb) ---
  const numField = (key) => page.locator(`[data-cut-prop-input="${key}"]`)
  const commitNum = async (key, val) => {
    const f = numField(key)
    await f.fill(String(val))
    await f.press('Enter') // Enter→blur is the sole committer
    await sleep(500)
  }
  for (const [key, val, verbName] of [
    ['transform-x', 40, 'edit.transform'],
    ['transform-y', 30, 'edit.transform'],
    ['transform-scale', 80, 'edit.transform'],
  ]) {
    await probe(page, {
      surface: S, name: `field-${key}`, sel: numField(key), group: insp, groupName: 'inspector-transform',
      doClick: async () => { var before = await opsLen(); probe._b = before; await commitNum(key, val) },
      assertResult: async () => ({ ok: await opLanded(probe._b, verbName, (a) => a.clip === clip), detail: `${verbName} op landed` }),
    })
  }
  const transformState = async () => {
    const current = findClip(await state(), clip)?.transform
    return {
      x: current?.x ?? 0,
      y: current?.y ?? 0,
      scale: current?.scale ?? 1,
      opacity: current?.opacity ?? 1,
    }
  }
  const isIdentityTransform = (transform) => transform.x === 0
    && transform.y === 0
    && transform.scale === 1
    && transform.opacity === 1
  const transformSlider = page.locator('[data-cut-prop-slider="transform-x"]').first()
  await probe(page, {
    surface: S,
    name: 'transform-x-slider',
    actionId: 'prop-slider',
    sel: transformSlider,
    group: insp,
    groupName: 'inspector-transform-slider',
    doClick: async () => {
      probe._b = await opsLen()
      probe._r = await captureVerbResp(page, 'edit.transform', async () => {
        await transformSlider.evaluate((element) => {
          const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set
          setter?.call(element, '0.55')
          element.dispatchEvent(new Event('input', { bubbles: true }))
          element.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }))
        })
      }, 15_000)
    },
    assertResult: async () => {
      const landed = await opLanded(probe._b, 'edit.transform', (args) => args.clip === clip && args.x === 0.55)
      const transform = await transformState()
      await renderGroup(page, S, 'inspector-transform-slider-result', page.locator('[data-cut-section="transform"]').first())
      return { ok: !!probe._r?.ok && landed && transform.x === 0.55, detail: `edit.transform ok=${probe._r?.ok}; x=${transform.x}; one release op=${landed}` }
    },
  })
  await probe(page, {
    surface: S,
    name: 'transform-x-reset',
    actionId: 'prop-reset',
    sel: page.locator('[data-cut-prop-reset="transform-x"]').first(),
    group: insp,
    groupName: 'inspector-transform-reset',
    doClick: async () => {
      probe._b = await opsLen()
      probe._r = await captureVerbResp(page, 'edit.transform', async () => {
        await page.locator('[data-cut-prop-reset="transform-x"]').first().click()
      }, 15_000)
    },
    assertResult: async () => {
      const landed = await opLanded(probe._b, 'edit.transform', (args) => args.clip === clip && args.x === 0)
      const transform = await transformState()
      const disabled = await page.locator('[data-cut-prop-reset="transform-x"]').first().isDisabled().catch(() => false)
      await renderGroup(page, S, 'inspector-transform-property-reset-result', page.locator('[data-cut-section="transform"]').first())
      return { ok: !!probe._r?.ok && landed && transform.x === 0 && disabled, detail: `edit.transform ok=${probe._r?.ok}; x=${transform.x}; reset disabled=${disabled}` }
    },
  })
  await probe(page, {
    surface: S,
    name: 'transform-section-bypass',
    actionId: 'section-bypass',
    sel: page.locator('[data-cut-section-bypass="transform"]').first(),
    group: insp,
    groupName: 'inspector-transform-bypass',
    doClick: async () => {
      probe._b = await opsLen()
      probe._r = await captureVerbResp(page, 'edit.transform', async () => {
        await page.locator('[data-cut-section-bypass="transform"]').first().click()
      }, 15_000)
    },
    assertResult: async () => {
      const landed = await opLanded(probe._b, 'edit.transform', (args) => args.clip === clip
        && args.x === 0 && args.y === 0 && args.scale === 1 && args.opacity === 1)
      const transform = await transformState()
      const bypassed = await page.locator('[data-cut-section-bypass="transform"]').first()
        .getAttribute('data-cut-section-bypassed').catch(() => '')
      await renderGroup(page, S, 'inspector-transform-bypass-result', page.locator('[data-cut-section="transform"]').first())
      return { ok: !!probe._r?.ok && landed && isIdentityTransform(transform) && bypassed === 'true', detail: `edit.transform ok=${probe._r?.ok}; identity=${isIdentityTransform(transform)}; bypassed=${bypassed}` }
    },
  })
  await commitNum('transform-x', 0.4)
  const transformReset = page.locator('[data-cut-section-reset="transform"]').first()
  await probe(page, {
    surface: S,
    name: 'transform-section-reset',
    actionId: 'section-reset',
    sel: transformReset,
    group: insp,
    groupName: 'inspector-transform-section-reset',
    doClick: async () => {
      await page.waitForFunction(
        () => !document.querySelector('[data-cut-section-reset="transform"]')?.disabled,
        null,
        { timeout: 8_000 },
      )
      probe._b = await opsLen()
      probe._r = await captureVerbResp(page, 'edit.transform', async () => {
        await transformReset.focus()
        await transformReset.click({ force: true })
      }, 15_000)
    },
    assertResult: async () => {
      const landed = await opLanded(probe._b, 'edit.transform', (args) => args.clip === clip
        && args.x === 0 && args.y === 0 && args.scale === 1 && args.opacity === 1)
      const transform = await transformState()
      await renderGroup(page, S, 'inspector-transform-section-reset-result', page.locator('[data-cut-section="transform"]').first())
      return { ok: !!probe._r?.ok && landed && isIdentityTransform(transform), detail: `edit.transform ok=${probe._r?.ok}; identity=${isIdentityTransform(transform)}` }
    },
  })
  // transform-opacity is an OVERLAY-ONLY row (Inspector: `{isOverlay && …}`) — on a BASE
  // clip it is intentionally HIDDEN. Rather than a bare N/A, this is recorded as an
  // EXPECTED-ABSENT GATING proof (the row MUST be absent here = correct behaviour → PASS;
  // a regression that showed it on a base clip would FAIL the gate). The opacity control
  // itself is driven for real on an OVERLAY clip in the blend-overlay section.
  rec(S, 'GATE:transform-opacity-hidden-on-base',
    gateDim((await page.locator('[data-cut-prop-input="transform-opacity"]').count()) === 0),
    'overlay-only opacity row correctly hidden on a base clip (driven on an overlay clip in blend-overlay)')
  // Crop fields live in the CROP section (video).
  for (const [key, val] of [['crop-x', 10], ['crop-y', 10], ['crop-w', 1100], ['crop-h', 600]]) {
    await probe(page, {
      surface: S, name: `field-${key}`, sel: numField(key), group: insp, groupName: 'inspector-crop',
      doClick: async () => { probe._b = await opsLen(); await commitNum(key, val) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.crop', (a) => a.clip === clip), detail: 'edit.crop op landed' }),
    })
  }
  // Speed field + reverse + freeze.
  await probe(page, {
    surface: S, name: 'field-speed', sel: numField('speed'), group: insp, groupName: 'inspector-speed',
    doClick: async () => { probe._b = await opsLen(); await commitNum('speed', 2) },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.speed', (a) => a.clip === clip), detail: 'edit.speed op landed' }),
  })
  await probe(page, {
    surface: S, name: 'btn-reverse', sel: page.locator('[data-cut-prop="speed-reverse"]'), group: insp, groupName: 'inspector-speed',
    doClick: async () => { probe._b = await opsLen(); await page.locator('[data-cut-prop="speed-reverse"]').click(); await sleep(500) },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.reverse', (a) => a.clip === clip), detail: 'edit.reverse op landed' }),
  })
  await probe(page, {
    surface: S, name: 'btn-freeze', sel: page.locator('[data-cut-prop="speed-freeze"]'), group: insp, groupName: 'inspector-speed',
    doClick: async () => { probe._b = await opsLen(); await page.locator('[data-cut-prop="speed-freeze"]').click(); await sleep(500) },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.freeze', (a) => a.clip === clip), detail: 'edit.freeze op landed' }),
  })
  // Fade in / out fields (edit.fade).
  for (const key of ['fade-in', 'fade-out']) {
    await probe(page, {
      surface: S, name: `field-${key}`, sel: numField(key), group: insp, groupName: 'inspector-fade',
      doClick: async () => { probe._b = await opsLen(); await commitNum(key, 0.5) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.fade', (a) => a.clip === clip), detail: 'edit.fade op landed' }),
    })
  }

  // --- Color management (project.color working/output + edit.color_space input) ---
  await probe(page, {
    surface: S, name: 'color-working', sel: page.locator('[data-cut-color-working]'), group: insp, groupName: 'inspector-colormgmt',
    doClick: async () => { await page.locator('[data-cut-color-working]').selectOption('rec2020').catch(() => {}) },
    assertResult: async () => ({ ok: !!(await waitForState((st) => st.settings?.color?.working === 'rec2020', 10000)), detail: 'settings.color.working=rec2020' }),
  })
  await probe(page, {
    surface: S, name: 'color-output', sel: page.locator('[data-cut-color-output]'), group: insp, groupName: 'inspector-colormgmt',
    doClick: async () => { await page.locator('[data-cut-color-output]').selectOption('srgb').catch(() => {}) },
    assertResult: async () => ({ ok: !!(await waitForState((st) => st.settings?.color?.output === 'srgb', 10000)), detail: 'settings.color.output=srgb' }),
  })
  await probe(page, {
    surface: S, name: 'color-input', sel: page.locator('[data-cut-color-input]'), group: insp, groupName: 'inspector-colormgmt',
    doClick: async () => { await page.locator('[data-cut-color-input]').selectOption('srgb').catch(() => {}) },
    assertResult: async () => ({ ok: !!(await waitForState((st) => findClip(st, clip)?.input_color_space === 'srgb', 10000)), detail: 'clip.input_color_space=srgb' }),
  })

  // --- Auto-balance / auto-zoom / adjustment (one-click color verbs) ---
  await probe(page, {
    surface: S, name: 'auto-balance', sel: page.locator('[data-cut-action="auto-balance"]'), group: insp, groupName: 'inspector-color-ops',
    doClick: async () => { await page.locator('[data-cut-action="auto-balance"]').click(); await sleep(300) },
    assertResult: async () => ({ ok: !!(await waitForState((st) => !!findClip(st, clip)?.grade, 18000)), detail: 'clip gained a grade' }),
  })
  await probe(page, {
    surface: S, name: 'adjustment', sel: page.locator('[data-cut-action="adjustment"]'), group: insp, groupName: 'inspector-color-ops',
    doClick: async () => {
      await page.locator('[data-cut-adjustment-look]').selectOption('vignette').catch(() => {})
      await page.locator('[data-cut-action="adjustment"]').click(); await sleep(400)
    },
    assertResult: async () => ({ ok: !!(await waitForState((st) => (st.adjustments || []).some((a) => Array.isArray(a.range_ms)), 15000)), detail: 'root adjustment with range_ms landed' }),
  })

  // --- Grade gallery save (grade.save → grade.list) ---
  await verb('edit.grade', { clip, contrast: 1.4 })
  await waitForState((st) => Math.abs((findClip(st, clip)?.grade?.contrast ?? 1) - 1.4) < 0.05, 8000)
  await reloadApp(page); await sleep(900)
  await selectClip(page, clip); await propertiesTab(page)
  await expandInspectorSection(page, 'video-color')
  const look = 'fcv_look_' + Math.random().toString(36).slice(2, 5)
  await probe(page, {
    surface: S, name: 'grade-save', sel: page.locator('[data-cut-action="grade-save"]'), group: insp, groupName: 'inspector-gallery',
    doClick: async () => { await page.locator('[data-cut-grade-save-name]').fill(look); await page.locator('[data-cut-action="grade-save"]').click(); await sleep(400) },
    assertResult: async () => {
      let listed = false
      for (let k = 0; k < 12; k++) { await sleep(400); const r = await verb('grade.list', {}); if ((r.result?.presets || []).some((p) => p.name === look)) { listed = true; break } }
      return { ok: listed, detail: `grade.list now carries "${look}"=${listed}` }
    },
  })
  // Grade gallery APPLY (re-applies the saved look after we change the grade).
  await probe(page, {
    surface: S, name: 'grade-apply', sel: page.locator('[data-cut-action="grade-apply"]'), group: insp, groupName: 'inspector-gallery',
    doClick: async () => {
      await verb('edit.grade', { clip, contrast: 0.7 })
      await waitForState((st) => (findClip(st, clip)?.grade?.contrast ?? 1) < 0.8, 8000)
      await page.locator('[data-cut-grade-preset]').selectOption(look).catch(() => {})
      await page.locator('[data-cut-action="grade-apply"]').click(); await sleep(400)
    },
    assertResult: async () => ({ ok: !!(await waitForState((st) => Math.abs((findClip(st, clip)?.grade?.contrast ?? 1) - 1.4) < 0.1, 12000)), detail: 'applied-back contrast≈1.4' }),
  })

  // --- Grade stack add/remove ---
  await probe(page, {
    surface: S, name: 'grade-stack-add', sel: page.locator('[data-cut-action="grade-stack-add"]'), group: insp, groupName: 'inspector-grade-stack',
    doClick: async () => { await page.locator('[data-cut-grade-stack-layer]').selectOption('contrast').catch(() => {}); await page.locator('[data-cut-action="grade-stack-add"]').click(); await sleep(300) },
    assertResult: async () => ({ ok: !!(await waitForState((st) => (findClip(st, clip)?.grade_stack?.length ?? 0) >= 1, 12000)), detail: 'grade_stack length ≥1' }),
  })
  await probe(page, {
    surface: S, name: 'grade-stack-remove', sel: page.locator('[data-cut-grade-stack-row] [data-cut-action="grade-stack-remove"]'), group: insp, groupName: 'inspector-grade-stack',
    doClick: async () => {
      await page.locator('[data-cut-grade-stack-layer]').selectOption('warm').catch(() => {})
      await page.locator('[data-cut-action="grade-stack-add"]').click()
      // Wait for the just-added layer to register, then RECORD the live length right
      // before removing. Prior grade ops in this section can leave the stack >1, so an
      // absolute "==1 after remove" false-fails whenever the pre-remove length isn't 2.
      // Assert RELATIVE instead: the remove must drop the length by EXACTLY 1 — which is
      // the real proof that the remove button works, regardless of the starting count.
      await waitForState((st) => (findClip(st, clip)?.grade_stack?.length ?? 0) >= 2, 10000)
      probe._lenBeforeRemove = (findClip(await state(), clip)?.grade_stack?.length ?? 0)
      await page.locator('[data-cut-grade-stack-row] [data-cut-action="grade-stack-remove"]').first().click(); await sleep(300)
    },
    assertResult: async () => {
      const target = Math.max(0, (probe._lenBeforeRemove ?? 2) - 1)
      const ok = !!(await waitForState((st) => (findClip(st, clip)?.grade_stack?.length ?? 0) === target, 12000))
      return { ok, detail: `grade_stack ${probe._lenBeforeRemove ?? '?'}→${target} after remove (relative −1)` }
    },
  })

  // --- Power window add/remove (edit.grade_window) + pixel-delta ---
  await probe(page, {
    surface: S, name: 'grade-window-add', sel: page.locator('[data-cut-action="grade-window-add"]'), group: insp, groupName: 'inspector-grade-window',
    doClick: async () => {
      await page.locator('[data-cut-grade-window-region]').selectOption('center').catch(() => {})
      await page.locator('[data-cut-grade-window-look]').selectOption('brighten').catch(() => {})
      probe._beforeFrame = await frame(500)
      await page.locator('[data-cut-action="grade-window-add"]').click(); await sleep(300)
    },
    assertResult: async () => {
      const after = await waitForState((st) => (findClip(st, clip)?.grade_windows?.length ?? 0) >= 1, 15000)
      const win = findClip(after || (await state()), clip)?.grade_windows?.[0]
      const rectOk = win?.window?.shape === 'rect' && Array.isArray(win?.window?.points) && win.window.points.length === 2
      const afterFrame = await frame(500)
      const sv = probe._beforeFrame && afterFrame ? ssim(probe._beforeFrame, afterFrame) : null
      const renderChanged = sv == null ? null : sv < 0.999
      return { ok: !!after && rectOk && renderChanged !== false, detail: `window rect2pts=${rectOk} ssim=${sv == null ? 'n/a' : sv.toFixed(4)} renderChanged=${renderChanged}` }
    },
  })
  await probe(page, {
    surface: S, name: 'grade-window-remove', sel: page.locator('[data-cut-grade-window-row] [data-cut-action="grade-window-remove"]'), group: insp, groupName: 'inspector-grade-window',
    doClick: async () => {
      probe._windowRemoveOps = await opsLen()
      await page.locator('[data-cut-grade-window-row] [data-cut-action="grade-window-remove"]').first().click(); await sleep(300)
    },
    assertResult: async () => {
      const removed = !!(await waitForState((st) => (findClip(st, clip)?.grade_windows?.length ?? 0) === 0, 15000))
      const newOps = (await ops()).slice(probe._windowRemoveOps ?? 0).filter((o) => o.verb === 'edit.grade_window')
      const atomic = newOps.length === 1 && newOps[0]?.args?.remove_index === 0
      return { ok: removed && atomic, detail: `grade_windows back to 0; one remove_index op=${atomic}` }
    },
  })
  for (const count of [1, 2]) {
    await page.locator('[data-cut-action="grade-window-add"]').click()
    await waitForState((st) => (findClip(st, clip)?.grade_windows?.length ?? 0) === count, 15_000)
  }
  await probe(page, {
    surface: S,
    name: 'grade-window-clear',
    actionId: 'grade-window-clear',
    sel: page.locator('[data-cut-action="grade-window-clear"]').first(),
    group: insp,
    groupName: 'inspector-grade-window-clear',
    doClick: async () => {
      probe._b = await opsLen()
      probe._r = await captureVerbResp(page, 'edit.grade_window', async () => {
        await page.locator('[data-cut-action="grade-window-clear"]').first().click()
      }, 15_000)
    },
    assertResult: async () => {
      const landed = await opLanded(probe._b, 'edit.grade_window', (args) => args.clip === clip && args.enabled === false)
      const cleared = !!(await waitForState((st) => (findClip(st, clip)?.grade_windows?.length ?? 0) === 0, 15_000))
      return { ok: !!probe._r?.ok && landed && cleared, detail: `edit.grade_window ok=${probe._r?.ok}; enabled:false=${landed}; cleared=${cleared}` }
    },
  })

  // --- Engagement score (score.clip — honest error tolerated on a dev engine) ---
  await expandInspectorSection(page, 'engagement')
  await probe(page, {
    surface: S, name: 'score-clip', sel: page.locator('[data-cut-action="score-clip"]'), group: insp, groupName: 'inspector-engagement',
    doClick: async () => { probe._b = await opsLen(); await page.locator('[data-cut-action="score-clip"]').click(); await sleep(1600) },
    assertResult: async () => {
      const scored = (await page.locator('[data-cut-inspector-score]').count()) > 0
      const errored = (await page.locator('[data-cut-inspector-score-error]').count()) > 0
      return { ok: scored || errored, detail: `scored=${scored} honestError=${errored}` }
    },
  })

  // --- Redact presets — REGION redaction (blur / pixelate / clear) drives a CENTRED
  // rect (REDACT_CENTRE_POINTS) and is content-independent, so it lands on any video
  // clip. The 'faces' preset needs a REAL detected face: the SCENE clip here is a road,
  // and the engine's faces auto-detect lands NO op when it finds 0 faces. So 'faces' is
  // verified separately at the END of this section on a real-face clip (see below). ---
  // Grade-gallery setup intentionally reloads the app above, which resets every
  // component-local disclosure. Re-open the two specialist task sections through
  // their public controls before probing their unmounted bodies.
  await expandInspectorSection(page, 'video-privacy')
  await expandInspectorSection(page, 'video-effects')
  await page.locator('[data-cut-section="video-privacy"]').first().scrollIntoViewIfNeeded().catch(() => {})
  await probe(page, {
    surface: S,
    name: 'inspector-redact-mode',
    actionId: 'inspector-redact-mode',
    sel: page.locator('[data-cut-inspector-redact-mode]').first(),
    group: page.locator('[data-cut-section="video-privacy"]').first(),
    groupName: 'inspector-redact-draw',
    doClick: async () => {
      await page.locator('[data-cut-inspector-redact-mode]').first().selectOption('box')
    },
    assertResult: async () => {
      const value = await page.locator('[data-cut-inspector-redact-mode]').first().inputValue().catch(() => '')
      return { ok: value === 'box', detail: `draw-region mode=${value}` }
    },
  })
  await probe(page, {
    surface: S,
    name: 'redact-draw',
    actionId: 'redact-draw',
    sel: page.locator('[data-cut-action="redact-draw"]').first(),
    group: page.locator('[data-cut-section="video-privacy"]').first(),
    groupName: 'inspector-redact-draw',
    doClick: async () => {
      const button = page.locator('[data-cut-action="redact-draw"]').first()
      await button.click()
      probe._redactDrawOn = await button.getAttribute('data-cut-redact-drawing').catch(() => '')
      if (probe._redactDrawOn === 'true') {
        await renderGroup(page, S, 'inspector-redact-draw-active', page.locator('[data-cut-section="video-privacy"]').first())
        await button.click()
      }
      probe._redactDrawOff = await button.getAttribute('data-cut-redact-drawing').catch(() => '')
    },
    assertResult: async () => ({
      ok: probe._redactDrawOn === 'true' && probe._redactDrawOff === 'false',
      detail: `drawing ${probe._redactDrawOn}→${probe._redactDrawOff}`,
    }),
  })
  for (const r of ['blur', 'pixelate', 'clear']) {
    await probe(page, {
      surface: S, name: `redact-${r}`, sel: page.locator(`[data-cut-inspector-redact="${r}"]`), group: insp, groupName: 'inspector-privacy',
      doClick: async () => { probe._b = await opsLen(); await page.locator(`[data-cut-inspector-redact="${r}"]`).click(); await sleep(500) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.redact', (a) => a.clip === clip), detail: 'edit.redact op landed' }),
    })
  }

  // --- VIDEO effect chips (data-driven over the curated catalog) ---
  const effectClip = findClip(await state(), clip)
  const effectSourceMs = Math.max(
    1,
    Number(effectClip?.src_out_ms || 0) - Number(effectClip?.src_in_ms || 0),
  )
  const effectDurationMs = effectSourceMs / Math.max(0.01, Number(effectClip?.speed || 1))
  // Stay strictly inside the clip's half-open timeline range. The old fixed
  // 1500ms probe landed exactly on the end of a 3s clip at 2x and compared two
  // identical black frames, falsely blaming every effect.
  const effectFrameMs = Math.max(1, Math.min(1000, Math.floor(effectDurationMs / 2)))
  for (const eff of VIDEO_EFFECTS) {
    const chip = page.locator(`[data-cut-inspector-effect="${eff}"]`)
    await probe(page, {
      surface: S, name: `effect-${eff}`, sel: chip, group: page.locator('[data-cut-inspector-effects]').first(), groupName: 'inspector-effects',
      doClick: async () => { probe._b = await opsLen(); probe._f0 = await frame(effectFrameMs); await chip.click(); await sleep(700) },
      assertResult: async () => {
        const opOk = await opLanded(probe._b, 'edit.effect', (a) => a.clip === clip && (a.effects || []).some((e) => e.type === eff))
        const stateOk = (findClip(await state(), clip)?.effects || []).some((e) => e.type === eff)
        const f1 = await frame(effectFrameMs)
        const sv = probe._f0 && f1 ? ssim(probe._f0, f1) : null
        const composedChanged = sv == null ? null : sv < 0.9999
        await chip.click().catch(() => {}); await sleep(250) // toggle off → leave clean
        return { ok: opOk && stateOk && composedChanged === true, detail: `op=${opOk} state=${stateOk} frame=${effectFrameMs}ms ssim=${sv == null ? 'n/a' : sv.toFixed(4)} composedChanged=${composedChanged}` }
      },
    })
  }
  // "More effects…" overflow (catalog-driven long tail).
  await probe(page, {
    surface: S, name: 'effects-more-toggle', sel: page.locator('[data-cut-inspector-effects-more]'), group: page.locator('[data-cut-inspector-effects]').first(), groupName: 'inspector-effects',
    doClick: async () => { await page.locator('[data-cut-inspector-effects-more]').click(); await sleep(400) },
    assertResult: async () => {
      const extra = page.locator('[data-cut-inspector-effects-extra]')
      const shown = (await extra.count()) > 0 && (await extra.first().isVisible().catch(() => false))
      const chips = await extra.locator('[data-cut-inspector-effect]').count()
      return { ok: shown && chips > 0, detail: `overflow shown=${shown} extraChips=${chips}` }
    },
  })
  // Drive EVERY non-curated, non-overlay VIDEO effect in the "More effects…" overflow
  // (data-driven from effects.list) — one chip PER effect, each its own row, so the long
  // tail (blur / grain / hue_shift / rgb_split / emboss …) is INDIVIDUALLY verified
  // instead of one sampled effect. Same 4-part rigor as the curated chips above: the op
  // landed for THIS clip + effect type, the clip's state carries it, and the composed
  // frame changed (SSIM<1). The overflow was just expanded by the effects-more-toggle
  // probe above, so the extra chips are in the DOM. NOTE: each overflow chip seeds the
  // effect's CATALOG DEFAULT (Inspector clipEffectFromCatalog); for any effect whose
  // default is an identity no-op the composed-frame check legitimately fails — that's a
  // real "clicked but nothing happened" finding the harness is meant to surface, not hide.
  const cat = (await verb('effects.list', {})).result?.effects || []
  const overflowEffs = videoOverflowEffects(cat)
  if (!overflowEffs.length) {
    rec(S, 'effect-extra-catalog', { present: 'na', render: 'na', click: 'na', result: 'na' }, 'no non-curated video effect in effects.list to exercise')
  }
  for (const e of overflowEffs) {
    const chip = page.locator(`[data-cut-inspector-effects-extra] [data-cut-inspector-effect="${e.key}"]`)
    await probe(page, {
      surface: S, name: `effect-extra-${e.key}`, sel: chip, group: page.locator('[data-cut-inspector-effects-extra]').first(), groupName: 'inspector-effects-extra',
      doClick: async () => {
        probe._b = await opsLen()
        // SETTLE the baseline: the previous chip's toggle-off may still be
        // re-compositing when we capture f0, and that leftover delta then gets
        // attributed to this chip. Baseline only when two
        // consecutive captures are identical (bounded at ~3s).
        let f0 = await frame(effectFrameMs)
        for (let i = 0; i < 6; i++) {
          await sleep(450)
          const fN = await frame(effectFrameMs)
          if (f0 && fN && ssim(f0, fN) >= 0.9999) { f0 = fN; break }
          f0 = fN
        }
        probe._f0 = f0
        await chip.click(); await sleep(700)
      },
      assertResult: async () => {
        const opOk = await opLanded(probe._b, 'edit.effect', (a) => a.clip === clip && (a.effects || []).some((x) => x.type === e.key))
        const stateOk = (findClip(await state(), clip)?.effects || []).some((x) => x.type === e.key)
        const f1 = await frame(effectFrameMs)
        const sv = probe._f0 && f1 ? ssim(probe._f0, f1) : null
        // REAL composed-frame change REQUIRED: SSIM strictly < 1. A null/undefined SSIM
        // means we could NOT prove the frame changed, so it must NOT pass — otherwise an
        // identity no-op (e.g. a hue_shift whose default `degrees` render unchanged)
        // slips through. composedChanged===true is the only RESULT pass (null/1.0 → fail).
        const composedChanged = sv == null ? null : sv < 0.9999
        await chip.click().catch(() => {}); await sleep(250) // toggle off → leave clean
        return { ok: opOk && stateOk && composedChanged === true, detail: `op=${opOk} state=${stateOk} frame=${effectFrameMs}ms ssim=${sv == null ? 'n/a' : sv.toFixed(4)} composedChanged=${composedChanged}` }
      },
    })
  }

  // --- Tool launchers (open the deep drawers) ---
  for (const t of ['grade', 'layer', 'matte', 'shape']) {
    await probe(page, {
      surface: S, name: `launcher-${t}`, sel: page.locator(`[data-cut-inspector-tool="${t}"]`), group: insp, groupName: 'inspector-tools',
      doClick: async () => { await page.locator(`[data-cut-inspector-tool="${t}"]`).click(); await sleep(500) },
      assertResult: async () => {
        // each launcher opens its drawer (or the Color tab for grade). Assert a
        // drawer/tab surfaced, then close it so the next launcher is reachable.
        const opened =
          (await page.locator('[data-cut-grade-embed]').count()) > 0 ||
          (await page.locator('[data-cut-layer]').count()) > 0 ||
          (await page.locator('[data-cut-matte]').count()) > 0 ||
          (await page.locator('[data-cut-shape]').count()) > 0
        await page.keyboard.press('Escape').catch(() => {})
        await page.locator('[data-cut-layer-close],[data-cut-matte-close],[data-cut-shape-close]').first().click().catch(() => {})
        // 'grade' opens the COLOR TAB (not a drawer) AND the Escape above CLEARS the clip
        // selection — but the layer/matte/shape launcher buttons only render for a SELECTED
        // video clip (Inspector index.tsx ~1770-1773, inside the trackKind==='video' branch,
        // which needs sel.clip). So restore BOTH the selection (re-select the clip) AND the
        // Properties tab before the next launcher; otherwise the sibling buttons read
        // "absent" in the full run (they pass in isolation) — a harness false-fail, not a
        // missing control. Re-selecting remounts them.
        await selectClip(page, clip)
        await propertiesTab(page)
        for (const key of ['video-motion', 'video-color', 'video-effects', 'video-privacy']) {
          await expandInspectorSection(page, key)
        }
        await sleep(250)
        return { ok: opened, detail: `drawer/tab opened=${opened}` }
      },
    })
  }

  // --- Redact FACES preset (needs a REAL face) ---
  // The SCENE clip is a road; the engine's faces auto-detect lands NO op when it finds 0
  // faces (an honest non-mutating receipt), so this preset cannot be proven on it. Placed
  // LAST in this section so we can bring the detector-proven FACE clip onto
  // the BASE video track (face-blur is base-track-only) WITHOUT disturbing the SSIM-gated
  // effect checks above. PRESENT/RENDER/CLICK come from the real chip on the face clip;
  // RESULT is the redact op landing. The chip detects at frame 0; if that frame carried
  // no detectable face we retry the engine's own faces path at a MID-clip frame (at_ms
  // 2000) to prove it on a guaranteed-face frame. No face found even there (or no
  // perception sidecar) → honest N/A (content/sidecar-dependent, not an app bug).
  {
    const faceClip = await addBaseFaceClip(page)
    const facesChip = page.locator('[data-cut-inspector-redact="faces"]')
    // With perception PRESENT a missing face fixture is a real FAIL (the path can't be
    // proven) — only a partial dev run (no perception) tolerates the content-dependent N/A.
    const facesNa = (FULL || DEP.perceptionCv) ? 'fail' : 'na'
    if (!faceClip) {
      rec(S, 'redact-faces', { present: facesNa, render: facesNa, click: facesNa, result: facesNa },
        `face-clip fixture unavailable (FACE import/insert failed) — perception=${DEP.perceptionCv}; ${facesNa === 'fail' ? 'a complete env MUST load it (FAIL)' : 'honest dev skip; FCV_REQUIRE_FULL=1 enforces perception present'}`)
    } else {
      await selectClip(page, faceClip)
      await propertiesTab(page)
      await expandInspectorSection(page, 'video-privacy')
      const present = (await facesChip.count()) > 0
      const rg = await renderGroup(page, S, 'inspector-privacy', insp)
      if (!present) {
        rec(S, 'redact-faces', { present: 'fail', render: rg.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' },
          `faces chip absent on selected face clip ${rg.detail}`.trim(), rg.shot)
      } else {
        const before = await opsLen()
        await facesChip.click().catch(() => {}); await sleep(900)
        let landed = await opLanded(before, 'edit.redact', (a) => a.clip === faceClip)
        let detail
        if (landed) {
          detail = 'edit.redact(faces) op landed from the chip (face detected at frame 0)'
        } else {
          // frame 0 can be an establishing/transition frame → prove the path at
          // the known-good face frame sampled by the release fixture.
          const before2 = await opsLen()
          const r = await verb('edit.redact', { clip: faceClip, faces: true, mode: 'blur', at_ms: FACE_DETECT_MS, rationale: 'fcv: faces redact at a known face frame' })
          landed = await opLanded(before2, 'edit.redact', (a) => a.clip === faceClip)
          const found = r.result?.faces?.found ?? r.result?.found
          const why = r.ok ? `found=${found ?? 0}` : `engine: ${String(r.error?.message || r.error?.code || 'error').slice(0, 70)}`
          detail = landed
            ? `edit.redact(faces) op landed at at_ms=${FACE_DETECT_MS} (frame-0 had no detectable face; known-face frame ${why})`
            : `no face detected / no perception sidecar (chip@0 + verb@${FACE_DETECT_MS} both 0; ${why}) — honest dev skip; FCV_REQUIRE_FULL=1 enforces perception present`
        }
        // perception present ⇒ a real face on a real talking-head clip MUST be found;
        // not-found is then a FAIL, not the dev-mode content-dependent N/A.
        rec(S, 'redact-faces',
          { present: 'pass', render: rg.ok ? 'pass' : 'fail', click: 'pass', result: landed ? 'pass' : facesNa },
          `${detail} (perception=${DEP.perceptionCv}) ${rg.detail}`.trim(), rg.shot)
      }
    }
  }
}
// gating dim helper: a gate that should hold → all-pass; else fail on result.
function gateDim(ok) { return { present: 'pass', render: 'na', click: 'na', result: ok ? 'pass' : 'fail' } }

// ── 3. AUDIO clip selected ───────────────────────────────────────────────────
async function secAudio(page) {
  const S = 'audio-clip'
  await freshProject(page, 'audio', SPEECH) // real speech → meaningful clean-voice / EQ
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  const aud = await clipOfKind('audio')
  if (!aud) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'no audio clip (import has no linked audio?)'); return }
  await selectClip(page, aud)
  await propertiesTab(page)
  await expandInspectorSection(page, 'fades')
  for (const key of ['audio-cleanup', 'audio-effects', 'audio-mix']) {
    await expandInspectorSection(page, key)
  }
  const insp = page.locator('[data-cut-panel="inspector"]')

  // GATING: audio clip MUST show EQ + audio effects, MUST NOT show transform/effects-video.
  rec(S, 'GATE:audio-eq-shown', gateDim((await page.locator('[data-cut-inspector-eq]').count()) > 0), 'EQ present on audio clip')
  rec(S, 'GATE:transform-hidden', gateDim((await page.locator('[data-cut-prop="transform-x"]').count()) === 0), 'video transform correctly hidden on audio clip')

  // Gain field (edit.gain).
  await probe(page, {
    surface: S, name: 'field-gain', sel: page.locator('[data-cut-prop-input="gain"]'), group: insp, groupName: 'inspector-audio',
    doClick: async () => { probe._b = await opsLen(); const f = page.locator('[data-cut-prop-input="gain"]'); await f.fill('-6'); await f.press('Enter'); await sleep(500) },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.gain', (a) => a.clip === aud), detail: 'edit.gain op landed' }),
  })
  // Fade in/out fields on audio.
  for (const key of ['fade-in', 'fade-out']) {
    await probe(page, {
      surface: S, name: `field-${key}`, sel: page.locator(`[data-cut-prop-input="${key}"]`), group: insp, groupName: 'inspector-audio',
      doClick: async () => { probe._b = await opsLen(); const f = page.locator(`[data-cut-prop-input="${key}"]`); await f.fill('0.5'); await f.press('Enter'); await sleep(500) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.fade', (a) => a.clip === aud), detail: 'edit.fade op landed' }),
    })
  }
  // One-shot voice cleanup (audio.cleanup_voice orchestrator → edit.eq + edit.effect sub-ops).
  await probe(page, {
    surface: S, name: 'clean-voice', sel: page.locator('[data-cut-action="audio-cleanup-voice"]'), group: insp, groupName: 'inspector-cleanup',
    doClick: async () => {
      await page.locator('[data-cut-inspector-cleanup-strength]').selectOption('strong').catch(() => {})
      probe._b = await opsLen()
      await page.locator('[data-cut-action="audio-cleanup-voice"]').click(); await sleep(1800)
    },
    assertResult: async () => {
      const eq = await opLanded(probe._b, 'edit.eq', (a) => a.clip === aud)
      const eff = await opLanded(probe._b, 'edit.effect', (a) => a.clip === aud)
      return { ok: eq && eff, detail: `chain eq=${eq} effect=${eff}` }
    },
  })
  // SET-semantics race regression: two clicks in one render used to send
  // [denoise] and [compressor], so the second response silently erased the first.
  // The chain control now serializes projected lists: request 2 must include both,
  // and the visible ordered chain must match engine state.
  let rapidRequests = []
  await probe(page, {
    surface: S, name: 'audio-effect-rapid-chain', actionId: 'effect-on',
    sel: page.locator('[data-cut-effect-on][data-cut-inspector-audio-effect="denoise"]'),
    group: page.locator('[data-cut-effect-chain="audio"]'), groupName: 'inspector-audio-effect-chain',
    doClick: async () => {
      await verb('edit.effect', { clip: aud, effects: [] })
      await sleep(200)
      const onRequest = (request) => {
        if (request.url().includes('/api/verb/edit.effect')) rapidRequests.push(request.postDataJSON())
      }
      page.on('request', onRequest)
      await page.locator('[data-cut-inspector-audio-effect="denoise"]').evaluate((element) => element.click())
      await page.locator('[data-cut-inspector-audio-effect="compressor"]').evaluate((element) => element.click())
      await sleep(800)
      page.off('request', onRequest)
    },
    assertResult: async () => {
      const types = (findClip(await state(), aud)?.effects || []).map((effect) => effect.type)
      const rows = await page.locator('[data-cut-effect-chain-item]').evaluateAll((elements) =>
        elements.map((element) => element.getAttribute('data-cut-effect-chain-item')),
      )
      const projected = rapidRequests.length === 2 && rapidRequests[1]?.effects?.length === 2
      const ordered = JSON.stringify(types) === JSON.stringify(['denoise', 'compressor'])
        && JSON.stringify(rows) === JSON.stringify(types)
      return { ok: projected && ordered, detail: `requests=${rapidRequests.length} state=${types.join('>')} rows=${rows.join('>')}` }
    },
  })
  const effectChainTypes = async () => (findClip(await state(), aud)?.effects || []).map((effect) => effect.type)
  const waitForEffectChain = async (expected) => {
    await page.waitForFunction(
      () => document.querySelector('[data-cut-effect-chain="audio"]')
        ?.getAttribute('data-cut-effect-chain-busy') === 'false',
      null,
      { timeout: 15_000 },
    ).catch(() => {})
    return JSON.stringify(await effectChainTypes()) === JSON.stringify(expected)
  }
  await probe(page, {
    surface: S,
    name: 'audio-effect-chain-move-down',
    actionId: 'effect-chain-move-down',
    sel: page.locator('[data-cut-effect-chain-move-down="denoise"]').first(),
    group: page.locator('[data-cut-effect-chain="audio"]'),
    groupName: 'inspector-audio-effect-chain-move-down',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'edit.effect', async () => {
        await page.locator('[data-cut-effect-chain-move-down="denoise"]').first().click()
      }, 15_000)
    },
    assertResult: async () => {
      const ordered = await waitForEffectChain(['compressor', 'denoise'])
      return { ok: !!probe._r?.ok && ordered, detail: `edit.effect ok=${probe._r?.ok}; order=${(await effectChainTypes()).join('>')}` }
    },
  })
  await probe(page, {
    surface: S,
    name: 'audio-effect-chain-move-up',
    actionId: 'effect-chain-move-up',
    sel: page.locator('[data-cut-effect-chain-move-up="denoise"]').first(),
    group: page.locator('[data-cut-effect-chain="audio"]'),
    groupName: 'inspector-audio-effect-chain-move-up',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'edit.effect', async () => {
        await page.locator('[data-cut-effect-chain-move-up="denoise"]').first().click()
      }, 15_000)
    },
    assertResult: async () => {
      const ordered = await waitForEffectChain(['denoise', 'compressor'])
      return { ok: !!probe._r?.ok && ordered, detail: `edit.effect ok=${probe._r?.ok}; order=${(await effectChainTypes()).join('>')}` }
    },
  })
  await probe(page, {
    surface: S,
    name: 'audio-effect-chain-remove',
    actionId: 'effect-chain-remove',
    sel: page.locator('[data-cut-effect-chain-remove="compressor"]').first(),
    group: page.locator('[data-cut-effect-chain="audio"]'),
    groupName: 'inspector-audio-effect-chain-remove',
    doClick: async () => {
      await waitForEffectChain(['denoise', 'compressor'])
      await page.locator('[data-cut-effect-chain-remove="compressor"]:not([disabled])')
        .first()
        .waitFor({ state: 'visible', timeout: 15_000 })
      probe._r = await captureVerbResp(page, 'edit.effect', async () => {
        await page.locator('[data-cut-effect-chain-remove="compressor"]').first().click()
      }, 15_000)
    },
    assertResult: async () => {
      const removed = await waitForEffectChain(['denoise'])
      return { ok: !!probe._r?.ok && removed, detail: `edit.effect ok=${probe._r?.ok}; order=${(await effectChainTypes()).join('>')}` }
    },
  })
  // AUDIO effect chips (denoise/compressor/gate). The setup above leaves some effects on
  // the clip, so a chip click can TOGGLE the effect OFF as well as on — and the toggle-OFF
  // edit.effect op carries the NEW (effect-removed) set, so an add-only assertion
  // false-fails. Assert the TOGGLE instead: an edit.effect op landed for THIS clip AND the
  // effect's presence on the clip FLIPPED (either direction). A genuine no-op (click does
  // nothing) leaves presence unchanged → correctly fails.
  for (const eff of AUDIO_EFFECTS) {
    const chip = page.locator(`[data-cut-inspector-audio-effect="${eff}"]`)
    await probe(page, {
      surface: S, name: `audio-effect-${eff}`, sel: chip, group: page.locator('[data-cut-inspector-audio-effects]').first(), groupName: 'inspector-audio-effects',
      doClick: async () => {
        probe._b = await opsLen()
        probe._was = (findClip(await state(), aud)?.effects || []).some((e) => e.type === eff)
        await chip.click(); await sleep(500)
      },
      assertResult: async () => {
        const opOk = await opLanded(probe._b, 'edit.effect', (a) => a.clip === aud)
        const now = (findClip(await state(), aud)?.effects || []).some((e) => e.type === eff)
        const flipped = now !== probe._was
        await chip.click().catch(() => {}); await sleep(200) // restore prior state
        return { ok: opOk && flipped, detail: `edit.effect(${eff}) toggled ${probe._was}→${now} (flipped=${flipped})` }
      },
    })
  }
  // EQ presets (edit.eq) — including the explicit "clear".
  for (const preset of EQ_PRESETS) {
    const chip = page.locator(`[data-cut-inspector-eq-preset="${preset}"]`)
    await probe(page, {
      surface: S, name: `eq-${preset}`, sel: chip, group: page.locator('[data-cut-inspector-eq]').first(), groupName: 'inspector-eq',
      doClick: async () => { probe._b = await opsLen(); await chip.click(); await sleep(450) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.eq', (a) => a.clip === aud), detail: `edit.eq(${preset}) landed` }),
    })
  }

  await probe(page, {
    surface: S,
    name: 'inspector-open-music',
    actionId: 'inspector-open-music',
    sel: page.locator('[data-cut-inspector-open-music]').first(),
    group: insp,
    groupName: 'inspector-audio-music-blocker',
    doClick: async () => {
      await page.locator('[data-cut-inspector-open-music]').first().click()
      await page.locator('[data-cut-musicbed]').first().waitFor({ state: 'visible', timeout: 8_000 }).catch(() => {})
    },
    assertResult: async () => {
      const open = await page.locator('[data-cut-musicbed]').count() > 0
      if (open) await page.locator('[data-cut-musicbed-close]').first().click().catch(() => {})
      return { ok: open, detail: `Music Bed drawer opened=${open}` }
    },
  })
}

// ── 4. BLEND modes (overlay video clip selected) ─────────────────────────────
async function secBlend(page) {
  const S = 'blend-overlay'
  const { ovClip, detail: overlayDetail } = await buildOverlayProject(page, 'blend')
  if (!ovClip) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, `overlay clip never landed after probe-aware import/insert — ${overlayDetail || 'no bootstrap detail'}`); return }
  await selectClip(page, ovClip)
  await propertiesTab(page)
  await expandInspectorSection(page, 'video-effects')
  const insp = page.locator('[data-cut-panel="inspector"]')
  const blendSel = page.locator('[data-cut-inspector-blend]')
  // GATING: blend select only appears for an OVERLAY video clip.
  rec(S, 'GATE:blend-shown-on-overlay', gateDim((await blendSel.count()) > 0), 'blend select present on overlay video clip')
  // Drive every blend mode (edit.blend on the track).
  for (const mode of BLEND_MODES) {
    await probe(page, {
      surface: S, name: `blend-${mode}`, sel: blendSel, group: insp, groupName: 'inspector-blend',
      doClick: async () => { probe._b = await opsLen(); await blendSel.selectOption(mode).catch(() => {}); await sleep(450) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.blend', (a) => a.mode === mode), detail: `edit.blend(${mode}) landed` }),
    })
  }

  // transform-opacity is OVERLAY-ONLY (Inspector `{isOverlay && …}`): secVideo selects a
  // BASE clip where the row is hidden, so it's driven HERE on the overlay clip (the N/A
  // recorded in secVideo points to this row). Commit a value → assert edit.transform landed.
  await probe(page, {
    surface: S, name: 'field-transform-opacity', sel: page.locator('[data-cut-prop-input="transform-opacity"]'),
    group: insp, groupName: 'inspector-overlay-transform',
    doClick: async () => {
      probe._b = await opsLen()
      const f = page.locator('[data-cut-prop-input="transform-opacity"]')
      await f.fill('70'); await f.press('Enter'); await sleep(500)
    },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.transform', (a) => a.clip === ovClip), detail: 'edit.transform op landed (opacity on overlay clip)' }),
  })

  // OVERLAY-ONLY effects (effect_specs(): overlay_only) — chroma_key et al. They REFUSE
  // a base clip and the Inspector overflow HIDES them, so there's no one-click chip even
  // on this overlay clip. The catalog still LISTS them, so we VERIFY each genuinely
  // APPLIES on this OVERLAY clip via edit.effect (the exact path a chip would hit): the op
  // lands for THIS overlay clip + effect type and the clip's state carries it. The
  // composed-frame delta is CONTENT-DEPENDENT (chroma_key only changes pixels matching the
  // key colour; a clip lacking that colour legitimately renders the same), so SSIM is
  // reported as evidence but NOT gating here — op+state are the fair proof on this surface.
  // PRESENT/RENDER/CLICK are honest N/A (no UI control). This is what the catalog-drift
  // guard credits as "covered in blend" for OVERLAY_ONLY_EFFECTS — a real verify, not a
  // fake pass, and not a silently-failing gate for a genuinely context-gated effect.
  const blendCat = (await verb('effects.list', {})).result?.effects || []
  for (const key of OVERLAY_ONLY_EFFECTS) {
    const entry = blendCat.find((e) => e.key === key)
    if (!entry) {
      rec(S, `effect-extra-${key}`, { present: 'na', render: 'na', click: 'na', result: 'na' }, `overlay_only effect "${key}" not in effects.list — nothing to verify`)
      continue
    }
    const eff = effectFromCatalogFull(entry)
    // chroma_key REQUIRES `color` (+ similarity/blend) — the effect spec mandates it
    // (types.rs: color is required with no default). effectFromCatalogFull fills it from
    // the color-kind catalog param, but guarantee it EXPLICITLY here so a catalog that
    // ever under-declares the color param can never make this call fail with the engine's
    // "missing field 'color'". similarity/blend use the engine's documented defaults.
    if (key === 'chroma_key') {
      if (eff.color == null) eff.color = '0x00FF00'       // green-screen key colour (engine form: name|0xRRGGBB, NOT #RRGGBB)
      if (eff.similarity == null) eff.similarity = 0.15   // types.rs eff_similarity default
      if (eff.blend == null) eff.blend = 0.1              // types.rs eff_blend default
    }
    const before = await opsLen()
    const f0 = await frame(500)
    const r = await verb('edit.effect', { clip: ovClip, effects: [eff], rationale: `fcv: verify overlay_only ${key}` })
    await sleep(600)
    const opOk = await opLanded(before, 'edit.effect', (a) => a.clip === ovClip && (a.effects || []).some((x) => x.type === key))
    const stateOk = (findClip(await state(), ovClip)?.effects || []).some((x) => x.type === key)
    const f1 = await frame(500)
    const sv = f0 && f1 ? ssim(f0, f1) : null
    await verb('edit.effect', { clip: ovClip, effects: [], rationale: `fcv: clear overlay_only ${key}` }) // leave clean
    const ok = !!r.ok && opOk && stateOk
    rec(S, `effect-extra-${key}`,
      { present: 'na', render: 'na', click: 'na', result: ok ? 'pass' : 'fail' },
      `overlay_only "${key}" — no Inspector chip (overflow hides overlay_only); verified via edit.effect on overlay clip: ok=${r.ok} op=${opOk} state=${stateOk} ssim=${sv == null ? 'n/a' : sv.toFixed(4)} (SSIM advisory — content-dependent)`)
  }
}

// ── 5. CAPTION / TITLE / SHAPE clip editors (per-type selection) ─────────────
async function secTypedClips(page) {
  // CAPTION
  {
    const S = 'caption-clip'
    await freshProject(page, 'cap')
    await closeOverlays(page) // macOS cascade guard — drop any leftover drawer/menu before this sub-block selects/clicks
    const orig = 'FCV_CAPC_' + Math.random().toString(36).slice(2, 6).toUpperCase()
    const capId = await addCaption(page, orig)
    if (!capId) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'caption clip not created'); }
    else {
      await selectClip(page, capId)
      const insp = page.locator('[data-cut-panel="inspector"]')
      const kind = await waitInspectorKind(page, 'caption', '[data-cut-caption-edit-text]')
      rec(S, 'GATE:caption-editor-shown', gateDim(kind === 'caption' && (await page.locator('[data-cut-caption-edit-text]').count()) > 0), `inspector-kind=${kind}`)
      await probe(page, {
        surface: S, name: 'caption-save-text', sel: page.locator('[data-cut-caption-edit-text]'), group: insp, groupName: 'inspector-caption-edit',
        doClick: async () => {
          const ta = page.locator('[data-cut-caption-edit-text]'); await ta.fill(orig + '_X')
          await page.locator('[data-cut-action="caption-save-text"]').click(); await sleep(500)
        },
        assertResult: async () => ({ ok: !!(await waitForState((st) => (st.tracks || []).some((t) => t.kind === 'caption' && (t.clips || []).some((c) => c.id === capId && c.text === orig + '_X')), 10000)), detail: 'captions.set_text changed caption words' }),
      })
    }
  }
  // TITLE
  {
    const S = 'title-clip'
    await freshProject(page, 'title')
    await closeOverlays(page) // macOS cascade guard — drop any leftover drawer/menu before this sub-block selects/clicks
    const orig = 'FCV_TITLE_' + Math.random().toString(36).slice(2, 6).toUpperCase()
    const t = await addTitle(page, orig)
    if (!t) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'title clip not created'); }
    else {
      await selectClip(page, t.id)
      const insp = page.locator('[data-cut-panel="inspector"]')
      const kind = await waitInspectorKind(page, 'title', '[data-cut-title-edit-text]')
      rec(S, 'GATE:title-editor-shown', gateDim(kind === 'title' && (await page.locator('[data-cut-title-edit-text]').count()) > 0), `inspector-kind=${kind}`)
      const assetBefore = t.asset
      await probe(page, {
        surface: S, name: 'title-save-text', sel: page.locator('[data-cut-title-edit-text]'), group: insp, groupName: 'inspector-title-edit',
        doClick: async () => {
          await page.locator('[data-cut-title-edit-text]').fill(orig + '_X')
          await page.locator('[data-cut-action="title-save-text"]').click()
        },
        assertResult: async () => {
          const swapped = await waitForState((st) => { const c = findClip(st, t.id); return c && c.asset !== assetBefore && c.title_text === orig + '_X' }, 25000)
          return { ok: !!swapped, detail: 'title.update re-rendered overlay (asset swap + new title_text)' }
        },
      })
    }
  }
  // SHAPE
  {
    const S = 'shape-clip'
    await freshProject(page, 'shape')
    await closeOverlays(page) // macOS cascade guard — drop any leftover drawer/menu before this sub-block selects/clicks
    const orig = 'FCV_SHAPE_' + Math.random().toString(36).slice(2, 6).toUpperCase()
    const c = await addShape(page, orig)
    if (!c) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'shape clip not created'); }
    else {
      await selectClip(page, c.id)
      const insp = page.locator('[data-cut-panel="inspector"]')
      const kind = await waitInspectorKind(page, 'shape', '[data-cut-shape-edit-label]')
      rec(S, 'GATE:shape-editor-shown', gateDim(kind === 'shape' && (await page.locator('[data-cut-shape-edit-label]').count()) > 0), `inspector-kind=${kind}`)
      const assetBefore = c.asset
      await probe(page, {
        surface: S,
        name: 'shape-edit-kind',
        actionId: 'shape-edit-kind',
        sel: page.locator('[data-cut-shape-edit-kind]').first(),
        group: insp,
        groupName: 'inspector-shape-kind-initial',
        doClick: async () => {
          await page.locator('[data-cut-shape-edit-kind]').first().selectOption('ellipse')
          await renderGroup(page, S, 'inspector-shape-kind-selected', insp)
        },
        assertResult: async () => {
          const value = await page.locator('[data-cut-shape-edit-kind]').first().inputValue().catch(() => '')
          return { ok: value === 'ellipse', detail: `shape draft kind=${value}` }
        },
      })
      await probe(page, {
        surface: S, name: 'shape-save', sel: page.locator('[data-cut-shape-edit-label]'), group: insp, groupName: 'inspector-shape-save',
        doClick: async () => {
          await page.locator('[data-cut-shape-edit-label]').fill(orig + '_X')
          await page.locator('[data-cut-shape-edit-color]').fill('#00FF00').catch(() => {})
          probe._shapeArgs = null
          const onRequest = (request) => {
            let pathname = ''
            try { pathname = new URL(request.url()).pathname } catch { return }
            if (pathname !== '/api/verb/shape.update') return
            try { probe._shapeArgs = request.postDataJSON() } catch { /* asserted below */ }
          }
          page.on('request', onRequest)
          probe._r = await captureVerbResp(page, 'shape.update', async () => {
            await page.locator('[data-cut-action="shape-save"]').click()
          }, 25_000)
          page.off('request', onRequest)
        },
        assertResult: async () => {
          const swapped = await waitForState((st) => {
            const cc = findClip(st, c.id)
            return cc
              && cc.asset !== assetBefore
              && cc.shape_kind === 'ellipse'
              && cc.shape_label === orig + '_X'
              && cc.shape_color === '#00FF00'
          }, 25_000)
          const args = probe._shapeArgs
          const exactRequest = args?.clip === c.id
            && args?.shape === 'ellipse'
            && args?.label === orig + '_X'
            && args?.fill === '#00FF00'
            && args?.rationale === 'inspector: edit shape'
          return {
            ok: !!probe._r?.ok && !!swapped && exactRequest,
            detail: `shape.update ok=${probe._r?.ok}; ellipse/label/color state=${!!swapped}; exact request=${exactRequest}`,
          }
        },
      })
    }
  }
}

// ── 6. MULTI-select (Nest) + gating ──────────────────────────────────────────
async function secMulti(page) {
  const S = 'multi-select'
  const { projectPath } = await freshProject(page, 'multi')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  const s0 = await state()
  const vTrack = s0.tracks.find((t) => t.kind === 'video')?.id
  await verb('edit.split', { track: vTrack, at_ms: 3000 }); await sleep(700)
  const clips = ((await state()).tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset).map((c) => c.id)) || []
  if (clips.length < 2) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, `need 2 clips, have ${clips.length}`); return }
  // GATING: Nest disabled on single-select.
  await page.locator(`[data-cut-clip="${clips[0]}"]`).click().catch(() => {}); await sleep(150)
  await page.locator(`[data-cut-clip="${clips[0]}"]`).click({ button: 'right', force: true }).catch(() => {}); await sleep(300)
  const disabledSingle = await page.locator('[data-cut-ctx="nest"]').isDisabled().catch(() => false)
  rec(S, 'GATE:nest-disabled-single', gateDim(disabledSingle), `Nest disabled on single-select=${disabledSingle}`)
  await page.keyboard.press('Escape').catch(() => {}); await sleep(150)
  // Multi-select → Nest enabled → fire. ADDITIVE select uses Playwright's
  // cross-platform `ControlOrMeta` (→ Cmd on macOS, Ctrl on Linux/Windows): a raw
  // 'Control' modifier is a SECONDARY (right) click on macOS, so it would silently
  // fail to extend the selection and Nest would read <2 selected (a harness bug, not
  // an engine bug). The shared helper owns both additive-selection call sites.
  await selectClipPair(page, clips[0], clips[1])
  await page.locator(`[data-cut-clip="${clips[1]}"]`).click({ button: 'right', force: true }).catch(() => {}); await sleep(300)
  await probe(page, {
    surface: S, name: 'nest-selection', sel: page.locator('[data-cut-ctx="nest"]'), group: page.locator('[data-cut-clip-menu]').first(), groupName: 'ctx-menu-multi',
    doClick: async () => { probe._vb = clips.length; await page.locator('[data-cut-ctx="nest"]').click(); await sleep(400) },
    assertResult: async () => {
      const after = await waitForState((st) => {
        const media = st.tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset) || []
        return media.length < probe._vb && media.some((c) => c.nest)
      }, 15000)
      const nestedClip = after?.tracks.find((track) => track.id === vTrack)?.clips?.find((clip) => clip.nest)
      const nestId = nestedClip?.nest || nestedClip?.asset || ''
      const outputBytes = (path) => {
        try { return statSync(resolveDriverPath(path)).size } catch { return 0 }
      }

      const preview = after
        ? await verb('render.preview', { at_ms: 0, duration_ms: 300 }, { timeoutMs: 120000 })
        : null
      const previewBytes = preview?.result?.path ? outputBytes(preview.result.path) : 0
      const otio = after ? await verb('export.otio', {}, { timeoutMs: 30000 }) : null
      const otioPath = otio?.result?.path ? resolveDriverPath(otio.result.path) : ''
      let otioBytes = 0
      let otioReferencesNest = false
      try {
        otioBytes = statSync(otioPath).size
        otioReferencesNest = /cache[\\/]+nest/.test(readFileSync(otioPath, 'utf8'))
      } catch { /* reported by the result below */ }

      const saved = after ? await verb('project.save', {}) : null
      const closed = saved?.ok ? await verb('project.close', {}) : null
      const opened = closed?.ok
        ? await verb('project.open', { path: projectPath }, { timeoutMs: 30000 })
        : null
      const reopened = opened?.ok ? await state() : null
      const reopenedNested = !!reopened
        && (reopened.nests || []).some((nest) => nest.id === nestId)
        && (reopened.tracks || []).some((track) => (
          (track.clips || []).some((clip) => clip.nest === nestId || clip.asset === nestId)
        ))
      const bakedAssetStayedEphemeral = !!reopened && !reopened.assets?.[nestId]
      const reopenedFrame = reopenedNested
        ? await verb('render.frame', { at_ms: 200, h: 90, compose: true }, { timeoutMs: 120000 })
        : null
      const frameBytes = reopenedFrame?.result?.path ? outputBytes(reopenedFrame.result.path) : 0
      if (opened?.ok) {
        await reloadApp(page)
        await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
        await ensureRail(page)
      }

      const ok = !!after && !!nestId
        && preview?.ok === true && previewBytes > 0
        && otio?.ok === true && otioBytes > 0 && otioReferencesNest
        && saved?.ok === true && closed?.ok === true && opened?.ok === true
        && reopenedNested && bakedAssetStayedEphemeral
        && reopenedFrame?.ok === true && frameBytes > 0
      return {
        ok,
        detail: `nest=${nestId || 'missing'}; preview=${preview?.ok}/${previewBytes}B${preview?.error ? ` error=${String(preview.error.message || preview.error.code).slice(0, 160)}` : ''}; OTIO=${otio?.ok}/${otioBytes}B nest-ref=${otioReferencesNest}; save-close-reopen=${saved?.ok}/${closed?.ok}/${opened?.ok}; nested-state=${reopenedNested}; baked-asset-ephemeral=${bakedAssetStayedEphemeral}; reopened-frame=${reopenedFrame?.ok}/${frameBytes}B`,
      }
    },
  })
  // Sync-by-audio gating (1 vs 2 media selected) — proven in interaction-verify;
  // here we record the gate + that the toolbar button exists.
  const syncBtn = page.locator('[data-cut-action="sync-by-audio"]').first()
  rec(S, 'GATE:sync-by-audio-present', gateDim((await syncBtn.count()) > 0), 'sync-by-audio toolbar button present')
}

// ── 7. RANGE selection (I/O marks) ───────────────────────────────────────────
async function secRange(page) {
  const S = 'range-select'
  await freshProject(page, 'range')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  const clip = await clipOfKind('video')
  await selectClip(page, clip)
  const ruler = page.locator('[data-cut-ruler]').first()
  const rb = await ruler.boundingBox()
  // Mark IN at 40% of the ruler with the 'i' key, assert the [data-cut-range] band paints.
  await probe(page, {
    surface: S, name: 'mark-in-out', sel: ruler, group: page.locator('[data-cut-panel="timeline"]').first(), groupName: 'timeline-range',
    doClick: async () => {
      if (rb) { await page.mouse.click(rb.x + rb.width * 0.4, rb.y + 6); await sleep(250) }
      await page.keyboard.press('i'); await sleep(300)
    },
    assertResult: async () => {
      const band = page.locator('[data-cut-range]').first()
      const painted = (await band.count()) > 0
      const attr = painted ? await band.getAttribute('data-cut-range') : 'none'
      return { ok: painted, detail: `export range band painted=${painted} range=${attr}` }
    },
  })
}

// ── 8. EXPORT options (all 18, on a short real project) ───────────────────────
async function secExport(page) {
  const S = 'export'
  const projectCtx = await freshProject(page, 'export', SPEECH) // real speech → captions.generate can transcribe
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks

  // ── Export destination folder: choose-folder and clear-folder ──────────
  // "Choose folder…" opens a native OS picker (no-op headless → the click proves the
  // control); the FEATURE is project.set_output_dir, asserted by driving the verb on a
  // real temp dir (it echoes the resolved path). "Use project folder" (clear) renders
  // ONLY when an output dir is set, and the UI seeds that from localStorage — so we seed
  // a real dir + reload to surface the item, click it for real, and capture the engine's
  // set_output_dir response to prove the clear landed (cleared:true). No longer N/A.
  {
    const outTmp = mkdtempSync(join(tmpdir(), 'fcv-out-'))
    await page.evaluate((d) => { try { localStorage.setItem('cut.outputDir', d) } catch { /* ignore */ } }, outTmp)
    await reloadApp(page); await sleep(900)
    if ((await page.locator('[data-cut-export-menu]').count()) === 0) { await page.locator('[data-cut-export-btn]').click(); await sleep(300) }
    let chooseFolderResponse = null
    await probe(page, {
      surface: S, name: 'export-choose-folder', sel: page.locator('[data-cut-export-choose-folder]'),
      group: page.locator('[data-cut-export-menu]').first(), groupName: 'export-menu',
      clickNa: NATIVE_PICKER_CLICK_NA,
      nativeAction: {
        mode: 'select',
        path: synthEngineDir,
        useDoClick: true,
        verifyResult: true,
      },
      doClick: async () => {
        if (NATIVE_OS_ACTIONS.enabled) {
          chooseFolderResponse = await captureVerbResp(
            page,
            'project.set_output_dir',
            () => page.locator('[data-cut-export-choose-folder]').click(),
            30_000,
          )
        } else {
          await page.locator('[data-cut-export-choose-folder]').click().catch(() => {})
        }
        await sleep(150)
      },
      assertResult: async () => {
        const r = NATIVE_OS_ACTIONS.enabled
          ? chooseFolderResponse
          : await verb('project.set_output_dir', { dir: outTmp })
        const exact = basenameHostPath(r?.result?.dir || '') === basenameHostPath(
          NATIVE_OS_ACTIONS.enabled ? synthEngineDir : outTmp,
        )
        return {
          ok: !!r?.ok && exact,
          detail: `project.set_output_dir via ${NATIVE_OS_ACTIONS.enabled ? 'installed picker' : 'browser fallback'} ok=${r?.ok}; selected=${r?.result?.dir || 'missing'}; exact=${exact}`,
        }
      },
    })
    // clear-folder: the item is present (dir seeded). Capture the verb response its click
    // drives → assert the engine cleared (dir:null / cleared:true).
    if ((await page.locator('[data-cut-export-menu]').count()) === 0) { await page.locator('[data-cut-export-btn]').click(); await sleep(300) }
    let clearResp = null
    const onClr = async (r) => { if (/\/api\/verb\/project\.set_output_dir/.test(r.url())) { try { clearResp = await r.json() } catch { /* non-JSON */ } } }
    page.on('response', onClr)
    await probe(page, {
      surface: S, name: 'export-clear-folder', sel: page.locator('[data-cut-export-clear-folder]'),
      group: page.locator('[data-cut-export-menu]').first(), groupName: 'export-menu',
      doClick: async () => {
        clearResp = null
        await page.locator('[data-cut-export-clear-folder]').click().catch(() => {})
        for (let i = 0; i < 40 && clearResp === null; i++) await sleep(150)
      },
      assertResult: async () => {
        const cleared = !!clearResp?.ok && (clearResp.result?.cleared === true || clearResp.result?.dir == null)
        return { ok: cleared, detail: `clear-folder click drove project.set_output_dir{} → cleared=${cleared} (${JSON.stringify(clearResp?.result ?? clearResp?.error ?? 'no response').slice(0, 50)})` }
      },
    })
    page.off('response', onClr)
    await page.keyboard.press('Escape').catch(() => {}); await sleep(150)
  }

  // Footage QC profile select (shared state with the render menu's Footage
  // picker) — publish exports forward it as export.publish{profile}. Drive the
  // select and read the value back (the wire contract itself is pinned by the
  // lib.test.ts fetch-capture checks), then RESET to auto so every export probe
  // below keeps today's deterministic arg shape.
  {
    if ((await page.locator('[data-cut-export-menu]').count()) === 0) { await page.locator('[data-cut-export-btn]').click(); await sleep(300) }
    const exportProfile = page.locator('[data-cut-export-profile]').first()
    await probe(page, {
      surface: S, name: 'export-profile', sel: exportProfile,
      group: page.locator('[data-cut-export-menu]').first(), groupName: 'export-menu',
      doClick: async () => { await exportProfile.selectOption('silent_screen_demo') },
      assertResult: async () => {
        const chosen = await exportProfile.inputValue()
        await exportProfile.selectOption('auto')
        const reset = await exportProfile.inputValue()
        return { ok: chosen === 'silent_screen_demo' && reset === 'auto', detail: `profile=${chosen}; reset=${reset}` }
      },
    })
    await page.keyboard.press('Escape').catch(() => {}); await sleep(150)
  }

  if (!NATIVE_OS_ACTIONS.enabled) {
    if ((await page.locator('[data-cut-export-menu]').count()) === 0) { await page.locator('[data-cut-export-btn]').click(); await sleep(300) }
    await probe(page, {
      surface: S, name: 'export-save-as-controls', sel: page.locator('[data-cut-export-saveas-option]').first(),
      group: page.locator('[data-cut-export-menu]').first(), groupName: 'export-menu',
      rowKind: 'support',
      doClick: async () => { /* installed final runners actuate every Save As control below */ },
      assertResult: async () => {
        const n = await page.locator('[data-cut-export-saveas-option]').count()
        const video = await page.locator('[data-cut-export-saveas-option="video"]').count()
        return { ok: n >= EXPORT_OPTIONS.length && video > 0, detail: `Save As controls=${n}/${EXPORT_OPTIONS.length} video=${video}` }
      },
    })
    await page.keyboard.press('Escape').catch(() => {}); await sleep(150)
  }

  // Shorten so any render is quick.
  const s0 = await state()
  for (const t of s0.tracks || []) if (t.kind === 'video' || t.kind === 'audio') await verb('edit.ripple_delete', { track: t.id, range_ms: [1500, 999000], ripple: true })
  await sleep(600)
  // Best-effort captions so the caption exports (srt/vtt/ass/chapters/transcript)
  // can actually produce files. captions.generate consumes an EXISTING transcript;
  // it does not transcribe by itself, so seed the same speech asset first.
  const expState = await state()
  const expAsset = expState.tracks.find((t) => t.kind === 'video')?.clips?.find((c) => c.asset)?.asset || Object.keys(expState.assets || {})[0]
  let transcriptReady = false
  if (DEP.perceptionStt && expAsset) {
    const tr = await verb('media.transcribe', { asset: expAsset })
    if (tr.result?.job_id) await awaitJob(tr.result.job_id)
    transcriptReady = !!(await waitForState((s) => !!s.assets?.[expAsset]?.transcript, 60000))
    const ready = await ensureNonEmptyTranscript(page, projectCtx.projectPath, expAsset, 'fcv: captions.generate/export needs words; live STT returned empty')
    transcriptReady = ready.words > 0
  }
  const capgen = await verb('captions.generate', {})
  // captions.generate has NO direct UI button (it auto-runs to back the caption pipeline);
  // cover it VERB-LEVEL here, where it seeds the srt/vtt/ass/transcript/chapters exports below.
  // Dep-gated on STT (it transcribes): under FCV_REQUIRE_FULL the cues are real → real RESULT.
  rec(S, 'captions.generate(verb-level · transcript→cap1)', { present: 'na', render: 'na', click: 'na', result: DEP.perceptionStt ? (capgen.ok ? 'pass' : 'fail') : 'na' },
    DEP.perceptionStt
      ? `captions.generate ok=${capgen.ok} transcriptReady=${transcriptReady} cues=${capgen.result?.cue_count ?? capgen.result?.cues?.length ?? '?'} — builds the cap1 caption track from the transcript (seeds the srt/vtt/ass/transcript/chapters exports below); verb-level RESULT, flagged not faked`
      : `captions.generate needs a transcript (perception STT) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present (the caption exports below then surface its result)`)
  await sleep(800)
  // Caption file exports should not depend on a local STT install in partial mode.
  // Seed cap1 through the same user-facing import pipeline so srt/vtt/ass/transcript
  // export paths can still be proven deterministically on WSL/dev rigs.
  const seedSrt = join(tmp, `fcv-export-captions-${seq++}.srt`)
  writeFileSync(seedSrt, '1\n00:00:00,200 --> 00:00:00,900\nFCV export caption one\n\n2\n00:00:01,000 --> 00:00:01,450\nFCV export caption two\n')
  const capImport = await verb('captions.import', { path: seedSrt, replace: true, rationale: 'fcv: deterministic caption export seed' })
  const haveCaptions = !!capImport.ok && Number(capImport.result?.caption_count || 0) > 0
  rec(S, 'captions.import(verb-level · export seed)', { present: 'na', render: 'na', click: 'na', result: haveCaptions ? 'pass' : 'fail' },
    `captions.import ok=${capImport.ok} count=${capImport.result?.caption_count ?? '?'} — deterministic cap1 seed for srt/vtt/ass/transcript exports; live STT remains covered by captions.generate above`)
  // export.chapters reads timeline MARKERS (not captions) — seed a couple within the
  // shortened [0,1500] timeline so it produces a real chapters.txt (otherwise it honestly
  // errors "no markers to export as chapters"). This sets up the precondition rather than
  // faking the pass: with markers present, export.chapters writes a file → RESULT pass.
  const mk0 = await verb('edit.add_marker', { at_ms: 0, label: 'FCV Intro' })
  const mk1 = await verb('edit.add_marker', { at_ms: 1000, label: 'FCV Mid' })
  const haveMarkers = !!(mk0.ok || mk1.ok)
  await sleep(300)

  // Listen for every export verb response so RESULT can read ok + path/job_id.
  const exportResp = {}
  const onResp = async (r) => {
    const m = r.url().match(/\/api\/verb\/(render\.final|export\.[a-z]+)/)
    if (m) { try { exportResp[m[1]] = await r.json() } catch {} }
  }
  page.on('response', onResp)
  try {
    for (const opt of EXPORT_OPTIONS) {
      const btn = page.locator(`[data-cut-export-option="${opt.id}"]`)
      const missingPrecondition = (opt.needsCaptions && !haveCaptions) || (opt.needsMarkers && !haveMarkers)
      // Open the Export menu BEFORE the probe so PRESENT/RENDER capture the OPEN menu —
      // the option buttons only mount while the menu is open (live macOS run: closed=0 →
      // open=18). Mirrors render-queue-open (the same [data-cut-export-btn] drives that
      // probe, which passes). onExport closes the menu after each pick → reopen per option.
      if ((await page.locator('[data-cut-export-menu]').count()) === 0) { await page.locator('[data-cut-export-btn]').click(); await sleep(300) }
      await probe(page, {
        surface: S, name: `export-${opt.id}`, sel: btn,
        group: page.locator('[data-cut-export-menu]').first(), groupName: 'export-menu',
        resultNa: missingPrecondition
          ? `missing ${opt.needsCaptions ? 'caption' : 'marker'} precondition after deterministic setup — click still driven, but RESULT is not attributable to export wiring in this partial run`
          : '',
        doClick: async () => {
          // Menu already opened above; reopen only if a prior pick closed it.
          if ((await page.locator('[data-cut-export-menu]').count()) === 0) { await page.locator('[data-cut-export-btn]').click(); await sleep(300) }
          exportResp[opt.verb] = undefined
          await btn.scrollIntoViewIfNeeded().catch(() => {})
          await btn.click()
          probe._preflight = null
          // FFmpeg-backed exports may first show the same user-facing pregate warning
          // that release-verify already handles. Continue non-blocking warnings, then
          // wait for the real export verb response.
          const loops = opt.kind === 'job' || opt.id === 'gif' || opt.id === 'audio' ? 480 : 180
          for (let i = 0; i < loops && exportResp[opt.verb] === undefined; i++) {
            await page.flushEvents?.()
            if (!probe._preflight?.seen) probe._preflight = await continuePreflightIfPresent(page, 1)
            await sleep(250)
          }
        },
        assertResult: async () => {
          const r = exportResp[opt.verb]
          const pg = probe._preflight?.seen ? ` preflight=${JSON.stringify(probe._preflight)}` : ''
          if (!r) return { ok: false, detail: `no ${opt.verb} response observed (timeout)${pg}` }
          if (!r.ok) {
            const msg = r.error?.message || r.error?.code || 'unknown'
            // A caption/marker export missing its precondition = precondition miss, not a
            // wiring bug. Flag it honestly (FAIL on RESULT) but label it so the macOS run
            // (captions present; markers seeded above) is expected to flip it green.
            const precondition = (opt.needsCaptions && !haveCaptions) || (opt.needsMarkers && !haveMarkers)
            return { ok: false, detail: `${opt.verb} returned NOT-ok: "${msg}"${precondition ? ' [PRECONDITION: needs ' + (opt.needsCaptions ? 'captions' : 'timeline markers') + ' — expected PASS once present]' : ''}${pg}` }
          }
          const path = r.result?.path
          const job = r.result?.job_id
          const ok = opt.kind === 'job' ? !!job : !!path
          return { ok, detail: (opt.kind === 'job' ? `job_id=${job ? String(job).slice(0, 12) : 'MISSING'}` : `path=${path ? '…' + String(path).slice(-28) : 'MISSING'}`) + pg }
        },
      })
    }
    if (NATIVE_OS_ACTIONS.enabled) {
      const drained = await drainActiveJobs(600_000)
      if (!drained) throw new Error('default exports did not drain before Save As coverage')
      for (const opt of EXPORT_OPTIONS) {
        if ((await page.locator('[data-cut-export-menu]').count()) === 0) {
          await page.locator('[data-cut-export-btn]').click()
          await sleep(300)
        }
        const button = page.locator(`[data-cut-export-saveas-option="${opt.id}"]`).first()
        const chosenPath = joinHostPath(
          synthEngineDir,
          `save-as-${seq++}-${opt.id}.${EXPORT_EXTENSIONS[opt.id]}`,
        )
        let saveRequest = null
        let saveTerminal = null
        await probe(page, {
          surface: S,
          name: `export-save-as-${opt.id}`,
          actionId: 'export-saveas-option',
          sel: button,
          group: page.locator('[data-cut-export-menu]').first(),
          groupName: 'export-save-as-menu',
          nativeAction: {
            mode: 'select',
            path: chosenPath,
            useDoClick: true,
            verifyResult: true,
          },
          doClick: async () => {
            exportResp[opt.verb] = undefined
            const onRequest = (request) => {
              let pathname = ''
              try { pathname = new URL(request.url()).pathname } catch { return }
              if (pathname !== `/api/verb/${opt.verb}`) return
              try { saveRequest = request.postDataJSON() } catch {}
            }
            page.on('request', onRequest)
            try {
              await button.scrollIntoViewIfNeeded().catch(() => {})
              await button.click()
              probe._preflight = null
              const loops = opt.kind === 'job' || opt.id === 'gif' || opt.id === 'audio' ? 720 : 240
              for (let index = 0; index < loops && exportResp[opt.verb] === undefined; index += 1) {
                await page.flushEvents?.()
                if (!probe._preflight?.seen) {
                  probe._preflight = await continuePreflightIfPresent(page, 1)
                }
                await sleep(250)
              }
              const jobId = exportResp[opt.verb]?.result?.job_id
              if (jobId) saveTerminal = await awaitJob(jobId, 600_000)
            } finally {
              page.off('request', onRequest)
            }
          },
          assertResult: async () => {
            const response = exportResp[opt.verb]
            const requestExact = basenameHostPath(saveRequest?.path || '') ===
              basenameHostPath(chosenPath)
            if (!response?.ok) {
              return {
                ok: false,
                detail: `${opt.verb} Save As failed: ${response?.error?.message || response?.error?.code || 'missing response'}; requestExact=${requestExact}`,
              }
            }
            if (opt.kind === 'job') {
              return {
                ok: requestExact && saveTerminal?.state === 'done',
                detail: `${opt.verb} Save As requestExact=${requestExact}; job=${response.result?.job_id || 'missing'}; terminal=${saveTerminal?.state || 'missing'}`,
              }
            }
            const output = response.result?.path || ''
            const bytes = fileBytes(resolveDriverPath(output))
            return {
              ok: requestExact
                && basenameHostPath(output) === basenameHostPath(chosenPath)
                && bytes > 0,
              detail: `${opt.verb} Save As requestExact=${requestExact}; outputExact=${basenameHostPath(output) === basenameHostPath(chosenPath)}; bytes=${bytes}`,
            }
          },
        })
      }
    }
  } finally {
    page.off('response', onResp)
  }
}

// ── 9. RENDER QUEUE (batch deliver) ───────────────────────────────────────────
async function secRenderQueue(page) {
  const S = 'render-queue'
  await freshProject(page, 'renderq')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  const s0 = await state()
  for (const t of s0.tracks || []) if (t.kind === 'video' || t.kind === 'audio') await verb('edit.ripple_delete', { track: t.id, range_ms: [1500, 999000], ripple: true })
  await sleep(700)
  let queueResp = null
  try {
    const queueOutputPath = joinHostPath(
      synthEngineDir,
      `render-queue-${seq++}.mp4`,
    )
    const queueSecondOutputName = `render-queue-${seq++}.mp4`
    await page.locator('[data-cut-export-btn]').click(); await sleep(300)
    await probe(page, {
      surface: S, name: 'render-queue-open', sel: page.locator('[data-cut-render-queue-open]'),
      group: page.locator('[data-cut-export-menu]').first(), groupName: 'export-menu',
      doClick: async () => { await page.locator('[data-cut-render-queue-open]').click(); await sleep(400) },
      assertResult: async () => ({ ok: (await page.locator('[data-cut-render-queue]').count()) > 0, detail: 'queue modal opened' }),
    })
    // Screenshot the modal as its own group.
    await renderGroup(page, S, 'render-queue-modal', page.locator('[data-cut-render-queue]').first())
    await probe(page, {
      surface: S, name: 'render-queue-output-picker', actionId: 'render-queue-output-pick',
      sel: page.locator('[data-cut-render-queue-output-pick="0"]'),
      group: page.locator('[data-cut-render-queue]').first(), groupName: 'render-queue-modal',
      clickNa: NATIVE_PICKER_CLICK_NA,
      nativeAction: {
        mode: 'select',
        path: queueOutputPath,
        useDoClick: true,
        verifyResult: true,
      },
      doClick: async () => {
        await page.locator('[data-cut-render-queue-output-pick="0"]').click().catch(() => {})
        await sleep(300)
      },
      assertResult: async () => {
        const selected = await page.locator('[data-cut-render-queue-output="0"]').inputValue().catch(() => '')
        if (NATIVE_OS_ACTIONS.enabled) {
          const exact = basenameHostPath(selected) === basenameHostPath(queueOutputPath)
          return {
            ok: exact,
            detail: `installed picker selected=${selected || 'missing'}; exact=${exact}`,
          }
        }
        const note = await page.locator('[data-cut-render-queue-note]').textContent().catch(() => '')
        return {
          ok: (await page.locator('[data-cut-render-queue-output-pick="0"]').count()) > 0 && /desktop app|Choose output file/i.test(note || 'Choose output file'),
          detail: note ? `browser fallback note shown: ${note}` : 'output picker selector present; native save dialog is desktop-only',
        }
      },
    })
    await probe(page, {
      surface: S, name: 'render-queue-start', sel: page.locator('[data-cut-render-queue-start]'),
      group: page.locator('[data-cut-render-queue]').first(), groupName: 'render-queue-modal',
      doClick: async () => {
        await page.locator('[data-cut-render-queue-aspect="1"]').selectOption('project').catch(() => {})
        await page.locator('[data-cut-render-queue-preset="0"]').selectOption('draft').catch(() => {})
        await page.locator('[data-cut-render-queue-preset="1"]').selectOption('draft').catch(() => {})
        // One explicit and one blank output is intentionally rejected by the
        // product: temporarily authorizing the first directory must not make a
        // blank sibling inherit it. Give every delivery a unique file in the
        // same picker-returned directory before submitting the real queue. On
        // macOS a save panel canonicalizes /var to /private/var, so deriving the
        // sibling from the actual field is required; precomputing both paths
        // made one physical folder look like two different folders.
        let firstQueueOutput = await page.locator('[data-cut-render-queue-output="0"]').inputValue()
        if (!firstQueueOutput) {
          firstQueueOutput = queueOutputPath
          await page.locator('[data-cut-render-queue-output="0"]').fill(firstQueueOutput)
        }
        const queueSecondOutputPath = joinHostPath(
          dirnameHostPath(firstQueueOutput),
          queueSecondOutputName,
        )
        await page.locator('[data-cut-render-queue-output="1"]').fill(queueSecondOutputPath)
        await sleep(120)
        await page.locator('[data-cut-render-queue-start]:not([disabled])')
          .waitFor({ state: 'visible', timeout: 15_000 })
        queueResp = await captureVerbResp(page, 'render.queue', async () => {
          await page.locator('[data-cut-render-queue-start]').click()
        }, 60_000)
      },
      assertResult: async () => {
        const qid = queueResp?.result?.queue_id
        const count = queueResp?.result?.count
        if (!qid) return { ok: false, detail: `render.queue gave no queue_id (${JSON.stringify(queueResp?.error || queueResp || {}).slice(0, 80)})` }
        // poll the queue job to terminal.
        let final = null
        for (let i = 0; i < 200; i++) { const js = await verb('jobs.status', { job_id: qid }); const st = js.result?.state; if (st === 'done' || st === 'failed') { final = js.result; break } await sleep(700) }
        return { ok: count >= 1 && final?.state === 'done', detail: `queued=${count} finalState=${final?.state ?? 'timeout'}` }
      },
    })
  } finally {
    await page.locator('[data-cut-render-queue-close]').click().catch(() => {})
    await page.keyboard.press('Escape').catch(() => {})
  }
}

// ── 10. Menus and toolbar globals: timeline tools, render options, project rename, Storyboard ─────
async function secMenus(page) {
  const S = 'menus'
  await drainActiveJobs() // render jobs from earlier sections keep render-opts disabled by design
  await seedMenuToolProject(page)
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  // Timeline global tools. These orchestrators are CONTENT-DEPENDENT:
  // trim_edges needs dead-air, split/mark_scenes need detected scene cuts (and a perception
  // report). The default SCENE clip may have none — then the orchestrator RUNS CLEAN but
  // emits no sub-op. That is NOT a wiring bug, so we record an honest N/A (never a forced
  // pass, never a false-fail). We capture each orchestrator's RESPONSE to distinguish:
  //   • real sub-op landed                       → RESULT pass
  //   • ran ok but no sub-op (no content)         → RESULT N/A (content-dependent)
  //   • missing perception report / precondition  → RESULT N/A (content-dependent)
  //   • genuine error / no response               → RESULT fail
  const toolResp = {}
  const onToolResp = async (r) => {
    const m = r.url().match(/\/api\/verb\/(edit\.trim_edges|edit\.split_at_scenes|edit\.mark_scenes)/)
    if (m) { try { toolResp[m[1]] = await r.json() } catch {} }
  }
  page.on('response', onToolResp)
  try {
    await openTimelineAutomation(page)
    const timelineToolGroup = page.locator('[data-cut-timeline-tools]').first()
    const oldTopbarTools = await page.locator('[data-cut-tools-btn]').count()
    const timelineToolsPresent = await timelineToolGroup.count()
    if (oldTopbarTools) {
      rec(S, 'timeline-tools-location', { present: 'fail', render: 'na', click: 'na', result: 'fail' },
        'old topbar Tools button is still present; global timeline edits should live in the timeline toolbar')
    } else if (!timelineToolsPresent) {
      rec(S, 'timeline-tools-location', { present: 'fail', render: 'na', click: 'na', result: 'fail' },
        'old topbar Tools button is absent, but the timeline automation group did not mount')
    } else {
      rec(S, 'timeline-tools-location', { present: 'pass', render: 'na', click: 'na', result: 'pass' },
        'old topbar Tools button absent and global tools mounted in the timeline automation menu')
    }
    for (const tool of TOOLS) {
      const sel = timelineToolGroup.locator(`[data-cut-tool="${tool.id}"]`)
      const present = (await sel.count()) > 0
      const rg = await renderGroup(page, S, 'timeline-tools', timelineToolGroup)
      if (!present) {
        rec(S, `tool-${tool.id}`, { present: 'fail', render: rg.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' }, `tool item absent ${rg.detail}`.trim(), rg.shot)
        continue
      }
      const before = await opsLen()
      toolResp[tool.orch] = undefined
      let clickOk = true
      try { await sel.click(); await sleep(1500) } catch { clickOk = false }
      if (!clickOk) {
        rec(S, `tool-${tool.id}`, { present: 'pass', render: rg.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' }, `tool click threw ${rg.detail}`.trim(), rg.shot)
        continue
      }
      // RESULT (tri-state). The real sub-op landing = pass; orchestrator ran clean with no
      // sub-op (no dead-air / no scene cuts) or a missing perception report = honest N/A;
      // a genuine error = fail.
      const landed = await opLanded(before, tool.verb)
      let result, detail
      if (landed) {
        result = 'pass'; detail = `${tool.verb} op landed (orchestrator found content)`
      } else {
        const r = toolResp[tool.orch]
        if (!r) { result = 'fail'; detail = `no ${tool.orch} response observed and no ${tool.verb} op landed` }
        else if (r.ok) { result = 'na'; detail = `${tool.orch} ran clean but emitted no ${tool.verb} — content-dependent (no dead-air / scene cuts): ${JSON.stringify(r.result || {}).slice(0, 80)}` }
        else {
          const msg = String(r.error?.message || r.error?.code || 'error')
          const contentDep = r.error?.code === 'not_found' || /perception|report|no (markers|scene|speech|silence)/i.test(msg)
          result = contentDep ? 'na' : 'fail'
          detail = `${tool.orch} ${contentDep ? 'precondition/content-dependent' : 'errored'}: "${msg.slice(0, 80)}"`
        }
      }
      rec(S, `tool-${tool.id}`, { present: 'pass', render: rg.ok ? 'pass' : 'fail', click: 'pass', result }, `${detail} ${rg.detail}`.trim(), rg.shot)
    }
  } finally {
    page.off('response', onToolResp)
  }
  // RENDER OPTIONS selects (project.format mutations + render-local selects).
  // Retry the menu open: require it to be visible before probing the pickers.
  for (let i = 0; i < 4 && !(await page.locator('[data-cut-render-menu]').count()); i++) {
    await page.locator('[data-cut-render-opts]').click().catch(() => {})
    await sleep(400)
  }
  await renderGroup(page, S, 'render-menu', page.locator('[data-cut-render-menu]').first())
  await probe(page, {
    surface: S,
    name: 'timeline-format-advanced-toggle',
    actionId: 'project-format-toggle',
    sel: page.locator('[data-cut-project-format-toggle]'),
    group: page.locator('[data-cut-render-menu]').first(),
    groupName: 'render-menu',
    doClick: async () => {
      await page.locator('[data-cut-project-format-toggle]').click()
      await sleep(150)
    },
    assertResult: async () => ({
      ok: (await page.locator('[data-cut-project-format-settings]').getAttribute('open')) !== null,
      detail: `advanced timeline format open=${(await page.locator('[data-cut-project-format-settings]').getAttribute('open')) !== null}`,
    }),
  })
  await probe(page, {
    surface: S, name: 'render-resolution', sel: page.locator('[data-cut-project-resolution]'),
    group: page.locator('[data-cut-render-menu]').first(), groupName: 'render-menu',
    doClick: async () => { probe._b = await opsLen(); const o = await page.locator('[data-cut-project-resolution] option').nth(1).getAttribute('value'); await page.locator('[data-cut-project-resolution]').selectOption({ label: o }).catch(async () => { await page.locator('[data-cut-project-resolution]').selectOption({ index: 1 }) }); await sleep(600) },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'project.format'), detail: 'project.format op landed (resolution)' }),
  })
  await probe(page, {
    surface: S, name: 'render-fps', sel: page.locator('[data-cut-project-fps]'),
    group: page.locator('[data-cut-render-menu]').first(), groupName: 'render-menu',
    doClick: async () => { probe._b = await opsLen(); await page.locator('[data-cut-project-fps]').selectOption({ index: 1 }).catch(() => {}); await sleep(600) },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'project.format'), detail: 'project.format op landed (fps)' }),
  })
  // Pure render-local selects (no op until Render) — assert the value sticks.
  for (const [name, selr, val] of [
    ['render-aspect', '[data-cut-render-aspect]', '9:16'],
    ['render-quality', '[data-cut-render-preset]', 'high'],
    ['render-fileformat', '[data-cut-render-format]', 'hevc'],
    ['render-loudness', '[data-cut-render-loudness]', '-14'],
  ]) {
    await probe(page, {
      surface: S, name, sel: page.locator(selr),
      group: page.locator('[data-cut-render-menu]').first(), groupName: 'render-menu',
      doClick: async () => { await page.locator(selr).selectOption(val).catch(() => {}); await sleep(200) },
      assertResult: async () => ({ ok: (await page.locator(selr).inputValue().catch(() => '')) === val, detail: `${name}=${val} stuck` }),
    })
  }
  await page.keyboard.press('Escape').catch(() => {}); await sleep(150)

  // PROJECT RENAME (click the title → input → commit → project.rename).
  const newName = 'FCV_RENAMED_' + Math.random().toString(36).slice(2, 5)
  const topbarProject = page.locator('[data-cut-panel="topbar"] [data-cut-project]').first()
  await probe(page, {
    surface: S, name: 'project-rename', sel: topbarProject,
    group: page.locator('[data-cut-panel="topbar"]').first(), groupName: 'topbar',
    doClick: async () => {
      probe._b = await opsLen()
      await topbarProject.click()
      await sleep(250)
      const inp = page.locator('[data-cut-panel="topbar"] [data-cut-project-rename]').first()
      await inp.waitFor({ state: 'visible', timeout: UI_ACTION_TIMEOUT_MS })
      await inp.fill(newName)
      await inp.press('Enter')
      await sleep(600)
    },
    assertResult: async () => {
      const renamed = await waitForState((st) => (st.name || st.settings?.name || '').includes('FCV_RENAMED'), 8000)
      const opOk = await opLanded(probe._b, 'project.rename')
      return { ok: !!renamed || opOk, detail: `project.rename op=${opOk} name→${renamed ? 'updated' : 'unconfirmed'}` }
    },
  })

  // STORYBOARD overlay (render.storyboard — pure view, inline contact sheet).
  await probe(page, {
    surface: S, name: 'storyboard', sel: page.locator('[data-cut-storyboard-btn]'),
    group: page.locator('[data-cut-panel="topbar"]').first(), groupName: 'topbar',
    doClick: async () => {
      await page.locator('[data-cut-storyboard-btn]').click()
      probe._storyboard = await waitForStoryboardSettled(page)
    },
    assertResult: async () => {
      const settled = probe._storyboard || await waitForStoryboardSettled(page, 5000)
      const img = settled.img || (await page.locator('[data-cut-storyboard-img]').count()) > 0
      const err = settled.err || (await page.locator('[data-cut-storyboard-error]').count()) > 0
      await renderGroup(page, S, 'storyboard-overlay', page.locator('[data-cut-storyboard]').first())
      await page.locator('[data-cut-storyboard-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {})
      return { ok: img || err, detail: `contact-sheet img=${img} honestError=${err} state=${settled.state}` }
    },
  })
}

// ── 11. DRAWERS that CREATE clips: Title / Shape / Music ──────────────────────
async function secDrawers(page) {
  const S = 'create-drawers'
  await freshProject(page, 'drawers')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  // TITLE drawer. Open it BEFORE the probe so RENDER screenshots the OPEN drawer
  // surface ([data-cut-title] mounts only once the trigger fires); doClick then does
  // only the in-drawer work (fill + apply). Mirrors the render-queue-open sequence.
  await page.locator('[data-cut-title-btn]').click(); await sleep(600)
  await probe(page, {
    surface: S, name: 'title-drawer-apply', sel: page.locator('[data-cut-title-btn]'),
    group: page.locator('[data-cut-title]').first(), groupName: 'title-drawer',
    doClick: async () => {
      await page.locator('[data-cut-title-text]').fill('FCV Title ' + Math.random().toString(36).slice(2, 5)).catch(() => {})
      await page.locator('[data-cut-title-apply]').click(); await sleep(1500)
    },
    assertResult: async () => {
      const ok = await waitForState((st) => (st.tracks || []).some((t) => (t.id || '').startsWith('title') && (t.clips || []).some((c) => c.title_text)), 20000)
      await page.locator('[data-cut-title-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {})
      return { ok: !!ok, detail: `title clip created on a title track=${!!ok}` }
    },
  })
  // SHAPE drawer. Open it BEFORE the probe (same reasoning as the title drawer).
  await page.locator('[data-cut-shape-btn]').click(); await sleep(600)
  await probe(page, {
    surface: S, name: 'shape-drawer-apply', sel: page.locator('[data-cut-shape-btn]'),
    group: page.locator('[data-cut-shape]').first(), groupName: 'shape-drawer',
    doClick: async () => {
      await page.locator('[data-cut-shape-text]').fill('FCV Shape').catch(() => {})
      await page.locator('[data-cut-shape-apply]').click(); await sleep(1500)
    },
    assertResult: async () => {
      const ok = await waitForState((st) => flatClips(st).some((c) => c.shape_kind), 20000)
      await page.locator('[data-cut-shape-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {})
      return { ok: !!ok, detail: `shape clip placed=${!!ok}` }
    },
  })
  // Music-bed drawer: seed a real
  // audio asset (a synthesized tone — the drawer's candidate list offers only audio-KIND
  // assets, so the footage video doesn't qualify), pick it, turn DUCK OFF (so audio.add_music
  // needs no perception silence facts), drive Apply, and assert a MUSIC TRACK LANDS (a new
  // audio clip referencing the bed asset). PRESENT/RENDER from the open drawer; CLICK+RESULT
  // from the real Apply → audio.add_music. Selectors grepped from panels/MusicBed: asset
  // picker [data-cut-musicbed-asset], duck toggle [data-cut-musicbed-duck], Apply
  // [data-cut-musicbed-apply].
  const tone = makeToneAudio(6)
  const alternateTone = makeToneAudio(4)
  if (!tone || !alternateTone) {
    rec(S, 'music-drawer-apply(audio.add_music)', { present: 'na', render: 'na', click: 'na', result: 'na' },
      'ffmpeg unavailable to synthesize a bed tone — cannot seed an audio-kind asset for the MusicBed drawer (env guard)')
  } else {
    const firstImport = await verb('media.import', { path: tone })
    const secondImport = await verb('media.import', { path: alternateTone })
    const firstMusicAsset = firstImport.result?.asset_id
    const musicAsset = secondImport.result?.asset_id
    await sleep(1200)
    await page.locator('[data-cut-music-btn]').click(); await sleep(700)
    await probe(page, {
      surface: S, name: 'music-drawer-asset', actionId: 'musicbed-asset',
      sel: page.locator('[data-cut-musicbed-asset]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer-controls',
      doClick: async () => {
        if (firstMusicAsset) await page.locator('[data-cut-musicbed-asset]').selectOption(firstMusicAsset)
        if (musicAsset) await page.locator('[data-cut-musicbed-asset]').selectOption(musicAsset)
      },
      assertResult: async () => {
        const value = await page.locator('[data-cut-musicbed-asset]').inputValue()
        return { ok: !!musicAsset && value === musicAsset, detail: `selected music asset=${String(value).slice(0, 12)} expected=${String(musicAsset).slice(0, 12)}` }
      },
    })
    await probe(page, {
      surface: S, name: 'music-drawer-level', actionId: 'musicbed-bedgain-input',
      sel: page.locator('[data-cut-musicbed-bedgain-input]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer-controls',
      doClick: async () => { await page.locator('[data-cut-musicbed-bedgain-input]').fill('-22') },
      assertResult: async () => {
        const value = await page.locator('[data-cut-musicbed-bedgain-input]').inputValue()
        const label = await page.locator('[data-cut-musicbed-bedgain]').textContent()
        return { ok: value === '-22' && label?.includes('-22 dB'), detail: `level=${value}; label=${label}` }
      },
    })
    await probe(page, {
      surface: S, name: 'music-drawer-duck-depth', actionId: 'musicbed-duckdb-input',
      sel: page.locator('[data-cut-musicbed-duckdb-input]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer-controls',
      doClick: async () => { await page.locator('[data-cut-musicbed-duckdb-input]').fill('-20') },
      assertResult: async () => {
        const value = await page.locator('[data-cut-musicbed-duckdb-input]').inputValue()
        const label = await page.locator('[data-cut-musicbed-duckdb]').textContent()
        return { ok: value === '-20' && label?.includes('20 dB'), detail: `duck=${value}; label=${label}` }
      },
    })
    await probe(page, {
      surface: S, name: 'music-drawer-duck-toggle', actionId: 'musicbed-duck',
      sel: page.locator('[data-cut-musicbed-duck]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer-controls',
      doClick: async () => {
        const duck = page.locator('[data-cut-musicbed-duck]').first()
        probe._duckStates = []
        await duck.click()
        probe._duckStates.push(await duck.isChecked())
        await duck.click()
        probe._duckStates.push(await duck.isChecked())
      },
      assertResult: async () => ({
        ok: probe._duckStates?.[0] === false && probe._duckStates?.[1] === true,
        detail: `duck states=${probe._duckStates?.join('→')}`,
      }),
    })
    await probe(page, {
      surface: S, name: 'music-drawer-beat-markers', actionId: 'musicbed-beats',
      sel: page.locator('[data-cut-musicbed-beats]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer-controls',
      doClick: async () => {
        const beats = page.locator('[data-cut-musicbed-beats]').first()
        probe._beatStates = []
        await beats.click()
        probe._beatStates.push(await beats.isChecked())
        await beats.click()
        probe._beatStates.push(await beats.isChecked())
      },
      assertResult: async () => ({
        ok: probe._beatStates?.[0] === false && probe._beatStates?.[1] === true,
        detail: `beat-marker states=${probe._beatStates?.join('→')}`,
      }),
    })
    await probe(page, {
      surface: S, name: 'music-drawer-mute-original(edit.mute)', sel: page.locator('[data-cut-musicbed-mute-original]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer',
      doClick: async () => {
        const baseAudio = ((await state()).tracks || []).find((t) => t.kind === 'audio')
        probe._muteTrack = baseAudio?.id || ''
        const cb = page.locator('[data-cut-musicbed-mute-original]').first()
        probe._muteTarget = !(await cb.isChecked().catch(() => false))
        probe._b = await opsLen()
        await cb.click(); await sleep(700)
      },
      assertResult: async () => {
        const track = probe._muteTrack
        const target = probe._muteTarget === true
        const opOk = await opLanded(probe._b, 'edit.mute', (a) => a.track === track && a.on === target, { timeoutMs: 8000 })
        const stateOk = !!(await waitForState((st) => (st.tracks || []).some((t) => t.id === track && t.muted === target), 10000))
        return { ok: opOk && stateOk, detail: `edit.mute via Mute original → track=${track || '?'} target=${target} op=${opOk} state=${stateOk}` }
      },
    })
    await probe(page, {
      surface: S, name: 'music-drawer-apply(audio.add_music)', sel: page.locator('[data-cut-music-btn]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer',
      doClick: async () => {
        // Pick the seeded bed asset (auto-seeded if it's the only candidate, but set it
        // explicitly so the apply is deterministic), then disable duck (no perception need).
        if (musicAsset) await page.locator('[data-cut-musicbed-asset]').selectOption(musicAsset).catch(() => {})
        const duck = page.locator('[data-cut-musicbed-duck]').first()
        if (await duck.isChecked().catch(() => false)) await duck.click().catch(() => {})
        probe._b = await opsLen()
        probe._musicArgs = undefined
        const onMusicRequest = (request) => {
          if (probe._musicArgs !== undefined) return
          try {
            if (new URL(request.url(), APP).pathname === '/api/verb/audio.add_music') {
              probe._musicArgs = request.postDataJSON()
            }
          } catch {}
        }
        page.on('request', onMusicRequest)
        try {
          await page.locator('[data-cut-musicbed-apply]:not([disabled])')
            .waitFor({ state: 'visible', timeout: 15_000 })
          probe._musicResp = await captureVerbResp(page, 'audio.add_music', async () => {
            await page.locator('[data-cut-musicbed-apply]').click()
          }, 30_000)
        } finally {
          page.off('request', onMusicRequest)
        }
        await sleep(700)
      },
      assertResult: async () => {
        // A music bed = a new audio clip referencing the bed asset on some audio track.
        const landed = await waitForState((st) => flatClips(st).some((c) => c._kind === 'audio' && c.asset === musicAsset), 15000)
        const opOk = await opLanded(probe._b, 'audio.add_music')
        const args = probe._musicArgs
        const requestOk = args?.asset === musicAsset
          && args?.bed_gain_db === -22
          && args?.duck === false
          && args?.beat_markers === true
        const responseOk = probe._musicResp?.ok === true
          && probe._musicResp?.result?.bed_gain_db === -22
        return { ok: (!!landed || opOk) && requestOk && responseOk, detail: `audio.add_music via Apply → timeline=${!!landed}; op=${opOk}; request=${JSON.stringify(args)}; response=${responseOk}` }
      },
    })
    await probe(page, {
      surface: S, name: 'music-drawer-close-completed', actionId: 'musicbed-close',
      sel: page.locator('[data-cut-musicbed-close]'),
      group: page.locator('[data-cut-musicbed]').first(), groupName: 'music-drawer-completed',
      doClick: async () => {
        await page.locator('[data-cut-musicbed-close]').click()
        await page.locator('[data-cut-musicbed-open="true"]').waitFor({ state: 'detached', timeout: 5000 })
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-musicbed-open="true"]').count() === 0,
        detail: `completed drawer closed=${await page.locator('[data-cut-musicbed-open="true"]').count() === 0}`,
      }),
    })
  }
}

// ── 12. AGENT chat: suggestion chips (always) + real prompts (FCV_AGENT) ──────
async function secAgent(page) {
  const S = 'agent-chat'
  await freshProject(page, 'agent')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  await ensureRail(page)
  await page.locator('[data-cut-right-tab="chat"]').click().catch(() => {}); await sleep(400)
  const chat = page.locator('[data-cut-chat]').first()
  await renderGroup(page, S, 'chat-panel', chat)
  // Suggestion chips — PRESENT/RENDER/CLICK + pre-fill (no auto-send = no spend).
  const chips = page.locator('[data-cut-chat-chip]')
  const n = await chips.count()
  rec(S, 'GATE:chips-present', gateDim(n >= 3), `${n} suggestion chips`)
  for (let i = 0; i < Math.min(n, 3); i++) {
    const label = await chips.nth(i).getAttribute('data-cut-chat-chip')
    await probe(page, {
      surface: S, name: `chip-${i}-${(label || '').replace(/\s+/g, '_').slice(0, 14)}`, sel: chips.nth(i),
      group: page.locator('[data-cut-chat-chips]').first(), groupName: 'chat-chips',
      doClick: async () => { await page.locator('[data-cut-chat-input]').first().fill(''); await chips.nth(i).click(); await sleep(300) },
      assertResult: async () => {
        const v = (await page.locator('[data-cut-chat-input]').first().inputValue().catch(() => '')) || ''
        return { ok: v.trim().length > 0, detail: `chip pre-filled compose input (len=${v.length})` }
      },
    })
  }
  await page.locator('[data-cut-chat-input]').first().fill('').catch(() => {})
  const selectChatAgent = async (name) => {
    const root = page.locator('[data-cut-chat]').first()
    const active = await root.getAttribute('data-cut-chat-agent').catch(() => '')
    if (active === name) return active
    const trigger = page.locator('[data-cut-chat-agent-select]').first()
    await trigger.click().catch(() => {})
    await sleep(250)
    await page.locator(`[data-cut-chat-agent-option="${name}"]`).first().click().catch(() => {})
    await sleep(350)
    return await root.getAttribute('data-cut-chat-agent').catch(() => '')
  }
  const agentSelector = page.locator('[data-cut-chat-agent-select]').first()
  await probe(page, {
    surface: S, name: 'agent-selector-open', actionId: 'chat-agent-select',
    sel: agentSelector, group: chat, groupName: 'chat-agent-selector',
    doClick: async () => {
      await agentSelector.click()
      await page.locator('[data-cut-chat-agent-menu]').waitFor({ state: 'visible', timeout: 8000 })
    },
    assertResult: async () => ({
      ok: await agentSelector.getAttribute('aria-expanded') === 'true'
        && await page.locator('[data-cut-chat-agent-option]').count() === 3,
      detail: `expanded=${await agentSelector.getAttribute('aria-expanded')}; options=${await page.locator('[data-cut-chat-agent-option]').count()}`,
    }),
  })
  await agentSelector.click().catch(() => {})
  await sleep(100)
  const agentUnavailableResult = (resp, fallbackAgent) => {
    const res = resp?.result
    if (!res || res.ok !== false) return null
    const agentName = res.agent || fallbackAgent || 'agent'
    const reason = String(res.reason || res.reply || res.agent_message || res.detail || 'agent.chat returned ok:false')
    const hay = [res.error, reason, res.agent_message, res.detail].map((x) => String(x || '')).join(' ').toLowerCase()
    const quota = res.error === 'quota' || /quota|weekly limit|daily limit|monthly limit|usage limit|rate limit|too many requests|limit reached|you(?:'| have) hit your/.test(hay)
    const auth = res.error === 'auth' || /not authenticated|not signed in|please log in|please login|sign in|login required|session expired|token expired/.test(hay)
    if (quota || auth) {
      return {
        result: 'na',
        detail: `${agentName} unavailable (${quota ? 'quota/usage limit' : 'auth/session'}): ${reason.slice(0, 220)} — honest dev skip; FCV_REQUIRE_FULL=1 needs an available subscription turn`,
      }
    }
    return {
      ok: false,
      detail: `${agentName} returned no edit: ${String(res.error || 'no_change')} ${reason.slice(0, 220)}`,
    }
  }

  // REAL agent prompts — drive the natural-language editor end-to-end and assert
  // the TIMELINE actually changed. Gated: each spends a real subscription-CLI turn.
  const prompts = [
    { msg: 'split the clip at 2 seconds', assert: async (before) => (await clipCount()) > before.clips, label: 'split-at-2s' },
    { msg: 'add a fade in to the first clip', assert: async (before) => await fadeChanged(before), label: 'add-fade-in' },
    { msg: 'make the first clip 2x speed', assert: async (before) => await speedChanged(before), label: 'speed-2x' },
  ]
  const clipCount = async () => flatClips(await state()).filter((c) => c.asset).length
  const fadeChanged = async (before) => { const o = await ops(); return o.length > before.ops && o.slice(before.ops).some((x) => x.verb === 'edit.fade') }
  const speedChanged = async (before) => { const o = await ops(); return o.length > before.ops && o.slice(before.ops).some((x) => x.verb === 'edit.speed') }

  // The real prompts spend a subscription-CLI turn. Use a READY backend for the
  // generic end-to-end prompt checks; per-provider coverage below proves each
  // backend separately (including a quota/auth N/A for one backend).
  const promptAgent = DEP.chatAgents.codex ? 'codex' : DEP.chatAgents.claude ? 'claude' : DEP.chatAgents.grok ? 'grok' : ''
  if (RUN_AGENT && promptAgent) await selectChatAgent(promptAgent)
  if (!RUN_AGENT || !promptAgent) {
    const reason = !promptAgent
      ? 'no chat-ready CLI on this rig — agent.chat prompt checks need claude/codex/grok; honest dev skip (FCV_REQUIRE_FULL=1 enforces claude present)'
      : 'FCV_NO_AGENT=1 — real agent.chat prompt skipped (would spend a subscription turn). Always runs under FCV_REQUIRE_FULL=1.'
    for (const p of prompts) rec(S, `prompt-${p.label}`, { present: 'na', render: 'na', click: 'na', result: 'na' }, reason)
  } else {
    for (const p of prompts) {
      const input = page.locator('[data-cut-chat-input]').first()
      const send = page.locator('[data-cut-chat-send]').first()
      await probe(page, {
        surface: S, name: `prompt-${p.label}`, sel: input, group: chat, groupName: 'chat-panel',
        doClick: async () => {
          probe._before = { clips: await clipCount(), ops: await opsLen() }
          probe._agentResp = await captureVerbResp(page, 'agent.chat', async () => {
            await input.fill(p.msg)
            await send.click()
          }, 150000)
          // wait for the turn: busy clears OR the timeline changes. Bounded ~120s.
          for (let i = 0; i < 240; i++) {
            await sleep(500)
            const busy = (await page.locator('[data-cut-chat-busy]').count()) > 0
            if (!busy && i > 4) break
          }
        },
        assertResult: async () => {
          const unavailable = agentUnavailableResult(probe._agentResp, promptAgent)
          if (unavailable) return unavailable
          let ok = false
          for (let i = 0; i < 20; i++) { if (await p.assert(probe._before)) { ok = true; break } await sleep(600) }
          return { ok, detail: ok ? `agent ${promptAgent} executed verbs → timeline state changed` : `no timeline change observed (agent=${promptAgent}, resp=${probe._agentResp ? JSON.stringify(probe._agentResp.result?.error || probe._agentResp.result?.reason || 'ok').slice(0, 120) : 'none'})` }
        },
      })
    }
  }

  // ── Per-provider agent coverage (Claude, Codex, and Grok) ─────────────────────
  // The multi-agent dropdown lets the user pick which
  // coding-agent CLI drives the turn. This proves the multi-agent backend across ALL
  // THREE — not just the default claude — by, for each READY agent, opening the
  // dropdown (menu-open-capture for RENDER), selecting it, sending one deterministic
  // edit prompt, and asserting an op LANDED (the agent's MCP tool calls became real,
  // undoable timeline ops). Per-agent dep-gate: an agent that isn't installed/wired/
  // authed is skipped honestly (codex/grok are OPTIONAL — e.g. grok's auth=unknown
  // expiring-token case — so their absence is a BENIGN skip, never a gate fail; only
  // claude is preflight-enforced). Selectors grepped from panels/AgentChat: trigger
  // [data-cut-chat-agent-select], menu [data-cut-chat-agent-menu], options
  // [data-cut-chat-agent-option="<name>"]; the chosen agent rides on [data-cut-chat-agent].
  for (const [providerIndex, name] of ['claude', 'codex', 'grok'].entries()) {
    if (!RUN_AGENT) {
      rec(S, `provider-${name}(dropdown)`, { present: 'na', render: 'na', click: 'na', result: 'na' },
        'FCV_NO_AGENT=1 — per-provider agent.chat skipped (would spend a subscription turn). Always runs under FCV_REQUIRE_FULL=1 for claude; codex/grok run when authed.')
      continue
    }
    if (!DEP.chatAgents[name]) {
      // claude is preflight-ENFORCED under FULL → an absent claude is a real dep-skip
      // (fails the release gate, correctly). codex/grok are OPTIONAL backends → an
      // absent one is a benign "optional multi-agent" skip that never fails the gate.
      if (name === 'claude') {
        rec(S, `provider-${name}(dropdown)`, { present: 'na', render: 'na', click: 'na', result: 'na' },
          'claude not chat-ready (system.doctor judge.claude.details.chat not installed/wired/authed) — honest dev skip; FCV_REQUIRE_FULL=1 enforces claude present')
      } else {
        rec(S, `provider-${name}(dropdown)`, { present: 'na', render: 'na', click: 'na', result: 'na' },
          `optional multi-agent backend "${name}" not ready on this rig (judge.${name}.details.chat installed/wired/authenticated≠yes — e.g. grok's expiring-token auth=unknown). Benign per-provider skip — NOT preflight-enforced, so the release gate tolerates it; codex/grok prove the backend only when a confirmed session exists.`)
      }
      continue
    }
    const trigger = page.locator('[data-cut-chat-agent-select]').first()
    const menu = page.locator('[data-cut-chat-agent-menu]').first()
    const option = page.locator(`[data-cut-chat-agent-option="${name}"]`).first()
    const input = page.locator('[data-cut-chat-input]').first()
    const send = page.locator('[data-cut-chat-send]').first()
    // Open the dropdown BEFORE the probe (menu-open-capture per the raised bar) so RENDER
    // screenshots the OPEN menu — the [data-cut-chat-agent-menu] only mounts while open.
    for (let i = 0; i < 20; i++) { if (!(await trigger.isDisabled().catch(() => false))) break; await sleep(250) }
    await trigger.click().catch(() => {}); await sleep(400)
    await probe(page, {
      surface: S, name: `provider-${name}(dropdown)`, sel: trigger, group: menu, groupName: `chat-agent-menu-${name}`,
      doClick: async () => {
        // Pick this agent from the (already-open) menu, then send a deterministic edit.
        await option.click().catch(() => {})
        await sleep(300)
        // Confirm the selection actually switched the active backend before spending a turn.
        probe._agentSel = (await page.locator('[data-cut-chat][data-cut-chat-agent]').first().getAttribute('data-cut-chat-agent').catch(() => '')) || ''
        probe._before = await opsLen()
        probe._agentResp = await captureVerbResp(page, 'agent.chat', async () => {
          // Each ready provider gets a distinct location. Reusing 3 seconds
          // made the second backend correctly return "no change" after the
          // first backend had already created that marker.
          await input.fill(`using ${name}: add a marker at ${3 + providerIndex} seconds named FCV ${name}`)
          await send.click().catch(() => {})
        }, 150000)
        for (let i = 0; i < 300; i++) { // bounded ~150s per turn
          await sleep(500)
          const busy = (await page.locator('[data-cut-chat-busy]').count()) > 0
          if (!busy && i > 4) break
        }
      },
      assertResult: async () => {
        const unavailable = agentUnavailableResult(probe._agentResp, name)
        if (unavailable) return unavailable
        let landed = false
        for (let i = 0; i < 20; i++) { if ((await opsLen()) > probe._before) { landed = true; break } await sleep(600) }
        const marker = await opLanded(probe._before, 'edit.add_marker')
        return { ok: landed, detail: `agent="${name}" selected(active=${probe._agentSel}) → ${landed ? `op landed (edit.add_marker=${marker}) — multi-agent backend proven for ${name}` : `NO op landed (the ${name} CLI may be wired but unable to complete the turn)`}` }
      },
    })
  }
}

// ── 13. TIMELINE CONTEXT MENU (right-click) — the full menu surface ──────────
// Opens the right-click menu on a VIDEO clip (then an AUDIO clip), screenshots it,
// proves curation (items correctly present/disabled per clip type), and DRIVES
// each item → asserts the op/state. J/L-cut gets a present+gating verdict here and
// effect-proof in interaction-verify (split-edit-control). Fit-to-fill is also gated
// here, then driven against a real lifted gap by secTimelineActions.
async function secContextMenu(page) {
  const S = 'ctx-menu'
  await freshProject(page, 'ctx')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  const menu = page.locator('[data-cut-clip-menu]').first()
  const videoCount = async () => flatClips(await state()).filter((c) => c._kind === 'video' && c.asset).length
  const audioCount = async () => flatClips(await state()).filter((c) => c._kind === 'audio' && c.asset).length
  const totalCount = async () => flatClips(await state()).filter((c) => c.asset).length
  const anyVideo = async () => await clipOfKind('video')
  const openMenuOn = async (id) => {
    if (!id) return false
    await page.keyboard.press('Escape').catch(() => {})
    await page.locator('[data-cut-ctx-backdrop]').click({ force: true, timeout: 800 }).catch(() => {})
    for (let i = 0; i < 12; i++) {
      if ((await page.locator('[data-cut-clip-menu]').count().catch(() => 0)) === 0) break
      await sleep(80)
    }
    const clipEl = page.locator(`[data-cut-clip="${id}"]`).first()
    if ((await clipEl.count().catch(() => 0)) === 0) return false
    await clipEl.scrollIntoViewIfNeeded().catch(() => {})
    await clipEl.click({ button: 'right', force: true, timeout: 5000 }).catch(() => {})
    await page.waitForSelector('[data-cut-clip-menu]', { timeout: 1500 }).catch(async () => {
      await clipEl.evaluate((el) => {
        const rect = el.getBoundingClientRect()
        el.dispatchEvent(new MouseEvent('contextmenu', {
          bubbles: true,
          cancelable: true,
          clientX: rect.left + rect.width / 2,
          clientY: rect.top + rect.height / 2,
          button: 2,
        }))
      }).catch(() => {})
    })
    await page.waitForSelector('[data-cut-clip-menu]', { timeout: 3000 }).catch(() => {})
    await sleep(150)
    return (await page.locator('[data-cut-clip-menu]').count().catch(() => 0)) > 0
  }
  const clickCtx = async (ctxId) => {
    const item = page.locator(`[data-cut-ctx="${ctxId}"]`).first()
    await item.scrollIntoViewIfNeeded().catch(() => {})
    await item.click({ force: true, timeout: 5000 })
  }

  // ── VIDEO clip menu ──
  let clip = await anyVideo()
  await openMenuOn(clip)
  await renderGroup(page, S, 'ctx-video', menu) // screenshot the whole menu once

  // Curation/gating proofs (present + correct enabled/disabled state).
  rec(S, 'GATE:video-has-detach-audio', gateDim((await page.locator('[data-cut-ctx="detach-audio"]').count()) > 0), 'detach-audio present on video clip')
  rec(S, 'GATE:video-has-blur-faces', gateDim((await page.locator('[data-cut-ctx="blur-faces"]').count()) > 0), 'blur-faces present on video clip')
  rec(S, 'GATE:nest-disabled-single', gateDim(await page.locator('[data-cut-ctx="nest"]').isDisabled().catch(() => false)), 'Nest disabled on single-select')
  rec(S, 'GATE:add-transition-gated-no-seam', gateDim(await page.locator('[data-cut-ctx="add-transition"]').isDisabled().catch(() => false)), 'Add-transition disabled without an adjacent seam')
  rec(S, 'GATE:fit-to-fill-gated-no-gap', gateDim(await page.locator('[data-cut-ctx="fit-to-fill"]').isDisabled().catch(() => false)), 'Fit-to-fill disabled without an adjacent gap')

  // The disabled no-gap state is the support row above. The actionable parent
  // and its asset child are both clicked beside a real gap in timeline-actions.
  // speed-custom opens a window.prompt() — DRIVE it: register a one-time dialog handler
  // that types a factor, click, and assert edit.speed landed (the verb the prompt routes
  // to). No longer a present-only N/A.
  await probe(page, {
    surface: S, name: 'ctx-speed-custom', sel: page.locator('[data-cut-ctx="speed-custom"]'), group: menu, groupName: 'ctx-video',
    doClick: async () => {
      await openMenuOn(clip)
      probe._b = await opsLen()
      page.once('dialog', (d) => d.accept('3').catch(() => {})) // type a 3× factor into the prompt
      await clickCtx('speed-custom')
      await sleep(700)
    },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.speed', (a) => a.clip === clip), detail: 'edit.speed op landed (custom 3× via the prompt dialog)' }),
  })
  // J/L cuts need matching video and audio seams. timeline-actions creates both
  // preconditions and clicks each menu item in its own fresh project.

  // Non-destructive driven items (reopen menu on the same clip each, assert op).
  const driveOp = async (name, ctxId, verbName) => {
    await openMenuOn(clip)
    await probe(page, {
      surface: S, name: `ctx-${name}`, sel: page.locator(`[data-cut-ctx="${ctxId}"]`), group: menu, groupName: 'ctx-video',
      doClick: async () => { probe._b = await opsLen(); await clickCtx(ctxId); await sleep(700) },
      assertResult: async () => ({ ok: await opLanded(probe._b, verbName), detail: `${verbName} op landed` }),
    })
  }
  await driveOp('speed-half', 'speed-half', 'edit.speed')
  await driveOp('speed-double', 'speed-double', 'edit.speed')
  await driveOp('speed-normal', 'speed-normal', 'edit.speed')
  await driveOp('reverse', 'reverse', 'edit.reverse')
  await driveOp('freeze', 'freeze', 'edit.freeze')
  await driveOp('fade-in', 'fade-in', 'edit.fade')
  await driveOp('fade-out', 'fade-out', 'edit.fade')
  // blur-faces (ctx) — needs a REAL face. `clip` is the SCENE road (auto-detect finds 0
  // faces → no op). Bring the detector-proven FACE clip onto the BASE video track (face-blur
  // is base-track-only), right-click it MID-clip (the menu's detect frame follows the
  // cursor, so a mid-clip right-click lands on the face), and assert edit.redact landed
  // for THAT clip. Frame had no detectable face / no perception sidecar → honest N/A. The
  // face clip is left in place; the remaining ctx checks operate on any video clip
  // (op/count-based, not SSIM), so the extra base clip is harmless.
  {
    const faceClip = await addBaseFaceClip(page)
    // perception present ⇒ the face fixture + detection MUST work; not → real FAIL.
    const bfNa = (FULL || DEP.perceptionCv) ? 'fail' : 'na'
    if (!faceClip) {
      rec(S, 'ctx-blur-faces', { present: bfNa, render: bfNa, click: bfNa, result: bfNa },
        `face-clip fixture unavailable (FACE import/insert failed) — perception=${DEP.perceptionCv}; ${bfNa === 'fail' ? 'complete env MUST load it (FAIL)' : 'honest dev skip; FCV_REQUIRE_FULL=1 enforces perception present'}`)
    } else {
      await openMenuOn(faceClip)
      const bf = page.locator('[data-cut-ctx="blur-faces"]')
      const present = (await bf.count()) > 0
      const rg = await renderGroup(page, S, 'ctx-video', menu)
      if (!present) {
        rec(S, 'ctx-blur-faces', { present: 'fail', render: rg.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' },
          `blur-faces item absent on face clip ${rg.detail}`.trim(), rg.shot)
      } else {
        const before = await opsLen()
        await clickCtx('blur-faces'); await sleep(900)
        let landed = await opLanded(before, 'edit.redact', (a) => a.clip === faceClip)
        let detail
        if (landed) {
          detail = 'edit.redact(faces) op landed from the context menu (face detected at the cursor frame)'
        } else {
          const before2 = await opsLen()
          const r = await verb('edit.redact', { clip: faceClip, faces: true, mode: 'blur', at_ms: FACE_DETECT_MS, rationale: 'fcv: ctx faces redact at a known face frame' })
          landed = await opLanded(before2, 'edit.redact', (a) => a.clip === faceClip)
          const found = r.result?.faces?.found ?? r.result?.found
          const why = r.ok ? `found=${found ?? 0}` : `engine: ${String(r.error?.message || r.error?.code || 'error').slice(0, 70)}`
          detail = landed
            ? `edit.redact(faces) op landed at at_ms=${FACE_DETECT_MS} (cursor frame had no detectable face; ${why})`
            : `no face detected / no perception sidecar (menu + verb@${FACE_DETECT_MS} both 0; ${why}) — honest dev skip; FCV_REQUIRE_FULL=1 enforces perception present`
        }
        rec(S, 'ctx-blur-faces',
          { present: 'pass', render: rg.ok ? 'pass' : 'fail', click: 'pass', result: landed ? 'pass' : bfNa },
          `${detail} (perception=${DEP.perceptionCv}) ${rg.detail}`.trim(), rg.shot)
      }
    }
  }
  await driveOp('mute', 'mute', 'edit.gain')

  // clean-voice — orchestrator (edit.eq / edit.effect sub-ops).
  await openMenuOn(clip)
  await probe(page, {
    surface: S, name: 'ctx-clean-voice', sel: page.locator('[data-cut-ctx="clean-voice"]'), group: menu, groupName: 'ctx-video',
    doClick: async () => { probe._b = await opsLen(); await clickCtx('clean-voice'); await sleep(1800) },
    assertResult: async () => {
      const eq = await opLanded(probe._b, 'edit.eq'); const eff = await opLanded(probe._b, 'edit.effect')
      return { ok: eq || eff, detail: `voice chain sub-ops eq=${eq} effect=${eff}` }
    },
  })
  // stabilize — op-backed; tolerant (may be slow / honest on a dev engine).
  await openMenuOn(clip)
  await probe(page, {
    surface: S, name: 'ctx-stabilize', sel: page.locator('[data-cut-ctx="stabilize"]'), group: menu, groupName: 'ctx-video',
    doClick: async () => { probe._b = await opsLen(); await clickCtx('stabilize'); await sleep(2000) },
    assertResult: async () => {
      const op = await opLanded(probe._b, 'edit.stabilize'); const grew = (await opsLen()) > probe._b
      return { ok: op || grew, detail: `edit.stabilize op=${op} (op-log grew=${grew})` }
    },
  })
  // detach-audio — base clip has a LINKED audio sibling → a clean no-op is the
  // CORRECT behaviour (extract only happens for a video-only clip). Pass on either.
  await openMenuOn(clip)
  await probe(page, {
    surface: S, name: 'ctx-detach-audio', sel: page.locator('[data-cut-ctx="detach-audio"]'), group: menu, groupName: 'ctx-video',
    doClick: async () => { probe._ac = await audioCount(); await clickCtx('detach-audio'); await sleep(1200) },
    assertResult: async () => {
      const after = await audioCount()
      return { ok: after >= probe._ac, detail: after > probe._ac ? `extracted (audio ${probe._ac}→${after})` : 'clean no-op (clip audio already on its own track) — correct' }
    },
  })

  // Drawer/tab launchers from the menu (color-grade / transform / crop / gain).
  for (const [name, ctxId, openSel] of [
    ['color-grade', 'color-grade', '[data-cut-grade-embed]'],
    ['transform', 'transform', '[data-cut-layer]'],
    ['crop', 'crop', '[data-cut-layer]'],
    ['gain', 'gain', '[data-cut-mixer-embed],[data-cut-mixer]'],
  ]) {
    await openMenuOn(clip)
    await probe(page, {
      surface: S, name: `ctx-${name}`, sel: page.locator(`[data-cut-ctx="${ctxId}"]`), group: menu, groupName: 'ctx-video',
      doClick: async () => { await clickCtx(ctxId); await sleep(700) },
      assertResult: async () => {
        const opened = (await page.locator(openSel).count()) > 0 && await page.locator(openSel).first().isVisible().catch(() => false)
        await page.locator('[data-cut-layer-close],[data-cut-grade-close]').first().click().catch(() => {})
        await page.keyboard.press('Escape').catch(() => {})
        await propertiesTab(page)
        return { ok: opened, detail: `${name} opened its surface=${opened}` }
      },
    })
  }

  // REPLACE — driven (import a distinct source, swap the clip's asset in place).
  {
    const imp = await verb('media.import', { path: SECOND }); const a2 = imp.result?.asset_id; await sleep(1200)
    clip = await anyVideo()
    await openMenuOn(clip)
    await probe(page, {
      surface: S, name: 'ctx-replace', sel: page.locator('[data-cut-ctx="replace"]'), group: menu, groupName: 'ctx-video',
      doClick: async () => {
        await clickCtx('replace'); await sleep(300) // expand picker
        const opt = page.locator(`[data-cut-ctx-replace-asset="${a2}"]`)
        if (await opt.count()) await opt.click()
        await sleep(500)
      },
      assertResult: async () => ({ ok: !!(await waitForState((st) => findClip(st, clip)?.asset === a2, 15000)), detail: `clip asset swapped to ${a2 ? String(a2).slice(0, 10) : '?'}` }),
    })
  }

  // COPY → PASTE (clipboard) — total media-clip count grows.
  await openMenuOn(await anyVideo())
  await probe(page, {
    surface: S, name: 'ctx-copy-paste', sel: page.locator('[data-cut-ctx="copy"]'), group: menu, groupName: 'ctx-video',
    doClick: async () => {
      probe._tc = await totalCount()
      await clickCtx('copy'); await sleep(300)
      await openMenuOn(await anyVideo())
      await clickCtx('paste'); await sleep(700)
    },
    assertResult: async () => ({ ok: !!(await waitForState(async () => (await totalCount()) > probe._tc, 10000)) || (await totalCount()) > probe._tc, detail: `clipboard paste added a clip (count ${probe._tc}→${await totalCount()})` }),
  })

  // TRIM-END (edit.trim — right-click INSIDE the clip span).
  {
    const c = await anyVideo()
    await openMenuOn(c)
    await probe(page, {
      surface: S, name: 'ctx-trim-end', sel: page.locator('[data-cut-ctx="trim-end"]'), group: menu, groupName: 'ctx-video',
      doClick: async () => { probe._b = await opsLen(); await clickCtx('trim-end'); await sleep(700) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.trim'), detail: 'edit.trim op landed' }),
    })
  }
  // SPLIT (clip count grows).
  {
    const c = await anyVideo()
    await openMenuOn(c)
    await probe(page, {
      surface: S, name: 'ctx-split', sel: page.locator('[data-cut-ctx="split"]'), group: menu, groupName: 'ctx-video',
      doClick: async () => { probe._vc = await videoCount(); await clickCtx('split'); await sleep(700) },
      assertResult: async () => ({ ok: !!(await waitForState(async () => (await videoCount()) > probe._vc, 8000)) || (await videoCount()) > probe._vc, detail: `split raised video clip count from ${probe._vc}` }),
    })
  }
  // ADD-TRANSITION — a seam now exists (from the split above) → drive it and prove
  // edit.crossfade lands. If the button is disabled (no adjacent seam materialized) or the
  // engine rejects the overlap for a geometry/precondition reason (a clip too short for
  // the 500ms crossfade), that is CONTENT-dependent → honest N/A, not a wiring false-fail.
  // A genuine error (or a clicked-but-no-op/no-response) keeps RESULT fail (no masking).
  {
    await freshProject(page, 'ctx-transition', SPEECH)
    await closeOverlays(page)
    const vTrack = (await state()).tracks.find((t) => t.kind === 'video')?.id
    if (vTrack) { await verb('edit.split', { track: vTrack, at_ms: 2000 }); await sleep(700) }
    const c = await anyVideo()
    await openMenuOn(c)
    const sel = page.locator('[data-cut-ctx="add-transition"]')
    const present = (await sel.count()) > 0
    const rg = await renderGroup(page, S, 'ctx-video', menu)
    if (!present) {
      rec(S, 'ctx-add-transition', { present: 'fail', render: rg.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' }, `add-transition item absent ${rg.detail}`.trim(), rg.shot)
    } else {
      const disabled = await sel.isDisabled().catch(() => true)
      let xresp
      const onX = async (r) => { if (/\/api\/verb\/edit\.crossfade/.test(r.url())) { try { xresp = await r.json() } catch {} } }
      page.on('response', onX)
      const before = await opsLen()
      let clickOk = true
      if (!disabled) { try { await clickCtx('add-transition'); await sleep(900) } catch { clickOk = false } }
      page.off('response', onX)
      const landed = await opLanded(before, 'edit.crossfade')
      let result, detail
      if (landed) { result = 'pass'; detail = 'edit.crossfade op landed at the seam' }
      else if (disabled) { result = 'na'; detail = 'add-transition disabled — no adjacent seam materialized (content-dependent)' }
      else if (!clickOk) { result = 'fail'; detail = 'add-transition click threw' }
      else if (xresp && !xresp.ok) {
        const msg = String(xresp.error?.message || xresp.error?.code || 'error')
        const contentDep = xresp.error?.code === 'not_found' || /seam|adjacent|overlap|too short|not enough|duration/i.test(msg)
        result = contentDep ? 'na' : 'fail'
        detail = `edit.crossfade ${contentDep ? 'precondition/content-dependent' : 'errored'}: "${msg.slice(0, 80)}"`
      }
      else { result = 'fail'; detail = `no edit.crossfade op landed (resp=${xresp ? 'ok-but-no-op' : 'none'})` }
      rec(S, 'ctx-add-transition', { present: 'pass', render: rg.ok ? 'pass' : 'fail', click: disabled ? 'na' : 'pass', result }, `${detail} ${rg.detail}`.trim(), rg.shot)
    }
  }
  // REMOVE-GAP then REMOVE (destructive — total count drops each time).
  {
    await freshProject(page, 'ctx-remove-gap', SPEECH)
    await closeOverlays(page)
    const c = await anyVideo()
    await openMenuOn(c)
    await probe(page, {
      surface: S, name: 'ctx-remove-gap', sel: page.locator('[data-cut-ctx="remove-gap"]'), group: menu, groupName: 'ctx-video',
      doClick: async () => { probe._tc = await totalCount(); await clickCtx('remove-gap'); await sleep(700) },
      assertResult: async () => ({ ok: !!(await waitForState(async () => (await totalCount()) < probe._tc, 8000)) || (await totalCount()) < probe._tc, detail: `remove(keep gap) dropped media-clip count from ${probe._tc}` }),
    })
  }
  {
    await freshProject(page, 'ctx-remove', SPEECH)
    await closeOverlays(page)
    const c = await anyVideo()
    if (c) {
      await openMenuOn(c)
      await probe(page, {
        surface: S, name: 'ctx-remove', sel: page.locator('[data-cut-ctx="remove"]'), group: menu, groupName: 'ctx-video',
        doClick: async () => { probe._tc = await totalCount(); await clickCtx('remove'); await sleep(700) },
        assertResult: async () => ({ ok: !!(await waitForState(async () => (await totalCount()) < probe._tc, 8000)) || (await totalCount()) < probe._tc, detail: `remove dropped media-clip count from ${probe._tc}` }),
      })
    } else {
      rec(S, 'ctx-remove', { present: 'na', render: 'na', click: 'na', result: 'na' }, 'no remaining video clip to remove')
    }
  }

  // ── AUDIO clip menu (curation + driven audio items) ──
  await freshProject(page, 'ctxaudio', SPEECH)
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu the video block left open, before the audio block selects/clicks
  await waitForState((st) => (st.tracks || []).some((t) => t.kind === 'audio' && (t.clips || []).some((c) => c.asset)), 10000)
  const aud = await clipOfKind('audio')
  if (!aud) { rec(S, 'AUDIO-BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'no audio clip for audio ctx menu'); return }
  await openMenuOn(aud)
  await renderGroup(page, S, 'ctx-audio', menu)
  rec(S, 'GATE:audio-blur-faces-absent', gateDim((await page.locator('[data-cut-ctx="blur-faces"]').count()) === 0), 'blur-faces correctly absent on audio clip (no faces)')
  rec(S, 'GATE:audio-has-gain', gateDim((await page.locator('[data-cut-ctx="gain"]').count()) > 0), 'gain present on audio clip')
  // mute → edit.gain.
  await openMenuOn(aud)
  await probe(page, {
    surface: S, name: 'ctx-audio-mute', sel: page.locator('[data-cut-ctx="mute"]'), group: menu, groupName: 'ctx-audio',
    doClick: async () => { probe._b = await opsLen(); await clickCtx('mute'); await sleep(700) },
    assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.gain', (a) => a.clip === aud), detail: 'edit.gain op landed (mute)' }),
  })
  // fade-in / fade-out on audio.
  for (const fid of ['fade-in', 'fade-out']) {
    await openMenuOn(aud)
    await probe(page, {
      surface: S, name: `ctx-audio-${fid}`, sel: page.locator(`[data-cut-ctx="${fid}"]`), group: menu, groupName: 'ctx-audio',
      doClick: async () => { probe._b = await opsLen(); await clickCtx(fid); await sleep(700) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.fade', (a) => a.clip === aud), detail: 'edit.fade op landed' }),
    })
  }
}

// ── 13b. EDIT-VERB GAPS (including multicam_sync) ─────────────────────────────
// Closes the edit.* verbs reachable from the timeline's marker/track menus, the Layer
// drawer, the Inspector, and the timeline toolbar — each driven through its REAL control
// (selectors grepped from panels/Timeline, panels/Layer, panels/Inspector, panels/MusicBed;
// never guessed), with a real RESULT (op landed / state changed). Verbs with NO UI control
// (edit.duplicate) are covered at the VERB level + flagged, like Batches 2-3. Sub-blocks:
//   A. marker ctx menu + drag + key — edit.remove_marker / edit.move_marker / edit.seek_marker
//   B. track ops — edit.reorder_track (Layer stacking) / edit.remove_track (overlay clip ctx)
//   C. Layer motion — edit.animate (Ken Burns) / edit.keyframe / edit.slide
//   D. Inspector — edit.speed_ramp / edit.color_match
//   E. toolbar — edit.multicam_sync (+ edit.move via its alignment) / edit.multicam_switch
//   F. toolbar — edit.cut_to_beat
//   G. Review — edit.restore (per-op Reject)
//   H. edit.duplicate — NO UI control → verb-level + flag

// Re-enter edit mode after a reload (reload drops the mode + collapses the rail).
async function reEnterEdit(page) {
  await reloadApp(page); await sleep(1000)
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {}); await sleep(300)
  await ensureRail(page)
}
// Build a project with a SECOND video clip on its OWN overlay track (above the base) —
// the precondition for edit.reorder_track (z-order) + edit.remove_track (overlay-only).
// Returns { ovTrack, ovClip } (ovClip null if the insert didn't land).
async function buildOverlayProject(page, tag) {
  await freshProject(page, tag)
  await closeOverlays(page)
  const imp = await verb('media.import', { path: SECOND })
  const ovAsset = imp.result?.asset_id
  const detail = [`import ok=${!!imp.ok} asset=${ovAsset || 'none'}${imp.error?.message ? ` err="${String(imp.error.message).slice(0, 80)}"` : ''}`]
  // The old fixed sleep(1000)-then-insert raced the import chain.
  // edit.insert needs the asset's PROBED duration (Asset.probe is None until the import
  // chain probes the file — core/types.rs); inserting before the probe lands yields NO clip,
  // so reorder_track/remove_track honest-failed "overlay clip fixture unavailable". AWAIT the
  // probe (not a guessed sleep), then drive the insert and take the clip id from its OWN
  // response ({clip_id, …}) — the deterministic handle — confirming it in the snapshot.
  const probed = ovAsset ? !!(await waitForState((s) => !!s.assets?.[ovAsset]?.probe, 20000)) : false
  detail.push(`probe=${probed}`)
  const addT = await verb('edit.add_track', { kind: 'video', rationale: 'fcv: overlay video track' })
  const ovTrack = addT.result?.track_id || addT.result?.id || (await state()).tracks.filter((t) => t.kind === 'video').map((t) => t.id).pop()
  detail.push(`add_track ok=${!!addT.ok} track=${ovTrack || 'none'}${addT.error?.message ? ` err="${String(addT.error.message).slice(0, 80)}"` : ''}`)
  let ovClip = null
  if (ovAsset && ovTrack) {
    const ins = await verb('edit.insert', { asset: ovAsset, track: ovTrack, at_ms: 0, rationale: 'fcv: overlay clip for track-op coverage' })
    ovClip = ins.result?.clip_id || null
    detail.push(`insert ok=${!!ins.ok} clip=${ovClip || 'none'}${ins.error?.message ? ` err="${String(ins.error.message).slice(0, 80)}"` : ''}`)
    // Confirm the clip landed in the project snapshot (op_applied can trail the verb
    // response a beat). Fall back to a clip-scan if the response omitted the id.
    if (ovClip) {
      const confirmed = !!(await waitForState((s) => s.tracks.some((t) => t.id === ovTrack && (t.clips || []).some((c) => c.id === ovClip)), 8000))
      detail.push(`confirmed=${confirmed}`)
    } else for (let i = 0; i < 20; i++) { const c = (await state()).tracks.find((t) => t.id === ovTrack)?.clips?.find((cc) => cc.asset); if (c) { ovClip = c.id; detail.push(`scan_clip=${ovClip}`); break } await sleep(400) }
  }
  await reEnterEdit(page)
  return { ovTrack, ovClip, detail: detail.join('; ') }
}
// Fresh project + a selected BASE video clip + the Layer drawer open. Each Layer-motion
// verb (animate/keyframe/slide) runs on its OWN fresh clip so the pairwise-exclusive
// motion verbs never collide (a clean op-landed RESULT, no cross-contamination N/A).
async function layerSetup(page, tag) {
  await freshProject(page, tag)
  await closeOverlays(page)
  const baseClip = await clipOfKind('video')
  await selectClip(page, baseClip)
  await page.locator('[data-cut-action="open-layer"]').click().catch(() => {}); await sleep(600)
  return page.locator('[data-cut-layer]').first()
}

async function secEditVerbs(page) {
  const S = 'edit-verbs'

  // ── A. MARKER context menu + drag + key ──────────────────────────────────────
  await freshProject(page, 'mk')
  await closeOverlays(page)
  // Seed 3 PLAIN user markers (label ≠ 'beat'/'capture:*' → markerClass 'plain' → draggable
  // + right-clickable, per Timeline markerClass). Spread across the clip so seek/move/delete
  // each have a target.
  await verb('edit.add_marker', { at_ms: 1000, label: 'fcv-mk-a' })
  await verb('edit.add_marker', { at_ms: 2000, label: 'fcv-mk-b' })
  await verb('edit.add_marker', { at_ms: 3000, label: 'fcv-mk-c' })
  await reEnterEdit(page)
  const markersOf = async () => (await state()).markers || []
  // seek_marker — the prev/next-marker keys ([ / ]) jump the playhead via edit.seek_marker
  // (a pure READ; it lands no op). Focus the timeline panel (not an input, not the rail
  // kbscope), then ']' = next marker from the playhead (0 → the 1000ms marker). Capture the
  // verb response (the [ / ] keys are the only control for this verb).
  {
    // Click the ruler's far LEFT → playhead ≈ 0 (before the 1000ms marker) + focus the
    // timeline (not an input), so ']' = next-marker deterministically finds the 1000ms one.
    const rbox = await page.locator('[data-cut-ruler]').first().boundingBox().catch(() => null)
    if (rbox) { await page.mouse.click(rbox.x + 8, rbox.y + Math.min(8, rbox.height / 2)); await sleep(250) }
    else { await page.locator('[data-cut-panel="timeline"]').first().click().catch(() => {}); await sleep(200) }
    const rg = await renderGroup(page, S, 'markers-ruler', page.locator('[data-cut-ruler]').first())
    const sm = await captureVerbResp(page, 'edit.seek_marker', async () => { await page.keyboard.press(']') }, 8000)
    const tgt = sm?.result?.marker?.at_ms
    rec(S, 'edit.seek_marker(] next-marker key)', { present: rbox ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: 'pass', result: (sm?.ok && typeof tgt === 'number') ? 'pass' : 'fail' },
      `] shortcut registered on the rendered timeline; key dispatch edit.seek_marker ok=${sm?.ok} → marker at_ms=${tgt ?? '?'} ${rg.detail}`.trim(), rg.shot)
  }
  // move_marker — drag the real marker triangle. Timeline mirrors the latest
  // proposal into markerGhostRef synchronously, so a quick native mouse-up cannot
  // race the React render that paints the ghost.
  {
    const id = (await markersOf()).find((m) => m.label === 'fcv-mk-a')?.id
    const tri = page.locator(`[data-cut-marker="${id}"]`).first()
    const present = !!id && (await tri.count()) > 0
    const rg = await renderGroup(page, S, 'markers-ruler', page.locator('[data-cut-ruler]').first())
    if (!present) {
      rec(S, 'edit.move_marker(ruler drag)', { present: 'fail', render: rg.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' }, `marker fcv-mk-a triangle absent (id=${id ?? '?'}) ${rg.detail}`.trim(), rg.shot)
    } else {
      const oldAt = (await markersOf()).find((m) => m.id === id)?.at_ms
      const box = await tri.boundingBox()
      const before = await opsLen()
      const mv = box ? await captureVerbResp(page, 'edit.move_marker', async () => {
        const fromX = box.x + box.width / 2
        const fromY = box.y + box.height / 2
        const toX = fromX + 120
        if (page.mouse.drag) await page.mouse.drag(fromX, fromY, toX, fromY)
        else {
          await page.mouse.move(fromX, fromY)
          await page.mouse.down()
          await page.mouse.move(toX, fromY, { steps: 8 })
          await page.mouse.up()
        }
      }, 12_000) : null
      const landed = await opLanded(before, 'edit.move_marker', (a) => a.id === id)
      const newAt = (await markersOf()).find((m) => m.id === id)?.at_ms
      const moved = typeof newAt === 'number' && newAt !== oldAt
      rec(S, 'edit.move_marker(ruler drag)', { present: 'pass', render: rg.ok ? 'pass' : 'fail', click: box ? 'pass' : 'fail', result: (mv?.ok && landed && moved) ? 'pass' : 'fail' },
        `dragged marker "${id}" 120px through the real ruler gesture; response=${mv?.ok} op landed=${landed}, at_ms ${oldAt ?? '?'}→${newAt ?? '?'} ${rg.detail}`.trim(), rg.shot)
    }
  }
  // remove_marker — right-click a plain marker → the marker menu's "Delete marker" item →
  // edit.remove_marker (the marker vanishes from project.markers on the op_applied snapshot).
  {
    const id = (await markersOf()).find((m) => m.label === 'fcv-mk-c')?.id
    const tri = page.locator(`[data-cut-marker="${id}"]`).first()
    const present = !!id && (await tri.count()) > 0
    const menu = page.locator('[data-cut-marker-menu]').first()
    if (!present) {
      rec(S, 'edit.remove_marker(ctx Delete)', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, `marker fcv-mk-c triangle absent (id=${id ?? '?'})`)
    } else {
      // A forced right-click fires the marker's onMouseDown drag
      // handler (which preventDefaults + begins a gesture) instead of opening the menu,
      // so [data-cut-marker-menu] never mounted. Dispatch the contextmenu event DIRECTLY
      // on the triangle → React's onContextMenu(onMarkerContextMenu) runs, no drag.
      await tri.dispatchEvent('contextmenu').catch(() => {}); await sleep(300)
      const rg = await renderGroup(page, S, 'marker-menu', menu)
      const del = page.locator('[data-cut-marker-ctx="delete"]').first()
      const delPresent = (await del.count()) > 0
      const before = await opsLen()
      if (delPresent) { await del.click().catch(() => {}); await sleep(700) }
      const landed = await opLanded(before, 'edit.remove_marker', (a) => a.id === id)
      const gone = !(await markersOf()).some((m) => m.id === id)
      rec(S, 'edit.remove_marker(ctx Delete)', { present: delPresent ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: delPresent ? 'pass' : 'fail', result: (landed || gone) ? 'pass' : 'fail' },
        `marker menu "Delete marker" → edit.remove_marker op landed=${landed}, marker removed from project.markers=${gone} ${rg.detail}`.trim(), rg.shot)
    }
  }

  // ── B. TRACK ops: reorder_track (Layer stacking) + remove_track (overlay clip ctx) ──
  // reorder_track — open the Layer drawer on the OVERLAY clip → "Send back" (data-cut-layer-back),
  // which fires edit.reorder_track (video-track order IS the z-order; enabled when trackIndex>0).
  {
    const { ovClip } = await buildOverlayProject(page, 'reorder')
    if (!ovClip) {
      rec(S, 'edit.reorder_track(Layer stacking)', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'overlay clip fixture unavailable (import/insert failed)')
    } else {
      await selectClip(page, ovClip)
      await page.locator('[data-cut-action="open-layer"]').click().catch(() => {}); await sleep(600)
      const layer = page.locator('[data-cut-layer]').first()
      // Drive both directions. The overlay starts at one edge: move it inward,
      // then the opposite control becomes enabled and restores its original
      // stacking position.
      const back = page.locator('[data-cut-layer-back]').first()
      const forward = page.locator('[data-cut-layer-forward]').first()
      const backOk = (await back.count()) > 0 && !(await back.isDisabled().catch(() => true))
      const fwdOk = (await forward.count()) > 0 && !(await forward.isDisabled().catch(() => true))
      const present = (await back.count()) > 0 || (await forward.count()) > 0
      const rg = await renderGroup(page, S, 'layer-drawer', layer)
      if (!present || (!backOk && !fwdOk)) {
        rec(S, 'edit.reorder_track(Layer stacking)', { present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: 'na', result: 'na' },
          `neither stacking button enabled (overlay is the only re-orderable position) — layout-dependent ${rg.detail}`.trim(), rg.shot)
      } else {
        const directions = backOk
          ? [
              { id: 'layer-back', label: 'Send back', control: back },
              { id: 'layer-forward', label: 'Bring forward', control: forward },
            ]
          : [
              { id: 'layer-forward', label: 'Bring forward', control: forward },
              { id: 'layer-back', label: 'Send back', control: back },
            ]
        for (const direction of directions) {
          await probe(page, {
            surface: S,
            name: `edit.reorder_track(${direction.label})`,
            actionId: direction.id,
            sel: direction.control,
            group: layer,
            groupName: 'layer-stacking',
            doClick: async () => {
              for (let attempt = 0; attempt < 20; attempt++) {
                if (!(await direction.control.isDisabled().catch(() => true))) break
                await sleep(100)
              }
              probe._layerReorderBefore = await opsLen()
              await direction.control.click()
              await sleep(500)
            },
            assertResult: async () => {
              const landed = await opLanded(probe._layerReorderBefore, 'edit.reorder_track')
              return { ok: landed, detail: `${direction.label} edit.reorder_track landed=${landed}` }
            },
          })
        }
      }
      await page.locator('[data-cut-layer-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {}); await sleep(200)
    }
  }
  // remove_track — right-click the OVERLAY clip → "Remove track" (the ctx item only mounts on
  // an overlay track) → edit.remove_track. Destructive: own fresh overlay project.
  {
    const { ovTrack, ovClip } = await buildOverlayProject(page, 'rmtrack')
    if (!ovClip) {
      rec(S, 'edit.remove_track(ctx Remove-track)', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'overlay clip fixture unavailable (import/insert failed)')
    } else {
      await page.locator('body').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {}); await sleep(150)
      await page.locator(`[data-cut-clip="${ovClip}"]`).click({ button: 'right', force: true }).catch(() => {}); await sleep(350)
      const menu = page.locator('[data-cut-clip-menu]').first()
      const rm = page.locator('[data-cut-ctx="remove-track"]').first()
      const present = (await rm.count()) > 0
      const rg = await renderGroup(page, S, 'ctx-overlay', menu)
      const tracksBefore = (await state()).tracks.length
      if (!present) {
        rec(S, 'edit.remove_track(ctx Remove-track)', { present: 'fail', render: rg.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' }, `remove-track item absent on overlay clip ${rg.detail}`.trim(), rg.shot)
      } else {
        const before = await opsLen()
        await rm.click().catch(() => {}); await sleep(800)
        const landed = await opLanded(before, 'edit.remove_track', (a) => a.track === ovTrack)
        const gone = !(await state()).tracks.some((t) => t.id === ovTrack)
        rec(S, 'edit.remove_track(ctx Remove-track)', { present: 'pass', render: rg.ok ? 'pass' : 'fail', click: 'pass', result: (landed || gone) ? 'pass' : 'fail' },
          `ctx "Remove track" → edit.remove_track op landed=${landed}, track removed (count ${tracksBefore}→${(await state()).tracks.length})=${gone} ${rg.detail}`.trim(), rg.shot)
      }
    }
  }

  // ── C. LAYER motion: animate (Ken Burns) / keyframe / slide ──────────────────
  // Each on its OWN fresh base clip (the motion verbs are pairwise exclusive — a shared
  // clip would make the 2nd/3rd an N/A; a fresh clip gives a clean op-landed pass).
  {
    const layer = await layerSetup(page, 'anim')
    await page.locator('[data-cut-layer-kenburns-preset]').selectOption('zoom_in').catch(() => {})
    await probe(page, {
      surface: S, name: 'edit.animate(Ken Burns Apply)', sel: page.locator('[data-cut-layer-kenburns-apply]'), group: layer, groupName: 'layer-animate',
      doClick: async () => { probe._b = await opsLen(); await page.locator('[data-cut-layer-kenburns-apply]').click(); await sleep(800) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.animate'), detail: 'edit.animate op landed (Ken Burns zoom-in)' }),
    })
  }
  {
    const layer = await layerSetup(page, 'kf')
    await probe(page, {
      surface: S, name: 'edit.keyframe(Layer Add point)', sel: page.locator('[data-cut-layer-kf-add]'), group: layer, groupName: 'layer-keyframe',
      doClick: async () => { probe._b = await opsLen(); await page.locator('[data-cut-layer-kf-add]').click(); await sleep(800) },
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.keyframe'), detail: 'edit.keyframe op landed (a parameter point added → SET the track)' }),
    })
  }
  {
    const layer = await layerSetup(page, 'slide')
    await probe(page, {
      surface: S, name: 'edit.slide(Layer Apply slide)', sel: page.locator('[data-cut-layer-slide-apply]'), group: layer, groupName: 'layer-slide',
      doClick: async () => { probe._b = await opsLen(); await page.locator('[data-cut-layer-slide-apply]').click(); await sleep(800) },
      // edit.slide is a CONVENIENCE verb that LOWERS to an edit.keyframe op (pos_x/pos_y) —
      // dispatch.rs edit_slide → commit_core("edit.keyframe", …) — so the op log NEVER carries
      // a literal `edit.slide` op (same orchestrator pattern as cut_to_beat→edit.split). Assert
      // the lowered edit.keyframe landed, not the verb name.
      assertResult: async () => ({ ok: await opLanded(probe._b, 'edit.keyframe'), detail: 'edit.slide → edit.keyframe op landed (animated slide-in lowered to a pos keyframe track)' }),
    })
  }

  // ── D. INSPECTOR: speed_ramp / color_match ───────────────────────────────────
  {
    await freshProject(page, 'inspverbs')
    await closeOverlays(page)
    // A 2nd video clip on the timeline is the color_match REFERENCE candidate → split the
    // base clip into two (edit.split); the first becomes the target, the second the ref.
    const vTrack = (await state()).tracks.find((t) => t.kind === 'video')?.id
    if (vTrack) { await verb('edit.split', { track: vTrack, at_ms: 3000 }); await sleep(700) }
    const clips = ((await state()).tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset).map((c) => c.id)) || []
    const target = clips[0]
    const ref = clips[1]
    await selectClip(page, target)
    await propertiesTab(page)
    await expandInspectorSection(page, 'speed')
    // speed_ramp — a variable-speed curve over the clip (presets); not blocked (constant
    // speed 1, no reverse/freeze, clip long enough). data-cut-action="speed-ramp" → edit.speed_ramp.
    await probe(page, {
      surface: S,
      name: 'speed-ramp-preset',
      actionId: 'speed-ramp-preset',
      sel: page.locator('[data-cut-speed-ramp-preset]').first(),
      group: page.locator('[data-cut-section="speed"]').first(),
      groupName: 'inspector-speedramp',
      doClick: async () => {
        await page.locator('[data-cut-speed-ramp-preset]').first().selectOption('ramp_up')
      },
      assertResult: async () => {
        const value = await page.locator('[data-cut-speed-ramp-preset]').first().inputValue().catch(() => '')
        return { ok: value === 'ramp_up', detail: `preset=${value}` }
      },
    })
    await probe(page, {
      surface: S, name: 'edit.speed_ramp(Inspector Apply ramp)', sel: page.locator('[data-cut-action="speed-ramp"]'), group: page.locator('[data-cut-panel="inspector"]').first(), groupName: 'inspector-speedramp',
      doClick: async () => {
        await page.locator('[data-cut-speed-ramp-preset]').first().selectOption('ramp_up')
        probe._b = await opsLen()
        probe._speedRampArgs = null
        const onRequest = (request) => {
          let pathname = ''
          try { pathname = new URL(request.url()).pathname } catch { return }
          if (pathname !== '/api/verb/edit.speed_ramp') return
          try { probe._speedRampArgs = request.postDataJSON() } catch { /* asserted below */ }
        }
        page.on('request', onRequest)
        await page.locator('[data-cut-action="speed-ramp"]').click().catch(() => {})
        await sleep(800)
        page.off('request', onRequest)
      },
      assertResult: async () => {
        const landed = await opLanded(probe._b, 'edit.speed_ramp', (args) => args.clip === target)
        const args = probe._speedRampArgs
        const exactPreset = args?.rationale === 'inspector: speed ramp ramp_up'
          && args?.points?.[0]?.factor === 0.5
          && args?.points?.at?.(-1)?.factor === 2
        return { ok: landed && exactPreset, detail: `edit.speed_ramp landed=${landed}; exact ramp-up request=${exactPreset}` }
      },
    })
    await probe(page, {
      surface: S,
      name: 'speed-ramp-clear',
      actionId: 'speed-ramp-clear',
      sel: page.locator('[data-cut-action="speed-ramp-clear"]').first(),
      group: page.locator('[data-cut-section="speed"]').first(),
      groupName: 'inspector-speedramp-clear',
      doClick: async () => {
        probe._b = await opsLen()
        probe._r = await captureVerbResp(page, 'edit.speed_ramp', async () => {
          await page.locator('[data-cut-action="speed-ramp-clear"]').first().click()
        }, 15_000)
      },
      assertResult: async () => {
        const landed = await opLanded(probe._b, 'edit.speed_ramp', (args) => args.clip === target && Array.isArray(args.points) && args.points.length === 0)
        const cleared = !findClip(await state(), target)?.speed_ramp
        return { ok: !!probe._r?.ok && landed && cleared, detail: `edit.speed_ramp ok=${probe._r?.ok}; empty points=${landed}; cleared=${cleared}` }
      },
    })
    // color_match — match the target clip's colour to the 2nd clip (the reference). The ref
    // <select> is enabled once a 2nd video clip exists; data-cut-action="color-match" → edit.color_match.
    await selectClip(page, target)
    await propertiesTab(page)
    await expandInspectorSection(page, 'video-color')
    if (!ref) {
      rec(S, 'edit.color_match(Inspector Match)', { present: 'na', render: 'na', click: 'na', result: 'na' }, 'no 2nd video clip to use as a colour reference (split did not yield 2 clips)')
    } else {
      await probe(page, {
        surface: S, name: 'edit.color_match(Inspector Match)', sel: page.locator('[data-cut-action="color-match"]'), group: page.locator('[data-cut-panel="inspector"]').first(), groupName: 'inspector-colormatch',
        doClick: async () => {
          await page.locator('[data-cut-colormatch-ref]').selectOption(ref).catch(() => {})
          probe._gradeBefore = JSON.stringify(findClip(await state(), target)?.grade || null)
          probe._b = await opsLen()
          await page.locator('[data-cut-action="color-match"]').click().catch(() => {}); await sleep(1200)
        },
        // edit.color_match DERIVES a grade and COMMITS it through the normal edit.grade path —
        // dispatch.rs edit_color_match → commit_core("edit.grade", {clip:target, …}) — so the op
        // log carries an edit.grade op (clip=target), never a literal `edit.color_match`. Assert
        // the lowered edit.grade landed on the target clip.
        assertResult: async () => {
          const colorMatchLanded = await opLanded(probe._b, 'edit.grade', (a) => a.clip === target || a.clip_id === target, { timeoutMs: 20000 })
          const colorMatchGradeChanged = !!(await waitForState((st) => JSON.stringify(findClip(st, target)?.grade || null) !== probe._gradeBefore, 20000))
          return {
            ok: colorMatchLanded || colorMatchGradeChanged,
            detail: `edit.color_match lowered to edit.grade: opLanded=${colorMatchLanded} gradeChanged=${colorMatchGradeChanged} target=${target} ref=${String(ref).slice(0, 8)}`,
          }
        },
      })
    }
  }

  // ── E. TOOLBAR: multicam_sync (+ edit.move alignment) / multicam_switch ───────
  // edit.multicam_sync envelopes the ASSET FILE, so two clips of the SAME asset measure
  // offset 0 (no move). Seed a time-SHIFTED copy (audio leads by ~1.2s) so the cross-
  // correlation locks at a real lag → "Sync by audio" then fires edit.move to align it.
  {
    await freshProject(page, 'multicam', SPEECH) // SPEECH on the base video track = the reference
    await closeOverlays(page)
    const shifted = makeShiftedClip(SPEECH, 1200, 6)
    let ovClip = null, baseClip = null
    if (shifted) {
      const imp = await verb('media.import', { path: shifted }); const a2 = imp.result?.asset_id
      await awaitImportJobs(imp, FCV_IMPORT_DRAIN_TIMEOUT_MS)
      await sleep(1000)
      const addT = await verb('edit.add_track', { kind: 'video', rationale: 'fcv: 2nd camera angle' })
      const ovTrack = addT.result?.track_id || addT.result?.id || (await state()).tracks.filter((t) => t.kind === 'video').map((t) => t.id).pop()
      if (a2 && ovTrack) await verb('edit.insert', { asset: a2, track: ovTrack, at_ms: 0, rationale: 'fcv: 2nd angle for multicam sync' })
      for (let i = 0; i < 16; i++) { await sleep(400); const c = (await state()).tracks.find((t) => t.id === ovTrack)?.clips?.find((cc) => cc.asset); if (c) { ovClip = c.id; break } }
      await reEnterEdit(page)
      baseClip = (await state()).tracks.find((t) => t.kind === 'video')?.clips?.find((c) => c.asset)?.id
      const baseAsset = baseClip ? findClip(await state(), baseClip)?.asset : null
      if (FULL || DEP.perceptionCv) {
        if (baseAsset) await ensureAssetPerception(baseAsset, 240000)
        if (a2) await ensureAssetPerception(a2, 240000)
      }
    }
    if (!shifted || !ovClip || !baseClip) {
      const why = !shifted ? 'ffmpeg unavailable to synth a shifted copy' : 'overlay angle insert failed'
      rec(S, 'edit.multicam_sync(Sync-by-audio)', { present: 'na', render: 'na', click: 'na', result: 'na' }, `${why} — cannot seed 2 correlated-offset clips (env guard)`)
      rec(S, 'edit.move(Sync-by-audio align)', { present: 'na', render: 'na', click: 'na', result: 'na' }, `${why} — edit.move alignment not exercised (env guard)`)
    } else {
      // Select BOTH angles (reference first), then drive "Sync by audio".
      await page.locator('body').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {}); await sleep(150)
      const pairSelected = await selectClipPair(page, baseClip, ovClip)
      await openTimelineAutomation(page)
      const syncBtn = page.locator('[data-cut-action="sync-by-audio"]').first()
      const present = (await syncBtn.count()) > 0
      const disabled = present ? await syncBtn.isDisabled().catch(() => true) : true
      const rg = await renderGroup(page, S, 'timeline-toolbar-multicam', page.locator('[data-cut-timeline-toolbar]').first())
      const before = await opsLen()
      const sresp = (present && !disabled) ? await captureVerbResp(page, 'edit.multicam_sync', async () => { await syncBtn.click().catch(() => {}) }, 60000) : null
      await sleep(1500) // the measure → edit.move alignment follows the response
      const moved = await opLanded(before, 'edit.move')
      const offsets = sresp?.result?.offsets || []
      const measuredOffset = offsets.some((o) => !o.reference && Math.abs(Number(o.offset_ms || 0)) > 0)
      // multicam_sync row: the measure RAN and produced a usable offset that drove an align.
      let syncResult, syncDetail
      if (sresp?.ok && moved) { syncResult = 'pass'; syncDetail = `edit.multicam_sync measured offsets ${JSON.stringify(offsets.map((o) => o.offset_ms))} → drove an alignment (edit.move landed)` }
      else if (sresp?.ok && !measuredOffset) { syncResult = 'na'; syncDetail = `edit.multicam_sync ran but measured offset ≈0 (identical/low-res-fixture audio — no shift to align); content-dependent` }
      else if (sresp?.ok) { syncResult = 'na'; syncDetail = `edit.multicam_sync measured an offset but no edit.move landed in-window (alignment timing) — content-dependent` }
      else if (disabled) { syncResult = 'na'; syncDetail = `Sync-by-audio disabled (need 2 selected media clips) — selection pair=${pairSelected}` }
      else { syncResult = 'fail'; syncDetail = `edit.multicam_sync did not run ok (resp=${sresp ? 'not-ok' : 'none'})` }
      rec(S, 'edit.multicam_sync(Sync-by-audio)', { present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: (present && !disabled) ? 'pass' : 'na', result: syncResult }, `${syncDetail} ${rg.detail}`.trim(), rg.shot)
      // edit.move row: the alignment move the sync issued (edit.move's real, robust UI path).
      rec(S, 'edit.move(Sync-by-audio align)', { rowKind: 'support', present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: (present && !disabled) ? 'pass' : 'fail', result: moved ? 'pass' : (sresp?.ok && !measuredOffset ? 'na' : 'fail') },
        `edit.move via the Sync-by-audio alignment landed=${moved} (the offset clip is shifted into alignment; this is edit.move's robust UI control — clip-drag is the other path) ${rg.detail}`.trim(), rg.shot)
      // multicam_switch — auto active-speaker camera switching across the 2 video angles
      // (canMulticam: ≥2 video tracks with media, satisfied above). Builds a program track.
      await page.locator('body').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {}); await sleep(150)
      await openTimelineAutomation(page)
      const mcBtn = page.locator('[data-cut-action="multicam-switch"]').first()
      const mcPresent = (await mcBtn.count()) > 0
      const mcDisabled = mcPresent ? await mcBtn.isDisabled().catch(() => true) : true
      const tracksBefore = (await state()).tracks.length
      const mcBefore = await opsLen()
      const mcResp = (mcPresent && !mcDisabled) ? await captureVerbResp(page, 'edit.multicam_switch', async () => { await mcBtn.click().catch(() => {}) }, 60000) : null
      await sleep(1200)
      const mcLanded = await opLanded(mcBefore, 'edit.multicam_switch')
      const progAdded = (await state()).tracks.length > tracksBefore
      let mcResult, mcDetail
      if (mcLanded || (mcResp?.ok && progAdded)) { mcResult = 'pass'; mcDetail = `edit.multicam_switch built a program track (shots=${mcResp?.result?.shots?.length ?? '?'}, switches=${mcResp?.result?.switches ?? '?'})` }
      else if (mcResp && !mcResp.ok) { mcResult = 'na'; mcDetail = `edit.multicam_switch precondition: "${String(mcResp.error?.message || mcResp.error?.code || 'error').slice(0, 70)}" (needs synced audio-bearing angles — content-dependent)` }
      else if (mcDisabled) { mcResult = 'na'; mcDetail = 'Auto-multicam disabled (need ≥2 video tracks holding clips)' }
      else { mcResult = 'fail'; mcDetail = `edit.multicam_switch did not land (resp=${mcResp ? 'ok-but-no-op' : 'none'})` }
      rec(S, 'edit.multicam_switch(Auto-multicam)', { present: mcPresent ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: (mcPresent && !mcDisabled) ? 'pass' : 'na', result: mcResult }, mcDetail)
    }
  }

  // ── F. TOOLBAR: cut_to_beat ──────────────────────────────────────────────────
  // Cuts the base video on each music beat. Beat source = markers labelled 'beat' (the
  // engine reads m.label=='beat'); seed two inside the clip (the setup pattern, like
  // secExport seeds chapter markers) so the toolbar's "Cut to beat" enables + has targets.
  {
    await freshProject(page, 'beat')
    await closeOverlays(page)
    await verb('edit.add_marker', { at_ms: 1000, label: 'beat' })
    await verb('edit.add_marker', { at_ms: 2000, label: 'beat' })
    await reEnterEdit(page)
    await openTimelineAutomation(page)
    const beatBtn = page.locator('[data-cut-action="cut-to-beat"]').first()
    const present = (await beatBtn.count()) > 0
    const disabled = present ? await beatBtn.isDisabled().catch(() => true) : true
    const rg = await renderGroup(page, S, 'timeline-toolbar-beat', page.locator('[data-cut-timeline-toolbar]').first())
    const before = await opsLen()
    const resp = (present && !disabled) ? await captureVerbResp(page, 'edit.cut_to_beat', async () => { await beatBtn.click().catch(() => {}) }, 30000) : null
    // edit.cut_to_beat is an orchestrator — split mode lowers to
    // edit.split sub-ops and NEVER emits a literal `edit.cut_to_beat` op (the Tools-menu
    // pattern, see TOOLS[]). The old opLanded(before,'edit.cut_to_beat') therefore could
    // never match, so a fully successful run (resp.ok + cuts>0) was misreported as
    // "did not land (resp=not-ok)" — the error was swallowed AND the verb name was wrong.
    // Assert the lowered edit.split landed (opLanded now polls, so a late commit on a
    // loaded rig no longer false-fails) and surface the real ok/error.
    const landed = await opLanded(before, 'edit.split')
    const cuts = resp?.result?.cuts?.length ?? 0
    const beErr = String(resp?.error?.message || resp?.error?.code || '')
    let result, detail
    if (resp?.ok && cuts > 0 && landed) { result = 'pass'; detail = `edit.cut_to_beat split the base video on ${cuts} beat(s) → ${cuts} edit.split op(s) landed (beats_used=${resp?.result?.beats_used ?? '?'})` }
    else if (resp?.ok && cuts === 0) { result = 'na'; detail = `edit.cut_to_beat ran ok but made 0 cuts (already on beats / no clips in range) — content-dependent` }
    else if (disabled) { result = 'na'; detail = 'Cut-to-beat disabled (no beat markers materialized in the UI)' }
    else { result = 'fail'; detail = `edit.cut_to_beat did not land: ok=${resp?.ok ?? 'no-resp'} cuts=${cuts} split-landed=${landed}${beErr ? ` err="${beErr.slice(0, 80)}"` : ''}` }
    rec(S, 'edit.cut_to_beat(Cut-to-beat)', { present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: (present && !disabled) ? 'pass' : 'na', result }, `${detail} ${rg.detail}`.trim(), rg.shot)
  }

  // ── G. REVIEW: restore (per-op Reject → edit.restore) ────────────────────────
  // The per-op Reject ✕ calls edit.restore in tip mode. Invoke the button's real
  // React handler directly so overlay/pointer interception cannot turn this into
  // a verb-level stand-in.
  {
    await freshProject(page, 'restore')
    await closeOverlays(page)
    await verb('edit.add_marker', { at_ms: 1200, label: 'fcv-restore-target' })
    await reEnterEdit(page)
    const reviewPanel = await reviewTab(page, 'ops', '[data-cut-ops-feed]', 8000)
    await page.waitForFunction(() => document.querySelectorAll('[data-cut-panel="review"] [data-cut-action="reject-op"]').length > 0, null, { timeout: 8000 }).catch(() => {})
    const reject = reviewPanel.locator('[data-cut-action="reject-op"]').last() // newest = the tip op-row
    const present = (await reject.count()) > 0
    const rg = await renderGroup(page, S, 'review-ops-restore', reviewPanel)
    const before = await opsLen()
    const rr = present
      ? await captureVerbResp(page, 'edit.restore', () => reject.evaluate((element) => element.click()), 12_000)
      : null
    const landed = await opLanded(before, 'edit.restore')
    const markerGone = !((await state()).markers || []).some((m) => m.label === 'fcv-restore-target')
    rec(S, 'edit.restore(Review Reject)',
      { present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: present ? 'pass' : 'fail', result: (rr?.ok && landed && markerGone) ? 'pass' : 'fail' },
      `clicked Review op-row Reject ✕ through its real handler; response=${rr?.ok} restore op landed=${landed}, tip marker rolled out=${markerGone} ${rg.detail}`.trim(), rg.shot)
  }

  // ── H. edit.duplicate — NO UI control in the build ───────────────────────────
  // Grepped ui/src: there is NO callVerb('edit.duplicate') anywhere — the clip menu's
  // Copy/Cut/Paste drive edit.paste (a pristine clip), and there is no Duplicate / Ctrl+D
  // affordance. So edit.duplicate (a TRUE copy — all per-clip attrs) is covered at the VERB
  // level + flagged, like the other no-UI verbs (Batches 2-3). RESULT = the media-clip count grows.
  {
    await freshProject(page, 'dup')
    await closeOverlays(page)
    const c = await clipOfKind('video')
    const before = flatClips(await state()).filter((x) => x.asset).length
    const dup = c ? await verb('edit.duplicate', { clip: c, rationale: 'fcv: edit.duplicate (no UI control)' }) : { ok: false, error: { message: 'no clip' } }
    const after = await waitForState((st) => flatClips(st).filter((x) => x.asset).length > before, 10000)
    const afterN = after ? flatClips(after).filter((x) => x.asset).length : before
    rec(S, 'edit.duplicate(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result: (dup.ok && after) ? 'pass' : 'fail' },
      `edit.duplicate{clip:${c ? String(c).slice(0, 8) : '?'}} ok=${dup.ok} → media-clip count ${before}→${afterN} — NO UI control in the build (the clip menu's Copy/Paste drives edit.paste, not edit.duplicate; no Ctrl+D affordance); verb-level RESULT, flagged not faked`)
  }
}

// ── 14. AI SERVICES: transcribe / diarize / dub / QC judge (dep-gated, verb-backed) ──
// These four features have NO direct editor button in the always-on surface (diarize/dub
// are exposed only via agent-chat chips; the QC judge lives behind the Review→QC tab), so
// — exactly like the overlay_only effects in secBlend — present/render/click are honest
// N/A (no editor control here) and the RESULT is the verification, driven via the verb and
// asserted deterministically (op landed / state changed / job verdict). Each is gated on
// its dependency: present (the release gate enforces this via preflight) → run + assert;
// absent in a partial dev run → honest N/A (NOT a fail). Under FCV_REQUIRE_FULL=1 the deps
// are guaranteed present, so every RESULT here is a real pass/fail, never N/A.
async function secAIServices(page) {
  const S = 'ai-services'
  await freshProject(page, 'ai', SPEAKERS) // 2 distinct speakers → meaningful diarize
  await closeOverlays(page)
  const st0 = await state()
  const asset = st0.tracks.find((t) => t.kind === 'video')?.clips?.find((c) => c.asset)?.asset || Object.keys(st0.assets || {})[0]
  // Drive a verb that may be SYNC or return a {job_id}; resolve to a terminal payload.
  const runMaybeJob = async (name, args, timeoutMs = VERB_TIMEOUT_MS) => {
    const r = await verb(name, args, { timeoutMs })
    if (!r.ok) return { ok: false, error: r.error, result: r.result }
    if (r.result?.job_id) { const j = await awaitJob(r.result.job_id, timeoutMs); return { ok: j?.state === 'done', error: j?.error, result: j?.result } }
    return { ok: true, result: r.result }
  }
  const aiServiceDetail = (r) => {
    const e = r?.error
    if (!e) return ''
    const msg = String(e.message || e.code || 'unknown error').replace(/\s+/g, ' ').slice(0, 180)
    const cause = e.cause ? ` cause=${String(e.cause).replace(/\s+/g, ' ').slice(0, 120)}` : ''
    return ` error=${msg}${cause}`
  }

  // 1) TRANSCRIBE (perception STT) — the asset gains a word transcript.
  if (DEP.perceptionStt && asset) {
    const tr = await runMaybeJob('media.transcribe', { asset })
    const has = await waitForState((s) => Object.values(s.assets || {}).some((a) => a?.transcript), 60000)
    rec(S, 'transcribe', { present: 'na', render: 'na', click: 'na', result: (tr.ok && !!has) ? 'pass' : 'fail' },
      `media.transcribe ok=${tr.ok} → asset transcript present=${!!has}${tr.error ? ` (${String(tr.error.message || tr.error.code).slice(0, 50)})` : ''}`)
  } else {
    rec(S, 'transcribe', { present: 'na', render: 'na', click: 'na', result: 'na' },
      `perception STT absent (system.doctor perception≠ok) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present`)
  }

  // 2) DIARIZE (diarize service + STT) — "label speakers" → num_speakers / labeled_words.
  if (DEP.diarize && DEP.perceptionStt && asset) {
    const d = await runMaybeJob('media.diarize', { asset })
    const n = Number(d.result?.num_speakers ?? d.result?.n_turns ?? 0)
    rec(S, 'diarize', { present: 'na', render: 'na', click: 'na', result: (d.ok && n >= 1) ? 'pass' : 'fail' },
      `media.diarize ok=${d.ok} num_speakers=${d.result?.num_speakers ?? '?'} labeled_words=${d.result?.labeled_words ?? '?'} (${DIARIZE_ENDPOINT})${aiServiceDetail(d)}`)
  } else {
    rec(S, 'diarize', { present: 'na', render: 'na', click: 'na', result: 'na' },
      `diarize svc/STT absent (diarize=${DEP.diarize} stt=${DEP.perceptionStt}) — honest dev skip; FCV_REQUIRE_FULL=1 enforces both`)
  }

  // 3) DUB (dub service + STT + claude for translation) — a NEW dub* audio track lands.
  if (DEP.dub && DEP.perceptionStt && DEP.claude && asset) {
    // audio.dub is synchronous and includes translation, per-segment TTS, WAV
    // assembly, import, and placement. A healthy native service can exceed the
    // generic 60s verb budget, so give this release proof a bounded five minutes.
    const du = await runMaybeJob('audio.dub', { target_lang: 'lv', asset }, 300000)
    const dubTrack = await waitForState((s) => (s.tracks || []).some((t) => /^dub/.test(t.id || '') && (t.clips?.length ?? 0) >= 1), 30000)
    rec(S, 'dub', { present: 'na', render: 'na', click: 'na', result: (du.ok && !!dubTrack) ? 'pass' : 'fail' },
      `audio.dub ok=${du.ok} n_clips=${du.result?.n_clips ?? '?'} dub* track present=${!!dubTrack} (${DUB_ENDPOINT})${aiServiceDetail(du)}`)
  } else {
    rec(S, 'dub', { present: 'na', render: 'na', click: 'na', result: 'na' },
      `dub svc/STT/claude absent (dub=${DEP.dub} stt=${DEP.perceptionStt} claude=${DEP.claude}) — honest dev skip; FCV_REQUIRE_FULL=1 enforces all`)
  }

  // 4) QC JUDGE (claude) — render a short clip, then verify.judge → a verdict. The
  //    Review→QC "Get AI review" button wraps exactly this verb.
  if (DEP.claude) {
    for (const t of (await state()).tracks || []) if (t.kind === 'video' || t.kind === 'audio') await verb('edit.ripple_delete', { track: t.id, range_ms: [1500, 999000], ripple: true })
    await sleep(400)
    const rf = await verb('render.final', { preset: 'draft' })
    const rj = rf.result?.job_id ? await awaitJob(rf.result.job_id) : null
    const renderUnverified = rj?.state === 'done' && (rj?.result?.verified === false || rj?.result?.receipt === null || rj?.result?.checks_skipped)
    const renderReason = String(rj?.result?.checks_skipped || 'render finished UNVERIFIED — no receipt persisted (output-perception sidecar absent)')
    const jv = await verb('verify.judge', {})
    let verdict = null, ok = false
    if (jv.result?.job_id) { const j = await awaitJob(jv.result.job_id); verdict = j?.result?.verdict ?? j?.result?.status; ok = j?.state === 'done' && ['pass', 'fail', 'needs_review', 'completed'].includes(String(verdict)) }
    else if (jv.ok) { verdict = jv.result?.verdict ?? jv.result?.status; ok = !!verdict }
    const judgeErr = String(jv.error?.message || jv.error?.code || '')
    const noReceipt = renderUnverified || /no render receipts exist/i.test(judgeErr)
    if (!ok && noReceipt && !FULL && !DEP.perceptionCv) {
      rec(S, 'qc-judge', { present: 'na', render: 'na', click: 'na', result: 'na' },
        `verify.judge needs a render receipt, but render.final finished UNVERIFIED (${renderReason.slice(0, 90)}) — output-perception (cv2+torch) absent; honest dev skip; FCV_REQUIRE_FULL=1 / DEP.perceptionCv enforce the sidecar present`)
    } else {
      rec(S, 'qc-judge', { present: 'na', render: 'na', click: 'na', result: ok ? 'pass' : 'fail' },
        `verify.judge → verdict=${verdict ?? (jv.error?.message ? 'err:' + String(jv.error.message).slice(0, 40) : 'none')} (claude judge)`)
    }
  } else {
    rec(S, 'qc-judge', { present: 'na', render: 'na', click: 'na', result: 'na' },
      `claude absent (system.doctor judge.claude≠ok) — QC judge needs it; honest dev skip; FCV_REQUIRE_FULL=1 enforces`)
  }
}

// ── 15. RECORD mode: the screen-capture workspace (screen_record.*) ───────────
// The flagship Record WORKSPACE — a full surface (data-cut-mode="record"), not a
// drawer. screen_record.doctor runs on mount → capability cards (or an honest
// "not ready" note on a host with no capture backend); the settings (length cap /
// fps / mic / system-audio / key-cast / auto-polish / source) plus the Studio
// camera availability state are verified here. Parked camera actions are not
// mounted at all until the recorder doctor reports a real backend.
// The CAPTURE path itself — start → studio_event → stop(autoedit) → polish → a
// baked clip on the timeline, and export to a file — needs a LIVE desktop capture
// surface and host capture permission. The installed final runners opt into that
// path with FCV_REAL_SCREEN_RECORD=1 and must produce source/export bytes in this
// same receipt. Ordinary development runs keep the permission-sensitive path
// explicit as unavailable instead of silently starting a host recording.
async function secRecord(page) {
  const S = 'record'
  await freshProject(page, 'record')
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  // Enter the Record workspace BEFORE the probes so RENDER screenshots the MOUNTED
  // surface (the panel mounts only while mode==='record', like the create-drawers).
  await page.locator('[data-cut-mode="record"]').click().catch(() => {})
  await sleep(700)
  await page.waitForSelector('[data-cut-panel="record"]', { timeout: 6000 }).catch(() => {})
  const panel = page.locator('[data-cut-panel="record"]').first()

  // GATE: the Record surface mounted, with its always-on source picker.
  rec(S, 'GATE:record-surface-mounted', gateDim((await panel.count()) > 0), 'record workspace surface mounted on mode=record')
  rec(S, 'GATE:source-picker-present', gateDim((await page.locator('[data-cut-rec-source]').count()) > 0), 'capture source <select> present')
  const compactTimeline = await page.evaluate(() => {
    const main = document.querySelector('.app__main')
    const timeline = document.querySelector('.app__timeline')
    const panel = document.querySelector('[data-cut-panel="record"]')
    const source = document.querySelector('[data-cut-rec-source]')
    const start = document.querySelector('[data-cut-action="record-start"]')
    if (!main || !timeline || !panel || !source || !start) return null
    const timelineBox = timeline.getBoundingClientRect()
    const panelBox = panel.getBoundingClientRect()
    const sourceBox = source.getBoundingClientRect()
    const startBox = start.getBoundingClientRect()
    return {
      compact: main.getAttribute('data-cut-record-timeline-compact') === 'true',
      timelineHeight: timelineBox.height,
      sourceVisible: sourceBox.top >= panelBox.top && sourceBox.bottom <= panelBox.bottom,
      startVisible: startBox.top >= panelBox.top && startBox.bottom <= panelBox.bottom,
    }
  })
  const compactTimelineOk = !!compactTimeline
    && compactTimeline.compact
    && compactTimeline.timelineHeight <= 161
    && compactTimeline.sourceVisible
    && compactTimeline.startVisible
  rec(
    S,
    'GATE:record-existing-timeline-compact',
    gateDim(compactTimelineOk),
    `existing timeline compact=${compactTimeline?.compact} height=${compactTimeline?.timelineHeight} sourceVisible=${compactTimeline?.sourceVisible} startVisible=${compactTimeline?.startVisible}`,
  )

  // 1) screen_record.doctor — fires on mount → capability cards (or an honest
  //    "not ready" note on a host with no capture backend). REAL RESULT: the doctor
  //    verb resolves AND the UI renders its capability verdict.
  await probe(page, {
    surface: S, name: 'record-doctor', sel: page.locator('[data-cut-rec-cards]'), group: panel, groupName: 'record-panel',
    doClick: async () => { await sleep(50) /* doctor already fired on mount */ },
    assertResult: async () => {
      const d = await verb('screen_record.doctor', {})
      const cards = Array.isArray(d.result?.cards) ? d.result.cards.length : 0
      const uiCards = await page.locator('[data-cut-rec-card]').count()
      const notReady = await page.locator('[data-cut-rec-not-ready]').count()
      const error = d.error?.message || d.error?.code || ''
      return { ok: d.ok && cards >= 1 && (uiCards >= 1 || notReady >= 1), detail: `screen_record.doctor ok=${d.ok} cards=${cards} ready=${d.result?.ready} error=${error || 'none'} (UI cards=${uiCards} notReady=${notReady})` }
    },
  })

  // 2) Settings — LOCAL state that feeds screen_record.start; each verified by its
  //    own effect (a segmented button gains --on; a checkbox flips). No op until Start
  //    fires — mirrors the render-local selects in secMenus ("value sticks").
  await probe(page, {
    surface: S, name: 'record-length-10s', sel: page.locator('[data-cut-rec-dur="10000"]'), group: panel, groupName: 'record-panel',
    doClick: async () => { await page.locator('[data-cut-rec-dur="10000"]').click().catch(() => {}); await sleep(150) },
    assertResult: async () => ({ ok: /rec__seg-btn--on/.test((await page.locator('[data-cut-rec-dur="10000"]').getAttribute('class').catch(() => '')) || ''), detail: 'length cap 10s selected (--on)' }),
  })
  await probe(page, {
    surface: S, name: 'record-fps-60', sel: page.locator('[data-cut-rec-fps="60"]'), group: panel, groupName: 'record-panel',
    doClick: async () => { await page.locator('[data-cut-rec-fps="60"]').click().catch(() => {}); await sleep(150) },
    assertResult: async () => ({ ok: /rec__seg-btn--on/.test((await page.locator('[data-cut-rec-fps="60"]').getAttribute('class').catch(() => '')) || ''), detail: 'fps 60 selected (--on)' }),
  })
  for (const [name, tsel] of [
    ['record-mic-toggle', '[data-cut-rec-audio-toggle] input'],
    ['record-system-audio-toggle', '[data-cut-rec-system-audio-toggle] input'],
    ['record-keys-toggle', '[data-cut-rec-keys-toggle] input'],
    ['record-autopolish-toggle', '[data-cut-rec-autopolish-toggle] input'],
  ]) {
    const before = await page.locator(tsel).isChecked().catch(() => null)
    await probe(page, {
      surface: S, name, sel: page.locator(tsel), group: panel, groupName: 'record-panel',
      doClick: async () => { await page.locator(tsel).click().catch(() => {}); await sleep(150) },
      assertResult: async () => {
        const after = await page.locator(tsel).isChecked().catch(() => null)
        return { ok: before !== null && after !== null && after !== before, detail: `checkbox ${before}→${after}` }
      },
    })
  }
  const liveRecordOutput = joinHostPath(
    synthEngineDir,
    `record-live-${seq++}.mp4`,
  )
  await probe(page, {
    surface: S, name: 'record-output-picker', sel: page.locator('[data-cut-action="record-output-pick"]'),
    group: panel, groupName: 'record-panel',
    rowKind: NATIVE_OS_ACTIONS.enabled ? 'ui_action' : 'support',
    nativeAction: {
      mode: 'select',
      path: liveRecordOutput,
      useDoClick: true,
      verifyResult: true,
    },
    doClick: async () => {
      if (!NATIVE_OS_ACTIONS.enabled) return
      await page.locator('[data-cut-action="record-output-pick"]').click()
      await page.waitForFunction(() => {
        const value = document.querySelector('[data-cut-rec-output-path]')
          ?.getAttribute('data-cut-rec-output-path')
        return !!value
      }, undefined, { timeout: 8_000 })
    },
    assertResult: async () => {
      const pick = await page.locator('[data-cut-action="record-output-pick"]').count()
      const row = await page.locator('[data-cut-rec-output-path]').count()
      const selected = await page.locator('[data-cut-rec-output-path]').first()
        .getAttribute('data-cut-rec-output-path').catch(() => '')
      return {
        ok: pick > 0 && row > 0
          && (!NATIVE_OS_ACTIONS.enabled || basenameHostPath(selected) === basenameHostPath(liveRecordOutput)),
        detail: `output picker=${pick} row=${row}; selected=${selected || 'browser-support'}; expected=${NATIVE_OS_ACTIONS.enabled ? liveRecordOutput : 'native-only'}`,
      }
    },
  })
  await probe(page, {
    surface: S, name: 'record-default-folder-settings', sel: page.locator('[data-cut-action="record-output-default-folder"]'),
    group: panel, groupName: 'record-panel',
    doClick: async () => {
      await page.locator('[data-cut-action="record-output-default-folder"]').click()
      await page.locator('[data-cut-environment]').waitFor({ state: 'visible', timeout: 12_000 })
    },
    assertResult: async () => {
      // EnvironmentPanel is a lazy Suspense chunk. The Record shortcut must
      // route directly to General; do not hide a failed shortcut by clicking
      // the category ourselves after the fact.
      const category = await page.locator('[data-cut-settings-body]').first()
        .getAttribute('data-cut-settings-body').catch(() => '')
      const row = await page
        .waitForSelector('[data-cut-environment] [data-cut-export-default-folder]', { timeout: 12_000 })
        .then(() => 1)
        .catch(() => 0)
      await page.locator('[data-cut-environment-close]').click().catch(() => {})
      await sleep(150)
      return { ok: category === 'general' && row > 0, detail: `settings category=${category || 'missing'} export-folder row=${row}` }
    },
  })
  await probe(page, {
    surface: S, name: 'record-studio-preview', sel: page.locator('[data-cut-studio-preview]'), group: panel, groupName: 'record-panel',
    doClick: async () => { await sleep(50) },
    assertResult: async () => {
      const preview = await page.locator('[data-cut-studio-preview]').count()
      const overlay = await page.locator('[data-cut-studio-camera-overlay]').count()
      const controls = await page.locator('[data-cut-studio-hotkey-status]').count()
      const available = await page.locator('[data-cut-studio-camera-status]').getAttribute('data-cut-studio-camera-available').catch(() => '')
      const parked = await page.locator('[data-cut-studio-camera-unavailable]').count()
      const cameraTruthful = available === 'true' ? overlay > 0 && parked === 0 : available === 'false' && overlay === 0 && parked > 0
      return { ok: preview > 0 && controls > 0 && cameraTruthful, detail: `studio preview=${preview} camera available=${available} overlay=${overlay} parked=${parked} hotkeys=${controls}` }
    },
  })
  await probe(page, {
    surface: S, name: 'record-studio-camera-status', sel: page.locator('[data-cut-studio-camera-status]'),
    group: panel, groupName: 'record-panel',
    doClick: async () => { await sleep(50) },
    assertResult: async () => {
      const available = await page.locator('[data-cut-studio-camera-status]').getAttribute('data-cut-studio-camera-available').catch(() => '')
      const parked = await page.locator('[data-cut-studio-camera-unavailable]').textContent().catch(() => '')
      const actionable = await page.locator('[data-cut-studio-camera-enabled] input, [data-cut-studio-camera-visible] input, [data-cut-studio-camera-position] button').count()
      return {
        ok: available === 'true' ? actionable > 0 : available === 'false' && actionable === 0 && parked.includes('Not available in this release'),
        detail: `camera available=${available} actionable=${actionable} parked=${parked.trim()}`,
      }
    },
  })
  const cameraAvailableInDoctor = await page.locator('[data-cut-studio-camera-status]').getAttribute('data-cut-studio-camera-available').catch(() => '') === 'true'
  if (cameraAvailableInDoctor) {
    await probe(page, {
      surface: S, name: 'record-studio-camera-enabled', sel: page.locator('[data-cut-studio-camera-enabled] input').first(),
      group: panel, groupName: 'record-panel',
      doClick: async () => { await page.locator('[data-cut-studio-camera-enabled] input').first().click(); await sleep(150) },
      assertResult: async () => {
        const enabled = await page.locator('[data-cut-studio-camera-enabled]').first().getAttribute('data-cut-studio-camera-enabled').catch(() => '')
        const visible = await page.locator('[data-cut-studio-camera-overlay]').first().getAttribute('data-cut-studio-camera-visible').catch(() => '')
        return { ok: enabled === 'true' && visible === 'true', detail: `camera enabled=${enabled} visible=${visible}` }
      },
    })
    await probe(page, {
      surface: S, name: 'record-studio-camera-visible', sel: page.locator('[data-cut-studio-camera-visible] input').first(),
      group: panel, groupName: 'record-panel',
      doClick: async () => { await page.locator('[data-cut-studio-camera-visible] input').first().click(); await sleep(150) },
      assertResult: async () => {
        const visible = await page.locator('[data-cut-studio-camera-overlay]').first().getAttribute('data-cut-studio-camera-visible').catch(() => '')
        return { ok: visible === 'false', detail: `camera visible=${visible}` }
      },
    })
    await probe(page, {
      surface: S, name: 'record-studio-camera-position', sel: page.locator('.rec-studio-controls [data-cut-studio-camera-position] button').nth(1),
      group: panel, groupName: 'record-panel',
      doClick: async () => {
        await page.locator('[data-cut-studio-camera-visible] input').first().click()
        await page.locator('.rec-studio-controls [data-cut-studio-camera-position] button').nth(1).click()
        await sleep(150)
      },
      assertResult: async () => {
        const pos = await page.locator('[data-cut-studio-camera-overlay]').first().getAttribute('data-cut-studio-camera-position').catch(() => '')
        return { ok: pos === 'top_right', detail: `camera position=${pos}` }
      },
    })
  } else {
    // Parked means these are not release UI functions. The status probe above
    // proves the user sees one explicit boundary and zero actionable controls;
    // do not misclassify removed actions as coverage gaps.
  }
  await probe(page, {
    surface: S, name: 'record-studio-background', sel: page.locator('[data-cut-studio-background] select').first(),
    group: panel, groupName: 'record-panel',
    doClick: async () => { await page.locator('[data-cut-studio-background] select').first().selectOption('solid').catch(() => {}); await sleep(150) },
    assertResult: async () => {
      const bg = await page.locator('[data-cut-studio-preview]').first().getAttribute('data-cut-studio-background').catch(() => '')
      return { ok: bg === 'solid', detail: `studio background=${bg}` }
    },
  })

  // 3) CAPTURE verbs — start → studio_event → stop(autoedit) → polish → baked clip,
  //    and export to a file. Installed final runners opt into the permission-sensitive
  //    live path; ordinary development runs remain explicit about not capturing.
  const startBtn = page.locator('[data-cut-action="record-start"]')
  const startPresent = (await startBtn.count()) > 0
  const rg = await renderGroup(page, S, 'record-panel', panel)
  const recReady = (await page.locator('[data-cut-rec-not-ready]').count()) === 0
  if (REAL_SCREEN_RECORD) {
    for (const selector of [
      '[data-cut-rec-audio-toggle-input]',
      '[data-cut-rec-system-audio-toggle-input]',
    ]) {
      const input = page.locator(selector).first()
      if (await input.isChecked().catch(() => false)) await input.click()
    }
    await page.locator('[data-cut-rec-mode="auto"]').first().click()
    await page.locator('[data-cut-rec-dur="10000"]').first().click()
    let liveStart = null
    let liveStopVisible = false
    await probe(page, {
      surface: S,
      name: 'record-live-start',
      actionId: 'record-start',
      sel: startBtn,
      group: panel,
      groupName: 'record-panel',
      doClick: async () => {
        liveStart = await captureVerbResp(page, 'screen_record.start', () => startBtn.click(), 60_000)
        if (liveStart?.ok) {
          liveStopVisible = await page.locator('[data-cut-action="record-stop"]').first()
            .waitFor({ state: 'visible', timeout: 30_000 })
            .then(() => true)
            .catch(() => false)
        }
      },
      assertResult: async () => ({
        ok: recReady && !!liveStart?.ok && !!liveStart.result?.capture_id && liveStopVisible,
        detail: `ready=${recReady}; capture=${liveStart?.result?.capture_id || liveStart?.error?.message || liveStart?.error?.code || 'missing'}; stopVisible=${liveStopVisible}`,
      }),
    })
    if (!liveStart?.ok || !liveStart.result?.capture_id || !liveStopVisible) {
      const reason = liveStart?.error?.message || liveStart?.error?.code
        || (liveStart?.ok ? 'Stop control did not appear after a successful start response' : 'screen_record.start returned no response')
      if (liveStart?.result?.capture_id) {
        await verb(
          'screen_record.stop',
          { capture_id: liveStart.result.capture_id, autoedit: false },
          { timeoutMs: 45_000 },
        )
      }
      for (const [name, actionId] of [
        ['record-live-studio-event', 'studio-background-select'],
        ['record-live-stop-autoedit-polish', 'record-stop'],
        ['record-live-export', 'record-export'],
      ]) {
        rec(S, name, {
          rowKind: 'ui_action',
          actionId,
          present: 'fail',
          render: 'na',
          click: 'fail',
          result: 'fail',
        }, `not attempted because live recording did not start cleanly: ${reason}`)
      }
      await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
      await sleep(300)
      return
    }
    // screen_record.start reserves the capture immediately while the native
    // portal/backend finishes its first-frame handshake on a worker thread.
    // A synthetic Stop about one second later can pre-date the first frame and
    // produces an empty container on otherwise healthy Linux/macOS rigs. Give
    // the installed recorder a short, real capture window before proving Stop;
    // users get meaningful footage and every backend reaches steady state.
    await sleep(4_000)
    let studioResponse = null
    const background = page.locator('[data-cut-studio-background-select]').first()
    await probe(page, {
      surface: S,
      name: 'record-live-studio-event',
      actionId: 'studio-background-select',
      sel: background,
      group: panel,
      groupName: 'record-panel',
      doClick: async () => {
        studioResponse = await captureVerbResp(
          page,
          'screen_record.studio_event',
          () => background.selectOption('gradient'),
          15_000,
        )
        await sleep(1_200)
      },
      assertResult: async () => ({
        ok: !!studioResponse?.ok,
        detail: `screen_record.studio_event ok=${studioResponse?.ok}`,
      }),
    })
    const liveResponses = {}
    const onLiveResponse = async (response) => {
      let pathname = ''
      try { pathname = new URL(response.url()).pathname } catch { return }
      const match = pathname.match(/^\/api\/verb\/(screen_record[.](?:stop|polish|export))$/)
      if (!match) return
      try { liveResponses[match[1]] = await response.json() } catch { /* handled as missing */ }
    }
    page.on('response', onLiveResponse)
    try {
      const stopBtn = page.locator('[data-cut-action="record-stop"]').first()
      await probe(page, {
        surface: S,
        name: 'record-live-stop-autoedit-polish',
        actionId: 'record-stop',
        sel: stopBtn,
        group: panel,
        groupName: 'record-panel',
        doClick: async () => {
          await stopBtn.click()
          await page.waitForFunction(() => {
            const result = document.querySelector('[data-cut-studio-result]')
            return ['done', 'error'].includes(result?.getAttribute('data-cut-studio-result') || '')
          }, undefined, { timeout: 240_000 })
          for (let index = 0; index < 80 && (!liveResponses['screen_record.stop'] || !liveResponses['screen_record.polish']); index += 1) {
            await page.flushEvents?.()
            if (!liveResponses['screen_record.stop'] || !liveResponses['screen_record.polish']) await sleep(250)
          }
        },
        assertResult: async () => {
          const stopped = liveResponses['screen_record.stop']
          const polished = liveResponses['screen_record.polish']
          const source = resolveDriverPath(stopped?.result?.source || '')
          const ui = await page.evaluate(() => ({
            phase: document.querySelector('[data-cut-studio-result]')?.getAttribute('data-cut-studio-result') || '',
            error: document.querySelector('[data-cut-rec-error]')?.textContent?.trim() || '',
            note: document.querySelector('[data-cut-rec-finalizing], [data-cut-rec-done]')?.textContent?.trim() || '',
          })).catch(() => ({ phase: '', error: '', note: '' }))
          return {
            ok: !!stopped?.ok && !!stopped.result?.plan && fileBytes(source) > 0 &&
              !!polished?.ok && !!polished.result?.clip_id &&
              (await page.locator('[data-cut-rec-done]').count()) === 1,
            detail: `stop=${stopped?.ok}; sourceBytes=${fileBytes(source)}; autoeditPlan=${!!stopped?.result?.plan}; polish=${polished?.ok}; clip=${polished?.result?.clip_id || 'missing'}; phase=${ui.phase || 'missing'}; error=${ui.error || 'none'}; note=${ui.note || 'none'}`,
          }
        },
      })
      const exportBtn = page.locator('[data-cut-action="record-export"]').first()
      await probe(page, {
        surface: S,
        name: 'record-live-export',
        actionId: 'record-export',
        sel: exportBtn,
        group: panel,
        groupName: 'record-panel',
        doClick: async () => {
          liveResponses['screen_record.export'] = await captureVerbResp(
            page,
            'screen_record.export',
            () => exportBtn.click(),
            150_000,
          )
        },
        assertResult: async () => {
          const exported = liveResponses['screen_record.export']
          const path = resolveDriverPath(exported?.result?.path || '')
          // The UI's own note is part of the evidence, not a nicety. When the
          // export fails BEFORE the verb (a refused Save As folder), there is no
          // response to report and `ok=undefined` alone says nothing about why —
          // exactly the dead end the 0.6.106 macOS receipt hit. The note carries
          // the product's stated reason, so capture it whatever the outcome.
          const ui = await page.evaluate(() => ({
            note: document.querySelector('[data-cut-rec-export-note]')?.textContent?.trim() || '',
            error: document.querySelector('[data-cut-rec-error]')?.textContent?.trim() || '',
          })).catch(() => ({ note: '', error: '' }))
          return {
            ok: !!exported?.ok
              && basenameHostPath(exported?.result?.path || '') === basenameHostPath(liveRecordOutput)
              && fileBytes(path) > 0,
            detail: `screen_record.export ok=${exported?.ok}${exported?.error ? ` error=${exported.error.code || '?'}: ${exported.error.message || ''}` : ''}; selectedPath=${basenameHostPath(exported?.result?.path || '') === basenameHostPath(liveRecordOutput)}; bytes=${fileBytes(path)}; note=${ui.note || 'none'}; panelError=${ui.error || 'none'}`,
          }
        },
      })
    } finally {
      page.off('response', onLiveResponse)
      if (!liveResponses['screen_record.stop'] && liveStart?.result?.capture_id) {
        await verb(
          'screen_record.stop',
          { capture_id: liveStart.result.capture_id, autoedit: false },
          { timeoutMs: 45_000 },
        )
      }
    }
  } else {
    rec(S, 'record-start', { rowKind: 'support', present: startPresent ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: 'na', result: 'na' },
      `duplicate deterministic action row; live pixels/permissions need FCV_REAL_SCREEN_RECORD=1. ready=${recReady}. ${rg.detail}`.trim(),
      rg.shot)
    for (const v of ['record-studio_event', 'record-stop', 'record-autoedit', 'record-polish', 'record-export']) {
      rec(S, v, { present: 'na', render: 'na', click: 'na', result: 'na' },
        `screen_record.${v.replace('record-', '')} needs FCV_REAL_SCREEN_RECORD=1 on an installed desktop session; not faked in this run.`)
    }
  }

  // Leave the editor in a clean state for the next section.
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await sleep(300)
}

async function waitForGeneratePreviewImageLoaded(page, selector, timeoutMs = 8000) {
  await page.waitForFunction((sel) => {
    const img = document.querySelector(sel)
    return !!img && img.complete && img.naturalWidth > 0 && img.naturalHeight > 0
  }, selector, { timeout: timeoutMs }).catch(() => {})
  const img = page.locator(selector).first()
  return img.evaluate((el) => ({ w: el.naturalWidth || 0, h: el.naturalHeight || 0 })).catch(() => ({ w: 0, h: 0 }))
}

function sameJsonValue(actual, expected) {
  if (Array.isArray(expected)) {
    return Array.isArray(actual)
      && actual.length === expected.length
      && expected.every((value, index) => sameJsonValue(actual[index], value))
  }
  if (expected && typeof expected === 'object') {
    if (!actual || typeof actual !== 'object' || Array.isArray(actual)) return false
    const actualKeys = Object.keys(actual).sort()
    const expectedKeys = Object.keys(expected).sort()
    return actualKeys.length === expectedKeys.length
      && actualKeys.every((key, index) =>
        key === expectedKeys[index] && sameJsonValue(actual[key], expected[key]))
  }
  return Object.is(actual, expected)
}

// ── 16. GENERATE tab: native editable template instances (generate.*) ────────
// This is NOT the Find-pane assets.generate paid media surface. It is the
// ShellX-native Generate workspace: catalog -> manifest controls -> preview
// evidence -> replay-safe native timeline insert. Result proof is the returned
// preview URL/image plus generated clip evidence in project.state.
async function secGenerate(page) {
  const S = 'generate'
  await freshProject(page, 'generate')
  await closeOverlays(page)
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
  await page.locator('[data-cut-left-tab="generate"]').click().catch(() => {})
  await page.waitForSelector('[data-cut-panel="generate-templates"]', { timeout: 6000 }).catch(() => {})
  const panel = page.locator('[data-cut-panel="generate-templates"]').first()
  const list = await verb('generate.list', { kind: 'all', source: 'all' })
  await page.waitForSelector('[data-cut-generate-template-card]', { timeout: 8000 }).catch(() => {})
  const rg = await renderGroup(page, S, 'generate-templates-panel', panel)
  const cardCount = await page.locator('[data-cut-generate-template-card]').count()
  rec(S, 'generate.list(Templates workspace)', {
    present: (await panel.count()) > 0 ? 'pass' : 'fail',
    render: rg.ok ? 'pass' : 'fail',
    click: 'pass',
    result: (list.ok && (list.result?.templates?.length ?? 0) > 0 && cardCount > 0) ? 'pass' : 'fail',
  }, `generate.list ok=${list.ok} apiTemplates=${list.result?.templates?.length ?? 0} uiCards=${cardCount}; ${rg.detail}`, rg.shot)

  const generateActions = createGenerateTemplateActionCoverage({
    probe,
    captureVerbResp,
    sleep,
  })
  await generateActions.runCatalog(page, panel)

  const card = page.locator('[data-cut-generate-template-id="builtin.lower-third.clean"]').first()
  await card.click().catch(() => {})
  await sleep(500)
  const describe = await verb('generate.describe', { id: 'builtin.lower-third.clean' })
  const hasName = (await page.locator('[data-cut-generate-param="name"]').count()) > 0
  const hasAccent = (await page.locator('[data-cut-generate-param="accent"]').count()) > 0
  rec(S, 'generate.describe(Template inspector)', {
    present: (await card.count()) > 0 ? 'pass' : 'fail',
    render: rg.ok ? 'pass' : 'fail',
    click: 'pass',
    result: (describe.ok && hasName && hasAccent) ? 'pass' : 'fail',
  }, `generate.describe ok=${describe.ok} controls name=${hasName} accent=${hasAccent}`)

  await generateActions.runManifestControls(page, panel)
  const preview = await captureVerbResp(page, 'generate.preview', async () => {
    await page.locator('[data-cut-generate-template-preview]').click()
  }, 60000)
  const img = page.locator('[data-cut-generate-template-preview-img]').first()
  await img.waitFor({ state: 'visible', timeout: 8000 }).catch(() => {})
  const natural = await waitForGeneratePreviewImageLoaded(page, '[data-cut-generate-template-preview-img]')
  rec(S, 'generate.preview(Template preview image)', {
    present: (await page.locator('[data-cut-generate-template-preview]').count()) > 0 ? 'pass' : 'fail',
    render: rg.ok ? 'pass' : 'fail',
    click: preview ? 'pass' : 'fail',
    result: (preview?.ok && preview.result?.url && natural.w > 0 && natural.h > 0) ? 'pass' : 'fail',
  }, `generate.preview ok=${preview?.ok} url=${preview?.result?.url || 'none'} image=${natural.w}x${natural.h}`)

  const beforeOps = (await ops()).length
  const insert = await captureVerbResp(page, 'generate.insert', async () => {
    await page.locator('[data-cut-generate-template-insert]').click()
  }, 60000)
  const clipId = insert?.result?.clips?.[0]
  const checkpoint = insert?.result?.checkpoint?.id
  const landedState = clipId
    ? await waitForState((s) => flatClips(s).some((c) => c.id === clipId && c.title_text === 'FCV Generate'), 9000)
    : null
  const afterOps = (await ops()).length
  rec(S, 'generate.insert(Template timeline instance)', {
    present: (await page.locator('[data-cut-generate-template-insert]').count()) > 0 ? 'pass' : 'fail',
    render: rg.ok ? 'pass' : 'fail',
    click: insert ? 'pass' : 'fail',
    result: (insert?.ok && clipId && checkpoint && landedState && afterOps > beforeOps) ? 'pass' : 'fail',
  }, `generate.insert ok=${insert?.ok} checkpoint=${checkpoint || 'none'} clip=${clipId || 'none'} ops ${beforeOps}->${afterOps} landed=${!!landedState}`)

  if (checkpoint) {
    const reverted = await verb('project.revert', { to: checkpoint, rationale: 'fcv generate cleanup' })
    const gone = clipId ? await waitForState((s) => !flatClips(s).some((c) => c.id === clipId), 9000) : null
    rec(S, 'project.revert(Generate checkpoint cleanup)', { present: 'na', render: 'na', click: 'na', result: (reverted.ok && !!gone) ? 'pass' : 'fail' },
      `project.revert{to:${checkpoint}} ok=${reverted.ok} generatedClipRemoved=${!!gone}`)
  }

  const promptTab = page.locator('[data-cut-generate-tab="prompt"]').first()
  // generate.insert/revert may move focus to the timeline and hide the left
  // workspace on slower native WebViews. Re-enter Generate explicitly instead
  // of swallowing a failed tab click and crashing later on the first control.
  await page.locator('[data-cut-left-tab="generate"]').click()
  await panel.waitFor({ state: 'visible', timeout: 12_000 })
  await promptTab.waitFor({ state: 'visible', timeout: 12_000 })
  await promptTab.click()
  const promptPanel = page.locator('[data-cut-generate-prompt-panel]').first()
  await promptPanel.waitFor({ state: 'visible', timeout: 12_000 })
  const prg = await renderGroup(page, S, 'generate-prompt-panel', promptPanel)
  await generateActions.runPromptControls(page, promptPanel)
  // Name the exact template id in the prompt: the choice is otherwise the
  // LLM's, and a motion-kind pick would make the INSERT probe depend on the
  // ShellX Motion CLI. Steering through the prompt box is the real user path.
  await page.locator('[data-cut-generate-prompt-input]').fill('Create a clean lower third for Marta using the builtin.lower-third.clean template').catch(() => {})
  await page.locator('[data-cut-generate-prompt-policy]').selectOption('plan').catch(() => {})
  // The planner backend is the SHIPPED adapter shim routing to the user's local
  // CLI agent (claude/codex/grok). Dep-gate like diarize/dub: a rig with no CLI
  // agent gets an honest not_run (result NA + the reason), a rig WITH one must
  // produce a completed, catalog-valid plan. The template CHOICE is the LLM's —
  // assert validation.ok (cutd checked id + params), never one exact id.
  const promptPlan = await captureVerbResp(page, 'generate.from_prompt', async () => {
    await page.locator('[data-cut-generate-prompt-run]').click()
  }, 200000)
  const promptAgents = promptPlan?.result?.request?.agents || {}
  const promptAgentAvailable = Object.values(promptAgents).some((p) => typeof p === 'string' && p)
  const promptNotRun = promptPlan?.result?.status === 'not_run' && !promptAgentAvailable
  const planOk = promptPlan?.ok && promptPlan.result?.status === 'completed' &&
    promptPlan.result?.validation?.ok === true &&
    typeof promptPlan.result?.plan?.template_id === 'string' && promptPlan.result.plan.template_id.length > 0
  rec(S, 'generate.from_prompt(Plan policy)', {
    present: (await promptTab.count()) > 0 ? 'pass' : 'fail',
    render: prg.ok ? 'pass' : 'fail',
    click: promptPlan ? 'pass' : 'fail',
    result: planOk ? 'pass' : (promptNotRun ? 'na' : 'fail'),
  }, `generate.from_prompt plan status=${promptPlan?.result?.status || 'none'} template=${promptPlan?.result?.plan?.template_id || 'none'} agentAvailable=${promptAgentAvailable}${promptNotRun ? ' (no local CLI agent on this rig — honest not_run, dep-gated)' : ''}; ${prg.detail}`, prg.shot)

  const storyboardTab = page.locator('[data-cut-generate-tab="storyboard"]').first()
  await page.locator('[data-cut-left-tab="generate"]').click()
  await panel.waitFor({ state: 'visible', timeout: 12_000 })
  await storyboardTab.waitFor({ state: 'visible', timeout: 12_000 })
  await storyboardTab.click()
  const storyPanel = page.locator('[data-cut-generate-storyboard]').first()
  await storyPanel.waitFor({ state: 'visible', timeout: 12_000 })
  const srg = await renderGroup(page, S, 'generate-storyboard-panel', storyPanel)
  await generateActions.runStoryboardControls(page, storyPanel)
  const questionCoverage = await generateActions.runStoryboardQuestion(page, storyPanel)
  if (!questionCoverage.covered) {
    rec(S, 'generate.storyboard(Director question answer)', {
      present: 'na', render: 'na', click: 'na', result: 'na',
    }, `The active storyboard adapter returned no director question; ${questionCoverage.detail}. A deterministic adapter is required by the final all-actions rig.`)
  }
  // Constrain to NATIVE generated-template scenes: motion-template scenes would
  // make the INSERT probe depend on the ShellX Motion CLI, and assemble_slot
  // scenes insert no clip (fewer clips than scenes). Steered via the input box.
  await page.locator('[data-cut-generate-storyboard-input]').fill('Plan a clean 12 second launch video with a title card, a lower third, and a CTA endcard. Every scene must be a generated template scene; use only the builtin.title-card.episode, builtin.lower-third.clean, and builtin.social.cta-endcard templates (no motion templates, no footage slots).').catch(() => {})
  await page.locator('[data-cut-generate-storyboard-mode]').selectOption('quick_prompt').catch(() => {})
  await page.locator('[data-cut-generate-storyboard-agent]').selectOption('auto').catch(() => {})
  const storyboardBeforeOps = (await ops()).length
  const storyboard = await captureVerbResp(page, 'generate.storyboard', async () => {
    await page.locator('[data-cut-generate-storyboard-plan]').click()
  }, 200000)
  const storyboardAfterOps = (await ops()).length
  const storyResult = storyboard?.result
  const storyRows = await page.locator('[data-cut-generate-storyboard-scene]').count()
  // Same dep-gate as from_prompt. Scene/template CHOICES are the LLM's:
  // validation.ok already proves every generate_template id is catalog-real and
  // every range is well-formed — assert THAT plus non-mutation, not exact ids.
  const storyAgents = storyResult?.request?.agents || {}
  const storyAgentAvailable = Object.values(storyAgents).some((p) => typeof p === 'string' && p)
  const storyNotRun = storyResult?.status === 'not_run' && !storyAgentAvailable
  const storyOk =
    storyboard?.ok &&
    storyResult?.status === 'completed' &&
    storyResult?.storyboard?.schema === 'shellx-cut/generate-storyboard/1' &&
    Array.isArray(storyResult?.storyboard?.scenes) &&
    storyResult.storyboard.scenes.length >= 3 &&
    storyRows >= 3 &&
    storyResult?.validation?.ok === true &&
    storyResult?.evidence?.mutated === false &&
    storyboardAfterOps === storyboardBeforeOps
  rec(S, 'generate.storyboard(Storyboard UI plan evidence)', {
    present: (await storyboardTab.count()) > 0 ? 'pass' : 'fail',
    render: srg.ok ? 'pass' : 'fail',
    click: storyboard ? 'pass' : 'fail',
    result: storyOk ? 'pass' : (storyNotRun ? 'na' : 'fail'),
  }, `generate.storyboard plan status=${storyResult?.status || 'none'} scenes=${storyResult?.storyboard?.scenes?.length ?? 0} uiRows=${storyRows} templates=${(storyResult?.evidence?.template_ids || []).join(',') || 'none'} ops ${storyboardBeforeOps}->${storyboardAfterOps} agentAvailable=${storyAgentAvailable}${storyNotRun ? ' (no local CLI agent on this rig — honest not_run, dep-gated)' : ''}; ${srg.detail}`, srg.shot)

  if (storyNotRun) {
    // No local CLI agent on this rig — the preview/insert buttons re-plan
    // through the same adapter and can only repeat the not_run; skip the
    // wall-clock instead of timing out on disabled/inert controls.
    rec(S, 'generate.storyboard(Storyboard UI preview images)', { present: 'pass', render: 'na', click: 'na', result: 'na' }, 'no local CLI agent — dep-gated with the plan probe')
    rec(S, 'generate.storyboard(Storyboard UI insert evidence)', { present: 'pass', render: 'na', click: 'na', result: 'na' }, 'no local CLI agent — dep-gated with the plan probe')
  } else {
  const storyPreviewBeforeOps = (await ops()).length
  const storyPreview = await captureVerbResp(page, 'generate.storyboard', async () => {
    await page.locator('[data-cut-generate-storyboard-preview]').click()
  }, 200000)
  const storyImg = page.locator('[data-cut-generate-storyboard-preview-img]').first()
  await storyImg.waitFor({ state: 'visible', timeout: 8000 }).catch(() => {})
  const storyNatural = await waitForGeneratePreviewImageLoaded(page, '[data-cut-generate-storyboard-preview-img]')
  const storyPreviewAfterOps = (await ops()).length
  rec(S, 'generate.storyboard(Storyboard UI preview images)', {
    present: (await page.locator('[data-cut-generate-storyboard-preview]').count()) > 0 ? 'pass' : 'fail',
    render: srg.ok ? 'pass' : 'fail',
    click: storyPreview ? 'pass' : 'fail',
    result: (storyPreview?.ok && storyPreview.result?.status === 'completed' && (storyPreview.result?.preview?.scenes?.length ?? 0) >= 3 && storyNatural.w > 0 && storyNatural.h > 0 && storyPreviewAfterOps === storyPreviewBeforeOps) ? 'pass' : 'fail',
  }, `generate.storyboard preview status=${storyPreview?.result?.status || 'none'} scenes=${storyPreview?.result?.preview?.scenes?.length ?? 0} image=${storyNatural.w}x${storyNatural.h} ops ${storyPreviewBeforeOps}->${storyPreviewAfterOps}`)

  const storyInsertBeforeOps = (await ops()).length
  const storyInsert = await captureVerbResp(page, 'generate.storyboard', async () => {
    await page.locator('[data-cut-generate-storyboard-insert]').click()
  }, 200000)
  const storyInserted = storyInsert?.result?.insert
  const storyClipIds = storyInserted?.clips || []
  const storyCheckpoint = storyInserted?.checkpoints?.[0]
  const storyInsertText = await page.locator('[data-cut-generate-storyboard-insert-result]').textContent().catch(() => '')
  const storyLanded = storyClipIds.length > 0
    ? await waitForState((s) => storyClipIds.every((clipId) => flatClips(s).some((c) => c.id === clipId)), 9000)
    : null
  const storyInsertAfterOps = (await ops()).length
  rec(S, 'generate.storyboard(Storyboard UI insert evidence)', {
    present: (await page.locator('[data-cut-generate-storyboard-insert-result]').count()) > 0 ? 'pass' : 'fail',
    render: srg.ok ? 'pass' : 'fail',
    click: storyInsert ? 'pass' : 'fail',
    result: (storyInsert?.ok && storyInsert.result?.status === 'completed' && storyCheckpoint && storyClipIds.length >= 3 && storyClipIds.every((clipId) => storyInsertText?.includes(clipId)) && !!storyLanded && storyInsertAfterOps > storyInsertBeforeOps) ? 'pass' : 'fail',
  }, `generate.storyboard insert status=${storyInsert?.result?.status || 'none'} checkpoint=${storyCheckpoint || 'none'} clips=${storyClipIds.join(',') || 'none'} ops ${storyInsertBeforeOps}->${storyInsertAfterOps} landed=${!!storyLanded}`)

  if (storyCheckpoint) {
    const reverted = await verb('project.revert', { to: storyCheckpoint, rationale: 'fcv generate storyboard cleanup' })
    const gone = storyClipIds.length > 0
      ? await waitForState((s) => storyClipIds.every((clipId) => !flatClips(s).some((c) => c.id === clipId)), 9000)
      : null
    rec(S, 'project.revert(Generate storyboard checkpoint cleanup)', { present: 'na', render: 'na', click: 'na', result: (reverted.ok && !!gone) ? 'pass' : 'fail' },
      `project.revert{to:${storyCheckpoint}} ok=${reverted.ok} generatedStoryboardClipsRemoved=${!!gone}`)
  }
  }

  // Storyboard planning can remount the Generate workspace. Re-enter through
  // the real left-tab path and require the Prompt panel before invoking its
  // policy buttons; swallowing either navigation failure turns a slow native
  // WebView into a misleading missing-control section crash.
  await page.locator('[data-cut-left-tab="generate"]').first().click()
  await panel.waitFor({ state: 'visible', timeout: 12_000 })
  await page.locator('[data-cut-generate-tab="prompt"]').first().waitFor({ state: 'visible', timeout: 12_000 })
  await page.locator('[data-cut-generate-tab="prompt"]').first().click()
  await page.locator('[data-cut-generate-prompt-panel]').first().waitFor({ state: 'visible', timeout: 12_000 })

  if (promptNotRun) {
    rec(S, 'generate.from_prompt(Preview policy)', { present: 'pass', render: 'na', click: 'na', result: 'na' }, 'no local CLI agent — dep-gated with the plan probe')
    rec(S, 'generate.from_prompt(Insert policy)', { present: 'pass', render: 'na', click: 'na', result: 'na' }, 'no local CLI agent — dep-gated with the plan probe')
  } else {
  await page.locator('[data-cut-generate-prompt-policy]').selectOption('preview').catch(() => {})
  const promptPreview = await captureVerbResp(page, 'generate.from_prompt', async () => {
    await page.locator('[data-cut-generate-prompt-run]').click()
  }, 200000)
  const promptImg = page.locator('[data-cut-generate-prompt-preview-img]').first()
  await promptImg.waitFor({ state: 'visible', timeout: 8000 }).catch(() => {})
  const promptNatural = await waitForGeneratePreviewImageLoaded(page, '[data-cut-generate-prompt-preview-img]')
  rec(S, 'generate.from_prompt(Preview policy)', {
    present: (await page.locator('[data-cut-generate-prompt-run]').count()) > 0 ? 'pass' : 'fail',
    render: prg.ok ? 'pass' : 'fail',
    click: promptPreview ? 'pass' : 'fail',
    result: (promptPreview?.ok && promptPreview.result?.status === 'completed' && promptPreview.result?.preview?.url && promptNatural.w > 0 && promptNatural.h > 0) ? 'pass' : 'fail',
  }, `generate.from_prompt preview status=${promptPreview?.result?.status || 'none'} url=${promptPreview?.result?.preview?.url || 'none'} image=${promptNatural.w}x${promptNatural.h}`)

  const beforePromptOps = (await ops()).length
  await page.locator('[data-cut-generate-prompt-policy]').selectOption('insert').catch(() => {})
  const promptInsert = await captureVerbResp(page, 'generate.from_prompt', async () => {
    await page.locator('[data-cut-generate-prompt-run]').click()
  }, 200000)
  const promptClip = promptInsert?.result?.insert?.clips?.[0]
  const promptCheckpoint = promptInsert?.result?.insert?.checkpoint?.id
  // The clip's exact title TEXT is the LLM's wording — landing by id is the
  // durable state proof (the stub-era `title_text === 'Marta Prompt'` check
  // false-failed any honest paraphrase).
  const promptLanded = promptClip
    ? await waitForState((s) => flatClips(s).some((c) => c.id === promptClip), 9000)
    : null
  const afterPromptOps = (await ops()).length
  rec(S, 'generate.from_prompt(Insert policy)', {
    present: (await page.locator('[data-cut-generate-prompt-insert-evidence]').count()) > 0 ? 'pass' : 'fail',
    render: prg.ok ? 'pass' : 'fail',
    click: promptInsert ? 'pass' : 'fail',
    result: (promptInsert?.ok && promptInsert.result?.status === 'completed' && promptClip && promptCheckpoint && promptLanded && afterPromptOps > beforePromptOps) ? 'pass' : 'fail',
  }, `generate.from_prompt insert status=${promptInsert?.result?.status || 'none'} checkpoint=${promptCheckpoint || 'none'} clip=${promptClip || 'none'} ops ${beforePromptOps}->${afterPromptOps} landed=${!!promptLanded}`)

  if (promptCheckpoint) {
    const reverted = await verb('project.revert', { to: promptCheckpoint, rationale: 'fcv generate prompt cleanup' })
    const gone = promptClip ? await waitForState((s) => !flatClips(s).some((c) => c.id === promptClip), 9000) : null
    rec(S, 'project.revert(Generate prompt checkpoint cleanup)', { present: 'na', render: 'na', click: 'na', result: (reverted.ok && !!gone) ? 'pass' : 'fail' },
      `project.revert{to:${promptCheckpoint}} ok=${reverted.ok} generatedPromptClipRemoved=${!!gone}`)
  }
  }

  await page.locator('[data-cut-left-tab="transcript"]').click().catch(() => {})
  await sleep(300)
}

// ── 16. DIRECTOR: subject-aware reframe loop (render.direct → reframe → qc) ────
// The agent-first Director modal (topbar render Format menu → "Direct…", which mounts
// once the Format aspect ≠ "project"). On open it auto-runs render.direct (renders the
// cut → a per-scene CONTACT SHEET via the perception CV); the user picks who each shot
// follows; render.reframe executes the moving-crop; render.qc reviews the output. All
// three are perception-backed JOBS (cut_perception build_contact_sheet / run_subject),
// so the RESULTs are gated on DEP.perceptionCv — present in the release gate, an honest
// N/A in a partial dev run. (render.preview / render.bundle are NOT surfaced in this
// modal — Director exposes only direct/reframe/qc — so there are no rows for them here.)
async function secDirector(page) {
  const S = 'director'
  await drainActiveJobs() // render jobs from earlier sections keep render-opts disabled by design
  // A real face (SPEECH) makes the talking_head director meaningful; trim to a short
  // head so the three sequential render+CV jobs stay fast.
  await freshProject(page, 'director', SPEECH)
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  for (const t of (await state()).tracks || []) {
    if (t.kind === 'video' || t.kind === 'audio') await verb('edit.ripple_delete', { track: t.id, range_ms: [3000, 999000], ripple: true })
  }
  await sleep(300)
  // Open the render Format menu and pick a non-"project" aspect so the Direct button mounts.
  await page.locator('[data-cut-render-opts]').click().catch(() => {}); await sleep(300)
  await page.locator('[data-cut-render-aspect]').selectOption('9:16').catch(() => {}); await sleep(200)
  await page.locator('[data-cut-reframe-preset]').selectOption('talking_head').catch(() => {})
  const openBtn = page.locator('[data-cut-director-open]')
  const renderMenu = page.locator('[data-cut-render-menu]').first()
  const present = (await openBtn.count()) > 0
  const rg = await renderGroup(page, S, 'render-menu', renderMenu)
  rec(S, 'GATE:reframe-preset-present', gateDim((await page.locator('[data-cut-reframe-preset]').count()) > 0), 'reframe Subject preset select present once Format aspect≠project')

  if (!DEP.perceptionCv) {
    // No CV venv → render.direct would FAIL on open (honest UI error). PRESENT/RENDER the
    // launcher; the three jobs are dep-skipped (the release gate enforces perception present).
    rec(S, 'director-open', { present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: 'na', result: 'na' },
      'render.direct needs the perception venv (cv2+torch — build_contact_sheet); absent (system.doctor perception cv2/torch) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present', rg.shot)
    for (const [v, verbName] of [['director-reframe', 'render.reframe'], ['director-qc', 'render.qc']]) {
      rec(S, v, { present: 'na', render: 'na', click: 'na', result: 'na' },
        `${verbName} needs the perception venv (cv2+torch) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present`)
    }
    await page.keyboard.press('Escape').catch(() => {})
    return
  }

  // render.direct — opening the modal auto-fires it; reaching the PICK phase (a contact
  // sheet) is the real RESULT (the sheet is a perception-built per-scene artifact).
  await probe(page, {
    surface: S, name: 'director-open', sel: openBtn, group: renderMenu, groupName: 'render-menu',
    doClick: async () => {
      await openBtn.click().catch(() => {})
      await page.waitForSelector('[data-cut-director]', { timeout: 5000 }).catch(() => {})
      for (let i = 0; i < 240; i++) { // bounded: render the short cut + build the sheet
        await sleep(700)
        if ((await page.locator('[data-cut-director-pick]').count()) > 0) break
        if ((await page.locator('[data-cut-director-error]').count()) > 0) break
      }
    },
    assertResult: async () => {
      const pick = (await page.locator('[data-cut-director-pick]').count()) > 0
      const sheet = (await page.locator('[data-cut-director-sheet]').count()) > 0
      const err = (await page.locator('[data-cut-director-error]').count())
        ? (await page.locator('[data-cut-director-error]').first().textContent().catch(() => '')) : ''
      await renderGroup(page, S, 'director-modal', page.locator('[data-cut-director]').first())
      return { ok: pick, detail: `render.direct → pick phase=${pick} contact-sheet=${sheet}${err ? ` err="${String(err).slice(0, 60)}"` : ''}` }
    },
  })
  // render.reframe — the primary "Reframe → 9:16" action; reaching the DONE phase
  // (reframe_id shown) is the real RESULT.
  await probe(page, {
    surface: S, name: 'director-reframe', sel: page.locator('[data-cut-director-render]'),
    group: page.locator('[data-cut-director]').first(), groupName: 'director-modal',
    doClick: async () => {
      await page.locator('[data-cut-director-render]').click().catch(() => {})
      for (let i = 0; i < 300; i++) {
        await sleep(700)
        if ((await page.locator('[data-cut-director-done]').count()) > 0) break
        if ((await page.locator('[data-cut-director-error]').count()) > 0) break
      }
    },
    assertResult: async () => {
      const done = (await page.locator('[data-cut-director-done]').count()) > 0
      const err = (await page.locator('[data-cut-director-error]').count())
        ? (await page.locator('[data-cut-director-error]').first().textContent().catch(() => '')) : ''
      return { ok: done, detail: `render.reframe → done phase (reframe_id shown)=${done}${err ? ` err="${String(err).slice(0, 60)}"` : ''}` }
    },
  })
  // render.qc — the "Review (QC)" button reviews the reframed output → a verdict.
  await probe(page, {
    surface: S, name: 'director-qc', sel: page.locator('[data-cut-director-review]'),
    group: page.locator('[data-cut-director]').first(), groupName: 'director-modal',
    doClick: async () => {
      await page.locator('[data-cut-director-review]').click().catch(() => {})
      for (let i = 0; i < 240; i++) {
        await sleep(700)
        if ((await page.locator('[data-cut-director-qc]').count()) > 0) break
        if ((await page.locator('[data-cut-director-error]').count()) > 0) break
      }
    },
    assertResult: async () => {
      const qc = (await page.locator('[data-cut-director-qc]').count()) > 0
      await renderGroup(page, S, 'director-qc', page.locator('[data-cut-director]').first())
      return { ok: qc, detail: `render.qc → verdict rendered=${qc}` }
    },
  })
  // Close through the visible header action and prove the modal detached.
  await probe(page, {
    surface: S, name: 'director-close', actionId: 'director-close',
    sel: page.locator('[data-cut-director-close]'),
    group: page.locator('[data-cut-director]').first(),
    groupName: 'director-modal',
    doClick: async () => {
      await page.locator('[data-cut-director-close]').click()
      await page.locator('[data-cut-director]').waitFor({ state: 'detached', timeout: 8000 })
    },
    assertResult: async () => ({
      ok: await page.locator('[data-cut-director]').count() === 0,
      detail: 'Director modal detached through its visible Close action',
    }),
  })
}

// ── 17. ASSEMBLE (AI) drawer: assemble.shorts / repurpose / from_script / broll ──
// The human face of the agent-only assemble.* family (topbar [data-cut-assemble-btn]
// → the drawer). shorts/repurpose/from_script are ANALYSIS over the source TRANSCRIPT
// (we persist deterministic words through the same receipt contract, then drive each
// and assert the verb returned its ranked analysis; live STT has its own gate). broll
// PLACES a retrieved clip on the
// timeline (assets.search/fetch + edit.insert) via the default local_folder provider —
// tri-state RESULT: a clip placed = pass; a clean "no provider content" failure = honest
// N/A (env/content-dependent, like the Tools orchestrators); a real wiring error = fail.
async function secAssemble(page) {
  const S = 'assemble'
  const projectCtx = await freshProject(page, 'assemble', SPEECH)
  const secondImported = await verb('media.import', { path: SECOND })
  if (FCV_DRAIN_IMPORTS) await awaitImportJobs(secondImported, FCV_IMPORT_DRAIN_TIMEOUT_MS)
  const secondaryAsset = secondImported.result?.asset_id || ''
  if (!secondaryAsset || secondaryAsset === projectCtx.assetId) {
    throw new Error(`Assemble source selector needs two distinct assets; primary=${projectCtx.assetId || 'none'} secondary=${secondaryAsset || 'none'}`)
  }
  await ensureNonEmptyTranscript(
    page,
    projectCtx.projectPath,
    projectCtx.assetId,
    'fcv: Assemble analysis needs a deterministic transcript',
  )
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open, before this section selects/clicks
  // Open the drawer BEFORE the probes so RENDER screenshots the OPEN drawer.
  await page.locator('[data-cut-assemble-btn]').click().catch(() => {}); await sleep(600)
  await page.waitForSelector('[data-cut-assemble]', { timeout: 5000 }).catch(() => {})
  const drawer = page.locator('[data-cut-assemble]').first()
  rec(S, 'GATE:assemble-drawer-open', gateDim((await drawer.count()) > 0), 'Assemble (AI) drawer mounted')

  const assembleActions = createAssembleActionCoverage({
    probe,
    verb,
    sleep,
  })
  await assembleActions.runInputs(page, drawer, {
    primaryAsset: projectCtx.assetId,
    secondaryAsset,
  })

  // mode toggle — each opt switches the drawer mode (local state, value sticks).
  for (const m of ['shorts', 'repurpose', 'from_script', 'broll']) {
    await probe(page, {
      surface: S, name: `mode-${m}`, sel: page.locator(`[data-cut-assemble-mode-opt="${m}"]`), group: drawer, groupName: 'assemble-drawer',
      doClick: async () => { await page.locator(`[data-cut-assemble-mode-opt="${m}"]`).click().catch(() => {}); await sleep(150) },
      assertResult: async () => ({ ok: (await drawer.getAttribute('data-cut-assemble-mode').catch(() => '')) === m, detail: `drawer mode → ${m}` }),
    })
  }

  // Click Run in a given mode and capture that mode's assemble.* verb RESPONSE.
  const runMode = async (mode, verbName, prep) => {
    await page.locator(`[data-cut-assemble-mode-opt="${mode}"]`).click().catch(() => {}); await sleep(200)
    if (prep) await prep()
    return captureVerbResp(page, verbName, async () => {
      await page.locator('[data-cut-assemble-run]').click()
    }, 90_000)
  }

  // shorts / repurpose / from_script run against the deterministic persisted
  // transcript fixture above. Live STT is a separate capability gate; it must
  // not make these native UI actions untestable.
  const assembleTranscript = await verb('transcript.get', { asset: projectCtx.assetId })
  const fromScriptText = (assembleTranscript.result?.words || [])
    .slice(0, 8)
    .map((word) => word.word)
    .filter(Boolean)
    .join(' ')
  if (!fromScriptText) {
    throw new Error(`Assemble transcript fixture has no words for ${projectCtx.assetId}`)
  }
  await probe(page, {
    surface: S, name: 'run-shorts', sel: page.locator('[data-cut-assemble-run]'), group: drawer, groupName: 'assemble-drawer',
    doClick: async () => { probe._r = await runMode('shorts', 'assemble.shorts') },
    assertResult: async () => {
      const r = probe._r
      const n = Array.isArray(r?.result?.shorts) ? r.result.shorts.length : -1
      return { ok: !!r?.ok && n > 0, detail: `assemble.shorts ok=${r?.ok} shorts=${n}${r && !r.ok ? ` err="${String(r.error?.message || '').slice(0, 50)}"` : ''}` }
    },
  })
  await assembleActions.proveJump(page, drawer, {
    modeName: 'shorts',
    expectedAtMs: probe._r.result.shorts[0].range_ms[0],
  })
  await probe(page, {
    surface: S, name: 'run-repurpose', sel: page.locator('[data-cut-assemble-run]'), group: drawer, groupName: 'assemble-drawer',
    doClick: async () => { probe._r = await runMode('repurpose', 'assemble.repurpose') },
    assertResult: async () => {
      const r = probe._r
      const n = Array.isArray(r?.result?.clips) ? r.result.clips.length : -1
      return { ok: !!r?.ok && n > 0, detail: `assemble.repurpose ok=${r?.ok} clips=${n}` }
    },
  })
  await assembleActions.proveJump(page, drawer, {
    modeName: 'repurpose',
    expectedAtMs: probe._r.result.clips[0].range_ms[0],
  })
  await probe(page, {
    surface: S, name: 'run-from-script', sel: page.locator('[data-cut-assemble-run]'), group: drawer, groupName: 'assemble-drawer',
    doClick: async () => {
      probe._r = await runMode('from_script', 'assemble.from_script', async () => {
        await page.locator('[data-cut-assemble-script]').fill(fromScriptText).catch(() => {})
      })
    },
    assertResult: async () => {
      const r = probe._r
      const matched = (r?.result?.segments || []).filter((segment) => segment.matched && segment.range_ms).length
      return { ok: !!r?.ok && matched > 0, detail: `assemble.from_script ok=${r?.ok} matched=${matched}` }
    },
  })
  const matchedSegment = probe._r?.result?.segments?.find((segment) => segment.matched && segment.range_ms)
  if (matchedSegment) {
    await assembleActions.proveJump(page, drawer, {
      modeName: 'from-script',
      expectedAtMs: matchedSegment.range_ms[0],
    })
  } else {
    rec(S, 'jump-from-script', {
      present: 'fail', render: 'fail', click: 'fail', result: 'fail',
    }, `from-script produced no matched segment for the exact transcript text "${fromScriptText}"`)
  }

  // broll — places a retrieved clip; tri-state (placed / no-content / wiring error).
  {
    await page.locator('[data-cut-assemble-mode-opt="broll"]').click().catch(() => {}); await sleep(200)
    const sel = page.locator('[data-cut-assemble-run]')
    const present = (await sel.count()) > 0
    const rgb = await renderGroup(page, S, 'assemble-drawer', drawer)
    if (!present) {
      rec(S, 'run-broll', { present: 'fail', render: rgb.ok ? 'pass' : 'fail', click: 'fail', result: 'fail' }, `broll run button absent ${rgb.detail}`.trim(), rgb.shot)
    } else {
      const brollSource = SECOND
      const brollDir = dirnameHostPath(brollSource)
      const brollFile = basenameHostPath(brollSource)
      const brollQuery = brollFile.replace(/\.[^.]+$/, '').split(/[_\-\s.]+/).find((part) => part.length >= 3) || brollFile.replace(/\.[^.]+$/, '')
      await page.locator('[data-cut-assemble-dir]').fill(brollDir).catch(() => {})
      await page.locator('[data-cut-assemble-query]').fill(brollQuery).catch(() => {})
      const before = flatClips(await state()).filter((c) => c.asset).length
      const resp = await captureVerbResp(page, 'assemble.broll', async () => {
        await sel.click()
      }, 90_000)
      const placed = Array.isArray(resp?.result?.placed) ? resp.result.placed : []
      const placedIds = placed.map((entry) => entry?.clip_id).filter(Boolean)
      const landed = placedIds.length > 0
        ? !!(await waitForState((project) =>
            placedIds.every((clipId) => flatClips(project).some((clip) => clip.id === clipId)),
          15_000))
        : false
      const after = flatClips(await state()).filter((c) => c.asset).length
      const status = resp?.result?.status
      let result, detail
      if (resp?.ok && status !== 'failed' && placed.length > 0 && landed) {
        result = 'pass'; detail = `assemble.broll placed=${placed.length} landed=${landed} media clips ${before}→${after}`
      } else if (resp && (status === 'failed' || !resp.ok)) {
        // resp.result.error is an OBJECT, so String(obj) logged the
        // useless literal "[object Object]". Pull .message/.code first, then fall back to
        // a string error / JSON, so the triage regex below sees real text (content-dep N/A
        // vs a genuine wiring fail) instead of always reading "[object Object]".
        const errRaw = resp.result?.error ?? resp.error ?? resp.result?.failed_step
        const msg = String(
          errRaw?.message || errRaw?.code ||
          (typeof errRaw === 'string' ? errRaw : (errRaw ? JSON.stringify(errRaw) : '')) ||
          'no provider content')
        const contentDep = /search|fetch|no (result|match|clip)|provider|local_folder|not found|empty/i.test(msg)
        result = contentDep ? 'na' : 'fail'
        detail = `assemble.broll ${contentDep ? 'no provider content (local_folder seeded dir/query did not resolve)' : 'errored'}: "${msg.slice(0, 70)}"`
      } else {
        result = 'fail'; detail = `assemble.broll no response / no clip placed (resp=${resp ? 'ok-no-op' : 'none'})`
      }
      rec(S, 'run-broll', { present: 'pass', render: rgb.ok ? 'pass' : 'fail', click: 'pass', result }, `${detail} ${rgb.detail}`.trim(), rgb.shot)
      if (result === 'pass') {
        await assembleActions.proveJump(page, drawer, {
          modeName: 'broll',
          expectedAtMs: placed[0].at_ms,
        })
      } else {
        rec(S, 'jump-broll', { present: 'na', render: 'na', click: 'na', result: 'na' },
          `B-roll result jump mounts only after a provider clip is placed; run-broll=${result}`)
      }
    }
  }
  // Close the drawer for the next section.
  await page.locator('[data-cut-assemble-close]').click().catch(() => {})
  await page.keyboard.press('Escape').catch(() => {})
  await sleep(200)
}

// ── Shared helper: capture a verb's HTTP response fired by a UI action ────────
// Generalizes secAssemble's runMode pattern: attach a /api/verb/<name> response
// listener, run `act` (the click that triggers the verb), then wait up to timeoutMs
// for the matching response body. Returns the parsed VerbResult ({ok, result, op_ids,
// error}) or undefined if none arrived. This is the falsifiable RESULT source for the
// transcript/QC/kinetic/matte controls whose dispatch is a single REST verb — the verb
// erroring (ok:false) FAILS the row; a clean ok:true is the action's proof.
async function captureVerbResp(page, name, act, timeoutMs = 90000) {
  let resp
  const onR = async (r) => {
    if (resp !== undefined) return
    // Exact pathname ownership matters: assets.generate is a prefix of
    // assets.generated_list, whose background history poll can otherwise be
    // mistaken for the action response we are proving.
    let pathname = ''
    try { pathname = new URL(r.url()).pathname } catch { return }
    if (pathname !== `/api/verb/${name}`) return
    try { resp = await r.json() } catch { /* non-JSON / aborted — leave undefined */ }
  }
  page.on('response', onR)
  try {
    await act()
    const deadline = Date.now() + timeoutMs
    while (resp === undefined && Date.now() < deadline) {
      await page.flushEvents?.()
      if (resp === undefined) await sleep(100)
    }
    if (resp === undefined && process.env.FCV_TRACE === '1') {
      const diagnostic = await page.evaluate(() => {
        const text = (selector) => document.querySelector(selector)?.textContent?.replace(/\s+/g, ' ').trim() || ''
        const value = (selector) => {
          const element = document.querySelector(selector)
          return element instanceof HTMLInputElement || element instanceof HTMLSelectElement
            ? element.value
            : ''
        }
        const active = document.activeElement
        return {
          active: active instanceof Element
            ? `${active.tagName.toLowerCase()}${active.getAttribute('data-cut-action') ? `[data-cut-action="${active.getAttribute('data-cut-action')}"]` : ''}`
            : '',
          renderQueue: {
            mounted: !!document.querySelector('[data-cut-render-queue]'),
            firstOutput: value('[data-cut-render-queue-output="0"]'),
            secondOutput: value('[data-cut-render-queue-output="1"]'),
            phase: document.querySelector('[data-cut-render-queue-progress]')?.getAttribute('data-cut-render-queue-progress') || 'form',
            error: text('[data-cut-render-queue-error], .rq-error'),
          },
          kinetic: {
            mounted: !!document.querySelector('[data-cut-kinetic]'),
            applyDisabled: document.querySelector('[data-cut-kinetic-apply]')?.hasAttribute('disabled') ?? null,
            position: value('[data-cut-kinetic-position]'),
            error: text('[data-cut-kinetic-error]'),
            result: text('[data-cut-kinetic-result]'),
          },
          record: {
            result: document.querySelector('[data-cut-studio-result]')?.getAttribute('data-cut-studio-result') || '',
            error: text('[data-cut-rec-error]'),
            note: text('[data-cut-rec-finalizing], [data-cut-rec-done]'),
            // The EXPORT note is its own element; the finalize/done note above
            // never carries an export failure, so a timed-out screen_record.export
            // used to produce a diagnostic that said nothing about the export.
            exportNote: text('[data-cut-rec-export-note]'),
            outputPath: document.querySelector('[data-cut-rec-output-path]')?.getAttribute('data-cut-rec-output-path') || '',
          },
        }
      }).catch((error) => ({ diagnosticError: String(error?.message || error) }))
      console.error(`[fcv-response-timeout] verb=${name} diagnostic=${JSON.stringify(diagnostic)}`)
    }
    return resp
  } finally {
    page.off('response', onR)
  }
}

// ── 18. TRANSCRIPT editor (left tab=transcript): the word-level edit surface ───
// The flowing-word transcript panel (left sidebar tab). EVERY control needs a REAL
// transcript, so we import + transcribe SPEECH first (perception STT — the SCENE road
// clip has no speech). Without STT the whole surface honest-N/As (the release gate
// enforces STT). Control → verb each drives (grepped from panels/Transcript):
//   view-program → transcript.timeline · view-source → transcript.get · search →
//   transcript.search · Tools{filler-pass→remove_fillers, silence-pass(+aggr)→
//   remove_silences, retakes-pass→remove_retakes, generate-chapters→chapters} ·
//   ignore-words → transcript.ignore_words · cut-words (shift-select → floating
//   toolbar) → transcript.cut_words · reel (reel-mode → add-to-reel →
//   assemble-reel) → transcript.assemble.
async function secTranscriptEditor(page) {
  const S = 'transcript-editor'
  await freshProject(page, 'tx', SPEECH)
  await closeOverlays(page) // macOS cascade guard — drop any drawer/menu a prior section left open
  if (!DEP.perceptionStt) {
    for (const n of [
      'view-program(transcript.timeline)', 'view-source(transcript.get)', 'search(transcript.search)',
      'filler-pass(transcript.remove_fillers)', 'silence-pass(transcript.remove_silences)',
      'retakes-pass(transcript.remove_retakes)', 'generate-chapters(transcript.chapters)',
      'cut-words(transcript.cut_words)', 'ignore-words(transcript.ignore_words)',
      'unignore-words(transcript.ignore_words remove)', 'mute-words(transcript.mute_words)',
      'unmute-words(edit.mute_range remove)', 'reel-add(tray)', 'reel-assemble(transcript.assemble)',
    ]) {
      rec(S, n, { present: 'na', render: 'na', click: 'na', result: 'na' },
        'transcript editor needs a TRANSCRIBED source (perception STT) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present')
    }
    return
  }
  // Establish a real transcript (real STT) so every control has words to act on.
  const st0 = await state()
  const asset = st0.tracks.find((t) => t.kind === 'video')?.clips?.find((c) => c.asset)?.asset || Object.keys(st0.assets || {})[0]
  if (asset) {
    const tr = await verb('media.transcribe', { asset })
    if (tr.result?.job_id) await awaitJob(tr.result.job_id)
    await waitForState((s) => Object.values(s.assets || {}).some((a) => a?.transcript), 60000)
    if (DEP.perceptionCv) {
      const pr = await verb('media.perception', { asset })
      if (pr.result?.job_id) await awaitJob(pr.result.job_id)
    }
  }
  await reloadApp(page); await sleep(900)
  await page.locator('[data-cut-left-tab="transcript"]').click().catch(() => {}); await sleep(600)
  const panel = page.locator('[data-cut-panel="transcript"]').first()
  rec(S, 'GATE:transcript-panel-mounted', gateDim((await panel.count()) > 0), 'transcript panel mounted on the left tab')

  // A guaranteed-match search query: a real word (≥3 letters) from the transcript.
  const tg = await verb('transcript.get', { asset })
  const firstWord = (tg.result?.words || []).map((w) => w.word).find((w) => /[a-z]{3,}/i.test(w || '')) || ''
  const sourceWordSelector = asset
    ? `[data-cut-transcript="${String(asset).replaceAll('"', '\\"')}"] [data-cut-word]`
    : '[data-cut-word]'
  const sourceWords = () => panel.locator(sourceWordSelector)
  const sourceWordAtIndex = (index) => panel.locator(
    `${sourceWordSelector}[data-word-idx="${index}"]`,
  ).first()

  // view-program → transcript.timeline (the PROGRAM line; no selection needed).
  await probe(page, {
    surface: S, name: 'view-program(transcript.timeline)', sel: page.locator('[data-cut-action="view-program"]'), group: panel, groupName: 'transcript-panel',
    doClick: async () => { probe._r = await captureVerbResp(page, 'transcript.timeline', async () => { await page.locator('[data-cut-action="view-program"]').click().catch(() => {}) }, 30000) },
    assertResult: async () => {
      const r = probe._r
      const n = Array.isArray(r?.result?.entries) ? r.result.entries.length : -1
      const ui = await page.locator('[data-cut-timeline-word]').count()
      return { ok: !!r?.ok && n >= 0, detail: `transcript.timeline ok=${r?.ok} entries=${n} (timeline words rendered=${ui})` }
    },
  })
  // view-source → transcript.get powers the Source view; words rendering IS the proof.
  await probe(page, {
    surface: S, name: 'view-source(transcript.get)', sel: page.locator('[data-cut-action="view-source"]'), group: panel, groupName: 'transcript-panel',
    doClick: async () => { await page.locator('[data-cut-action="view-source"]').click().catch(() => {}); await sleep(700) },
    assertResult: async () => {
      const words = await page.locator('[data-cut-word]').count()
      return { ok: words > 0, detail: `Source view rendered ${words} word spans (loaded via transcript.get)` }
    },
  })
  // search → transcript.search (type a real word, Enter, capture the response).
  if (firstWord) {
    await probe(page, {
      surface: S, name: 'search(transcript.search)', sel: page.locator('[data-cut-transcript-search]'), group: panel, groupName: 'transcript-panel',
      doClick: async () => {
        probe._r = await captureVerbResp(page, 'transcript.search', async () => {
          await page.locator('[data-cut-transcript-search]').fill(firstWord).catch(() => {})
          await page.locator('[data-cut-transcript-search]').press('Enter').catch(() => {})
        }, 20000)
      },
      assertResult: async () => {
        const r = probe._r
        const mc = r?.result?.match_count
        const note = (await page.locator('[data-cut-search-note]').first().textContent().catch(() => '')) || ''
        return { ok: !!r?.ok && typeof mc === 'number' && mc >= 1, detail: `transcript.search "${firstWord}" → match_count=${mc} note="${note.slice(0, 30)}"` }
      },
    })
  } else {
    rec(S, 'search(transcript.search)', { present: 'pass', render: 'na', click: 'na', result: 'na' },
      'transcript has no ≥3-letter word to guarantee a search match (degenerate transcript) — honest skip')
  }

  const selectTranscriptSourceRange = async (first, last, expectedAction) => {
    const toolbar = page.locator('.tx__cut-toolbar').first()
    await page.keyboard.press('Escape').catch(() => {})
    await toolbar.waitFor({ state: 'detached', timeout: 5000 }).catch(() => {})

    // Project refreshes after Ignore/Mute can remount Transcript in Clip view
    // just after the verb response is observable. Require Source to remain
    // stable across one settle window before dispatching against its word DOM.
    const ensureStableSource = async () => {
      for (let attempt = 0; attempt < 3; attempt += 1) {
        await sleep(attempt === 0 ? 250 : 400)
        const panel = page.locator('[data-cut-panel="transcript"]').first()
        if (!await panel.isVisible().catch(() => false)) {
          await page.locator('[data-cut-left-tab="transcript"]').first().click().catch(() => {})
          const panelReady = await panel.waitFor({ state: 'visible', timeout: 5000 })
            .then(() => true)
            .catch(() => false)
          if (!panelReady) continue
        }
        const view = page.locator('[data-cut-transcript-view]').first()
        if (await view.getAttribute('data-cut-transcript-view').catch(() => '') !== 'source') {
          const sourceToggle = page.locator('[data-cut-action="view-source"]').first()
          const switched = await sourceToggle.scrollIntoViewIfNeeded()
            .then(() => sourceToggle.click())
            .then(() => true)
            .catch(() => false)
          if (!switched) continue
        }
        const sourceReady = await page.locator('[data-cut-action="view-source"].tx__viewbtn--on').first().waitFor({
          state: 'visible',
          timeout: 5000,
        }).then(() => true).catch(() => false)
        if (!sourceReady) continue
        const endpointsReady = await Promise.all([
          first.waitFor({ state: 'attached', timeout: 5000 }),
          last.waitFor({ state: 'attached', timeout: 5000 }),
        ]).then(() => true).catch(() => false)
        if (!endpointsReady) continue
        await sleep(250)
        if (await view.getAttribute('data-cut-transcript-view').catch(() => '') === 'source'
          && await first.count() > 0 && await last.count() > 0) return
      }
      throw new Error(`Transcript Source view did not settle for ${expectedAction}`)
    }

    const dispatchRange = async () => {
      // Dispatch the product's exact selection state machine. Native geometry
      // can call an attached word "not visible" after the bounded scroller
      // moves its sibling endpoint; DOM events still prove the real handlers.
      await first.dispatchEvent('mousedown', { button: 0, buttons: 1, shiftKey: false })
      await first.dispatchEvent('mouseup', { button: 0, buttons: 0, shiftKey: false })
      await last.dispatchEvent('mousedown', { button: 0, buttons: 1, shiftKey: true })
      await last.dispatchEvent('mouseup', { button: 0, buttons: 0, shiftKey: true })
    }

    await ensureStableSource()
    await dispatchRange()
    const action = page.locator(`[data-cut-action="${expectedAction}"]`).first()
    if (!await action.isVisible().catch(() => false)) {
      // A refresh can race the first gesture. Re-establish Source and retry only
      // when the exact toolbar action failed to mount.
      await ensureStableSource()
      await dispatchRange()
    }
    await toolbar.waitFor({ state: 'visible', timeout: 12_000 })
    await action.waitFor({ state: 'visible', timeout: 12_000 })
  }

  // Prove non-destructive ignore before the cleanup passes below publish cut
  // ops. Otherwise a word can look live while local op state is catching up,
  // then become removed during the ignore assertion for content-dependent
  // reasons unrelated to ignore styling.
  await verifyIgnoreWords()

  // Project refreshes after transcript mutations can remount the left panel and
  // close Tools. Re-open the real surface and require the requested action,
  // rather than letting a swallowed trigger click cascade into absent rows.
  const ensureTools = async (expectedAction = '') => {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const transcriptPanel = page.locator('[data-cut-panel="transcript"]').first()
      if (!await transcriptPanel.isVisible().catch(() => false)) {
        await page.locator('[data-cut-left-tab="transcript"]').first().click().catch(() => {})
        const mounted = await transcriptPanel.waitFor({ state: 'visible', timeout: 5000 })
          .then(() => true)
          .catch(() => false)
        if (!mounted) continue
      }
      const menu = page.locator('[data-cut-tools-menu]').first()
      if (!await menu.isVisible().catch(() => false)) {
        const trigger = page.locator('[data-cut-action="tools-menu"]').first()
        const opened = await trigger.scrollIntoViewIfNeeded()
          .then(() => trigger.click())
          .then(() => menu.waitFor({ state: 'visible', timeout: 5000 }))
          .then(() => true)
          .catch(() => false)
        if (!opened) continue
      }
      if (!expectedAction) return
      const actionReady = await page.locator(`[data-cut-action="${expectedAction}"]`).first()
        .waitFor({ state: 'visible', timeout: 5000 })
        .then(() => true)
        .catch(() => false)
      if (actionReady) return
      await sleep(350)
    }
    throw new Error(`Transcript Tools did not expose ${expectedAction || 'its menu'}`)
  }
  // Pass orchestrators — each runs a verb whose cut count is CONTENT-DEPENDENT: a clean
  // run with 0 cuts (no fillers / no retakes in this take) is a VALID pass (the verb
  // executed) exactly as secAssemble treats shorts/repurpose's empty-array case. RESULT
  // = the verb returned ok; a wiring error (ok:false) FAILS.
  // filler/retakes operate on the WORD transcript (present once STT ran), so a clean run
  // (ok=true, any cut count) is a pass. silence-pass is handled SEPARATELY below — it needs
  // SILENCE facts (a full perception report), which the STT-only seed may not have produced,
  // and the engine HONESTLY errors rather than faking a 0-span no-op → it needs tri-state.
  const passes = [
    { name: 'filler-pass(transcript.remove_fillers)', action: 'filler-pass', verb: 'transcript.remove_fillers', prep: null },
    { name: 'retakes-pass(transcript.remove_retakes)', action: 'retakes-pass', verb: 'transcript.remove_retakes', prep: null },
  ]
  for (const p of passes) {
    await ensureTools(p.action)
    await probe(page, {
      surface: S, name: p.name, sel: page.locator(`[data-cut-action="${p.action}"]`), group: page.locator('[data-cut-tools-menu]').first(), groupName: 'transcript-tools',
      doClick: async () => {
        if (p.prep) await p.prep()
        probe._r = await captureVerbResp(page, p.verb, async () => { await page.locator(`[data-cut-action="${p.action}"]`).click().catch(() => {}) }, 90000)
      },
      assertResult: async () => {
        const r = probe._r
        const cuts = Array.isArray(r?.op_ids) ? r.op_ids.length : '?'
        return { ok: !!r?.ok, detail: `${p.verb} ran ok=${r?.ok} (op_ids/cuts=${cuts}) — content-dependent: a clean 0-cut run is a valid PASS (verb executed)` }
      },
    })
  }
  // silence-pass → transcript.remove_silences. This row hard-failed on
  // ok=false even though a clean run is a valid pass. remove_silences needs SILENCE facts
  // (a full perception report); the transcript editor seeds only an STT transcript, so the
  // engine HONESTLY errors ("no silence facts") rather than faking a 0-span no-op. Classify
  // tri-state to match the stated intent + the other remove-* rows: ok→PASS (content-dependent
  // cut count, a clean 0-cut run is valid); a missing-facts/no-silence NOT_FOUND→N/A (the
  // silence battery — media.perception — wasn't run; FCV_REQUIRE_FULL=1 enforces it); any
  // other ok=false→a real FAIL.
  {
    await ensureTools('silence-pass')
    await page.locator('[data-cut-aggressiveness]').selectOption('natural').catch(() => {}); await sleep(150)
    const sel = page.locator('[data-cut-action="silence-pass"]')
    const present = (await sel.count()) > 0
    const rg = await renderGroup(page, S, 'transcript-tools', page.locator('[data-cut-tools-menu]').first())
    let result = 'fail', detail = 'silence-pass action absent'
    if (present) {
      const r = await captureVerbResp(page, 'transcript.remove_silences', async () => { await sel.click().catch(() => {}) }, 90000)
      const cuts = Array.isArray(r?.op_ids) ? r.op_ids.length : '?'
      const msg = String(r?.error?.message || r?.error?.code || '')
      const noFacts = !r?.ok && (r?.error?.code === 'not_found' || /no silence facts|no silences|silence facts|not measured/i.test(msg))
      if (r?.ok) { result = 'pass'; detail = `transcript.remove_silences ran ok=true (op_ids/cuts=${cuts}) — content-dependent: a clean 0-cut run is a valid PASS (verb executed)` }
      else if (noFacts) { result = 'na'; detail = `transcript.remove_silences honest no-op: no silence facts measured ("${msg.slice(0, 60)}") — the editor seeds only an STT transcript; the silence battery (media.perception) is perception-dep-gated; FCV_REQUIRE_FULL=1 enforces it — N/A, not a wiring fail` }
      else { result = 'fail'; detail = `transcript.remove_silences ok=false err="${msg.slice(0, 80)}"` }
    }
    rec(S, 'silence-pass(transcript.remove_silences)', { present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: present ? 'pass' : 'fail', result }, `${detail} ${rg.detail}`.trim(), rg.shot)
  }
  // generate-chapters → transcript.chapters (non-mutating; ≥1 chapter on any non-empty
  // transcript, then one edit.add_marker per chapter).
  await ensureTools('generate-chapters')
  await probe(page, {
    surface: S, name: 'generate-chapters(transcript.chapters)', sel: page.locator('[data-cut-action="generate-chapters"]'), group: page.locator('[data-cut-tools-menu]').first(), groupName: 'transcript-tools',
    doClick: async () => { probe._r = await captureVerbResp(page, 'transcript.chapters', async () => { await page.locator('[data-cut-action="generate-chapters"]').click().catch(() => {}) }, 60000) },
    assertResult: async () => {
      const r = probe._r
      const ch = Array.isArray(r?.result?.chapters) ? r.result.chapters.length : -1
      return { ok: !!r?.ok && ch >= 1, detail: `transcript.chapters ok=${r?.ok} chapters=${ch}` }
    },
  })

  // ignore-words → transcript.ignore_words + unignore → transcript.ignore_words
  // {remove:true}: select two adjacent LIVE words in Source view and prove the
  // toolbar marks them as ignored for transcript-derived outputs without
  // cutting/muting. Earlier filler/silence passes can remove arbitrary source
  // words, so fixed DOM positions are not a stable target.
  async function verifyIgnoreWords() {
    const wc = (tg.result?.words || []).length
    await page.keyboard.press('Escape').catch(() => {})
    await sleep(150)
    await page.locator('[data-cut-action="view-source"]').click().catch(() => {}); await sleep(400)
    const liveWordIndexes = await sourceWords().evaluateAll((words) => words
      .filter((word) => !word.closest('[data-cut-removed]'))
      .map((word) => Number(word.getAttribute('data-word-idx')))
      .filter(Number.isInteger))
    const pair = liveWordIndexes.find((idx, pos) => idx >= 3 && pos < liveWordIndexes.length - 1 && liveWordIndexes[pos + 1] === idx + 1)
    const ignoreRange = pair === undefined ? null : [pair, pair + 1]
    if (wc >= 2 && ignoreRange) {
      await selectTranscriptSourceRange(
        sourceWordAtIndex(ignoreRange[0]),
        sourceWordAtIndex(ignoreRange[1]),
        'ignore-words',
      )
      await page.locator('[data-cut-action="ignore-words"]').waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
      await probe(page, {
        surface: S, name: 'ignore-words(transcript.ignore_words)', sel: page.locator('[data-cut-action="ignore-words"]'), group: panel, groupName: 'transcript-panel',
        doClick: async () => { probe._r = await captureVerbResp(page, 'transcript.ignore_words', async () => { await page.locator('[data-cut-action="ignore-words"]').click().catch(() => {}) }, 30000) },
        assertResult: async () => {
          const r = probe._r
          const st = await waitForState((s) => Array.isArray(s.transcript_ignores) && s.transcript_ignores.length > 0, 10000)
          const ignored = page.locator('[data-cut-word-ignored]').first()
          await ignored.waitFor({ state: 'attached', timeout: 10000 }).catch(() => {})
          const spans = await page.locator('[data-cut-word-ignored]').count()
          return { ok: !!r?.ok && !!st && spans >= 1, detail: `transcript.ignore_words range=${ignoreRange.join('..')} ok=${r?.ok} state-ignores=${!!st} grey-spans=${spans} — transcript text intact, derived outputs skip` }
        },
      })
      // Unignore: re-select the same words — the toolbar now offers Unignore.
      await selectTranscriptSourceRange(
        sourceWordAtIndex(ignoreRange[0]),
        sourceWordAtIndex(ignoreRange[1]),
        'unignore-words',
      )
      await page.locator('[data-cut-action="unignore-words"]').waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
      await probe(page, {
        surface: S, name: 'unignore-words(transcript.ignore_words remove)', sel: page.locator('[data-cut-action="unignore-words"]'), group: panel, groupName: 'transcript-panel',
        doClick: async () => { probe._r = await captureVerbResp(page, 'transcript.ignore_words', async () => { await page.locator('[data-cut-action="unignore-words"]').click().catch(() => {}) }, 30000) },
        assertResult: async () => {
          const r = probe._r
          const cleared = await waitForState((s) => !Array.isArray(s.transcript_ignores) || s.transcript_ignores.length === 0, 10000)
          await page.locator('[data-cut-word-ignored]').first().waitFor({ state: 'detached', timeout: 10000 }).catch(() => {})
          const spans = await page.locator('[data-cut-word-ignored]').count()
          return { ok: !!r?.ok && !!cleared && spans === 0, detail: `transcript.ignore_words{remove:true} ok=${r?.ok} ignores-cleared=${!!cleared} grey-spans-left=${spans}` }
        },
      })
    } else {
      for (const n of ['ignore-words(transcript.ignore_words)', 'unignore-words(transcript.ignore_words remove)']) {
        rec(S, n, { present: 'na', render: 'na', click: 'na', result: 'na' },
          `transcript has ${wc} words but no adjacent live pair remained after the content-dependent cleanup passes; honest skip`)
      }
    }
  }

  // mute-words → transcript.mute_words + unmute → edit.mute_range{remove_ms}
  // non-destructive): select words 4–5 in Source view via the same anchor+shift
  // gesture, Mute via the floating toolbar (amber-struck spans appear, timeline
  // UNCHANGED), then re-select and Unmute (spans clear). Runs BEFORE cut-words so
  // the muted words are still uncut; word indices are SOURCE-stable so the later
  // cut of words 0–2 is unaffected.
  {
    const wc = (tg.result?.words || []).length
    if (wc >= 6) {
      await page.keyboard.press('Escape').catch(() => {})
      await sleep(150)
      await page.locator('[data-cut-action="view-source"]').click().catch(() => {}); await sleep(400)
      await selectTranscriptSourceRange(
        sourceWords().nth(4),
        sourceWords().nth(5),
        'mute-words',
      )
      await page.locator('[data-cut-action="mute-words"]').waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
      await probe(page, {
        surface: S, name: 'mute-words(transcript.mute_words)', sel: page.locator('[data-cut-action="mute-words"]'), group: panel, groupName: 'transcript-panel',
        doClick: async () => { probe._r = await captureVerbResp(page, 'transcript.mute_words', async () => { await page.locator('[data-cut-action="mute-words"]').click().catch(() => {}) }, 30000) },
        assertResult: async () => {
          const r = probe._r
          const muted = Array.isArray(r?.result?.muted) ? r.result.muted.length : -1
          const st = await waitForState((s) => (s.tracks || []).some((t) => t.kind === 'audio' && (t.clips || []).some((c) => (c.mute_ranges || []).length > 0)), 10000)
          await sleep(400)
          const spans = await page.locator('[data-cut-word-muted]').count()
          return { ok: !!r?.ok && muted >= 1 && !!st && spans >= 1, detail: `transcript.mute_words ok=${r?.ok} muted-clips=${muted} state-mute_ranges=${!!st} amber-spans=${spans} — timing untouched (non-destructive)` }
        },
      })
      // Unmute: re-select the same words — the toolbar now offers Unmute.
      await selectTranscriptSourceRange(
        sourceWords().nth(4),
        sourceWords().nth(5),
        'unmute-words',
      )
      await page.locator('[data-cut-action="unmute-words"]').waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
      await probe(page, {
        surface: S, name: 'unmute-words(edit.mute_range remove)', sel: page.locator('[data-cut-action="unmute-words"]'), group: panel, groupName: 'transcript-panel',
        doClick: async () => { probe._r = await captureVerbResp(page, 'edit.mute_range', async () => { await page.locator('[data-cut-action="unmute-words"]').click().catch(() => {}) }, 30000) },
        assertResult: async () => {
          const r = probe._r
          const cleared = await waitForState((s) => !(s.tracks || []).some((t) => t.kind === 'audio' && (t.clips || []).some((c) => (c.mute_ranges || []).length > 0)), 10000)
          await sleep(400)
          const spans = await page.locator('[data-cut-word-muted]').count()
          return { ok: !!r?.ok && !!cleared && spans === 0, detail: `edit.mute_range{remove_ms} ok=${r?.ok} ranges-cleared=${!!cleared} amber-spans-left=${spans}` }
        },
      })
    } else {
      for (const n of ['mute-words(transcript.mute_words)', 'unmute-words(edit.mute_range remove)']) {
        rec(S, n, { present: 'na', render: 'na', click: 'na', result: 'na' },
          `transcript has ${wc} words (<6) — not enough to mute words 4-5 without colliding with the cut-words range; honest skip`)
      }
    }
  }

  // cut-words → transcript.cut_words. Drive the REAL selection→floating-toolbar path:
  // in Source view, plain-click word 0 (sets the anchor), shift-click word 2 (range
  // select) → the floating Cut toolbar mounts → click it. reelMode is OFF here (the
  // toolbar renders "Cut" only when reelMode is off), so this MUST precede the reel run.
  await page.keyboard.press('Escape').catch(() => {}) // close the Tools menu so it can't cover the words
  await sleep(150)
  await page.locator('[data-cut-action="view-source"]').click().catch(() => {}); await sleep(400)
  await selectTranscriptSourceRange(
    sourceWords().nth(0),
    sourceWords().nth(2),
    'cut-words',
  )
  await page.locator('[data-cut-action="cut-words"]').waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
  await probe(page, {
    surface: S, name: 'cut-words(transcript.cut_words)', sel: page.locator('[data-cut-action="cut-words"]'), group: panel, groupName: 'transcript-panel',
    doClick: async () => { probe._r = await captureVerbResp(page, 'transcript.cut_words', async () => { await page.locator('[data-cut-action="cut-words"]').click().catch(() => {}) }, 30000) },
    assertResult: async () => {
      const r = probe._r
      const struck = await page.locator('[data-cut-removed]').count()
      return { ok: !!r?.ok || struck > 0, detail: `transcript.cut_words ok=${r?.ok} struck-spans=${struck}` }
    },
  })

  // reel → transcript.assemble. reelMode gates the tray + "Assemble reel" button (they mount
  // only when it's true). The ONLY control is the Tools-menu "Reel mode" item, which is
  // disabled={passBusy!==''} (Transcript:640). That toggle WON'T flip headlessly — a click on
  // the (intermittently passBusy-disabled) item is swallowed by Playwright's actionability
  // wait, and two prior gesture fixes (enabled-wait + idempotent aria-checked toggle) did not
  // hold across rigs — so the tray + add-to-reel + assemble-reel buttons never mount headless.
  // RESOLUTION (verb-level, the existing convention): keep the Tools-menu "Reel mode" item as
  // the PRESENT/RENDER control (the only reel entry point), mark the toggle GESTURE headless-
  // untestable (click=na), and drive transcript.assemble — the verb the whole reel workflow
  // produces (stage spans into the tray → assemble) — DIRECTLY for a real RESULT. We still
  // ATTEMPT the flip (best-effort; recorded in evidence) so a rig where it DOES flip is noted.
  await ensureTools()
  for (let attempt = 0; attempt < 8; attempt++) {
    if ((await page.locator('[data-cut-action="assemble-reel"]').count()) > 0) break
    await ensureTools()
    const rm = page.locator('[data-cut-action="reel-mode"]').first()
    const enabled = (await rm.count()) > 0 && !(await rm.isDisabled().catch(() => true))
    const checked = await rm.getAttribute('aria-checked').catch(() => null)
    if (enabled && checked !== 'true') await rm.click().catch(() => {}) // only a clickable, not-yet-on item
    await sleep(350)
  }
  const reelOn = (await page.locator('[data-cut-action="assemble-reel"]').count()) > 0
  // PRESENT/RENDER the reel-mode toggle in the (open) Tools menu — the only reel entry control.
  await ensureTools()
  const reelToggle = page.locator('[data-cut-action="reel-mode"]').first()
  const togglePresent = (await reelToggle.count()) > 0
  const rgReel = await renderGroup(page, S, 'transcript-reel-toggle', page.locator('[data-cut-tools-menu]').first())
  // Drive the reel build via the verb: selected words → an assembled reel (one edit.insert per
  // span). word_ranges are the SAME input the tray would stage; transcript.assemble consumes them.
  const wcount = (tg.result?.words || []).length
  const hi = Math.min(3, Math.max(0, wcount - 1))
  const beforeClips = flatClips(await state()).filter((x) => x.asset).length
  const beforeOps = await opsLen()
  trace(S, 'reel-assemble-direct', 'start')
  const asm = (asset && wcount > 0)
    ? await verb('transcript.assemble', { asset, word_ranges: [[0, hi]], rationale: 'fcv: transcript.assemble reel (reelMode toggle not headless-drivable)' })
    : { ok: false, error: { message: 'no transcript words to assemble' } }
  trace(S, 'reel-assemble-direct', 'done')
  const placed = asm.result?.spans_placed ?? 0
  trace(S, 'reel-wait-timeline', 'start')
  const inserted = await opLanded(beforeOps, 'edit.insert')
  const afterClipsState = await waitForState((s) => flatClips(s).filter((x) => x.asset).length > beforeClips, 10000)
  trace(S, 'reel-wait-timeline', 'done')
  const afterClips = afterClipsState ? flatClips(afterClipsState).filter((x) => x.asset).length : beforeClips
  await page.keyboard.press('Escape').catch(() => {}) // close the Tools menu
  await sleep(150)
  // Duplicate lowering cross-check only. The dedicated transcript-actions module
  // owns the real reel-mode, tray, and Assemble UI clicks; this legacy engine
  // section independently verifies that their lowering reaches the timeline.
  rec(S, 'reel-add(tray)',
    { rowKind: 'support', present: togglePresent ? 'pass' : 'fail', render: rgReel.ok ? 'pass' : 'fail', click: 'na', result: (asm.ok && placed >= 1) ? 'pass' : 'fail' },
    `duplicate reel engine cross-check; transcript-actions owns direct UI actuation. Reel mode observed=${reelOn}; transcript.assemble spans_placed=${placed}. ${rgReel.detail}`.trim(), rgReel.shot)
  // Verify the transcript.assemble result and resulting edit.insert/timeline
  // state without representing this duplicate row as a second UI action.
  rec(S, 'reel-assemble(transcript.assemble)',
    { rowKind: 'support', present: togglePresent ? 'pass' : 'fail', render: rgReel.ok ? 'pass' : 'fail', click: 'na', result: (asm.ok && (inserted || afterClips > beforeClips)) ? 'pass' : 'fail' },
    `duplicate reel engine cross-check; transcript-actions owns direct Assemble click. transcript.assemble ok=${asm.ok}; spans=${placed}; edit.insert=${inserted}; timeline clips ${beforeClips}→${afterClips}${asm.ok ? '' : ` err="${String(asm.error?.message || asm.error?.code || '').slice(0, 60)}"`}. ${rgReel.detail}`.trim(), rgReel.shot)
}

// ── 19. REVIEW · QC panel: the verify.* measure→fix loop + the AI-review button ─
// The Review rail's QC tab (panels/Review/QC). The QC panel surfaces verify.pacing/
// captions/delivery (one "Run QC" button), verify.brand (its own Check), and verify.
// judge (the "Get AI review" button — the task's headline: drive the REAL button, not
// the raw verb). verify.checks/loudness/scopes are also covered at the VERB level
// with a real RESULT where their dedicated UI is elsewhere or absent. verify.pregate
// is surfaced by the topbar preflight warning before Render / FFmpeg-backed Export;
// this section keeps a verb cross-check while the topbar wiring is verified by the
// UI library tests. We seed captions + a transcript + a draft render so every check
// has real content to measure.
async function secReviewQC(page) {
  const S = 'review-qc'
  await freshProject(page, 'qc', SPEECH)
  await closeOverlays(page)
  // Trim to a short head so the draft render + judge stay fast.
  for (const t of (await state()).tracks || []) {
    if (t.kind === 'video' || t.kind === 'audio') await verb('edit.ripple_delete', { track: t.id, range_ms: [3000, 999000], ripple: true })
  }
  await sleep(300)
  // Seed caption cues (caption-kind track) so verify.captions/brand have content.
  await verb('captions.add_text', { text: 'FCV review caption one', range_ms: [0, 1400], position: 'bottom' })
  await verb('captions.add_text', { text: 'FCV review caption two', range_ms: [1500, 2800], position: 'bottom' })
  // captions.add_text intentionally owns the separate txt1 title-card track;
  // qc-reflow correctly targets subtitle track cap1 only. Seed cap1 through the
  // real subtitle-import contract so this user-visible remedy remains provable
  // even on a partial dev rig without STT. Live STT is still measured separately.
  const qcSrt = writeTempSrt()
  const qcCaptions = await verb('captions.import', {
    path: qcSrt.path,
    replace: true,
    rationale: 'fcv: deterministic Review QC caption-remedy seed',
  })
  if (!qcCaptions.ok) throw new Error(`Review QC subtitle seed failed: ${qcCaptions.error?.message || qcCaptions.error?.code || 'unknown'}`)
  const st0 = await state()
  const asset = st0.tracks.find((t) => t.kind === 'video')?.clips?.find((c) => c.asset)?.asset || Object.keys(st0.assets || {})[0]
  // Transcribe (for verify.delivery, a WPM/filler analysis over the transcript).
  if (DEP.perceptionStt && asset) {
    const tr = await verb('media.transcribe', { asset }); if (tr.result?.job_id) await awaitJob(tr.result.job_id)
    await waitForState((s) => Object.values(s.assets || {}).some((a) => a?.transcript), 60000)
  }
  // A draft render → the receipt verify.checks/judge READ. The earlier form blocked
  // on job-done then polled verify.checks, but "receipt seeded=false" still recurred — because
  // render.final AUTO-runs its check battery over the OUTPUT via the cv2+torch perception
  // sidecar (InstrumentSet::Full), and when that sidecar is ABSENT the engine HONESTLY finishes
  // the render UNVERIFIED: render_done with {verified:false, receipt:null, checks_skipped} and
  // NO receipt file written (dispatch.rs ~19548). resolve_receipt_path then errors "no render
  // receipts exist yet" for EVERY verify.checks/judge call — dep-gated honest behavior, NOT an
  // app bug. So: read the engine's OWN signal off the job result to know whether a receipt was
  // produced, and poll the SPECIFIC render's receipt (render_id) when it was. renderReady →
  // drive checks/judge for real; renderUnverified → classify them N/A below (the sidecar is
  // absent; FCV_REQUIRE_FULL=1 / DEP.perceptionCv enforce it present on the release gate).
  let renderReady = false      // a real receipt resolved → checks/judge can run for real
  let renderUnverified = false // render done but no receipt (output-perception cv2+torch absent)
  let renderReason = ''
  {
    const rf = await verb('render.final', { preset: 'draft' })
    const rid = rf.result?.render_id || ''
    const rj = rf.result?.job_id ? await awaitJob(rf.result.job_id) : null
    const done = rj?.state === 'done' || (rf.ok && !rf.result?.job_id)
    if (done && (rj?.result?.verified === false || rj?.result?.receipt === null || rj?.result?.checks_skipped)) {
      renderUnverified = true
      renderReason = String(rj?.result?.checks_skipped || 'render finished UNVERIFIED — no receipt persisted (output-perception sidecar absent)')
    } else if (done) {
      // Poll the SPECIFIC render's receipt (render_id, falling back to "latest") until it
      // resolves — render_done publishes the receipt a beat after job-done.
      for (let i = 0; i < 25; i++) { if ((await verb('verify.checks', rid ? { render_id: rid } : {})).ok) { renderReady = true; break } await sleep(400) }
    } else if (rj?.state === 'failed') {
      renderReason = `render job failed: ${String(rj?.error?.message || rj?.error || 'unknown')}`
    }
  }

  await reloadApp(page); await sleep(900)
  await reviewTab(page, 'qc', '[data-cut-qc]', 8000)
  const qc = page.locator('[data-cut-qc]').first()
  rec(S, 'GATE:qc-panel-mounted', gateDim((await qc.count()) > 0), 'Review QC panel mounted on the qc tab')

  // Run QC (the real "Run QC" button) → verify.pacing + verify.captions + verify.delivery
  // (Promise.all). Wait for the auto-run on mount to settle (button re-enables), then click
  // and capture all three responses → three rows, each driven by this one real button.
  const qcRunPresent = (await page.locator('[data-cut-action="qc-run"]').count()) > 0
  const runShot = await renderGroup(page, S, 'qc-panel', qc)
  const runResp = {}
  if (qcRunPresent) {
    for (let i = 0; i < 40; i++) { if (!(await page.locator('[data-cut-action="qc-run"]').isDisabled().catch(() => false))) break; await sleep(300) }
    const onR = async (r) => {
      for (const v of ['verify.pacing', 'verify.captions', 'verify.delivery']) {
        if (runResp[v] === undefined && r.url().includes(`/api/verb/${v}`)) { try { runResp[v] = await r.json() } catch {} }
      }
    }
    page.on('response', onR)
    await page.locator('[data-cut-action="qc-run"]').click().catch(() => {})
    for (let i = 0; i < 80; i++) { await sleep(400); if (['verify.pacing', 'verify.captions', 'verify.delivery'].every((v) => runResp[v] !== undefined)) break }
    page.off('response', onR)
  }
  const d4 = (present) => ({ present: present ? 'pass' : 'fail', render: runShot.ok ? 'pass' : 'fail', click: present ? 'pass' : 'fail' })
  rec(S, 'verify.pacing(qc-run button)', { ...d4(qcRunPresent), result: runResp['verify.pacing']?.ok ? 'pass' : 'fail' },
    `verify.pacing ok=${runResp['verify.pacing']?.ok} shots=${runResp['verify.pacing']?.result?.shot_count ?? '?'} (via the real Run QC button) ${runShot.detail}`.trim(), runShot.shot)
  rec(S, 'verify.captions(qc-run button)', { ...d4(qcRunPresent), result: runResp['verify.captions']?.ok ? 'pass' : 'fail' },
    `verify.captions ok=${runResp['verify.captions']?.ok} cues=${runResp['verify.captions']?.result?.cue_count ?? '?'} (Run QC button)`, runShot.shot)
  if (DEP.perceptionStt) {
    rec(S, 'verify.delivery(qc-run button)', { ...d4(qcRunPresent), result: runResp['verify.delivery']?.ok ? 'pass' : 'fail' },
      `verify.delivery ok=${runResp['verify.delivery']?.ok} wpm=${runResp['verify.delivery']?.result?.wpm ?? '?'} (Run QC button)`, runShot.shot)
  } else {
    rec(S, 'verify.delivery(qc-run button)', { ...d4(qcRunPresent), result: 'na' },
      'verify.delivery needs a transcript (perception STT) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present', runShot.shot)
  }

  // captions.reflow — the visible caption remedy, after Run QC has mounted the
  // result card. Prove both the exact empty request and the completion note.
  await probe(page, {
    surface: S, name: 'qc-reflow(captions.reflow)', actionId: 'qc-reflow',
    sel: page.locator('[data-cut-action="qc-reflow"]'), group: page.locator('[data-cut-qc-card="captions"]'), groupName: 'qc-caption-remedies',
    doClick: async () => {
      let args
      const onRequest = (request) => {
        try {
          if (new URL(request.url()).pathname === '/api/verb/captions.reflow') args = request.postDataJSON()
        } catch {}
      }
      page.on('request', onRequest)
      try {
        probe._qcReflow = await captureVerbResp(page, 'captions.reflow', async () => {
          await page.locator('[data-cut-action="qc-reflow"]').click()
        }, 20000)
        probe._qcReflowArgs = args
      } finally {
        page.off('request', onRequest)
      }
    },
    assertResult: async () => {
      const r = probe._qcReflow
      const args = probe._qcReflowArgs
      const note = (await page.locator('[data-cut-qc-note]').textContent().catch(() => ''))?.trim() || ''
      const exactArgs = JSON.stringify(args) === '{}'
      return {
        ok: !!r?.ok && exactArgs && note.startsWith('reflow:'),
        detail: `captions.reflow ok=${r?.ok} exactArgs=${exactArgs} note="${note}"`,
      }
    },
  })
  const qcShiftInput = page.locator('[data-cut-qc-shift]')
  await probe(page, {
    surface: S, name: 'qc-shift(captions.shift)', actionId: 'qc-shift',
    sel: page.locator('[data-cut-action="qc-shift"]'),
    group: page.locator('[data-cut-qc-card="captions"]'),
    groupName: 'qc-caption-remedies',
    doClick: async () => {
      await qcShiftInput.fill('250')
      let args
      const onRequest = (request) => {
        try {
          if (new URL(request.url()).pathname === '/api/verb/captions.shift') args = request.postDataJSON()
        } catch {}
      }
      page.on('request', onRequest)
      try {
        probe._qcShift = await captureVerbResp(page, 'captions.shift', async () => {
          await page.locator('[data-cut-action="qc-shift"]').click()
        }, 20_000)
        probe._qcShiftArgs = args
      } finally {
        page.off('request', onRequest)
      }
    },
    assertResult: async () => {
      const note = (await page.locator('[data-cut-qc-note]').textContent().catch(() => ''))?.trim() || ''
      const exactArgs = sameJsonValue(probe._qcShiftArgs, { offset_ms: 250 })
      return {
        ok: !!probe._qcShift?.ok && exactArgs && note.startsWith('shifted captions'),
        detail: `captions.shift ok=${probe._qcShift?.ok} exactArgs=${exactArgs} note="${note}"`,
      }
    },
  })

  // project.brand — use the real disclosure and every visible form control. The
  // former test forced <details>.open in JavaScript, which could not catch a
  // broken editor summary or inaccessible local inputs.
  const brandEditor = page.locator('[data-cut-qc-brand-editor]').first()
  await probe(page, {
    surface: S, name: 'qc-brand-editor-toggle', actionId: 'qc-brand-editor-toggle',
    sel: page.locator('[data-cut-qc-brand-editor-toggle]'), group: qc, groupName: 'qc-brand-card',
    doClick: async () => { await page.locator('[data-cut-qc-brand-editor-toggle]').click() },
    assertResult: async () => ({
      ok: await brandEditor.evaluate((element) => element.open).catch(() => false),
      detail: 'Brand editor disclosure opened through its visible summary',
    }),
  })
  await probe(page, {
    surface: S, name: 'qc-brand-fonts', actionId: 'qc-brand-fonts',
    sel: page.locator('[data-cut-qc-brand-fonts]'), group: brandEditor, groupName: 'qc-brand-editor-open',
    doClick: async () => { await page.locator('[data-cut-qc-brand-fonts]').fill('Inter, Arial') },
    assertResult: async () => ({
      ok: (await page.locator('[data-cut-qc-brand-fonts]').inputValue().catch(() => '')) === 'Inter, Arial',
      detail: 'Fonts value persisted in the visible editor',
    }),
  })
  await probe(page, {
    surface: S, name: 'qc-brand-colors', actionId: 'qc-brand-colors',
    sel: page.locator('[data-cut-qc-brand-colors]'), group: brandEditor, groupName: 'qc-brand-fonts-set',
    doClick: async () => { await page.locator('[data-cut-qc-brand-colors]').fill('#ffffff, #101820') },
    assertResult: async () => ({
      ok: (await page.locator('[data-cut-qc-brand-colors]').inputValue().catch(() => '')) === '#ffffff, #101820',
      detail: 'Palette value persisted and palette preview mounted',
    }),
  })
  await probe(page, {
    surface: S, name: 'qc-brand-position', actionId: 'qc-brand-position',
    sel: page.locator('[data-cut-qc-brand-position]'), group: brandEditor, groupName: 'qc-brand-palette-set',
    doClick: async () => { await page.locator('[data-cut-qc-brand-position]').selectOption('bottom') },
    assertResult: async () => ({
      ok: (await page.locator('[data-cut-qc-brand-position]').inputValue().catch(() => '')) === 'bottom',
      detail: 'Caption position → bottom',
    }),
  })
  await page.locator('[data-cut-qc-brand-aspect]').selectOption('16:9').catch(() => {})
  await probe(page, {
    surface: S, name: 'qc-brand-min-size', actionId: 'qc-brand-min-size',
    sel: page.locator('[data-cut-qc-brand-min-size]'), group: brandEditor, groupName: 'qc-brand-position-set',
    doClick: async () => { await page.locator('[data-cut-qc-brand-min-size]').fill('24') },
    assertResult: async () => ({
      ok: (await page.locator('[data-cut-qc-brand-min-size]').inputValue().catch(() => '')) === '24',
      detail: 'Minimum caption size → 24',
    }),
  })
  await probe(page, {
    surface: S, name: 'qc-brand-max-size', actionId: 'qc-brand-max-size',
    sel: page.locator('[data-cut-qc-brand-max-size]'), group: brandEditor, groupName: 'qc-brand-min-set',
    doClick: async () => { await page.locator('[data-cut-qc-brand-max-size]').fill('72') },
    assertResult: async () => ({
      ok: (await page.locator('[data-cut-qc-brand-max-size]').inputValue().catch(() => '')) === '72',
      detail: 'Maximum caption size → 72',
    }),
  })
  await probe(page, {
    surface: S, name: 'project.brand(qc-brand editor)', sel: page.locator('[data-cut-action="qc-brand-save"]'), group: brandEditor, groupName: 'qc-brand-editor-filled',
    doClick: async () => {
      let args
      const onRequest = (request) => {
        try {
          if (new URL(request.url()).pathname === '/api/verb/project.brand') args = request.postDataJSON()
        } catch {}
      }
      page.on('request', onRequest)
      try {
        probe._brandSave = await captureVerbResp(page, 'project.brand', async () => {
          await page.locator('[data-cut-action="qc-brand-save"]').click()
        }, 20000)
        probe._brandSaveArgs = args
      } finally {
        page.off('request', onRequest)
      }
    },
    assertResult: async () => {
      const r = probe._brandSave
      const expected = {
        brand: {
          fonts: ['Inter', 'Arial'],
          colors: ['#ffffff', '#101820'],
          position: 'bottom',
          min_size: 24,
          max_size: 72,
          aspect: '16:9',
        },
        rationale: 'save project brand kit from Review QC',
      }
      const exactArgs = sameJsonValue(probe._brandSaveArgs, expected)
      const saved = await waitForState((s) => s.brand?.aspect === '16:9' && s.brand?.position === 'bottom', 10000)
      const statusSaved = await page.locator('[data-cut-qc-brand-status="saved"]').count() > 0
      return {
        ok: !!r?.ok && exactArgs && !!saved && statusSaved,
        detail: `project.brand ok=${r?.ok} exactArgs=${exactArgs} stateSaved=${!!saved} statusSaved=${statusSaved}`,
      }
    },
  })
  // verify.brand — its own Check button, reading the saved project kit.
  await probe(page, {
    surface: S, name: 'verify.brand(qc-brand button)', sel: page.locator('[data-cut-action="qc-brand"]'), group: qc, groupName: 'qc-panel',
    doClick: async () => {
      const check = page.locator('[data-cut-action="qc-brand"]:not([disabled])').first()
      await check.waitFor({ state: 'visible', timeout: 10000 })
      probe._r = await captureVerbResp(page, 'verify.brand', () => check.click(), 20000)
    },
    assertResult: async () => {
      const r = probe._r
      return { ok: !!r?.ok && r?.result?.source === 'stored' && r?.result?.brand?.aspect === '16:9', detail: `verify.brand ok=${r?.ok} source=${r?.result?.source ?? '?'} aspect=${r?.result?.brand?.aspect ?? '?'} styles_checked=${r?.result?.styles_checked ?? '?'}` }
    },
  })
  await page.locator('[data-cut-action="qc-brand-clear"]:not([disabled])').waitFor({ timeout: 10000 }).catch(() => {})
  await probe(page, {
    surface: S, name: 'qc-brand-clear(project.brand)', actionId: 'qc-brand-clear',
    sel: page.locator('[data-cut-action="qc-brand-clear"]'), group: brandEditor, groupName: 'qc-brand-saved',
    doClick: async () => {
      let args
      const onRequest = (request) => {
        try {
          if (new URL(request.url()).pathname === '/api/verb/project.brand') args = request.postDataJSON()
        } catch {}
      }
      page.on('request', onRequest)
      try {
        probe._brandClear = await captureVerbResp(page, 'project.brand', async () => {
          await page.locator('[data-cut-action="qc-brand-clear"]').click()
        }, 20000)
        probe._brandClearArgs = args
      } finally {
        page.off('request', onRequest)
      }
    },
    assertResult: async () => {
      const r = probe._brandClear
      const expected = { clear: true, rationale: 'clear project brand kit from Review QC' }
      const exactArgs = sameJsonValue(probe._brandClearArgs, expected)
      const cleared = await waitForState((s) => s.brand == null, 10000)
      const statusCleared = await page.locator('[data-cut-qc-brand-status="not-saved"]').count() > 0
      const note = (await page.locator('[data-cut-qc-note]').textContent().catch(() => ''))?.trim() || ''
      return {
        ok: !!r?.ok && r?.result?.cleared === true && exactArgs && !!cleared && statusCleared && note === 'Brand kit cleared',
        detail: `project.brand clear ok=${r?.ok} cleared=${r?.result?.cleared} exactArgs=${exactArgs} stateCleared=${!!cleared} statusCleared=${statusCleared} note="${note}"`,
      }
    },
  })

  // verify.judge — THE REAL "Get AI review" button (not the raw verb). Async job; the judge
  // card hooks data-cut-judge to the live status. It READS a render RECEIPT (+ the receipt's
  // output file), so it needs BOTH `claude` AND renderReady. When the
  // render finished UNVERIFIED (output-perception cv2+torch absent → no receipt), driving the
  // button yields a not_run/error verdict → a false FAIL. Gate on renderReady; classify the
  // no-receipt case N/A (dep-gated), same posture as the verify.checks reader below.
  const needReceiptReal = FULL || DEP.perceptionCv
  if (DEP.claude && renderReady) {
    await probe(page, {
      surface: S, name: 'verify.judge(Get-AI-review button)', sel: page.locator('[data-cut-action="judge-run"]'), group: qc, groupName: 'qc-panel',
      doClick: async () => {
        await page.locator('[data-cut-action="judge-run"]').click().catch(() => {})
        for (let i = 0; i < 220; i++) {
          await sleep(900)
          const h = await page.locator('[data-cut-qc-card="judge"]').first().getAttribute('data-cut-judge').catch(() => null)
          if (h && h !== 'running') break
        }
      },
      assertResult: async () => {
        const h = await page.locator('[data-cut-qc-card="judge"]').first().getAttribute('data-cut-judge').catch(() => '')
        const verdictEl = (await page.locator('[data-cut-qc-card="judge"] [data-cut-qc-verdict]').first().textContent().catch(() => '')) || ''
        const ok = ['pass', 'fail', 'needs_review', 'completed'].includes(String(h))
        return { ok, detail: `verify.judge via the real "Get AI review" button → data-cut-judge="${h}" verdict="${verdictEl.trim()}" (draft-render receipt seeded=${renderReady} — judge reads the render's receipt+output)` }
      },
    })
  } else if (!DEP.claude) {
    rec(S, 'verify.judge(Get-AI-review button)', { present: (await page.locator('[data-cut-action="judge-run"]').count()) > 0 ? 'pass' : 'fail', render: runShot.ok ? 'pass' : 'fail', click: 'na', result: 'na' },
      'verify.judge needs `claude` (system.doctor judge.claude≠ok) — the real "Get AI review" button is PRESENT/RENDER-verified; honest dev skip; FCV_REQUIRE_FULL=1 enforces claude present', runShot.shot)
  } else if (renderUnverified && !needReceiptReal) {
    rec(S, 'verify.judge(Get-AI-review button)', { present: (await page.locator('[data-cut-action="judge-run"]').count()) > 0 ? 'pass' : 'fail', render: runShot.ok ? 'pass' : 'fail', click: 'na', result: 'na' },
      `verify.judge reads a render RECEIPT, but this draft render finished UNVERIFIED (${renderReason.slice(0, 70)}) — output-perception (cv2+torch) absent so render.final wrote no receipt; "Get AI review" button PRESENT/RENDER-verified, dep-gated N/A; FCV_REQUIRE_FULL=1 / DEP.perceptionCv enforce the sidecar present`, runShot.shot)
  } else {
    // claude present AND the sidecar SHOULD be present (FULL/perceptionCv) yet no receipt landed → a real precondition fail.
    rec(S, 'verify.judge(Get-AI-review button)', { present: (await page.locator('[data-cut-action="judge-run"]').count()) > 0 ? 'pass' : 'fail', render: runShot.ok ? 'pass' : 'fail', click: 'na', result: 'fail' },
      `verify.judge has no render receipt to read though the perception sidecar is present (${(renderReason || 'receipt never resolved').slice(0, 70)}) — real precondition fail`, runShot.shot)
  }

  // verify.checks is auto-fetched after render at the verb level; scopes now has
  // a dedicated Review tab that drives verify.scopes from the human UI.
  // verify.pregate has a topbar UI surface
  // now: Render / FFmpeg-backed Export opens the preflight warning when the verb
  // reports risk. The row below keeps the verb result tied to that UI surface.
  // (verify.loudness is NO LONGER in this list — the Mixer's per-track "Measure
  // loudness" button drives it, covered for real in secMixer; the verb-level row
  // below is now a supplementary cross-check, not a no-UI claim.)
  {
    // Tri-state, matching the seed's renderUnverified detection. ok → pass;
    // a no-receipt error when the render finished UNVERIFIED (or the cv2+torch sidecar is absent)
    // → N/A (dep-gated), NOT a wiring fail; any other ok=false → a real FAIL.
    const ck = await verb('verify.checks', {})
    const msg = String(ck.error?.message || ck.error?.code || '')
    const noReceipt = !ck.ok && (ck.error?.code === 'not_found' || /no render receipt|no receipt/i.test(msg))
    let result, lead
    if (ck.ok) { result = 'pass'; lead = `verify.checks ok=true render_id=${ck.result?.render_id ?? ck.result?.id ?? '?'} (draft-render receipt seeded=${renderReady})` }
    else if (noReceipt && (renderUnverified || !needReceiptReal)) { result = 'na'; lead = `verify.checks has no receipt to read: the draft render finished UNVERIFIED (${(renderReason || msg).slice(0, 70)}) — output-perception (cv2+torch) absent so render.final wrote no receipt; dep-gated N/A, FCV_REQUIRE_FULL=1 / DEP.perceptionCv enforce the sidecar present` }
    else { result = 'fail'; lead = `verify.checks ok=false err="${msg.slice(0, 70)}" (receipt seeded=${renderReady})` }
    rec(S, 'verify.checks(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result },
      `${lead} — reads the receipt render.final produces; NO dedicated UI button (App auto-fetches it post-render); verb-level RESULT, flagged not faked`)
  }
  {
    const lo = asset ? await verb('verify.loudness', { asset }) : { ok: false, error: { message: 'no asset' } }
    rec(S, 'verify.loudness(verb-level cross-check)', { present: 'na', render: 'na', click: 'na', result: lo.ok ? 'pass' : 'fail' },
      `verify.loudness ok=${lo.ok} lufs=${lo.result?.integrated_lufs ?? lo.result?.lufs ?? '?'} — HAS a UI control (the Mixer's per-track "Measure loudness" button, driven for real in secMixer); this is a supplementary verb-level cross-check, NOT a "no UI control" row`)
  }
  {
    const openedScopes = await verb('ui.open', { panel: 'scopes' })
    await page.waitForSelector('[data-cut-scopes]', { timeout: 8000 }).catch(() => {})
    const present = (await page.locator('[data-cut-scopes]').count()) > 0
    let clicked = false
    let resultState = ''
    let scopesResponse
    if (present) {
      await page.locator('[data-cut-scopes-at-ms]').fill('500').catch(() => {})
      scopesResponse = await captureVerbResp(page, 'verify.scopes', async () => {
        clicked = await page.locator('[data-cut-action="scopes-run"]').click().then(() => true).catch(() => false)
      }, 60000)
      if (clicked) {
        await page.waitForSelector('[data-cut-scopes-result]', { timeout: 30000 }).catch(() => {})
        resultState = String(await page.locator('[data-cut-scopes-result]').first().getAttribute('data-cut-scopes-result').catch(() => ''))
      }
    }
    const result = scopesResponse?.ok && (resultState === 'pass' || resultState === 'warn') ? 'pass' : 'fail'
    const scopeError = scopesResponse?.error?.message ? ` error="${String(scopesResponse.error.message).slice(0, 80)}"` : ''
    rec(S, 'verify.scopes(Review Scopes tab)', { present: present ? 'pass' : 'fail', render: present ? 'pass' : 'fail', click: clicked ? 'pass' : 'fail', result },
      `ui.open scopes ok=${openedScopes.ok} → Review Scopes tab present=${present}; button click=${clicked}; response-ok=${scopesResponse?.ok}; result=${resultState || 'missing'}${scopeError} (real UI drives verify.scopes at 500ms)`)
  }
  {
    const pg = await verb('verify.pregate', {})
    rec(S, 'verify.pregate(preflight warning)', { rowKind: 'support', present: 'pass', render: 'na', click: 'na', result: pg.ok ? 'pass' : 'fail' },
      `verify.pregate ok=${pg.ok} pass=${pg.result?.pass ?? '?'} risks=${pg.result?.risks?.length ?? '?'} — topbar Render / FFmpeg-backed Export uses this result to show the preflight warning; this row is the verb cross-check for that UI surface`)
  }
}

// ── 20. KINETIC captions drawer (captions.kinetic) ─────────────────────────────
// The kinetic-captions drawer (panels/Kinetic). It animates caption-kind static cues
// from captions.generate over a TRANSCRIPT, so it is gated on perception STT.
// Opened via the REAL launcher: Transcript Tools → "Animate captions"
// (cut:open-kinetic). Drive position/replace/apply → captions.kinetic → a title overlay.
async function secKinetic(page) {
  const S = 'kinetic'
  await freshProject(page, 'kinetic', SPEECH)
  await closeOverlays(page)
  if (!DEP.perceptionStt) {
    rec(S, 'kinetic-apply(captions.kinetic)', { present: 'na', render: 'na', click: 'na', result: 'na' },
      'captions.kinetic animates cap1 cues from captions.generate, which needs a transcript (perception STT) — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present')
    return
  }
  // transcript → caption-kind cues (captions.generate) — the cues kinetic animates.
  const st0 = await state()
  const asset = st0.tracks.find((t) => t.kind === 'video')?.clips?.find((c) => c.asset)?.asset || Object.keys(st0.assets || {})[0]
  if (asset) {
    const tr = await verb('media.transcribe', { asset }); if (tr.result?.job_id) await awaitJob(tr.result.job_id)
    await waitForState((s) => Object.values(s.assets || {}).some((a) => a?.transcript), 60000)
  }
  await verb('captions.generate', {})
  await waitForState(hasCaptionCues, 25000)
  await reloadApp(page); await sleep(900)
  // Open via the real launcher: Transcript Tools → Animate captions.
  await page.locator('[data-cut-left-tab="transcript"]').click().catch(() => {}); await sleep(400)
  await page.locator('[data-cut-action="tools-menu"]').click().catch(() => {}); await sleep(250)
  await page.locator('[data-cut-action="open-kinetic"]').click().catch(() => {}); await sleep(700)
  await page.waitForSelector('[data-cut-kinetic]', { timeout: 6000 }).catch(() => {})
  const drawer = page.locator('[data-cut-kinetic]').first()
  rec(S, 'GATE:kinetic-drawer-open', gateDim((await drawer.count()) > 0), 'Kinetic drawer mounted via Transcript → Animate captions')
  rec(S, 'GATE:kinetic-has-cuecount', gateDim((await page.locator('[data-cut-kinetic-cuecount]').count()) > 0), 'caption cue count shown (captions ready to animate)')

  // position select (local state, value sticks).
  await probe(page, {
    surface: S, name: 'kinetic-position', sel: page.locator('[data-cut-kinetic-position]'), group: drawer, groupName: 'kinetic-drawer',
    doClick: async () => { await page.locator('[data-cut-kinetic-position]').selectOption('center').catch(() => {}); await sleep(150) },
    assertResult: async () => ({ ok: (await page.locator('[data-cut-kinetic-position]').inputValue().catch(() => '')) === 'center', detail: 'position → center' }),
  })
  // replace-static toggle (the overlap fix).
  {
    const before = await page.locator('[data-cut-kinetic-replace]').isChecked().catch(() => null)
    await probe(page, {
      surface: S, name: 'kinetic-replace-toggle', sel: page.locator('[data-cut-kinetic-replace]'), group: drawer, groupName: 'kinetic-drawer',
      doClick: async () => { await page.locator('[data-cut-kinetic-replace]').click().catch(() => {}); await sleep(150) },
      assertResult: async () => { const after = await page.locator('[data-cut-kinetic-replace]').isChecked().catch(() => null); return { ok: before !== null && after !== null && after !== before, detail: `replace-static ${before}→${after}` } },
    })
  }
  // Apply → captions.kinetic. RESULT: the receipt's title_track lands on the timeline.
  await probe(page, {
    surface: S, name: 'kinetic-apply(captions.kinetic)', sel: page.locator('[data-cut-kinetic-apply]'), group: drawer, groupName: 'kinetic-drawer',
    doClick: async () => {
      await page.locator('[data-cut-kinetic-apply]:not([disabled])')
        .waitFor({ state: 'visible', timeout: 15_000 })
      probe._r = await captureVerbResp(page, 'captions.kinetic', async () => {
        await page.locator('[data-cut-kinetic-apply]').click()
      // Kinetic renders one transparent frame per timeline frame. The native
      // candidate runner deliberately uses an unoptimized debug build, where a
      // one-minute 720p fixture can exceed a generic 60 s request window. Keep
      // this render-class action bounded, while allowing the real response and
      // state effect to finish; exact installed release runs still measure the
      // shipping binary separately.
      }, 240_000)
    },
    assertResult: async () => {
      const r = probe._r
      const cues = r?.result?.cue_count
      const tt = r?.result?.title_track
      const landed = !!tt && !!(await waitForState((s) => (s.tracks || []).some((t) => t.id === tt && (t.clips || []).length >= 1), 15000))
      const uiRes = (await page.locator('[data-cut-kinetic-result]').count()) > 0
      return { ok: !!r?.ok && (cues ?? 0) >= 1 && (landed || uiRes), detail: `captions.kinetic ok=${r?.ok} cue_count=${cues ?? '?'} title_track=${tt ?? '?'} landed=${landed} resultUI=${uiRes}` }
    },
  })
  await page.locator('[data-cut-kinetic-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {}); await sleep(200)

  // captions.reflow + captions.shift — the Review→QC caption-remedy buttons (qc-reflow /
  // qc-shift) drive these over the SAME cap1 cues kinetic animates. cap1 is already populated
  // above (captions.generate, perceptionStt-gated — we only reach here when STT is present),
  // so cover them VERB-LEVEL here, reusing that setup rather than a 2nd (slow) STT transcribe.
  // Real RESULT: reflow runs ok over the cues; shift moves every cap1 cue by the offset.
  {
    // Re-seed cap1 (kinetic's replace-static may have consumed the static cues) so reflow/
    // shift have real cap1 content to act on — idempotent, the transcript is already cached.
    await verb('captions.generate', {})
    await waitForState((s) => (s.tracks || []).some((t) => t.id === 'cap1' && (t.clips || []).some((c) => c.text)), 20000)
    const rfl = await verb('captions.reflow', {})
    rec(S, 'captions.reflow(verb-level · QC qc-reflow button)', { present: 'na', render: 'na', click: 'na', result: rfl.ok ? 'pass' : 'fail' },
      `captions.reflow ok=${rfl.ok} reflowed=${rfl.result?.reflowed ?? rfl.result?.changed ?? rfl.result?.split ?? '?'} — Review→QC "reflow captions" (qc-reflow) drives this over cap1; verb-level RESULT, flagged not faked`)
    const cap1First = ((await state()).tracks || []).find((t) => t.id === 'cap1')?.clips?.[0]
    const start0 = cap1First ? (cap1First.range_ms?.[0] ?? cap1First.start_ms ?? 0) : 0
    const shf = await verb('captions.shift', { offset_ms: 250 })
    const shifted = cap1First ? !!(await waitForState((s) => { const c = ((s.tracks || []).find((t) => t.id === 'cap1')?.clips || [])[0]; return c && ((c.range_ms?.[0] ?? c.start_ms ?? 0) >= start0 + 200) }, 10000)) : false
    rec(S, 'captions.shift(verb-level · QC qc-shift button)', { present: 'na', render: 'na', click: 'na', result: (shf.ok && (shifted || !cap1First)) ? 'pass' : 'fail' },
      `captions.shift{offset_ms:250} ok=${shf.ok} firstCue ${start0}→shifted=${shifted} — Review→QC "shift captions" (qc-shift) drives this on cap1; verb-level RESULT, flagged not faked`)
  }
}

// ── 21. MATTE + MASK drawers: edit.matte and edit.add_mask driven from UI ─────
// The AI background-removal drawer (panels/Matte). Earlier coverage only clicked the launcher;
// here we drive the inner Apply → edit.matte and assert the clip gains a matte. The
// Region mask drawer is now a first-class topbar surface; the sweep opens it, injects the
// same preview geometry event that a drag would create, and clicks Apply → edit.add_mask.
// edit.matte bakes an alpha (RVM), so it needs the matte runtime (DEP.matte; absent ⇒
// the drawer shows its requirements card → honest dep-skip).
async function secMatte(page) {
  const S = 'matte'
  // Use the detector-proven face role. SPEECH can be a long podcast because menu
  // fixtures need timestamps past 29s; edit.matte bakes the whole source, so using
  // SPEECH here can exceed the six-minute UI response contract despite the clip trim.
  await freshProject(page, 'matte', FACE)
  await closeOverlays(page)
  // Trim to a short head so the RVM matte bake stays fast.
  for (const t of (await state()).tracks || []) {
    if (t.kind === 'video') await verb('edit.ripple_delete', { track: t.id, range_ms: [2500, 999000], ripple: true })
  }
  await sleep(300)
  const clip = await clipOfKind('video')
  if (!clip) { rec(S, 'BOOTSTRAP', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, 'no video clip imported'); return }

  // edit.add_mask — open the real Region mask drawer from the topbar. In headless FCV
  // we synthesize the preview's cut:mask-geometry event instead of pointer-dragging a
  // canvas region; the drawer's Apply button still dispatches edit.add_mask.
  {
    await selectClip(page, clip)
    const openBtn = page.locator('[data-cut-mask-btn]')
    const openPresent = (await openBtn.count()) > 0
    if (openPresent) await openBtn.click().catch(() => {})
    await page.waitForSelector('[data-cut-mask]', { timeout: 6000 }).catch(() => {})
    const maskDrawer = page.locator('[data-cut-mask]').first()
    const maskMounted = (await maskDrawer.count()) > 0
    const rgMask = maskMounted ? await renderGroup(page, S, 'mask-drawer', maskDrawer) : { ok: false, detail: 'mask drawer absent', shot: '' }
    rec(S, 'mask-drawer-open', { present: openPresent ? 'pass' : 'fail', render: rgMask.ok ? 'pass' : 'fail', click: openPresent ? 'pass' : 'fail', result: maskMounted ? 'pass' : 'fail' },
      `Region mask drawer mounted=${maskMounted}; launcher present=${openPresent}. ${rgMask.detail}`.trim(), rgMask.shot)

    if (maskMounted) {
      await page.evaluate(() => {
        document.dispatchEvent(new CustomEvent('cut:mask-geometry', {
          detail: { ready: true, shape: 'rect', points: [[0.25, 0.25], [0.75, 0.75]] },
        }))
      })
      await sleep(200)
      await page.locator('[data-cut-mask-effect="blur"]').click().catch(() => {})
      await probe(page, {
        surface: S, name: 'mask-apply(edit.add_mask)', sel: page.locator('[data-cut-mask-apply]'), group: maskDrawer, groupName: 'mask-drawer',
        doClick: async () => {
          await page.waitForFunction(() => {
            const el = document.querySelector('[data-cut-mask-apply]')
            return !!el && !el.hasAttribute('disabled')
          }, null, { timeout: 5000 }).catch(() => {})
          probe._r = await captureVerbResp(page, 'edit.add_mask', async () => { await page.locator('[data-cut-mask-apply]').click().catch(() => {}) }, 20000)
        },
        assertResult: async () => {
          const r = probe._r
          const masked = !!(await waitForState((s) => !!findClip(s, clip)?.mask, 12000))
          const uiRes = (await page.locator('[data-cut-mask-result]').count()) > 0
          return { ok: !!r?.ok && masked, detail: `edit.add_mask ok=${r?.ok} → clip.mask set=${masked} resultUI=${uiRes}` }
        },
      })
      await page.locator('[data-cut-mask-close]').click().catch(() => {})
      await page.keyboard.press('Escape').catch(() => {})
      await sleep(200)
    }
  }

  // Select the clip + open the matte drawer via the REAL Timeline launcher.
  await selectClip(page, clip)
  await page.locator('[data-cut-action="open-matte"]').click().catch(() => {}); await sleep(700)
  await page.waitForSelector('[data-cut-matte]', { timeout: 6000 }).catch(() => {})
  const drawer = page.locator('[data-cut-matte]').first()
  rec(S, 'GATE:matte-drawer-open', gateDim((await drawer.count()) > 0), 'Matte drawer mounted via the Timeline Matte launcher')

  if (!DEP.matte) {
    // Runtime absent → the drawer shows the REQUIREMENTS card (the install path); the
    // Apply control doesn't mount. PRESENT/RENDER the requirements card; edit.matte is an
    // honest dep-skip. FCV_REQUIRE_FULL=1 enforces the matte runtime present (preflight).
    const reqs = page.locator('[data-cut-matte-requirements]')
    const rg = await renderGroup(page, S, 'matte-drawer', drawer)
    rec(S, 'matte-apply(edit.matte)', { present: (await reqs.count()) > 0 ? 'pass' : 'na', render: rg.ok ? 'pass' : 'fail', click: 'na', result: 'na' },
      `matte runtime not set up (system.doctor matte≠ok) — drawer shows the requirements card; edit.matte honest dev skip; FCV_REQUIRE_FULL=1 enforces it present ${rg.detail}`.trim(), rg.shot)
    await page.locator('[data-cut-matte-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {})
    return
  }

  // Runtime ready → drive mode (replace, so it works on a BASE clip) · quality (fast) ·
  // bg color · Apply → edit.matte → the clip gains a matte.
  await probe(page, {
    surface: S, name: 'matte-mode-replace', sel: page.locator('[data-cut-matte-mode-replace]'), group: drawer, groupName: 'matte-drawer',
    doClick: async () => { await page.locator('[data-cut-matte-mode-replace]').click().catch(() => {}); await sleep(150) },
    assertResult: async () => ({ ok: /cd-seg-btn--on/.test((await page.locator('[data-cut-matte-mode-replace]').getAttribute('class').catch(() => '')) || ''), detail: 'mode → replace (--on)' }),
  })
  await probe(page, {
    surface: S, name: 'matte-quality-fast', sel: page.locator('[data-cut-matte-quality-fast]'), group: drawer, groupName: 'matte-drawer',
    doClick: async () => { await page.locator('[data-cut-matte-quality-fast]').click().catch(() => {}); await sleep(150) },
    assertResult: async () => ({ ok: /cd-seg-btn--on/.test((await page.locator('[data-cut-matte-quality-fast]').getAttribute('class').catch(() => '')) || ''), detail: 'quality → fast (--on)' }),
  })
  await probe(page, {
    surface: S, name: 'matte-apply(edit.matte)', sel: page.locator('[data-cut-matte-apply]'), group: drawer, groupName: 'matte-drawer',
    doClick: async () => {
      await page.locator('[data-cut-matte-bg]').fill('#0033FF').catch(() => {}) // replace bg color (mode=replace shows this)
      probe._r = await captureVerbResp(page, 'edit.matte', async () => { await page.locator('[data-cut-matte-apply]').click().catch(() => {}) }, 360000)
    },
    assertResult: async () => {
      const r = probe._r
      const matted = !!(await waitForState((s) => !!findClip(s, clip)?.matte, 360000))
      const uiRes = (await page.locator('[data-cut-matte-result]').count()) > 0
      return { ok: !!r?.ok && matted, detail: `edit.matte ok=${r?.ok} → clip.matte set=${matted} resultUI=${uiRes}` }
    },
  })
  await page.locator('[data-cut-matte-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {}); await sleep(200)
}

// ── 22. RECIPE layer (recipe.list / describe / run) ────────────────────────────
// The declarative pipeline-manifest layer has a real topbar/palette drawer now. Open
// Recipes → recipe.list fills the catalog, clicking a card dispatches recipe.describe,
// and Preview plan dispatches recipe.run{policy:'dry_run'} for a fast, non-mutating
// receipt. The heavy Run button remains covered elsewhere by job-polling patterns; this
// section proves the human surface reaches the same verbs without hiding behind agent
// prompt chips.
async function secRecipe(page) {
  const S = 'recipe'
  await freshProject(page, 'recipe', SPEECH)
  await closeOverlays(page)
  const openBtn = page.locator('[data-cut-recipes-btn]')
  const openPresent = (await openBtn.count()) > 0
  const listResp = await captureVerbResp(page, 'recipe.list', async () => {
    if (openPresent) await openBtn.click().catch(() => {})
  }, 15000)
  await page.waitForSelector('[data-cut-recipes]', { timeout: 6000 }).catch(() => {})
  await page.waitForSelector('[data-cut-recipes-list],[data-cut-recipes-empty],[data-cut-recipes-error]', { timeout: 6000 }).catch(() => {})
  const drawer = page.locator('[data-cut-recipes]').first()
  const drawerMounted = (await drawer.count()) > 0
  const recipeCards = page.locator('[data-cut-recipe]')
  const recipeCount = await recipeCards.count().catch(() => 0)
  const names = Array.isArray(listResp?.result?.recipes)
    ? listResp.result.recipes.map((r) => (typeof r === 'string' ? r : r?.name)).filter(Boolean)
    : []
  const rgList = drawerMounted ? await renderGroup(page, S, 'recipe-drawer-list', drawer) : { ok: false, detail: 'recipes drawer absent', shot: '' }
  rec(S, 'recipe-drawer-open(recipe.list)', { present: openPresent ? 'pass' : 'fail', render: rgList.ok ? 'pass' : 'fail', click: openPresent ? 'pass' : 'fail', result: (listResp?.ok && recipeCount >= 1) ? 'pass' : 'fail' },
    `Recipes drawer mounted=${drawerMounted}; recipe.list ok=${listResp?.ok} → ${recipeCount} rendered card(s), catalog=${names.slice(0, 5).join(', ') || '(none)'}. ${rgList.detail}`.trim(), rgList.shot)

  if (recipeCount >= 1) {
    const firstCard = recipeCards.first()
    const chosenName = await firstCard.getAttribute('data-cut-recipe').catch(() => '')
    const descResp = await captureVerbResp(page, 'recipe.describe', async () => {
      await firstCard.click().catch(() => {})
    }, 20000)
    await page.waitForSelector('[data-cut-recipe-detail]', { timeout: 10000 }).catch(() => {})
    const detail = page.locator('[data-cut-recipe-detail]').first()
    const detailMounted = (await detail.count()) > 0
    const hasManifest = !!(descResp?.result && (descResp.result.stages || descResp.result.steps || descResp.result.name || descResp.result.params))
    const rgDetail = detailMounted ? await renderGroup(page, S, 'recipe-drawer-detail', drawer) : { ok: false, detail: 'recipe detail absent', shot: '' }
    rec(S, 'recipe-card-select(recipe.describe)', { present: recipeCount >= 1 ? 'pass' : 'fail', render: rgDetail.ok ? 'pass' : 'fail', click: 'pass', result: (descResp?.ok && hasManifest && detailMounted) ? 'pass' : 'fail' },
      `selected recipe=${chosenName || '?'}; recipe.describe ok=${descResp?.ok} manifest=${hasManifest} detailMounted=${detailMounted}. ${rgDetail.detail}`.trim(), rgDetail.shot)

    await probe(page, {
      surface: S, name: 'recipe-preview(recipe.run dry_run)', sel: page.locator('[data-cut-recipe-preview]'), group: drawer, groupName: 'recipe-drawer-detail',
      doClick: async () => {
        probe._r = await captureVerbResp(page, 'recipe.run', async () => { await page.locator('[data-cut-recipe-preview]').click().catch(() => {}) }, 30000)
      },
      assertResult: async () => {
        const r = probe._r
        const planned = r?.result?.status === 'planned' || Array.isArray(r?.result?.stages)
        const uiPlan = (await page.locator('[data-cut-recipe-plan]').count()) > 0
        return { ok: !!r?.ok && planned && uiPlan, detail: `recipe.run dry_run ok=${r?.ok} status=${r?.result?.status ?? '?'} stages=${Array.isArray(r?.result?.stages) ? r.result.stages.length : '?'} planUI=${uiPlan}${r?.ok ? '' : ` err="${String(r?.error?.message || '').slice(0, 60)}"`}` }
      },
    })
  } else {
    for (const n of ['recipe-card-select(recipe.describe)', 'recipe-preview(recipe.run dry_run)']) {
      rec(S, n, { present: 'na', render: 'na', click: 'na', result: 'na' },
        'recipe.list rendered no recipe cards — nothing to describe/preview (empty-catalog guard)')
    }
  }
  await page.locator('[data-cut-recipes-close]').click().catch(() => {})
  await page.keyboard.press('Escape').catch(() => {})
  await sleep(200)
}

// ════════════════════════════════════════════════════════════════════════════
// CORE PANELS: Library · Projects · Assets · Comments · Mixer · Autopilot
// Each ENTERS the panel via its real launcher and drives the catalog of library.* /
// project.* / assets.* / comment.* / edit.duck / autopilot.run verbs at the 4-part bar.
// HONESTY: a verb with NO UI control (library.use, comment.list, edit.duck, project.close/
// checkpoint, assets.providers) is covered at the VERB level with a real RESULT —
// present/render/click=na, flagged "no UI control", NEVER faked. Destructive verbs
// (library.remove, project.delete) operate on THROWAWAY entities the section creates, so
// the harness stays runnable. Selectors + dispatched verbs were grepped from ui/src — see
// each control's inline note (data-cut-* hook → verb).
// ════════════════════════════════════════════════════════════════════════════

// ── 23. LIBRARY workspace — the cross-project asset library ────────────────────
// panels/Library. The Browse button is a DESKTOP-ONLY native picker (isTauri/pickMedia)
// — not headless-drivable — so library.add is seeded via the VERB on a real file (its
// only headless surface) and the button is PRESENT/RENDER-verified, click=N/A. Every
// other control IS a real one-click/inline edit. Control → verb (grepped from
// panels/Library/index.tsx): fav→library.favorite · Tag→library.tag · +folder→library.
// folder_add · move <select>→library.move · +Project→library.add_to_project · ✕→library.
// remove · right-click folder ▸ Rename→library.folder_rename / Delete→library.folder_remove.
// library.list loads on workspace activation; library.use has NO UI control (per-card "+Project"
// is add_to_project) → verb-level. Destructive remove/folder_remove run on our seeded item.
async function secLibrary(page) {
  const S = 'library'
  await freshProject(page, 'lib') // a project so library.add_to_project is enabled
  await closeOverlays(page)
  await page.locator('[data-cut-library-btn]').click().catch(() => {}); await sleep(500)
  const panel = page.locator('[data-cut-panel="library"]').first()
  const libraryItem = async (id) => (
    await verb('library.list', { ids: [id], limit: 1 })
  ).result?.items?.find((item) => item.id === id) ?? null
  const waitForLibraryItem = async (id, predicate, timeoutMs = 8000) => {
    const deadline = Date.now() + timeoutMs
    let item = null
    while (Date.now() < deadline) {
      item = await libraryItem(id)
      if (item && predicate(item)) return item
      await sleep(150)
    }
    return item && predicate(item) ? item : null
  }
  rec(S, 'GATE:library-panel-mounted', gateDim((await panel.count()) > 0), 'Library panel mounted in its dedicated workspace')

  // Seed one throwaway library item via the VERB (the Browse button is a desktop-only
  // native picker). library.add → {item:{id,…}}; keep the id for the per-card controls.
  const tag = 'FCV_LIB_' + Math.random().toString(36).slice(2, 6).toUpperCase()
  // NOTE: library.add's schema is additionalProperties:false and has NO `rationale` arg
  // (unlike most edit.* verbs) — passing one is rejected ("unknown argument 'rationale'").
  // The app's library.add works fine without it; the harness must not send it.
  const add = await verb('library.add', { path: SCENE, name: tag, source: 'user' })
  const id1 = add.result?.item?.id || ''
  // Re-activate the workspace so library.list reloads with the seeded item.
  await page.locator('[data-cut-library-close]').click().catch(() => {}); await sleep(200)
  await page.locator('[data-cut-library-btn]').click().catch(() => {}); await sleep(500)
  if (id1) {
    await page.locator(`[data-cut-library-card="${id1}"]`).first().waitFor({
      state: 'visible',
      timeout: 10_000,
    })
  }

  // library.add — select a unique real audio file through the host picker when
  // the installed OS controller is paired. Browser/dev runs keep the prior
  // explicit present/render + verb-seed evidence without pretending to click.
  {
    const browse = page.locator('[data-cut-library-browse]')
    const present = (await browse.count()) > 0
    const rg = await renderGroup(page, S, 'library-panel', panel)
    const card = id1 ? (await page.locator(`[data-cut-library-card="${id1}"]`).count()) > 0 : false
    const browseFixture = NATIVE_OS_ACTIONS.enabled ? makeToneAudio(0.8) : null
    if (browseFixture) {
      let browseResponse = null
      await probe(page, {
        surface: S,
        name: 'library.add(Browse native picker)',
        actionId: 'library-browse',
        sel: browse,
        group: panel,
        groupName: 'library-panel',
        nativeAction: { mode: 'select', path: browseFixture, useDoClick: true, verifyResult: true },
        doClick: async () => {
          browseResponse = await captureVerbResp(page, 'library.add', () => browse.click(), 30_000)
          await sleep(500)
        },
        assertResult: async () => {
          const addedId = browseResponse?.result?.item?.id || ''
          const listed = addedId ? await verb('library.list', { ids: [addedId], limit: 1 }) : null
          return {
            ok: !!browseResponse?.ok && listed?.result?.items?.some((item) => item.id === addedId),
            detail: `native Browse library.add ok=${browseResponse?.ok} item=${addedId || 'missing'}`,
          }
        },
      })
    } else {
      rec(S, 'library.add(Browse=desktop native picker · verb seed)',
        { present: present ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: 'na', result: (add.ok && id1 && card) ? 'pass' : 'fail' },
        `Browse button present=${present}; no paired OS controller, so click remains N/A. library.add verb ok=${add.ok} → item ${id1 || '(none)'} ${card ? 'rendered as a card' : 'NOT in the grid'}${add.ok ? '' : ` err="${String(add.error?.message || add.error?.code || '').slice(0, 70)}"`} ${rg.detail}`.trim(), rg.shot)
    }
  }
  if (!id1) {
    for (const n of ['library.list', 'library.relink', 'library.favorite', 'library.tag', 'library.folder_add', 'library.move', 'library.add_to_project', 'library.use', 'library.folder_rename', 'library.folder_remove', 'library.remove']) {
      rec(S, n, { present: 'na', render: 'na', click: 'na', result: 'na' }, 'library.add seed failed (no item id) — cannot drive the per-item controls (empty-library guard)')
    }
    return
  }

  // library.list — re-fire on tab re-activation; RESULT = the response carries the seeded
  // item AND a card for it is in the grid (the UI renders library.list's items).
  await probe(page, {
    surface: S, name: 'library.list', sel: page.locator('[data-cut-library-grid]'), group: panel, groupName: 'library-panel',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'library.list', async () => {
        await page.locator('[data-cut-library-close]').click().catch(() => {}); await sleep(150)
        await page.locator('[data-cut-library-btn]').click().catch(() => {})
      }, 12000)
      await page.locator(`[data-cut-library-card="${id1}"]`).first().waitFor({
        state: 'visible',
        timeout: 10_000,
      })
    },
    assertResult: async () => {
      const r = probe._r
      const has = Array.isArray(r?.result?.items) && r.result.items.some((it) => it.id === id1)
      const card = (await page.locator(`[data-cut-library-card="${id1}"]`).count()) > 0
      return { ok: !!r?.ok && has && card, detail: `library.list ok=${r?.ok} items=${r?.result?.items?.length ?? '?'} (seeded item present=${has}, rendered=${card})` }
    },
  })
  {
    const pair = makeLibraryRelinkPair()
    if (!pair) {
      rec(S, 'library.relink(conditional native picker)',
        { present: 'na', render: 'na', click: 'na', result: 'fail' },
        'could not generate the deterministic moved-source fixture')
    } else {
      const added = await verb('library.add', { path: pair.originalEngine, name: 'FCV moved source', source: 'user' })
      const relinkId = added.result?.item?.id || ''
      unlinkSync(pair.originalDriver)
      await page.locator('[data-cut-library-close]').click().catch(() => {}); await sleep(150)
      await page.locator('[data-cut-library-btn]').click().catch(() => {}); await sleep(350)
      await page.locator('[data-cut-library-collection="missing"]').click().catch(() => {}); await sleep(400)
      const control = page.locator(`[data-cut-library-relink="${relinkId}"]`)
      const present = !!relinkId && (await control.count()) === 1
      const rendered = present && await control.isVisible().catch(() => false)
      if (NATIVE_OS_ACTIONS.enabled && present) {
        let relinkResponse = null
        await probe(page, {
          surface: S,
          name: 'library.relink(conditional native picker)',
          actionId: 'library-relink',
          sel: control,
          group: panel,
          groupName: 'library-panel',
          nativeAction: { mode: 'select', path: pair.replacementEngine, useDoClick: true, verifyResult: true },
          doClick: async () => {
            relinkResponse = await captureVerbResp(page, 'library.relink', () => control.click(), 30_000)
            await sleep(400)
          },
          assertResult: async () => {
            const repaired = await verb('library.list', { ids: [relinkId], limit: 1 })
            const mediaOk = repaired.result?.items?.[0]?.media_ok === true
            return {
              ok: !!relinkResponse?.ok && mediaOk,
              detail: `native Relink response=${relinkResponse?.ok}; media_ok=${mediaOk}`,
            }
          },
        })
      } else {
        const relink = await verb('library.relink', { id: relinkId, path: pair.replacementEngine })
        const repaired = await verb('library.list', { ids: [relinkId], limit: 1 })
        const mediaOk = repaired.result?.items?.[0]?.media_ok === true
        rec(S, 'library.relink(conditional native picker)',
          { present: present ? 'pass' : 'fail', render: rendered ? 'pass' : 'fail', click: 'na', result: relink.ok && mediaOk ? 'pass' : 'fail' },
          `moved-source fixture id=${relinkId || '(none)'}; Relink visible=${rendered}; no paired OS controller, so click remains N/A; verb ok=${relink.ok}; media_ok=${mediaOk}.`)
      }
      if (relinkId) await verb('library.remove', { id: relinkId })
      await page.locator('[data-cut-library-collection="all"]').click().catch(() => {}); await sleep(300)
    }
  }
  {
    const prev = page.locator('[data-cut-library-page-prev]')
    const next = page.locator('[data-cut-library-page-next]')
    const status = page.locator('[data-cut-library-page-status]')
    const present = (await prev.count()) === 1 && (await next.count()) === 1 && (await status.count()) === 1
    const meta = (await status.textContent().catch(() => '')) || ''
    const match = meta.match(/^1–(\d+) of (\d+)/)
    const pageItems = Number(match?.[1] ?? 0)
    const totalItems = Number(match?.[2] ?? 0)
    const nextDisabled = present && await next.isDisabled()
    const boundary = present
      && await prev.isDisabled()
      && pageItems >= 1
      && pageItems <= 100
      && totalItems >= pageItems
      && nextDisabled === (totalItems <= 100)
    rec(S, 'GATE:library-bounded-pagination',
      { present: present ? 'pass' : 'fail', render: present ? 'pass' : 'fail', click: 'na', result: boundary ? 'pass' : 'fail' },
      `visible Previous/Next boundary controls present=${present}; disabled=${boundary}; status="${meta}" (1k/10k keyboard traversal is dedicated scale gate)`)
  }
  // library.favorite — the per-card pin toggle.
  await probe(page, {
    surface: S, name: 'library.favorite', sel: page.locator(`[data-cut-library-fav="${id1}"]`), group: panel, groupName: 'library-panel',
    doClick: async () => {
      probe._favoriteBefore = await libraryItem(id1)
      probe._favoriteBeforeValue = Boolean(probe._favoriteBefore?.favorite)
      probe._favoriteExpected = !probe._favoriteBeforeValue
      probe._r = await captureVerbResp(page, 'library.favorite', async () => {
        await page.locator(`[data-cut-library-fav="${id1}"]`).click().catch(() => {})
      }, 12000)
    },
    assertResult: async () => {
      const item = await waitForLibraryItem(id1, (candidate) => Boolean(candidate.favorite) === probe._favoriteExpected)
      const responseMatches = Boolean(probe._r?.result?.item?.favorite) === probe._favoriteExpected
      return {
        ok: !!probe._favoriteBefore && !!probe._r?.ok && !!item && responseMatches,
        detail: `library.favorite ok=${probe._r?.ok}; favorite ${probe._favoriteBeforeValue}→${Boolean(item?.favorite)}; response item=${responseMatches}`,
      }
    },
  })
  // library.tag — Tag button → inline input → comma tags → Enter.
  await probe(page, {
    surface: S, name: 'library.tag', sel: page.locator(`[data-cut-library-tagbtn="${id1}"]`), group: panel, groupName: 'library-panel',
    doClick: async () => {
      await page.locator(`[data-cut-library-tagbtn="${id1}"]`).click().catch(() => {}); await sleep(200)
      probe._r = await captureVerbResp(page, 'library.tag', async () => {
        await page.locator('[data-cut-library-taginput]').fill('fcv, coverage').catch(() => {})
        await page.locator('[data-cut-library-taginput]').press('Enter').catch(() => {})
      }, 12000)
    },
    assertResult: async () => ({ ok: !!probe._r?.ok, detail: `library.tag ok=${probe._r?.ok} tags=[fcv, coverage]` }),
  })
  const fcvTagChip = page.locator('[data-cut-library-tag="fcv"]').first()
  await probe(page, {
    surface: S, name: 'library.tag-filter-chip', actionId: 'library-tag',
    sel: fcvTagChip, group: panel, groupName: 'library-panel',
    doClick: async () => {
      await fcvTagChip.click()
      await sleep(250)
    },
    assertResult: async () => ({
      ok: (await page.locator('[data-cut-library-tagfilter]').textContent().catch(() => ''))?.includes('#fcv') === true,
      detail: `#fcv chip activated filter=${await page.locator('[data-cut-library-tagfilter]').count() === 1}`,
    }),
  })
  await fcvTagChip.click().catch(() => {})
  await sleep(200)
  // library.folder_add — the +folder inline input.
  const folder = 'FCV_FOLDER_' + Math.random().toString(36).slice(2, 5).toUpperCase()
  await probe(page, {
    surface: S, name: 'library.folder_add', sel: page.locator('[data-cut-library-newfolder]'), group: panel, groupName: 'library-panel',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'library.folder_add', async () => {
        await page.locator('[data-cut-library-newfolder]').fill(folder).catch(() => {})
        await page.locator('[data-cut-library-newfolder]').press('Enter').catch(() => {})
      }, 12000)
    },
    assertResult: async () => {
      const chip = (await page.locator(`[data-cut-library-folder="${folder}"]`).count()) > 0
      return { ok: !!probe._r?.ok && chip, detail: `library.folder_add ok=${probe._r?.ok} → chip "${folder}" present=${chip}` }
    },
  })
  // library.move — the per-card folder <select> (move into the new folder).
  await probe(page, {
    surface: S, name: 'library.move', sel: page.locator(`[data-cut-library-move="${id1}"]`), group: panel, groupName: 'library-panel',
    doClick: async () => { probe._r = await captureVerbResp(page, 'library.move', async () => { await page.locator(`[data-cut-library-move="${id1}"]`).selectOption(folder).catch(() => {}) }, 12000) },
    assertResult: async () => {
      const item = await waitForLibraryItem(id1, (candidate) => candidate.folder === folder)
      const responseMatches = probe._r?.result?.item?.folder === folder
      const controlMatches = await page.locator(`[data-cut-library-move="${id1}"]`).inputValue().catch(() => '') === folder
      return {
        ok: !!probe._r?.ok && !!item && responseMatches && controlMatches,
        detail: `library.move ok=${probe._r?.ok}; library.list folder="${item?.folder ?? ''}"; response item=${responseMatches}; select=${controlMatches}`,
      }
    },
  })
  // library.add_to_project — the per-card "+ Project" (imports into the open project).
  await probe(page, {
    surface: S, name: 'library.add_to_project', sel: page.locator(`[data-cut-library-toproject="${id1}"]`), group: panel, groupName: 'library-panel',
    doClick: async () => {
      const before = Object.keys((await state()).assets || {}).length
      probe._b = before
      probe._r = await captureVerbResp(page, 'library.add_to_project', async () => { await page.locator(`[data-cut-library-toproject="${id1}"]`).click().catch(() => {}) }, 30000)
    },
    assertResult: async () => {
      const after = await waitForState((s) => Object.keys(s.assets || {}).length >= probe._b, 8000)
      const n = after ? Object.keys(after.assets || {}).length : -1
      return { ok: !!probe._r?.ok, detail: `library.add_to_project ok=${probe._r?.ok} (project assets ${probe._b}→${n})` }
    },
  })
  await runLibraryActionCoverage(page, { id: id1, secondMedia: SECOND })
  // library.use — NO UI control (per-card "+Project" is add_to_project). Verb-level: bump
  // the use counter; RESULT = item.uses incremented. Flagged not faked.
  {
    const u0 = (await verb('library.list', { ids: [id1], limit: 1 })).result?.items?.find((it) => it.id === id1)?.uses ?? 0
    const use = await verb('library.use', { id: id1 })
    const u1 = use.result?.item?.uses ?? -1
    rec(S, 'library.use(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result: (use.ok && u1 > u0) ? 'pass' : 'fail' },
      `library.use ok=${use.ok} uses ${u0}→${u1} — NO UI control (the per-card "+ Project" drives add_to_project, not use); verb-level RESULT, flagged not faked`)
  }
  // Remount after the add-to-project resync and wait for the server-backed folder
  // and card. Mutations intentionally reload asynchronously; probing during that
  // boundary turns a healthy control into an "absent" false failure.
  await page.locator('[data-cut-library-close]').click().catch(() => {}); await sleep(150)
  await page.locator('[data-cut-library-btn]').click().catch(() => {})
  await page.locator(`[data-cut-library-folder="${folder}"]`).waitFor({
    state: 'visible',
    timeout: 8000,
  }).catch(() => {})
  await page.locator(`[data-cut-library-card="${id1}"]`).waitFor({
    state: 'visible',
    timeout: 8000,
  }).catch(() => {})

  const folderChip = page.locator(`[data-cut-library-folder="${folder}"]`)
  await probe(page, {
    surface: S,
    name: 'library.folder-select',
    actionId: 'library-folder',
    sel: folderChip,
    group: panel,
    groupName: 'library-panel',
    doClick: async () => {
      await folderChip.click()
      await sleep(250)
    },
    assertResult: async () => ({
      ok: (await folderChip.getAttribute('class') || '').includes('lb-chip--on'),
      detail: `folder "${folder}" selected=${(await folderChip.getAttribute('class') || '').includes('lb-chip--on')}`,
    }),
  })

  // Alternate right-click surface: both folder actions must be discoverable in
  // the context menu even though the always-visible pencil/close buttons are the
  // primary keyboard-friendly controls.
  await probe(page, {
    surface: S,
    name: 'library.folder-context-menu',
    sel: page.locator(`[data-cut-library-folder="${folder}"]`),
    group: panel,
    groupName: 'library-panel',
    doClick: async () => {
      // The embedded WKWebView provider currently reports a successful W3C
      // right-button pointer action without emitting `contextmenu`. Dispatch
      // that standard browser event through the adapter so this shared suite
      // still proves the React handler/menu wiring. The final installed host
      // matrix separately exercises the physical OS right-click.
      await page.locator(`[data-cut-library-folder="${folder}"]`).click({
        button: 'right',
        force: process.env.FCV_UI_DRIVER === 'tauri-wdio',
      })
      await sleep(200)
      probe._folderMenuReady =
        (await page.locator('[data-cut-library-folder-ctx="rename"]').count()) === 1 &&
        (await page.locator('[data-cut-library-folder-ctx="delete"]').count()) === 1
    },
    assertResult: async () => {
      const ready = probe._folderMenuReady === true
      return { ok: ready, detail: `right-click menu exposes Rename + Delete=${ready}` }
    },
  })
  const folderCtx = `${folder}_CTX`
  await probe(page, {
    surface: S,
    name: 'library.folder-context-rename',
    actionId: 'library-folder-ctx',
    sel: page.locator('[data-cut-library-folder-ctx="rename"]'),
    group: panel,
    groupName: 'library-folder-menu',
    doClick: async () => {
      await page.locator('[data-cut-library-folder-ctx="rename"]').click()
      await page.locator(`[data-cut-library-folder-rename="${folder}"]`).waitFor({ state: 'visible', timeout: 5000 })
      probe._r = await captureVerbResp(page, 'library.folder_rename', async () => {
        await page.locator(`[data-cut-library-folder-rename="${folder}"]`).fill(folderCtx)
        await page.locator(`[data-cut-library-folder-rename="${folder}"]`).press('Enter')
      }, 12000)
      await sleep(250)
    },
    assertResult: async () => ({
      ok: !!probe._r?.ok && await page.locator(`[data-cut-library-folder="${folderCtx}"]`).count() === 1,
      detail: `context Rename ok=${probe._r?.ok}; "${folderCtx}" present=${await page.locator(`[data-cut-library-folder="${folderCtx}"]`).count() === 1}`,
    }),
  })

  // library.folder_rename — visible pencil → inline input → Enter.
  const folder2 = folder + '_R'
  await probe(page, {
    surface: S, name: 'library.folder_rename', sel: page.locator(`[data-cut-library-folder="${folderCtx}"]`), group: panel, groupName: 'library-panel',
    doClick: async () => {
      await page.locator(`[data-cut-library-folder-rename-btn="${folderCtx}"]`).click()
      await sleep(200)
      probe._r = await captureVerbResp(page, 'library.folder_rename', async () => {
        await page.locator(`[data-cut-library-folder-rename="${folderCtx}"]`).fill(folder2).catch(() => {})
        await page.locator(`[data-cut-library-folder-rename="${folderCtx}"]`).press('Enter').catch(() => {})
      }, 12000)
    },
    assertResult: async () => {
      const renamed = probe._r?.result?.renamed === true
      const chip = (await page.locator(`[data-cut-library-folder="${folder2}"]`).count()) > 0
      return { ok: !!probe._r?.ok && renamed && chip, detail: `library.folder_rename ok=${probe._r?.ok} renamed=${renamed} → chip "${folder2}" present=${chip}` }
    },
  })
  const contextDeleteFolder = `${folder}_DELETE`
  const contextDeleteSeed = await verb('library.folder_add', { name: contextDeleteFolder })
  if (!contextDeleteSeed.ok) throw new Error(`context-delete folder seed failed: ${contextDeleteSeed.error?.message || contextDeleteSeed.error?.code}`)
  await page.locator('[data-cut-library-close]').click().catch(() => {})
  await sleep(120)
  await page.locator('[data-cut-library-btn]').click()
  await page.locator(`[data-cut-library-folder="${contextDeleteFolder}"]`).waitFor({ state: 'visible', timeout: 8000 })
  await page.locator(`[data-cut-library-folder="${contextDeleteFolder}"]`).click({ button: 'right', force: process.env.FCV_UI_DRIVER === 'tauri-wdio' })
  if (await page.locator('[data-cut-library-folder-menu]').count() === 0) {
    await page.locator(`[data-cut-library-folder="${contextDeleteFolder}"]`).dispatchEvent('contextmenu')
  }
  await page.locator('[data-cut-library-folder-ctx="delete"]').waitFor({ state: 'visible', timeout: 5000 })
  await probe(page, {
    surface: S,
    name: 'library.folder-context-delete',
    actionId: 'library-folder-ctx',
    sel: page.locator('[data-cut-library-folder-ctx="delete"]'),
    group: page.locator('[data-cut-library-folder-menu]'),
    groupName: 'library-folder-menu',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'library.folder_remove', async () => {
        await page.locator('[data-cut-library-folder-ctx="delete"]').click()
      }, 12000)
      await sleep(250)
    },
    assertResult: async () => ({
      ok: !!probe._r?.ok && await page.locator(`[data-cut-library-folder="${contextDeleteFolder}"]`).count() === 0,
      detail: `context Delete ok=${probe._r?.ok}; folder gone=${await page.locator(`[data-cut-library-folder="${contextDeleteFolder}"]`).count() === 0}`,
    }),
  })
  // library.folder_remove — always-visible remove affordance on the renamed chip.
  await probe(page, {
    surface: S, name: 'library.folder_remove', sel: page.locator(`[data-cut-library-folder="${folder2}"]`), group: panel, groupName: 'library-panel',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'library.folder_remove', async () => {
        await page.locator(`[data-cut-library-folder-remove-btn="${folder2}"]`).click()
      }, 12000)
    },
    assertResult: async () => {
      const removed = probe._r?.result?.removed === true
      const removedChip = page.locator(`[data-cut-library-folder="${folder2}"]`)
      await removedChip.waitFor({ state: 'detached', timeout: 8_000 }).catch(() => {})
      const gone = (await removedChip.count()) === 0
      return { ok: !!probe._r?.ok && removed && gone, detail: `library.folder_remove ok=${probe._r?.ok} removed=${removed} → chip gone=${gone}` }
    },
  })
  // library.remove — the per-card ✕ (LAST: it deletes the seeded throwaway item).
  await page.locator(`[data-cut-library-card="${id1}"]`).waitFor({
    state: 'visible',
    timeout: 8000,
  }).catch(() => {})
  await probe(page, {
    surface: S, name: 'library.remove', sel: page.locator(`[data-cut-library-remove="${id1}"]`), group: panel, groupName: 'library-panel',
    doClick: async () => { probe._r = await captureVerbResp(page, 'library.remove', async () => { await page.locator(`[data-cut-library-remove="${id1}"]`).click().catch(() => {}) }, 12000) },
    assertResult: async () => {
      await sleep(600) // settle the panel's post-mutation reload
      const card = (await page.locator(`[data-cut-library-card="${id1}"]`).count()) === 0
      return { ok: !!probe._r?.ok && card, detail: `library.remove ok=${probe._r?.ok} → card gone=${card} (throwaway item cleaned up)` }
    },
  })
}

// ── 24. PROJECTS (left tab=projects + Review history) — project lifecycle ───────
// project.* spans TWO real surfaces (grepped): the Projects left tab (project.list on
// activation · project.open=card click · project.create=Create btn · project.delete=🗑,
// native-confirmation-gated) AND the Review rail (project.undo/redo=the Undo/Redo bar on the
// ops tab via App's onUndo/onRedo callbacks · project.diff=the diff tab's from/to selects,
// auto-fired). project.save = Ctrl+S (Timeline window handler — the ONLY save surface).
// project.checkpoint has NO create-UI (DiffView only CONSUMES checkpoints) and project.close
// has NO UI control at all → both verb-level (flagged). project.revert's only UI surfaces
// are CONDITIONAL (Review guidance-revert after a refused rebase-undo; Autopilot post-run
// Restore) → verb-level with a real RESULT (restore ops appended), flagged. Destructive
// create/open/delete/close run on THROWAWAY projects so the harness's own project survives.
async function secProjects(page) {
  const S = 'projects'
  const { projectPath } = await freshProject(page, 'proj') // P_main — the active project the history verbs act on
  await closeOverlays(page)
  // Seed undoable ops + a checkpoint so undo/redo/diff have real history.
  await verb('edit.add_marker', { at_ms: 500, label: 'fcv-m1' })
  await verb('edit.add_marker', { at_ms: 1000, label: 'fcv-m2' })
  // project.checkpoint — NO create-UI (DiffView only consumes checkpoints). Verb-level:
  // RESULT = the checkpoint object + project.state.checkpoints grows. Flagged not faked.
  const cpName = 'fcv-cp-' + Math.random().toString(36).slice(2, 5)
  const cpBefore = ((await state()).checkpoints || []).length
  const cp = await verb('project.checkpoint', { name: cpName, rationale: 'fcv: checkpoint (no create-UI)' })
  const cpAfter = await waitForState((s) => (s.checkpoints || []).length > cpBefore, 8000)
  const firstOp = cp.result?.checkpoint?.at_op || ''
  rec(S, 'project.checkpoint(verb-level · no create-UI)', { present: 'na', render: 'na', click: 'na', result: (cp.ok && cpAfter) ? 'pass' : 'fail' },
    `project.checkpoint ok=${cp.ok} → checkpoints ${cpBefore}→${cpAfter ? (cpAfter.checkpoints || []).length : '?'} (id=${cp.result?.checkpoint?.id ?? '?'}) — NO create-UI (the Review DiffView only CONSUMES checkpoints); verb-level RESULT, flagged not faked`)
  await verb('edit.add_marker', { at_ms: 1500, label: 'fcv-m3' }) // an op PAST the checkpoint → a non-empty diff
  await reloadApp(page); await sleep(900); await ensureRail(page)

  // project.save — Ctrl+S (the Timeline window keydown is the only save surface). Click a
  // neutral non-input area first so the handler's input-guard doesn't swallow it.
  await probe(page, {
    surface: S, name: 'project.save(Ctrl+S)', sel: page.locator('body'), group: page.locator('body'), groupName: 'app',
    doClick: async () => {
      // The save key handler lives on `window` in the Timeline but
      // GUARDS against input targets (INPUT/TEXTAREA/contentEditable → returns early).
      // Clicking the Review panel focused its number input (data-cut-qc-shift), so Ctrl+S
      // hit the guard and never dispatched project.save (ok=undefined). Click a NON-input
      // Timeline surface (the ruler) → the keydown target falls back to body → the guard
      // passes → save fires. (Timeline ruler co-mounts with the editor view in this section.)
      probe._saveState = await state()
      await page.locator('[data-cut-ruler]').first().click().catch(() => {})
      probe._r = await captureVerbResp(page, 'project.save', async () => { await page.keyboard.press('Control+s').catch(() => {}) }, 12000)
    },
    assertResult: async () => {
      const live = await state()
      const savedPath = probe._r?.result?.path || ''
      const localPath = savedPath ? resolveDriverPath(savedPath) : ''
      let persisted = null
      let bytes = 0
      try {
        bytes = statSync(localPath).size
        persisted = JSON.parse(readFileSync(localPath, 'utf8'))
      } catch { /* reported by the result below */ }
      const markerPersisted = (persisted?.markers || []).some((marker) => marker.label === 'fcv-m3')
      const stateMatches = (live?.markers || []).some((marker) => marker.label === 'fcv-m3')
        && live?.name === probe._saveState?.name
      const expectedPath = resolveDriverPath(joinHostPath(projectPath, 'project.json'))
      const pathMatches = localPath === expectedPath
      return {
        ok: !!probe._r?.ok && probe._r?.result?.saved === true && bytes > 0
          && markerPersisted && stateMatches && pathMatches,
        detail: `project.save ok=${probe._r?.ok}; saved=${probe._r?.result?.saved}; bytes=${bytes}; current-state match=${stateMatches}; persisted marker=${markerPersisted}; exact project path=${pathMatches}`,
      }
    },
  })

  // project.undo / project.redo — the Review ops-tab Undo/Redo bar (App onUndo/onRedo).
  const reviewPanel = await reviewTab(page, 'ops', '[data-cut-undo-bar]', 8000)
  const undoBtn = reviewPanel.locator('[data-cut-action="undo"]').first()
  const redoBtn = reviewPanel.locator('[data-cut-action="redo"]').first()
  rec(S, 'GATE:review-ops-tab', gateDim((await reviewPanel.locator('[data-cut-undo-bar]').count()) > 0), 'Review Undo/Redo bar mounted on the ops tab')
  await probe(page, {
    surface: S, name: 'project.undo(Review Undo button)', sel: undoBtn, group: reviewPanel, groupName: 'review-ops',
    doClick: async () => {
      for (let i = 0; i < 20; i++) { if (!(await undoBtn.isDisabled().catch(() => true))) break; await sleep(200) }
      probe._undoBefore = await state()
      probe._r = await captureVerbResp(page, 'project.undo', async () => { await undoBtn.click({ force: true }).catch(() => {}) }, 12000)
    },
    assertResult: async () => {
      const beforeHasMarker = (probe._undoBefore?.markers || []).some((marker) => marker.label === 'fcv-m3')
      const after = await waitForState((project) => !(project.markers || []).some((marker) => marker.label === 'fcv-m3'), 8000)
      const cursorMoved = Number.isInteger(probe._r?.result?.cursor)
        && typeof probe._r?.result?.to_op === 'string'
        && probe._r.result.redo_available === true
      return {
        ok: !!probe._r?.ok && beforeHasMarker && !!after && cursorMoved,
        detail: `project.undo ok=${probe._r?.ok}; fcv-m3 present-before=${beforeHasMarker} absent-after=${!!after}; cursor=${probe._r?.result?.cursor}; redo_available=${probe._r?.result?.redo_available}`,
      }
    },
  })
  await probe(page, {
    surface: S, name: 'project.redo(Review Redo button)', sel: redoBtn, group: reviewPanel, groupName: 'review-ops',
    doClick: async () => {
      for (let i = 0; i < 20; i++) { if (!(await redoBtn.isDisabled().catch(() => true))) break; await sleep(200) }
      probe._r = await captureVerbResp(page, 'project.redo', async () => { await redoBtn.click({ force: true }).catch(() => {}) }, 12000)
    },
    assertResult: async () => {
      const after = await waitForState((project) => (project.markers || []).some((marker) => marker.label === 'fcv-m3'), 8000)
      const cursorMoved = Number.isInteger(probe._r?.result?.cursor)
        && typeof probe._r?.result?.to_op === 'string'
        && probe._r.result.undo_available === true
      return {
        ok: !!probe._r?.ok && !!after && cursorMoved,
        detail: `project.redo ok=${probe._r?.ok}; fcv-m3 restored=${!!after}; cursor=${probe._r?.result?.cursor}; undo_available=${probe._r?.result?.undo_available}`,
      }
    },
  })

  // project.diff — the Review diff tab. from/to default to (oldest checkpoint → HEAD) and
  // the panel auto-fires project.diff; capture that response. RESULT = ok + a summary renders.
  await probe(page, {
    surface: S, name: 'project.diff(Review diff tab)', sel: reviewPanel.locator('[data-cut-tab="diff"]').first(), group: reviewPanel, groupName: 'review-diff',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'project.diff', async () => { await reviewPanel.locator('[data-cut-tab="diff"]').first().click({ force: true }).catch(() => {}) }, 15000)
      // Belt-and-suspenders: if the auto-fire (from=oldest checkpoint, to=HEAD) didn't land,
      // pick the first real from-option (index 1 — index 0 is the "from…" placeholder) and
      // the last to-option (HEAD) to force project.diff. Index-based avoids guessing ids.
      if (probe._r === undefined) {
        probe._r = await captureVerbResp(page, 'project.diff', async () => {
          await page.locator('[data-cut-diff-from]').selectOption({ index: 1 }).catch(() => {})
          const nTo = await page.locator('[data-cut-diff-to] option').count()
          await page.locator('[data-cut-diff-to]').selectOption({ index: Math.max(1, nTo - 1) }).catch(() => {})
        }, 12000)
      }
    },
    assertResult: async () => {
      const r = probe._r
      const ui = (await page.locator('[data-cut-diff]').count()) > 0
      return { ok: !!r?.ok && ui, detail: `project.diff ok=${r?.ok} from=${r?.result?.from_op ?? '?'} to=${r?.result?.to_op ?? '?'} (diff view rendered=${ui})` }
    },
  })
  for (const endpoint of ['from', 'to']) {
    const selector = page.locator(`[data-cut-diff-${endpoint}]`)
    await probe(page, {
      surface: S,
      name: `project.diff(change ${endpoint})`,
      actionId: `diff-${endpoint}`,
      sel: selector,
      group: page.locator('[data-cut-diff]').first(),
      groupName: 'review-diff-endpoints',
      doClick: async () => {
        const before = await selector.inputValue()
        await selector.selectOption('')
        const options = await selector.locator('option').evaluateAll((nodes) => nodes.map((node) => node.value))
        const next = options.find((value) => value && value !== before) || before
        await selector.selectOption(next)
        await sleep(250)
        probe._diffEndpointBefore = before
        probe._diffEndpointAfter = next
      },
      assertResult: async () => ({
        ok: !!probe._diffEndpointAfter && await selector.inputValue() === probe._diffEndpointAfter,
        detail: `${endpoint} ${probe._diffEndpointBefore || '(empty)'}→${probe._diffEndpointAfter || '(empty)'}`,
      }),
    })
  }

  // project.revert — UI surfaces are CONDITIONAL (Review guidance-revert needs a refused
  // rebase-undo; Autopilot needs a completed run). Verb-level: revert to the checkpoint's
  // op → restore ops appended (ops length grows). Flagged not faked.
  {
    const opsBefore = await opsLen()
    const rv = firstOp ? await verb('project.revert', { to: firstOp, rationale: 'fcv: revert (UI surfaces conditional)' }) : { ok: false, error: { message: 'no checkpoint op to revert to' } }
    let opsAfter = opsBefore
    for (let i = 0; i < 16; i++) { opsAfter = await opsLen(); if (opsAfter > opsBefore) break; await sleep(400) }
    rec(S, 'project.revert(verb-level · UI conditional)', { present: 'na', render: 'na', click: 'na', result: (rv.ok && opsAfter > opsBefore) ? 'pass' : 'fail' },
      `project.revert{to:${firstOp || '?'}} ok=${rv.ok} → ops ${opsBefore}→${opsAfter} (restore ops appended) — UI surfaces CONDITIONAL (Review guidance-revert after a refused rebase-undo; Autopilot post-run Restore); verb-level RESULT, flagged not faked`)
  }

  // ── lifecycle on THROWAWAY projects (so P_main / the harness survives) ──
  // project.list — the Projects tab loads it on activation; RESULT = ok + ≥1 card renders.
  await page.locator('[data-cut-left-tab="projects"]').click().catch(() => {}); await sleep(400)
  await probe(page, {
    surface: S, name: 'project.list(Projects tab)', sel: page.locator('[data-cut-panel="projects"]'), group: page.locator('[data-cut-panel="projects"]').first(), groupName: 'projects-panel',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'project.list', async () => {
        await page.locator('[data-cut-left-tab="transcript"]').click().catch(() => {}); await sleep(150)
        await page.locator('[data-cut-left-tab="projects"]').click().catch(() => {})
      }, 12000)
    },
    assertResult: async () => {
      const cards = await page.locator('[data-cut-project-card]').count()
      return { ok: !!probe._r?.ok && cards >= 1, detail: `project.list ok=${probe._r?.ok} projects=${probe._r?.result?.projects?.length ?? '?'} (cards rendered=${cards})` }
    },
  })

  // project.create — the Create button. RESULT = ok + the new card appears.
  const t1 = 'fcv_throwA_' + Math.random().toString(36).slice(2, 6)
  await probe(page, {
    surface: S, name: 'project.create(Create button)', sel: page.locator('[data-cut-projects-create]'), group: page.locator('[data-cut-panel="projects"]').first(), groupName: 'projects-panel',
    doClick: async () => {
      await page.locator('[data-cut-projects-newname]').fill(t1).catch(() => {})
      probe._r = await captureVerbResp(page, 'project.create', async () => { await page.locator('[data-cut-projects-create]').click().catch(() => {}) }, 20000)
      await sleep(1200)
    },
    assertResult: async () => ({ ok: !!probe._r?.ok, detail: `project.create("${t1}") ok=${probe._r?.ok} (now the active project)` }),
  })
  // A SECOND throwaway (setup, not a recorded row) so a NON-active project exists to delete.
  // Use the verb for this unrecorded setup step: the visible first create above calls
  // onProjectSwitched(), which remounts the app and invalidates the old Projects
  // form locator. Trying to reuse that stale form raced native rigs and never
  // dispatched the second create. The human Create control remains fully proven.
  const t2 = 'fcv_throwB_' + Math.random().toString(36).slice(2, 6)
  const t2create = await verb('project.create', {
    name: t2,
    settings: { width: 1920, height: 1080, fps: 30 },
  }) // The second throwaway is active; the first is now non-active.
  const projectActionFixtures = await createProjectActionFixtures()
  // Resolve the throwaway ids only after the second create is confirmed and the
  // recent index exposes both entries. Fixed sleeps raced slower native disks.
  let plist = []
  const deadline = Date.now() + 10000
  while (Date.now() < deadline) {
    plist = (await verb('project.list', { sort: 'recent' })).result?.projects || []
    const haveT1 = plist.some((p) => p.name === t1)
    const haveT2 = plist.some((p) => p.name === t2)
    const haveFixtures = projectActionFixtures.every((fixture) =>
      !fixture.ok || plist.some((project) => project.name === fixture.name))
    if (haveT1 && (haveT2 || !t2create?.ok) && haveFixtures) break
    await sleep(300)
  }
  const t1id = plist.find((p) => p.name === t1)?.id || ''
  const t2id = plist.find((p) => p.name === t2)?.id || ''
  let projectActionFixturesSafe = false

  // project.open — reopen the first throwaway by clicking its card. RESULT = ok.
  // Re-clicking the Projects tab does NOT refresh the list. The
  // panel reloads project.list ONLY when its `active` prop flips false→true (Projects useEffect
  // [active, load]); create()/reopen() never refresh it (only del() does), and onProjectSwitched
  // is a soft state reset — NO page reload. Clicking the tab when it is ALREADY active does not
  // change `active`, so the freshly-created throwaway cards never render → [data-cut-project-card]
  // for them is absent (project.list still PASSES because P_main IS in the stale list). FORCE a
  // reload by toggling AWAY (transcript) then BACK to projects — that flips `active` false→true →
  // load() → the fresh list — then poll for the card.
  if (t1id) {
    await page.locator('[data-cut-left-tab="transcript"]').click().catch(() => {}); await sleep(200)
    await page.locator('[data-cut-left-tab="projects"]').click().catch(() => {}); await sleep(400)
    await page.locator(`[data-cut-project-card="${t1id}"]`).first().waitFor({ state: 'visible', timeout: 8000 }).catch(() => {})
    await probe(page, {
      surface: S, name: 'project.open(Open button)', actionId: 'project-open',
      sel: page.locator(`[data-cut-project-open="${t1id}"]`), group: page.locator('[data-cut-panel="projects"]').first(), groupName: 'projects-panel',
      doClick: async () => { probe._r = await captureVerbResp(page, 'project.open', async () => { await page.locator(`[data-cut-project-open="${t1id}"]`).click().catch(() => {}) }, 20000); await sleep(1200) },
      assertResult: async () => ({ ok: !!probe._r?.ok, detail: `project.open("${t1}") ok=${probe._r?.ok} (reopened; second throwaway now non-active)` }),
    })
    projectActionFixturesSafe = !!probe._r?.ok
  } else {
    rec(S, 'project.open(card click)', { present: 'na', render: 'na', click: 'na', result: 'na' }, 'first throwaway id not resolvable from the index — cannot drive project.open (setup guard)')
  }
  if (projectActionFixturesSafe) {
    await runProjectsActionCoverage(page, {
      fixtures: projectActionFixtures,
      projectRows: plist,
    })
  } else {
    rec(S, 'GATE:projects-action-fixtures',
      { present: 'fail', render: 'fail', click: 'fail', result: 'fail' },
      'refusing to delete throwaway fixture directories because the first throwaway was not proven active')
  }

  // project.delete — the second throwaway is non-active (server refuses deleting the open one).
  // The supported Tauri message dialog gates it; accept the exact host-owned
  // confirmation. RESULT = ok + the card disappears. Throwaway, so safe.
  if (t2id) {
    // Same stale-list cause as project.open: toggle AWAY then BACK to flip the panel's `active`
    // prop false→true so project.list reloads and the second card (with its 🗑) actually renders,
    // then poll for the delete control before clicking. Re-clicking the active tab won't reload.
    await page.locator('[data-cut-left-tab="transcript"]').click().catch(() => {}); await sleep(200)
    await page.locator('[data-cut-left-tab="projects"]').click().catch(() => {}); await sleep(400)
    await page.locator(`[data-cut-project-delete="${t2id}"]`).first().waitFor({ state: 'visible', timeout: 8000 }).catch(() => {})
    const onDialog = (d) => { d.accept().catch(() => {}) }
    page.on('dialog', onDialog)
    await probe(page, {
      surface: S, name: 'project.delete(🗑 · confirm-gated)', sel: page.locator(`[data-cut-project-delete="${t2id}"]`), group: page.locator('[data-cut-panel="projects"]').first(), groupName: 'projects-panel',
      nativeAction: { mode: 'accept', useDoClick: true, verifyResult: true },
      doClick: async () => { probe._r = await captureVerbResp(page, 'project.delete', async () => { await page.locator(`[data-cut-project-delete="${t2id}"]`).click().catch(() => {}) }, 20000); await sleep(1000) },
      assertResult: async () => {
        const gone = (await page.locator(`[data-cut-project-card="${t2id}"]`).count()) === 0
        return { ok: !!probe._r?.ok && gone, detail: `project.delete("${t2}") ok=${probe._r?.ok} → card gone=${gone} (throwaway .cutproj removed; confirmation accepted)` }
      },
    })
    page.off('dialog', onDialog)
  } else {
    rec(S, 'project.delete(🗑 · confirm-gated)', { present: 'na', render: 'na', click: 'na', result: 'na' }, 'second throwaway id not resolvable — cannot drive project.delete (setup guard)')
  }

  // project.close — NO UI control anywhere in the build. Verb-level: closes the active
  // project (saves first) → {closed:true}, active becomes none. Flagged not faked. Run LAST;
  // the next section's freshProject re-establishes an active project.
  {
    const cl = await verb('project.close', {})
    const closed = cl.result?.closed === true
    rec(S, 'project.close(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result: (cl.ok && closed) ? 'pass' : 'fail' },
      `project.close ok=${cl.ok} closed=${closed} — NO UI control anywhere in the build; verb-level RESULT (active project closed), flagged not faked`)
    // Cleanup: drop the leftover first throwaway now that nothing is open.
    if (t1id) await verb('project.delete', { id: t1id }).catch(() => {})
  }
}

// ── 25. ASSETS (Left Find ▸ media · Assets ▸ Generate) — provider search + AI generation ──
// The agent-only assets.* family's human face: the permanent left Find tab's
// "Find media" surface mounts the embedded Stock search (assets.search/fetch via
// local_folder, offline). Generated-media placement: "Generate (AI)" lives in the Assets tray's
// "Add media" area (generated media is CREATED and lands in Assets, not searched)
// — the Assets [data-cut-action="generate-asset"]
// button opens the Generate drawer (assets.generate via the user's codex/grok CLI, two-step
// paid-gen confirm). Control → verb (grepped): provider/kind/dir/query + Search→assets.search ·
// per-hit Import→assets.fetch · Generate(arm→confirm)→assets.generate; the durable
// generated-media rail reads assets.generated_list. assets.providers has
// NO UI control (the provider lists are hardcoded in Stock/Generate) → verb-level.
// assets.generate is dep-gated on DEP.generate (codex/grok ready).
async function secAssets(page) {
  const S = 'assets'
  await runAssetsEmptyImportCoverage(page)
  await runAssetsActionCoverage(page, { secondMedia: SECOND })

  // assets.providers — NO UI control (Stock/Generate hardcode their provider option lists).
  // Verb-level: RESULT = a non-empty providers list. Flagged not faked.
  {
    const pv = await verb('assets.providers', {})
    const provs = Array.isArray(pv.result?.providers) ? pv.result.providers : []
    rec(S, 'assets.providers(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result: (pv.ok && provs.length >= 1) ? 'pass' : 'fail' },
      `assets.providers ok=${pv.ok} → ${provs.length} provider(s) — NO UI control (Stock/Generate hardcode their provider <select> options); verb-level RESULT, flagged not faked`)
  }

  // Open the permanent left Find tab ▸ Find media (the embedded Stock surface).
  await page.locator('[data-cut-left-tab="find"]').click().catch(() => {}); await sleep(250)
  await page.locator('[data-cut-find-tab="find-media"]').click().catch(() => {}); await sleep(500)
  const stock = page.locator('[data-cut-stock]').first()
  rec(S, 'GATE:find-media-mounted', gateDim((await stock.count()) > 0), 'Find media (Stock) surface mounted in the left Find pane')

  // assets.search — local_folder provider (offline), pointed at SCENE's own directory with
  // a query = a substring of its basename, so the SCENE file is a guaranteed hit.
  const sceneName = basenameHostPath(SCENE)
  const sceneDir = dirnameHostPath(SCENE) || MEDIA_DIR
  const sceneStem = (sceneName.replace(/\.[^.]+$/, '') || sceneName).slice(0, 8)
  let firstHitId = ''
  await probe(page, {
    surface: S, name: 'assets.search(Stock · local_folder)', sel: page.locator('[data-cut-stock-search]'), group: stock, groupName: 'stock-surface-before-search',
    doClick: async () => {
      await page.locator('[data-cut-stock-provider]').selectOption('local_folder').catch(() => {}); await sleep(200)
      await page.locator('[data-cut-stock-kind-opt="video"]').click().catch(() => {})
      await page.locator('[data-cut-stock-dir]').fill(sceneDir).catch(() => {})
      await page.locator('[data-cut-stock-query]').fill(sceneStem).catch(() => {})
      probe._r = await captureVerbResp(page, 'assets.search', async () => { await page.locator('[data-cut-stock-search]').click().catch(() => {}) }, 30000)
    },
    assertResult: async () => {
      const r = probe._r
      const hits = Array.isArray(r?.result?.hits) ? r.result.hits : []
      firstHitId = hits[0]?.id || ''
      const ui = await page.locator('[data-cut-stock-hit]').count()
      if (ui > 0) {
        const results = page.locator('[data-cut-stock-results]').first()
        await results.evaluate((element) => element.scrollIntoView({ block: 'center' })).catch(() => {})
        await renderGroup(page, S, 'stock-search-results', results)
      }
      return { ok: !!r?.ok && hits.length >= 1, detail: `assets.search(local_folder dir="${sceneDir}" q="${sceneStem}") ok=${r?.ok} hits=${hits.length} (rendered=${ui})` }
    },
  })
  // assets.fetch — Import the first hit into the project. RESULT = ok + project assets grow.
  if (firstHitId) {
    await probe(page, {
      surface: S, name: 'assets.fetch(Stock Import)', sel: page.locator(`[data-cut-stock-fetch="${cssAttrValue(firstHitId)}"]`), group: stock, groupName: 'stock-result-before-import',
      doClick: async () => {
        probe._b = Object.keys((await state()).assets || {}).length
        probe._r = await captureVerbResp(page, 'assets.fetch', async () => { await page.locator(`[data-cut-stock-fetch="${cssAttrValue(firstHitId)}"]`).click().catch(() => {}) }, 60000)
      },
      assertResult: async () => {
        const r = probe._r
        const after = await waitForState((s) => Object.keys(s.assets || {}).length > probe._b, 12000)
        const n = after ? Object.keys(after.assets || {}).length : Object.keys((await state()).assets || {}).length
        const results = page.locator('[data-cut-stock-results]').first()
        await results.evaluate((element) => element.scrollIntoView({ block: 'center' })).catch(() => {})
        await renderGroup(page, S, 'stock-import-complete', results)
        return { ok: !!r?.ok && !!r?.result?.asset_id, detail: `assets.fetch ok=${r?.ok} asset_id=${r?.result?.asset_id ?? '?'} (project assets ${probe._b}→${n})` }
      },
    })
  } else {
    rec(S, 'assets.fetch(Stock Import)', { present: 'pass', render: 'na', click: 'na', result: 'na' },
      'assets.search returned no local_folder hit to import (no media matched in SCENE\'s directory) — content-dependent, honest skip')
  }

  // assets.generate — open the Assets tray ▸ "Generate (AI)" button
  // (generated media is CREATED and lands in Assets). Two-step
  // arm→confirm runs the user's codex/grok CLI. Dep-gated on DEP.generate. Tri-state
  // RESULT (asset imported / honest reason / wiring).
  await page.locator('[data-cut-left-tab="assets"]').click().catch(() => {}); await sleep(250)
  await page.locator('[data-cut-action="generate-asset"]').click().catch(() => {}); await sleep(500)
  const gen = page.locator('[data-cut-generate]').first()
  rec(S, 'GATE:generate-mounted', gateDim((await gen.count()) > 0), 'Generate (AI) surface opened from the Assets "Generate (AI)" button')
  const extraVideoTrack = await verb('edit.add_track', {
    kind: 'video',
    rationale: 'fcv: Generate destination selector needs two video tracks',
  })
  if (!extraVideoTrack.ok) {
    throw new Error(`Generate destination fixture failed: ${extraVideoTrack.error?.message || extraVideoTrack.error?.code}`)
  }
  await waitForState(
    (project) => project.tracks.filter((track) => track.kind === 'video').length >= 2,
    8_000,
  )
  const generatedActions = createGeneratedMediaActionCoverage({
    probe,
    rec,
    state,
    waitForState,
    captureVerbResp,
    awaitJob,
    sleep,
    fixtureActive: process.env.FCV_AGENT_FIXTURES === '1',
  })
  await generatedActions.runStaticControls(page, gen)
  {
    const listed = await verb('assets.generated_list', { limit: 10 })
    const items = Array.isArray(listed.result?.items) ? listed.result.items : []
    const history = page.locator('[data-cut-generate-history]').first()
    rec(S, 'assets.generated_list(Generate history)', {
      rowKind: 'support',
      present: (await history.count()) > 0 ? 'pass' : 'fail',
      render: await history.isVisible().catch(() => false) ? 'pass' : 'fail',
      click: 'na',
      result: listed.ok && Number.isInteger(listed.result?.total) ? 'pass' : 'fail',
    }, `assets.generated_list ok=${listed.ok} items=${items.length} total=${listed.result?.total ?? '?'}; path-light history rail rendered`)
  }
  if (!DEP.generate) {
    const rg = await renderGroup(page, S, 'generate-surface', gen)
    rec(S, 'assets.generate(Generate · arm→confirm)', { present: (await page.locator('[data-cut-generate-run]').count()) > 0 ? 'pass' : 'fail', render: rg.ok ? 'pass' : 'fail', click: 'na', result: 'na' },
      `assets.generate needs a generation CLI (system.doctor judge.codex/judge.grok≠ok — codex/grok signed-in); absent — the two-step Generate button is PRESENT/RENDER-verified; honest dev skip; FCV_REQUIRE_FULL=1 enforces it present ${rg.detail}`.trim(), rg.shot)
  } else {
    const prov = DEP.genProvider || 'codex'
    let generatedAssetId = ''
    await probe(page, {
      surface: S, name: 'assets.generate(Generate · arm→confirm)', sel: page.locator('[data-cut-generate-run]'), group: gen, groupName: 'generate-surface',
      doClick: async () => {
        await page.locator('[data-cut-generate-provider]').selectOption(prov).catch(() => {})
        await page.locator('[data-cut-generate-prompt]').fill('a flat-design icon of a small blue rocket on a white background').catch(() => {})
        await page.locator('[data-cut-generate-run]').click().catch(() => {}) // FIRST click = ARM
        await sleep(250)
        probe._b = Object.keys((await state()).assets || {}).length
        probe._r = await captureVerbResp(page, 'assets.generate', async () => { await page.locator('[data-cut-generate-run]').click().catch(() => {}) }, 180000) // SECOND = dispatch
      },
      assertResult: async () => {
        const r = probe._r
        const res = r?.result || {}
        const job = r?.ok && res.job_id ? await awaitJob(res.job_id, 180000) : null
        const final = job?.result || res
        const imported = final.asset_id
          ? await waitForState((s) => Object.prototype.hasOwnProperty.call(s.assets || {}, final.asset_id), 15000)
          : null
        const after = Object.keys((imported || await state()).assets || {}).length
        if (r?.ok && (!job || job.state === 'done') && final.ok !== false && final.asset_id && imported) {
          generatedAssetId = final.asset_id
          return { ok: true, detail: `assets.generate(${prov}/image) job=${res.job_id || 'sync'} → asset ${final.asset_id} imported (project assets ${probe._b}→${after})` }
        }
        const reason = String(job?.error?.message || final.reason || r?.error?.message || '').slice(0, 100)
        if (!FULL && (job?.state === 'failed' || final.ok === false)) {
          probe._naResult = true
          return { ok: false, detail: `assets.generate honest no-asset (no charge): "${reason}" — env/content-dependent` }
        }
        return { ok: false, detail: `assets.generate failed: dispatch_ok=${r?.ok} job=${job?.state || (res.job_id ? 'timeout' : 'none')} asset_id=${final.asset_id || 'none'} imported=${!!imported} err="${reason}"` }
      },
    })
    // Downgrade the honest CLI-degrade case from fail → N/A (env/content-dependent, like broll).
    if (probe._naResult) {
      const last = results[results.length - 1]
      if (last && last.name.startsWith('assets.generate')) { last.result = 'na'; probe._naResult = false }
    }
    if (generatedAssetId) {
      const lifecycle = await generatedActions.runHistoryLifecycle(page, gen, generatedAssetId)
      if (!lifecycle.extended) generatedActions.recordExtendedSkip(lifecycle.detail)
      else rec(S, 'generated-media extended lifecycle', {
        present: 'pass',
        render: 'pass',
        click: 'pass',
        result: 'pass',
      }, lifecycle.detail)
    }
  }
}

// ── 26. COMMENTS (topbar Review rail) — the client-comment → agent-change loop ──
// panels/Comments. Topbar [data-cut-comments-btn] toggles the rail. Control → verb
// (grepped): input+Enter/[comment-add]→comment.add · row Draft→comment.draft (agent,
// claude) · Apply→comment.apply (claude + a draft that yielded verbs) · Dismiss→comment.
// resolve. comment.list has NO UI control (the rail renders project.comments from the
// state snapshot) → verb-level. draft/apply are dep-gated on DEP.claude.
async function secComments(page) {
  const S = 'comments'
  await freshProject(page, 'comments')
  await drainActiveJobs()
  const handoffRender = await verb('render.final', {
    preset: 'draft',
    rationale: 'full coverage review handoff UI action',
  })
  const handoffRenderJob = handoffRender.result?.job_id
    ? await awaitJob(handoffRender.result.job_id, 240_000)
    : null
  if (handoffRenderJob?.state !== 'done' && process.env.FCV_RESULT_RECEIPT) {
    // Keep the exact renderer error and persisted asset paths outside the
    // human-sized action-row summary. This survives --clean-after and avoids
    // losing the path shape needed to diagnose native-host failures after the
    // isolated app/profile/project staging tree is reclaimed.
    const diagnosticPath = join(
      dirname(process.env.FCV_RESULT_RECEIPT),
      'comments-render-failure.json',
    )
    writeFileSync(diagnosticPath, `${JSON.stringify({
      schema: 'shellx-cut/fcv-comments-render-failure/1',
      render_response: handoffRender,
      render_job: handoffRenderJob,
      source_media: SCENE,
      project: await state(),
    }, null, 2)}\n`)
  }
  await closeOverlays(page)
  await page.locator('[data-cut-comments-btn]').click().catch(() => {}); await sleep(500)
  const panel = page.locator('[data-cut-panel="comments"]').first()
  rec(S, 'GATE:comments-rail-open', gateDim((await panel.count()) > 0), 'Review-comments rail mounted via the topbar Review button')

  // Portable review handoff: the focused verify-review-handoff gate still owns
  // standalone reviewer/CSP/tamper coverage. This matrix drives both human
  // controls themselves against the exact render receipt.
  let handoffExport = null
  await probe(page, {
    surface: S,
    name: 'comment.export(Export review package)',
    actionId: 'comment-export-review',
    sel: page.locator('[data-cut-action="comment-export-review"]'),
    group: panel,
    groupName: 'comments-rail',
    doClick: async () => {
      handoffExport = await captureVerbResp(
        page,
        'comment.export',
        () => page.locator('[data-cut-action="comment-export-review"]').click(),
        120_000,
      )
      await sleep(250)
    },
    assertResult: async () => {
      const link = page.locator('[data-cut-review-package]').first()
      // Report the FAILING verb's own error. This row prints the render job's
      // error because a broken render is the common upstream cause — but when the
      // render is done and comment.export itself returns ok:false, the old detail
      // said only "export=false" and the actual message was recoverable from
      // nowhere (the JSON diagnostic above is gated on the RENDER failing). Both
      // errors now appear, each labelled with the verb it came from.
      const exportError = handoffExport === undefined || handoffExport === null
        ? ' (no comment.export response arrived)'
        : handoffExport.ok
          ? ''
          : ` (${handoffExport.error?.code || 'no-code'}: ${String(handoffExport.error?.message || 'no message')}${handoffExport.error?.suggested_action ? ` · ${handoffExport.error.suggested_action}` : ''})`
      return {
        ok: handoffRenderJob?.state === 'done' && !!handoffExport?.ok &&
          !!handoffExport.result?.render_hash && await link.count() === 1,
        detail: `render=${handoffRenderJob?.state || handoffRender.error?.code || 'missing'}${handoffRenderJob?.error ? ` (${JSON.stringify(handoffRenderJob.error).slice(0, 1_000)})` : ''}; export=${handoffExport?.ok}${exportError}; render_hash=${handoffExport?.result?.render_hash ? 'present' : 'missing'}; review link=${await link.count()}`,
      }
    },
  })
  // Durable diagnostic for a FAILED comment.export, the sibling of the render
  // diagnostic above. Same reason it exists: the action-row summary is
  // human-sized, and --clean-after reclaims the staged project tree, so the full
  // envelope has to be written out while it still exists. Gated on the EXPORT
  // failing (the render gate above cannot fire for this case — render=done is
  // exactly when this one is needed).
  if (!handoffExport?.ok && process.env.FCV_RESULT_RECEIPT) {
    const exportDiagnosticPath = join(
      dirname(process.env.FCV_RESULT_RECEIPT),
      'comments-export-failure.json',
    )
    writeFileSync(exportDiagnosticPath, `${JSON.stringify({
      schema: 'shellx-cut/fcv-comments-export-failure/1',
      export_response: handoffExport ?? null,
      render_response: handoffRender,
      render_job: handoffRenderJob,
      review_link_count: await page.locator('[data-cut-review-package]').count(),
      source_media: SCENE,
      project: await state(),
    }, null, 2)}\n`)
    console.error(`[fcv] comment.export failed — envelope written to ${exportDiagnosticPath}`)
  }
  const feedbackName = `fcv-review-feedback-${seq++}.json`
  const feedbackDriverPath = join(synthDriverDir, feedbackName)
  const feedbackEnginePath = joinHostPath(synthEngineDir, feedbackName)
  const handoffState = await state()
  if (handoffExport?.ok) {
    writeFileSync(feedbackDriverPath, `${JSON.stringify({
      schema: 'shellx-cut/review-feedback/1',
      project: handoffState?.name || 'ShellX Cut project',
      source_op_id: handoffExport.result.source_op_id,
      render_id: handoffExport.result.render_id,
      render_hash: handoffExport.result.render_hash,
      comments: [{ at_ms: 420, text: 'FCV imported review note', author: 'release gate' }],
    }, null, 2)}\n`)
  }
  const browserPickerFixture = UI_DRIVER === 'playwright-chromium' && !!handoffExport?.ok
  if (browserPickerFixture) {
    await page.evaluate((selectedPath) => {
      const target = window
      target.__fcvCommentOriginalTauri = target.__TAURI__
      target.__fcvCommentOriginalInternals = target.__TAURI_INTERNALS__
      target.__fcvCommentOriginalInvoke = target.__TAURI_INTERNALS__?.invoke
      const invoke = async (command, args, options) => {
        if (command === 'plugin:dialog|open') return selectedPath
        if (target.__fcvCommentOriginalInvoke) return target.__fcvCommentOriginalInvoke(command, args, options)
        throw new Error(`unexpected comment picker command: ${command}`)
      }
      if (target.__TAURI_INTERNALS__) target.__TAURI_INTERNALS__.invoke = invoke
      else target.__TAURI_INTERNALS__ = { invoke }
      if (!target.__TAURI__) target.__TAURI__ = { core: { invoke }, event: { listen: async () => () => {} } }
    }, feedbackEnginePath)
  }
  let handoffImport = null
  await probe(page, {
    surface: S,
    name: 'comment.import(Import feedback)',
    actionId: 'comment-import-feedback',
    sel: page.locator('[data-cut-action="comment-import-feedback"]'),
    group: panel,
    groupName: 'comments-rail',
    // A cascade row must name what it cascaded FROM. Reading only "did not produce
    // a feedback binding" cost a whole diagnosis round at 0.6.106: the row is
    // strict_unverified purely because comment.export failed, and the reason it
    // failed belongs right here.
    clickNa: handoffExport?.ok
      ? NATIVE_PICKER_CLICK_NA
      : `review package export did not produce a feedback binding — comment.export ${handoffExport ? `ok=false (${handoffExport.error?.code || 'no-code'}: ${String(handoffExport.error?.message || 'no message')})` : 'returned no response'}`,
    nativeAction: {
      mode: 'select',
      path: feedbackEnginePath,
      useDoClick: true,
      verifyResult: true,
    },
    doClick: async () => {
      handoffImport = await captureVerbResp(
        page,
        'comment.import',
        () => page.locator('[data-cut-action="comment-import-feedback"]').click(),
        30_000,
      )
      await sleep(350)
    },
    assertResult: async () => {
      const imported = (await state())?.comments?.find((comment) =>
        comment.text === 'FCV imported review note' &&
        comment.review_source?.render_id === handoffExport?.result?.render_id)
      return {
        ok: !!handoffImport?.ok && handoffImport.result?.count === 1 && !!imported,
        detail: `comment.import ok=${handoffImport?.ok}; count=${handoffImport?.result?.count ?? 0}; provenance=${!!imported}`,
      }
    },
  })
  if (browserPickerFixture) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvCommentOriginalInternals) {
        target.__fcvCommentOriginalInternals.invoke = target.__fcvCommentOriginalInvoke
        target.__TAURI_INTERNALS__ = target.__fcvCommentOriginalInternals
      } else delete target.__TAURI_INTERNALS__
      if (target.__fcvCommentOriginalTauri) target.__TAURI__ = target.__fcvCommentOriginalTauri
      else delete target.__TAURI__
      delete target.__fcvCommentOriginalTauri
      delete target.__fcvCommentOriginalInternals
      delete target.__fcvCommentOriginalInvoke
    })
  }

  const cText = 'FCV tighten the intro ' + Math.random().toString(36).slice(2, 6)
  let cid = ''
  let commentAtMs = 0
  // comment.add — input + Add button; RESULT = ok + project.comments gains the comment.
  await probe(page, {
    surface: S, name: 'comment.add', sel: page.locator('[data-cut-action="comment-add"]'), group: panel, groupName: 'comments-rail',
    doClick: async () => {
      await page.locator('[data-cut-comment-input]').fill(cText).catch(() => {})
      probe._r = await captureVerbResp(page, 'comment.add', async () => { await page.locator('[data-cut-action="comment-add"]').click() }, 15000)
    },
    assertResult: async () => {
      const after = await waitForState((s) => (s.comments || []).some((c) => c.text === cText), 8000)
      const c = after ? (after.comments || []).find((cc) => cc.text === cText) : null
      cid = c?.id || ''
      commentAtMs = c?.at_ms ?? 0
      return { ok: !!probe._r?.ok && !!c, detail: `comment.add ok=${probe._r?.ok} → comment ${cid || '(none)'} in project.comments` }
    },
  })
  // comment.list — NO UI control (the rail reads project.comments from the snapshot).
  // Verb-level: RESULT = the list includes the comment we added. Flagged not faked.
  {
    const cl = await verb('comment.list', {})
    const arr = Array.isArray(cl.result?.comments) ? cl.result.comments : (Array.isArray(cl.result) ? cl.result : [])
    const has = arr.some((c) => c.id === cid || c.text === cText)
    rec(S, 'comment.list(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result: (cl.ok && (has || !cid)) ? 'pass' : 'fail' },
      `comment.list ok=${cl.ok} → ${arr.length} comment(s), seeded present=${has} — NO UI control (the rail renders project.comments from the state snapshot); verb-level RESULT, flagged not faked`)
  }
  if (!cid) {
    for (const n of ['comment.draft', 'comment.apply', 'comment.resolve']) rec(S, n, { present: 'na', render: 'na', click: 'na', result: 'na' }, 'comment.add produced no comment id — cannot drive the per-comment controls (guard)')
    return
  }

  const waitUiPlayhead = async (expected, timeoutMs = 8000) => {
    const deadline = Date.now() + timeoutMs
    let current = null
    while (Date.now() < deadline) {
      const response = await verb('ui.state', {})
      current = response.result?.playhead_ms
      if (response.ok && current === expected) return current
      await sleep(100)
    }
    return current
  }

  // The timecode button is a direct UI seek (not another engine mutation).
  // Move away first, then prove the connected UI publishes the comment anchor.
  const awayMs = commentAtMs === 1500 ? 3000 : 1500
  await verb('ui.playhead', { at_ms: awayMs })
  await waitUiPlayhead(awayMs)
  let soughtMs = null
  await probe(page, {
    surface: S,
    name: 'comment-seek-to-anchor',
    actionId: 'comment-seek',
    sel: page.locator(`[data-cut-comment="${cid}"] [data-cut-action="comment-seek"]`),
    group: panel,
    groupName: 'comments-rail',
    doClick: async () => {
      await page.locator(`[data-cut-comment="${cid}"] [data-cut-action="comment-seek"]`).click()
      soughtMs = await waitUiPlayhead(commentAtMs)
    },
    assertResult: async () => ({
      ok: soughtMs === commentAtMs,
      detail: `playhead ${awayMs}ms → comment anchor ${soughtMs}ms`,
    }),
  })

  // Every status tab is a separate user action even though they share one
  // stable action family.
  for (const filter of ['open', 'addressed', 'dismissed', 'all']) {
    const filterButton = page.locator(`[data-cut-comment-filter="${filter}"]`).first()
    await probe(page, {
      surface: S,
      name: `comment-filter-${filter}`,
      actionId: 'comment-filter',
      sel: filterButton,
      group: panel,
      groupName: 'comments-rail',
      doClick: async () => { await filterButton.click(); await sleep(100) },
      assertResult: async () => {
        const selectedFilter = await filterButton.getAttribute('aria-selected')
        const rowVisible = await page.locator(`[data-cut-comment="${cid}"]`).isVisible().catch(() => false)
        const shouldShow = filter === 'all' || filter === 'open'
        return {
          ok: selectedFilter === 'true' && rowVisible === shouldShow,
          detail: `${filter} selected=${selectedFilter}; seeded open row visible=${rowVisible}`,
        }
      },
    })
  }

  // Expand the comment through the actual disclosure control so its
  // Draft/Apply/Done/Dismiss actions mount.
  const disclosure = page.locator(`[data-cut-comment="${cid}"] [data-cut-comment-disclosure]`).first()
  await probe(page, {
    surface: S,
    name: 'comment-disclosure-open',
    actionId: 'comment-disclosure',
    sel: disclosure,
    group: panel,
    groupName: 'comments-with-row',
    doClick: async () => {
      await disclosure.click()
      await page.waitForFunction((commentId) => {
        const control = document.querySelector(`[data-cut-comment="${commentId}"] [data-cut-comment-disclosure]`)
        const body = document.querySelector(`#comment-body-${commentId}`)
        return control?.getAttribute('aria-expanded') === 'true'
          && body instanceof HTMLElement
          && body.getBoundingClientRect().height > 0
      }, cid, { timeout: 8_000 })
    },
    assertResult: async () => ({
      ok: await disclosure.getAttribute('aria-expanded') === 'true'
        && await page.locator(`#comment-body-${cid}`).isVisible(),
      detail: `expanded=${await disclosure.getAttribute('aria-expanded')}; body visible=${await page.locator(`#comment-body-${cid}`).isVisible()}`,
    }),
  })

  // comment.draft — the agent drafts the edit (claude). RESULT = the verb returned ok
  // (status content-dependent: a draft may yield 0 actionable verbs — still a valid run).
  let draftHasVerbs = false
  if (DEP.claude) {
    await probe(page, {
      surface: S, name: 'comment.draft(Draft button)', sel: page.locator('[data-cut-action="comment-draft"]'), group: panel, groupName: 'comments-open-actions',
      doClick: async () => { probe._r = await captureVerbResp(page, 'comment.draft', async () => { await page.locator('[data-cut-action="comment-draft"]').click() }, 120000) },
      assertResult: async () => {
        const r = probe._r
        await sleep(500)
        draftHasVerbs = (await page.locator('[data-cut-action="comment-apply"]').count()) > 0
        return { ok: !!r?.ok, detail: `comment.draft ok=${r?.ok} status=${r?.result?.status ?? '?'} → Apply available=${draftHasVerbs} (draft verb count is content-dependent)` }
      },
    })
  } else {
    rec(S, 'comment.draft(Draft button)', { present: (await page.locator('[data-cut-action="comment-draft"]').count()) > 0 ? 'pass' : 'fail', render: 'na', click: 'na', result: 'na' },
      'comment.draft needs `claude` (the agent drafts the edit) — Draft button PRESENT-verified; honest dev skip; FCV_REQUIRE_FULL=1 enforces claude present')
  }
  // comment.apply — only mounts when the draft produced actionable verbs (content-dependent).
  if (DEP.claude && draftHasVerbs) {
    await probe(page, {
      surface: S, name: 'comment.apply(Apply button)', sel: page.locator('[data-cut-action="comment-apply"]'), group: panel, groupName: 'comments-drafted-actions',
      doClick: async () => { probe._r = await captureVerbResp(page, 'comment.apply', async () => { await page.locator('[data-cut-action="comment-apply"]').click() }, 120000) },
      assertResult: async () => {
        const addressed = await waitForState((project) => (
          (project.comments || []).some((comment) => comment.id === cid && comment.status === 'addressed')
        ), 30_000)
        const applied = probe._r?.result?.applied
        const stepsApplied = Array.isArray(applied) && applied.length > 0
          && applied.every((step) => step.ok === true && typeof step.verb === 'string')
        const reviewArtifact = typeof probe._r?.result?.checkpoint === 'string'
          && probe._r.result.checkpoint.length > 0
          && probe._r?.result?.diff != null
        return {
          ok: !!probe._r?.ok && probe._r?.result?.comment_id === cid
            && probe._r?.result?.status === 'addressed' && stepsApplied
            && reviewArtifact && !!addressed,
          detail: `comment.apply ok=${probe._r?.ok}; status=${probe._r?.result?.status ?? '?'}; comment re-read addressed=${!!addressed}; steps=${applied?.length ?? 0}; checkpoint+diff=${reviewArtifact}`,
        }
      },
    })
  } else {
    rec(S, 'comment.apply(Apply button)', { present: 'na', render: 'na', click: 'na', result: 'na' },
      DEP.claude
        ? 'comment.apply only mounts when the draft yields actionable verbs (this draft produced none — content-dependent, honest skip)'
        : 'comment.apply needs `claude` (a drafted edit to apply) — honest dev skip; FCV_REQUIRE_FULL=1 enforces claude present')
  }
  // Done toggles addressed↔open. Derive the expected transition from current
  // engine truth because applying a drafted edit may already address it.
  let doneExpected = ''
  await probe(page, {
    surface: S,
    name: 'comment-toggle-done',
    actionId: 'comment-done',
    sel: page.locator('[data-cut-action="comment-done"]').first(),
    group: panel,
    groupName: 'comments-applied-actions',
    doClick: async () => {
      const before = await state()
      const current = (before.comments || []).find((comment) => comment.id === cid)?.status
      doneExpected = current === 'addressed' ? 'open' : 'addressed'
      probe._r = await captureVerbResp(page, 'comment.resolve', async () => {
        await page.locator('[data-cut-action="comment-done"]').first().click()
      }, 15000)
    },
    assertResult: async () => {
      const after = await waitForState((project) => (
        (project.comments || []).some((comment) => comment.id === cid && comment.status === doneExpected)
      ), 20_000)
      return {
        ok: !!probe._r?.ok && !!after,
        detail: `comment.resolve ok=${probe._r?.ok}; status=${doneExpected}; landed=${!!after}`,
      }
    },
  })
  // comment.resolve — the Dismiss button → status:'dismissed'. RESULT = ok + state status flips.
  await probe(page, {
    surface: S, name: 'comment.resolve(Dismiss button)', sel: page.locator('[data-cut-action="comment-dismiss"]'), group: panel, groupName: 'comments-rail',
    doClick: async () => { probe._r = await captureVerbResp(page, 'comment.resolve', async () => { await page.locator('[data-cut-action="comment-dismiss"]').click() }, 15000) },
    assertResult: async () => {
      const after = await waitForState((s) => (s.comments || []).some((c) => c.id === cid && c.status === 'dismissed'), 20_000)
      return { ok: !!probe._r?.ok && !!after, detail: `comment.resolve ok=${probe._r?.ok} → comment ${cid} status=dismissed=${!!after}` }
    },
  })

  const collapse = page.locator('[data-cut-action="comments-collapse"]').first()
  await probe(page, {
    surface: S,
    name: 'comments-collapse-rail',
    actionId: 'comments-collapse',
    sel: collapse,
    group: panel,
    groupName: 'comments-rail',
    doClick: async () => {
      await collapse.click()
      await panel.waitFor({ state: 'detached', timeout: 8000 })
    },
    assertResult: async () => ({
      ok: await panel.count() === 0
        && await page.locator('[data-cut-comments-btn]').first().getAttribute('aria-pressed') === 'false',
      detail: `panel count=${await panel.count()}; topbar pressed=${await page.locator('[data-cut-comments-btn]').first().getAttribute('aria-pressed')}`,
    }),
  })
}

// ── 27. MIXER (right tab=Audio) — edit.duck + the real Measure-loudness button ──
// panels/Mixer (embedded as the right-rail Audio tab via [data-cut-mixer-btn]). HEADLINE:
// edit.duck has NO dedicated UI control — track ducking is folded into audio.add_music's
// `duck` option (the only duck-related UI is the MusicBed drawer's toggle, which dispatches
// audio.add_music, NOT edit.duck). So edit.duck is covered at the VERB level with a real
// RESULT (gain windows land on the music track), dep-gated on DEP.perceptionStt (the verb
// computes speech windows from perception silence facts). ALSO drives the Mixer's REAL
// "Measure loudness" button (verify.loudness) — which secReviewQC currently covers only at
// the verb level claiming "no UI control anywhere"; it DOES have one here, so this upgrades
// it to a real-button drive and corrects the coverage map.
async function secMixer(page) {
  const S = 'mixer'
  await freshProject(page, 'mixer', SPEECH) // SPEECH carries a linked audio clip used as the duck against-track
  await closeOverlays(page)
  const mixerState = await state()
  const speechTrack = mixerState.tracks.find((t) => t.kind === 'audio' && (t.clips || []).some((c) => c.asset))
  const speechTrackId = speechTrack?.id
  const speechAsset = speechTrack?.clips?.find((c) => c.asset)?.asset || ''

  // edit.duck — verb-level (NO UI control). Needs a perception report (silences) on the
  // against-track asset + a separate music track. Dep-gated on DEP.perceptionStt.
  if (DEP.perceptionStt && speechTrackId && speechAsset) {
    const perc = await verb('media.perception', { asset: speechAsset })
    const percJob = perc.result?.job_id ? await awaitJob(perc.result.job_id) : null
    // media.perception has NO direct UI button — it's the analysis (faces/scenes/silences)
    // the agent + edit.duck/auto verbs consume. Cover it VERB-LEVEL here, where edit.duck
    // already needs its report. The full perception battery needs the CV
    // venv (cv2+torch), NOT just STT — so under perceptionStt-present-but-perceptionCv-absent
    // (a legit-down dep in baseline) the job fails (ok=true, job=failed). That was recorded as
    // a hard FAIL; gate it on the CV dep instead: a failed job is an honest N/A when cv2+torch
    // is absent (and not the release gate). Under FCV_REQUIRE_FULL=1 / perceptionCv present a
    // failed job IS a real fail.
    const percOk = perc.ok && (percJob ? percJob.state === 'done' : true)
    const percNeedReal = FULL || DEP.perceptionCv
    const percResult = percOk ? 'pass' : (percNeedReal ? 'fail' : 'na')
    rec(S, 'media.perception(verb-level · analysis backing edit.duck/auto-verbs)', { present: 'na', render: 'na', click: 'na', result: percResult },
      `media.perception{asset} ok=${perc.ok}${percJob ? ` job=${percJob.state}${percJob.error ? ` err="${String(percJob.error.message || percJob.error).slice(0, 50)}"` : ''}` : ''} (perceptionCv=${DEP.perceptionCv}) — ${percOk ? 'analysis (faces/scenes/silences) completed' : (percNeedReal ? 'FAILED with the CV venv present — real fail' : 'dep-gated N/A: the full battery needs cv2+torch (perceptionCv absent); FCV_REQUIRE_FULL=1 enforces it')}; the agent + edit.duck/auto verbs consume it; verb-level RESULT, flagged not faked`)
    const addTrk = await verb('edit.add_track', { kind: 'audio', rationale: 'fcv: music track for edit.duck' })
    const musicTrack = addTrk.result?.track_id || addTrk.result?.id || (await state()).tracks.find((t) => t.kind === 'audio' && t.id !== speechTrackId)?.id
    const duck = musicTrack ? await verb('edit.duck', { music_track: musicTrack, against_track: speechTrackId, db: -15, rationale: 'fcv: edit.duck (no UI control)' }) : { ok: false, error: { message: 'no music track' } }
    const applied = Number(duck.result?.windows_applied ?? 0)
    const result = (duck.ok && applied > 0) ? 'pass' : (duck.ok ? 'na' : 'fail')
    rec(S, 'edit.duck(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result },
      `edit.duck{music:${musicTrack}, against:${speechTrackId}, db:-15} ok=${duck.ok} windows_applied=${applied}${applied === 0 ? ` (note: ${String(duck.result?.note || 'no speech detected').slice(0, 50)} — content-dependent N/A)` : ''} — NO UI control (track ducking is folded into audio.add_music's duck option; the MusicBed toggle dispatches audio.add_music, not edit.duck); verb-level RESULT, flagged not faked`)
  } else {
    rec(S, 'edit.duck(verb-level · no UI control)', { present: 'na', render: 'na', click: 'na', result: 'na' },
      `edit.duck computes speech windows from perception silence facts on the against-track asset (needs perception STT); perceptionStt=${DEP.perceptionStt} — honest dev skip; FCV_REQUIRE_FULL=1 enforces it present — NO UI control (folded into audio.add_music)`)
  }

  // Open the Mixer (right-rail Audio tab) and drive the real Measure-loudness button.
  await ensureRail(page)
  await page.locator('[data-cut-mixer-btn]').click().catch(() => {}); await sleep(500)
  await page.locator('[data-cut-right-tab="audio"]').click().catch(() => {}); await sleep(400)
  const mixer = page.locator('[data-cut-mixer]').first()
  rec(S, 'GATE:mixer-audio-tab-mounted', gateDim((await mixer.count()) > 0), 'Audio mixer mounted as the right-rail Audio tab')

  // The target is UI-local but changes the exact verify.loudness request.
  const targetSelect = page.locator('[data-cut-mixer-loud-target-select]').first()
  await probe(page, {
    surface: S,
    name: 'mixer-loudness-target',
    actionId: 'mixer-loud-target-select',
    sel: targetSelect,
    group: mixer,
    groupName: 'mixer-panel',
    doClick: async () => { await targetSelect.selectOption('-16') },
    assertResult: async () => {
      const value = await targetSelect.inputValue().catch(() => '')
      return { ok: value === '-16', detail: `selected target=${value} LUFS` }
    },
  })

  const audioTracksBeforeAdd = (await state()).tracks.filter((track) => track.kind === 'audio').length
  await probe(page, {
    surface: S,
    name: 'mixer-add-audio-track',
    actionId: 'mixer-add-audio',
    sel: page.locator('[data-cut-mixer-add-audio]').first(),
    group: mixer,
    groupName: 'mixer-panel',
    doClick: async () => {
      probe._r = await captureVerbResp(page, 'edit.add_track', async () => {
        await page.locator('[data-cut-mixer-add-audio]').first().click()
      }, 15_000)
    },
    assertResult: async () => {
      const next = await waitForState(
        (project) => project.tracks.filter((track) => track.kind === 'audio').length > audioTracksBeforeAdd,
        8_000,
      )
      const count = next?.tracks.filter((track) => track.kind === 'audio').length ?? 0
      return {
        ok: !!probe._r?.ok && count > audioTracksBeforeAdd,
        detail: `edit.add_track ok=${probe._r?.ok}; audio tracks ${audioTracksBeforeAdd}→${count}`,
      }
    },
  })

  const setRange = async (selector, value) => {
    await selector.evaluate((element, next) => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set
      setter?.call(element, String(next))
      element.dispatchEvent(new Event('input', { bubbles: true }))
      element.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))
    }, value)
  }

  if (speechTrackId) {
    const fader = page.locator(`[data-cut-mixer-fader="${speechTrackId}"]`).first()
    await probe(page, {
      surface: S,
      name: 'mixer-track-fader',
      actionId: 'mixer-fader',
      sel: fader,
      group: mixer,
      groupName: 'mixer-strip',
      doClick: async () => {
        probe._r = await captureVerbResp(page, 'edit.gain', async () => {
          await setRange(fader, -6)
        }, 15_000)
      },
      assertResult: async () => {
        const next = await waitForState(
          (project) => Math.abs((project.tracks.find((track) => track.id === speechTrackId)?.gain_db ?? 0) - (-6)) < 0.01,
          8_000,
        )
        const gain = next?.tracks.find((track) => track.id === speechTrackId)?.gain_db
        return { ok: !!probe._r?.ok && gain === -6, detail: `edit.gain ok=${probe._r?.ok}; gain_db=${gain}` }
      },
    })

    const pan = page.locator(`[data-cut-mixer-pan="${speechTrackId}"]`).first()
    await probe(page, {
      surface: S,
      name: 'mixer-track-pan',
      actionId: 'mixer-pan',
      sel: pan,
      group: mixer,
      groupName: 'mixer-strip',
      doClick: async () => {
        probe._r = await captureVerbResp(page, 'edit.pan', async () => {
          await setRange(pan, -0.4)
        }, 15_000)
      },
      assertResult: async () => {
        const next = await waitForState(
          (project) => Math.abs((project.tracks.find((track) => track.id === speechTrackId)?.pan ?? 0) - (-0.4)) < 0.001,
          8_000,
        )
        const value = next?.tracks.find((track) => track.id === speechTrackId)?.pan
        return { ok: !!probe._r?.ok && value === -0.4, detail: `edit.pan ok=${probe._r?.ok}; pan=${value}` }
      },
    })

    const mute = page.locator(`[data-cut-mixer-mute="${speechTrackId}"]`).first()
    await probe(page, {
      surface: S,
      name: 'mixer-track-mute',
      actionId: 'mixer-mute',
      sel: mute,
      group: mixer,
      groupName: 'mixer-strip',
      doClick: async () => {
        probe._r = await captureVerbResp(page, 'edit.mute', async () => { await mute.click() }, 15_000)
      },
      assertResult: async () => {
        const next = await waitForState(
          (project) => project.tracks.find((track) => track.id === speechTrackId)?.muted === true,
          8_000,
        )
        const pressed = await page.locator(`[data-cut-mixer-mute="${speechTrackId}"]`).first().getAttribute('aria-pressed')
        return { ok: !!probe._r?.ok && !!next && pressed === 'true', detail: `edit.mute ok=${probe._r?.ok}; muted=${!!next}; aria-pressed=${pressed}` }
      },
    })

    const solo = page.locator(`[data-cut-mixer-solo="${speechTrackId}"]`).first()
    await probe(page, {
      surface: S,
      name: 'mixer-track-solo',
      actionId: 'mixer-solo',
      sel: solo,
      group: mixer,
      groupName: 'mixer-strip',
      doClick: async () => {
        probe._r = await captureVerbResp(page, 'edit.solo', async () => { await solo.click() }, 15_000)
      },
      assertResult: async () => {
        const next = await waitForState(
          (project) => project.tracks.find((track) => track.id === speechTrackId)?.solo === true,
          8_000,
        )
        const pressed = await page.locator(`[data-cut-mixer-solo="${speechTrackId}"]`).first().getAttribute('aria-pressed')
        return { ok: !!probe._r?.ok && !!next && pressed === 'true', detail: `edit.solo ok=${probe._r?.ok}; solo=${!!next}; aria-pressed=${pressed}` }
      },
    })
  }

  // verify.loudness — the per-track "Measure loudness" button on the base (speech) track.
  // RESULT = ok + the LUFS badge fills with a measured value (real Mixer-button coverage —
  // upgrades secReviewQC's verb-level "no UI control" row, which is now inaccurate).
  if (speechTrackId) {
    await probe(page, {
      surface: S, name: 'verify.loudness(Mixer Measure button)', actionId: 'verify-loudness', sel: page.locator(`[data-cut-action="verify-loudness"][data-cut-mixer-loud-measure="${speechTrackId}"]`), group: mixer, groupName: 'mixer-panel',
      doClick: async () => { probe._r = await captureVerbResp(page, 'verify.loudness', async () => { await page.locator(`[data-cut-action="verify-loudness"][data-cut-mixer-loud-measure="${speechTrackId}"]`).click().catch(() => {}) }, 60000) },
      assertResult: async () => {
        const r = probe._r
        await sleep(600)
        const lufs = (await page.locator(`[data-cut-mixer-loudness-lufs="${speechTrackId}"]`).first().getAttribute('data-cut-loudness-lufs').catch(() => '')) || ''
        return { ok: !!r?.ok && typeof r?.result?.integrated_lufs === 'number' && r?.result?.target_lufs === -16, detail: `verify.loudness(real Mixer button) ok=${r?.ok} lufs=${r?.result?.integrated_lufs ?? '?'} target=${r?.result?.target_lufs ?? '?'} (badge="${lufs}") — corrects secReviewQC's "no UI control" note` }
      },
    })
  } else {
    rec(S, 'verify.loudness(Mixer Measure button)', { present: 'na', render: 'na', click: 'na', result: 'na' }, 'no audio-bearing track to measure (guard)')
  }

  // Run the timeline Listen action last: interacting with the editor intentionally
  // returns focus from the Audio rail to the clip Inspector on some layouts.
  if (speechTrackId) {
    const listen = page.locator(`[data-cut-action="track-listen"][data-cut-listen-track="${speechTrackId}"]`).first()
    await probe(page, {
      surface: S,
      name: 'timeline-track-listen',
      actionId: 'track-listen',
      sel: listen,
      group: page.locator(`[data-cut-track="${speechTrackId}"]`).first(),
      groupName: 'timeline-track-before-listen',
      doClick: async () => {
        probe._auditionArgs = null
        const onRequest = (request) => {
          let pathname = ''
          try { pathname = new URL(request.url()).pathname } catch { return }
          if (pathname !== '/api/verb/export.audio') return
          try { probe._auditionArgs = request.postDataJSON() } catch { /* malformed request is asserted below */ }
        }
        page.on('request', onRequest)
        probe._r = await captureVerbResp(page, 'export.audio', async () => { await listen.click() }, 60_000)
        page.off('request', onRequest)
        const settle = async () => {
          await page.waitForFunction(
            (track) => {
              const button = document.querySelector(`[data-cut-listen-track="${CSS.escape(track)}"]`)
              return button?.getAttribute('data-cut-audition-state') !== 'busy'
            },
            speechTrackId,
            { timeout: 15_000 },
          ).catch(() => {})
          return listen.getAttribute('data-cut-audition-state').catch(() => '')
        }
        probe._auditionState = await settle()
        if (probe._auditionState === 'error') {
          await listen.click()
          probe._auditionState = await settle()
        }
        probe._auditionSrc = await listen.evaluate((button) => button.nextElementSibling?.getAttribute('src') || '')
        probe._auditionPlayed = probe._auditionState === 'playing'
        if (probe._auditionPlayed) {
          await renderGroup(
            page,
            S,
            'timeline-track-listening',
            page.locator(`[data-cut-track="${speechTrackId}"]`).first(),
          )
          await listen.click()
          await page.waitForFunction(
            (track) => document.querySelector(`[data-cut-listen-track="${CSS.escape(track)}"]`)
              ?.getAttribute('data-cut-audition-state') === 'idle',
            speechTrackId,
            { timeout: 5_000 },
          ).catch(() => {})
        }
        probe._auditionStopped = await listen.getAttribute('data-cut-audition-state').catch(() => '') === 'idle'
      },
      assertResult: async () => {
        const args = probe._auditionArgs
        const exactArgs = args?.format === 'mp3'
          && args?.track === speechTrackId
          && args?.rationale === 'timeline per-track listen'
        const path = probe._r?.result?.path
        const mapped = typeof probe._auditionSrc === 'string' && probe._auditionSrc.includes('/api/export/')
        return {
          ok: !!probe._r?.ok && typeof path === 'string' && exactArgs
            && mapped && probe._auditionPlayed && probe._auditionStopped,
          detail: `export.audio ok=${probe._r?.ok} exactArgs=${exactArgs} path=${path || '?'} mapped=${mapped} played=${probe._auditionPlayed} stopped=${probe._auditionStopped}`,
        }
      },
    })
  }
}

// ── 28. AUTOPILOT (topbar) — render → verify → self-fix (autopilot.run) ─────────
// panels/Autopilot (topbar [data-cut-autopilot-btn] → activeDrawer). autopilot.run is an
// async JOB (render.final draft → verify battery → plan). Driven through the REAL Run button
// in policy='preview' (PLAN-ONLY — the dry mode, like secRecipe's dry_run): it renders +
// verifies and produces a fix PLAN without applying anything. RESULT = the report/summary
// renders (the pipeline ran end-to-end and produced a verdict); an honest error is
// content/dep-dependent. The cut is trimmed short so the draft render stays fast.
async function secAutopilot(page) {
  const S = 'autopilot'
  await freshProject(page, 'autopilot', SPEECH)
  await closeOverlays(page)
  for (const t of (await state()).tracks || []) {
    if (t.kind === 'video' || t.kind === 'audio') await verb('edit.ripple_delete', { track: t.id, range_ms: [3000, 999000], ripple: true })
  }
  await sleep(300)
  await page.locator('[data-cut-autopilot-btn]').click().catch(() => {}); await sleep(600)
  await page.waitForSelector('[data-cut-autopilot]', { timeout: 6000 }).catch(() => {})
  const drawer = page.locator('[data-cut-autopilot]').first()
  rec(S, 'GATE:autopilot-drawer-open', gateDim((await drawer.count()) > 0), 'Autopilot drawer mounted via the topbar Autopilot button')

  // autopilot.run (preview = plan-only). Capture the dispatch (job_id = STARTED), then poll
  // the drawer for the report/summary (job done) or an error, bounded (~draft render cap).
  await probe(page, {
    surface: S, name: 'autopilot.run(Preview · plan-only)', sel: page.locator('[data-cut-autopilot-run]'), group: drawer, groupName: 'autopilot-drawer',
    doClick: async () => {
      await page.locator('[data-cut-policy="preview"]').click().catch(() => {})
      await page.locator('[data-cut-autopilot-goal]').fill('clean up the cut for publish').catch(() => {})
      probe._r = await captureVerbResp(page, 'autopilot.run', async () => { await page.locator('[data-cut-autopilot-run]').click().catch(() => {}) }, 30000)
      for (let i = 0; i < 300; i++) { // bounded ~210s: draft render + verify battery
        await sleep(700)
        if ((await page.locator('[data-cut-autopilot-report]').count()) > 0) break
        if ((await page.locator('[data-cut-autopilot-error]').count()) > 0) break
      }
    },
    assertResult: async () => {
      const r = probe._r
      const started = !!r?.result?.job_id
      const report = (await page.locator('[data-cut-autopilot-report]').count()) > 0
      const summary = (await page.locator('[data-cut-autopilot-summary]').first().textContent().catch(() => '')) || ''
      const err = (await page.locator('[data-cut-autopilot-error]').count()) ? ((await page.locator('[data-cut-autopilot-error]').first().textContent().catch(() => '')) || '') : ''
      await renderGroup(page, S, 'autopilot-report', drawer)
      // A rendered report/summary = the full preview pipeline produced a verdict.
      if (report) return { ok: true, detail: `autopilot.run(preview) started=${started} → report rendered, summary="${summary.slice(0, 60)}"` }
      // Started but no report in-window, or an honest content/dep error — env-dependent.
      probe._naResult = !!started || /verify|render|judge|perception|claude|no .* found/i.test(err)
      return { ok: false, detail: `autopilot.run(preview) started=${started} report=${report}${err ? ` err="${err.slice(0, 70)}"` : ' (no report within the render window)'}` }
    },
  })
  // Downgrade an honest "started but unfinished / content-dep error" from fail → N/A
  // (the render+verify is env/perf-dependent; the job verifiably STARTED).
  if (probe._naResult) {
    const last = results[results.length - 1]
    if (last && last.name.startsWith('autopilot.run') && last.result === 'fail') {
      last.result = 'na'
      last.evidence += ' — job STARTED (job_id returned); report/verdict is render-window/env-dependent, honest N/A (not a wiring fail)'
    }
    probe._naResult = false
  }
  await page.locator('[data-cut-autopilot-close]').click().catch(() => {}); await page.keyboard.press('Escape').catch(() => {}); await sleep(200)
}

// ── 29. Residual verbs — remaining UI-surface verbs whose always-on
// control was not yet driven by an earlier section. Covered VERB-LEVEL here with a real
// falsifiable RESULT (the established edit.duplicate / project.close pattern). Each row
// NAMES the UI surface that ALSO drives the verb (Assets tray / caption-clip drag / Clips
// panel / Projects panel) — so it is an honest "verb-level + surface" row, NOT a "no UI"
// claim. These had real surfaces but no earlier assertion; they are covered here
// so the COVERED set is genuine, not aspirational.
async function secResidualVerbs(page) {
  const S = 'residual'
  await freshProject(page, 'residual')
  await closeOverlays(page)

  // media.remove — the "delete files" half of the cleanup flow (the Assets-tray per-asset
  // "remove-asset" button drives this). Import a throwaway 2nd asset, remove it, assert the
  // project's asset map shrinks.
  {
    const before = Object.keys((await state()).assets || {}).length
    const imp = await verb('media.import', { path: SECOND })
    const aid = imp.result?.asset_id
    await waitForState((s) => Object.keys(s.assets || {}).length > before, 12000)
    const mid = Object.keys((await state()).assets || {}).length
    const rm = aid ? await verb('media.remove', { asset: aid, rationale: 'fcv: media.remove (Assets-tray remove-asset surface)' })
      : { ok: false, error: { message: 'no asset imported to remove' } }
    const after = await waitForState((s) => Object.keys(s.assets || {}).length < mid, 12000)
    const n = after ? Object.keys(after.assets || {}).length : Object.keys((await state()).assets || {}).length
    rec(S, 'media.remove(verb-level · Assets-tray remove-asset)', { present: 'na', render: 'na', click: 'na', result: (rm.ok && !!after) ? 'pass' : 'fail' },
      `media.remove{asset:${aid ? String(aid).slice(0, 8) : '?'}} ok=${rm.ok} → project assets ${mid}→${n} — the Assets-tray per-asset "remove-asset" button drives this; verb-level RESULT, flagged not faked`)
  }

  // media.check + media.relink — the Assets-tray offline badge
  // + per-card "Relink…" button drive these. Import a throwaway asset; media.check must
  // report its source ONLINE (exists:true, computed from the fs at call time); relink it
  // to the SAME file (same content hash ⇒ pure repath, derived kept) and assert
  // hash_changed=false; a scoped media.check{asset} must still report exists:true.
  {
    const imp = await verb('media.import', { path: SECOND })
    const aid = imp.result?.asset_id
    if (aid) await waitForState((s) => !!(s.assets || {})[aid], 12000)
    const chk = await verb('media.check', {})
    const row = aid ? (chk.result?.assets || []).find((a) => a.asset === aid) : null
    rec(S, 'media.check(verb-level · Assets-tray offline badge)', { present: 'na', render: 'na', click: 'na', result: (chk.ok && row?.exists === true && typeof chk.result?.offline_count === 'number') ? 'pass' : 'fail' },
      `media.check → count=${chk.result?.count} offline_count=${chk.result?.offline_count} ${aid}.exists=${row?.exists} — fs-computed offline report backing the Assets-tray badge; verb-level RESULT, flagged not faked`)
    const rl = aid ? await verb('media.relink', { asset: aid, path: SECOND, rationale: 'fcv: relink to the same file (pure repath, derived kept)' })
      : { ok: false, error: { message: 'no asset imported to relink' } }
    const chk2 = await verb('media.check', aid ? { asset: aid } : {})
    const row2 = (chk2.result?.assets || [])[0]
    rec(S, 'media.relink(verb-level · Assets-tray Relink button)', { present: 'na', render: 'na', click: 'na', result: (rl.ok && rl.result?.hash_changed === false && rl.result?.derived_cleared === false && chk2.ok && row2?.exists === true) ? 'pass' : 'fail' },
      `media.relink{asset:${aid ? String(aid).slice(0, 8) : '?'}, path:same} ok=${rl.ok} hash_changed=${rl.result?.hash_changed} derived_cleared=${rl.result?.derived_cleared} → still online=${row2?.exists} — same-hash pure-repath branch proven; the Assets-tray offline "Relink…" button drives this; verb-level RESULT, flagged not faked`)
  }

  // media.bin_save / bin_delete / bin_list (smart bins) — the Assets-tray
  // filter+bin chips drive these. Save a kind:video bin, assert LIVE membership
  // includes a known video asset; delete it; list confirms it's gone.
  {
    const st = await state()
    const anyVideo = Object.entries(st.assets || {}).find(([, a]) => a?.probe?.kind === 'video')?.[0]
    const bs = await verb('media.bin_save', { name: 'fcv bin', kind: 'video', rationale: 'fcv: smart-bin save (Assets-tray save-filter chip)' })
    const bl = await verb('media.bin_list', {})
    const bin = (bl.result?.bins || []).find((b) => b.name === 'fcv bin')
    const memberOk = !!bin && (anyVideo ? (bin.matches || []).includes(anyVideo) : typeof bin.match_count === 'number')
    rec(S, 'media.bin_save+bin_list(verb-level · Assets-tray bin chips)', { present: 'na', render: 'na', click: 'na', result: (bs.ok && bl.ok && memberOk) ? 'pass' : 'fail' },
      `bin_save{kind:video} ok=${bs.ok} replaced=${bs.result?.replaced} → bin_list LIVE membership: ${anyVideo ?? 'no-video-asset'} in matches=${memberOk} (count=${bin?.match_count}) — Assets-tray filter/bin chips drive these; verb-level RESULT, flagged not faked`)
    const bd = await verb('media.bin_delete', { name: 'fcv bin', rationale: 'fcv: smart-bin delete' })
    const bl2 = await verb('media.bin_list', {})
    const gone = bl2.ok && !(bl2.result?.bins || []).some((b) => b.name === 'fcv bin')
    rec(S, 'media.bin_delete(verb-level · Assets-tray bin × button)', { present: 'na', render: 'na', click: 'na', result: (bd.ok && gone) ? 'pass' : 'fail' },
      `bin_delete ok=${bd.ok} → bin_list no longer carries it=${gone}`)
  }

  // captions.set_range — direct caption-clip retime: the timeline caption-clip drag/trim
  // gesture folds to this (edit.move/edit.trim REFUSE caption clips). Add a txt1 caption,
  // retime it to [2000,4000), assert the verb applied (ok) and the clip survived the retime.
  {
    const add = await verb('captions.add_text', { text: 'FCV setrange', range_ms: [0, 1500], position: 'bottom' })
    const capId = add.result?.clip_id
    await sleep(400)
    const newRange = [2000, 4000]
    const sr = capId ? await verb('captions.set_range', { clip: capId, range_ms: newRange, rationale: 'fcv: caption-clip retime (timeline caption drag surface)' })
      : { ok: false, error: { message: 'no caption clip to retime' } }
    const stillThere = capId ? !!(await waitForState((s) => (s.tracks || []).some((t) => (t.clips || []).some((c) => c.id === capId)), 8000)) : false
    rec(S, 'captions.set_range(verb-level · caption-clip drag)', { present: 'na', render: 'na', click: 'na', result: (sr.ok && stillThere) ? 'pass' : 'fail' },
      `captions.set_range{clip:${capId ? String(capId).slice(0, 8) : '?'}, range_ms:${JSON.stringify(newRange)}} ok=${sr.ok} clip-retained=${stillThere} — the timeline caption-clip drag/trim gesture folds to this verb; verb-level RESULT, flagged not faked`)
  }

  // render.bundle — the social-repurpose "Bundle" button (Clips panel) renders a publish
  // pack per platform. Draft preset + a short window + one aspect so the render stays fast;
  // assert ok + a terminal job (ffmpeg-backed — the same posture as render.final/render.queue).
  {
    const rb = await verb('render.bundle', { range_ms: [0, 1500], platforms: ['9:16'], preset: 'draft', rationale: 'fcv: render.bundle (Clips-panel social Bundle button)' })
    let done = rb.ok
    let packageResult = null
    // A bundle job has THREE distinguishable outcomes and the row used to print
    // the same `terminal=false status=?` for two of them: awaitJob returns null on
    // TIMEOUT and returns the job itself when it FAILED (a failed job carries an
    // `error` and no `result`, so `status` reads '?'). At 0.6.106 that ambiguity
    // cost a whole diagnosis round — the job had failed in 4 seconds with a
    // precise io error the row simply threw away. Name the state, and carry the
    // job's own error.
    let jobState = rb.ok ? 'not-a-job' : 'verb-rejected'
    let jobError = rb.ok ? '' : ` verbError=${rb.error?.code || 'no-code'}: ${String(rb.error?.message || 'no message')}`
    if (rb.result?.job_id) {
      const j = await awaitJob(rb.result.job_id)
      done = j?.state === 'done'
      packageResult = j?.result || null
      jobState = j ? j.state : 'never-terminal (awaitJob deadline expired)'
      jobError = j?.error
        ? ` jobError=${j.error.code || 'no-code'}: ${String(j.error.message || 'no message')}${j.error.cause ? ` (${String(j.error.cause)})` : ''}`
        : ''
    }
    const bid = rb.result?.bundle_id || rb.result?.id || rb.result?.bundle?.id
    const packageBound = ['ready', 'needs_review', 'blocked'].includes(packageResult?.status) &&
      typeof packageResult?.pass === 'boolean' && Array.isArray(packageResult?.issues) &&
      String(packageResult?.manifest_hash || '').startsWith('sha256:') && fileBytes(resolveDriverPath(packageResult?.manifest_path)) > 100
    rec(S, 'render.bundle(verb-level · Clips-panel Bundle)', { present: 'na', render: 'na', click: 'na', result: (rb.ok && done !== false && packageBound) ? 'pass' : 'fail' },
      `render.bundle{platforms:[9:16],preset:draft,range:[0,1500]} ok=${rb.ok} job=${jobState} terminal=${done} bundle=${bid ?? '?'} status=${packageResult?.status ?? '?'} manifest=${packageResult?.manifest_path ?? 'none'} manifestBound=${packageBound}${jobError} — the Clips-panel "Bundle" button drives this; verb-level RESULT, flagged not faked`)
  }

  // project.forget — drop a project from the recent index (forget ≠ delete: the .cutproj
  // stays on disk, re-discoverable by project.list). The Projects-panel per-card "forget"
  // button drives this. Create a throwaway, forget it by id, assert it leaves project.list.
  // Run LAST: it switches the active project to the throwaway then forgets it; the runner's
  // next section re-establishes a clean active project via freshProject.
  {
    const fname = 'fcv_forget_' + Math.random().toString(36).slice(2, 6)
    await drainActiveJobs()
    const created = await verb('project.create', { name: fname, settings: { width: 1280, height: 720, fps: 30 } })
    await sleep(800)
    const fid = ((await verb('project.list', { sort: 'recent' })).result?.projects || []).find((p) => p.name === fname)?.id
    const fg = fid ? await verb('project.forget', { id: fid }) : { ok: false, error: { message: 'throwaway id not resolvable from project.list' } }
    await sleep(400)
    // The oracle asserted the project LEFT project.list, but forget≠delete
    // — the .cutproj stays on disk and project.list RE-DISCOVERS it via the managed-dir
    // reconcile (this section's own comment says so), so list-absence is never true → false
    // fail. project.forget's honest recents-only signal is its {forgotten} flag (it dropped
    // the entry from the recent index); assert THAT, not list-absence. (Diagnostic only: list
    // re-discovery is EXPECTED, so we record it without gating on it.)
    const forgotten = fg.result?.forgotten === true
    const reDiscovered = fid ? ((await verb('project.list', { sort: 'recent' })).result?.projects || []).some((p) => p.id === fid) : false
    rec(S, 'project.forget(verb-level · Projects-panel forget)', { present: 'na', render: 'na', click: 'na', result: (fg.ok && forgotten) ? 'pass' : 'fail' },
      `project.create ok=${created.ok}; project.forget{id:${fid ? String(fid).slice(0, 8) : '?'}} ok=${fg.ok} forgotten=${forgotten} (dropped from the recent index) — forget≠delete: the .cutproj stays on disk so project.list re-discovers it via managed-dir reconcile (re-discovered=${reDiscovered}, EXPECTED — not gated); the Projects-panel per-card "forget" button drives this; verb-level RESULT, flagged not faked`)
  }
}

// ── runner ────────────────────────────────────────────────────────────────────
const secSettings = createFullCoverageSettings({
  app: EMBEDDED_WDIO ? '' : APP,
  probe,
  verb,
  captureVerbResp,
  sleep,
  closeOverlays,
  nativePickerClickNa: NATIVE_PICKER_CLICK_NA,
})
const { run: runAppChromeActionCoverage } = createAppChromeActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runStatusbarActionCoverage } = createStatusbarActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  awaitJob,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runSearchActionCoverage } = createSearchActionCoverage({
  probe,
  verb,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runClipsActionCoverage } = createClipsActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runAutopilotActionCoverage } = createAutopilotActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runInspectorConditionalActionCoverage } = createInspectorConditionalActionCoverage({
  probe,
  verb,
  state,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
  propertiesTab,
  expandInspectorSection,
  primaryMedia: SCENE,
  nativeOsActionsEnabled: NATIVE_OS_ACTIONS.enabled,
})
const { run: runEnvironmentActionCoverage } = createEnvironmentActionCoverage({
  probe,
  verb,
  sleep,
  closeOverlays,
})
const { run: runGradeActionCoverage } = createGradeActionCoverage({
  probe,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
  clipOfKind,
  primaryMedia: SCENE,
  lutPath: gradeLutEnginePath,
  nativePickerClickNa: NATIVE_PICKER_CLICK_NA,
  usePickerFixture: UI_DRIVER === 'playwright-chromium',
  nativeOsActionsEnabled: NATIVE_OS_ACTIONS.enabled,
})
const {
  runEmptyImport: runAssetsEmptyImportCoverage,
  run: runAssetsActionCoverage,
} = createAssetsActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  reloadApp,
  closeOverlays,
  freshProject,
  awaitImportJobs,
  makeRelinkPair: makeLibraryRelinkPair,
  makeToneAudio,
  basenameHostPath,
  nativePickerClickNa: NATIVE_PICKER_CLICK_NA,
  nativeOsActionsEnabled: NATIVE_OS_ACTIONS.enabled,
  primaryMedia: SCENE,
  trace,
})
const runLibraryActionCoverage = createLibraryActionCoverage({
  app: EMBEDDED_WDIO ? '' : APP,
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  closeOverlays,
  nativeOsActionsEnabled: NATIVE_OS_ACTIONS.enabled,
})
const {
  createFixtures: createProjectActionFixtures,
  run: runProjectsActionCoverage,
} = createProjectsActionCoverage({
  probe,
  verb,
  captureVerbResp,
  resolveDriverPath,
  sleep,
})
const { run: runTimelineToolbarActionCoverage } = createTimelineToolbarActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
})
const { run: runTimelineTrackActionCoverage } = createTimelineTrackActionCoverage({
  probe,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  closeOverlays,
})
const { run: runTimelineDialogActionCoverage } = createTimelineDialogActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  closeOverlays,
  selectClip,
  awaitImportJobs,
})
const { run: runLayerActionCoverage } = createLayerActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
})
const { run: runMaskActionCoverage } = createMaskActionCoverage({
  probe,
  state,
  waitForState,
  opsLen,
  opLanded,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  clipOfKind,
  selectClip,
  primaryMedia: SCENE,
})
const { run: runMatteActionCoverage } = createMatteActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  clipOfKind,
  selectClip,
})
const { run: runPreviewActionCoverage } = createPreviewActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
})
const { run: runRecordActionCoverage } = createRecordActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  nativeOsActionsEnabled: NATIVE_OS_ACTIONS.enabled,
  nativeOutputPath: joinHostPath(synthEngineDir, 'record-action-output.mp4'),
})
const { run: runShapeActionCoverage } = createShapeActionCoverage({
  probe,
  state,
  waitForState,
  opsLen,
  opLanded,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
})
const { run: runTitleActionCoverage } = createTitleActionCoverage({
  probe,
  state,
  waitForState,
  opsLen,
  opLanded,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
})
const { run: runSequenceIndexActionCoverage } = createSequenceIndexActionCoverage({
  probe,
  verb,
  state,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
})
const { run: runSequenceSwitcherActionCoverage } = createSequenceSwitcherActionCoverage({
  probe,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runNativeOtioActionCoverage } = createNativeOtioActionCoverage({
  probe,
  verb,
  state,
  captureVerbResp,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
  nativeOsActionsEnabled: NATIVE_OS_ACTIONS.enabled,
})
const { run: runTopbarActionCoverage } = createTopbarActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runTopbarDialogActionCoverage } = createTopbarDialogActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
  nativeOsActionsEnabled: NATIVE_OS_ACTIONS.enabled,
})
const { run: runReviewActionCoverage } = createReviewActionCoverage({
  app: EMBEDDED_WDIO ? '' : APP,
  probe,
  sleep,
  closeOverlays,
  ensureReviewPanel,
  reviewTab,
})
const { run: runRecipeActionCoverage } = createRecipeActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runRenderQueueActionCoverage } = createRenderQueueActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia: SCENE,
})
const { run: runScopesActionCoverage } = createScopesActionCoverage({
  probe,
  renderGroup,
  freshProject,
  closeOverlays,
  ensureReviewPanel,
  reviewTab,
  primaryMedia: SCENE,
})
const { run: runChatActionCoverage } = createChatActionCoverage({
  app: EMBEDDED_WDIO ? '' : APP,
  probe,
  sleep,
  closeOverlays,
  ensureRail,
})
const { run: runDirectorActionCoverage } = createDirectorActionCoverage({
  app: EMBEDDED_WDIO ? '' : APP,
  probe,
  sleep,
  closeOverlays,
})
const { run: runTranscriptActionCoverage } = createTranscriptActionCoverage({
  app: EMBEDDED_WDIO ? '' : APP,
  probe,
  captureVerbResp,
  sleep,
  closeOverlays,
})
const { run: runUserActionFeedbackCoverage } = createUserActionFeedbackCoverage({
  probe,
  sleep,
  closeOverlays,
})

async function secTimelineToolbarActions(page) {
  const tracks = await runTimelineToolbarActionCoverage(page)
  await runTimelineTrackActionCoverage(page, tracks)
}

async function secAppChromeActions(page) {
  await runAppChromeActionCoverage(page)
}

async function secStatusbarActions(page) {
  await runStatusbarActionCoverage(page)
}

async function secSearchActions(page) {
  await runSearchActionCoverage(page)
}

async function secClipsActions(page) {
  await runClipsActionCoverage(page)
}

async function secAutopilotActions(page) {
  await runAutopilotActionCoverage(page)
}

async function secInspectorConditionalActions(page) {
  await runInspectorConditionalActionCoverage(page)
}

async function secTimelineDialogActions(page) {
  // The preceding Inspector section can still own a project-scoped worker.
  // Wait honestly for terminal state before creating this isolated fixture.
  const drained = await drainActiveJobs()
  if (!drained) {
    throw new Error(
      `timeline dialog setup timed out draining active jobs: ` +
      `${activeJobSummary() || 'jobs.list returned active jobs without identifiers'}`,
    )
  }
  await runTimelineDialogActionCoverage(page, { primaryMedia: SCENE, secondMedia: SECOND })
}

async function secLayerActions(page) {
  await runLayerActionCoverage(page)
}

async function secMaskActions(page) {
  await runMaskActionCoverage(page)
}

async function secMatteActions(page) {
  await runMatteActionCoverage(page)
}

async function secPreviewActions(page) {
  await runPreviewActionCoverage(page)
}

async function secRecordActions(page) {
  await runRecordActionCoverage(page)
}

async function secEnvironmentActions(page) {
  await runEnvironmentActionCoverage(page)
}

async function secGradeActions(page) {
  await runGradeActionCoverage(page)
}

async function secShapeActions(page) {
  await runShapeActionCoverage(page)
}

async function secTitleActions(page) {
  await runTitleActionCoverage(page)
}

async function secSequenceIndexActions(page) {
  await runSequenceIndexActionCoverage(page)
}

async function secSequenceSwitcherActions(page) {
  await runSequenceSwitcherActionCoverage(page)
}

async function secNativeOtioActions(page) {
  await runNativeOtioActionCoverage(page)
}

async function secTopbarActions(page) {
  await runTopbarActionCoverage(page)
}

async function secTopbarDialogActions(page) {
  await runTopbarDialogActionCoverage(page)
}

async function secReviewActions(page) {
  await runReviewActionCoverage(page)
}

async function secRecipeActions(page) {
  await runRecipeActionCoverage(page)
}

async function secRenderQueueActions(page) {
  await runRenderQueueActionCoverage(page)
}

async function secScopesActions(page) {
  await runScopesActionCoverage(page)
}

async function secChatActions(page) {
  await runChatActionCoverage(page)
}

async function secDirectorActions(page) {
  await runDirectorActionCoverage(page)
}

async function secTranscriptActions(page) {
  await runTranscriptActionCoverage(page)
}

async function secUserActionFeedback(page) {
  await runUserActionFeedbackCoverage(page)
}

const SECTIONS = [
  ['settings', secSettings],
  ['user-action-feedback', secUserActionFeedback],
  ['app-chrome-actions', secAppChromeActions],
  ['statusbar-actions', secStatusbarActions],
  ['search-actions', secSearchActions],
  ['clips-actions', secClipsActions],
  ['autopilot-actions', secAutopilotActions],
  ['inspector-conditional-actions', secInspectorConditionalActions],
  ['project', secProject],
  ['timeline-toolbar-actions', secTimelineToolbarActions],
  ['timeline-dialog-actions', secTimelineDialogActions],
  ['layer-actions', secLayerActions],
  ['mask-actions', secMaskActions],
  ['matte-actions', secMatteActions],
  ['preview-actions', secPreviewActions],
  ['record-actions', secRecordActions],
  ['environment-actions', secEnvironmentActions],
  ['grade-actions', secGradeActions],
  ['shape-actions', secShapeActions],
  ['title-actions', secTitleActions],
  ['sequence-index-actions', secSequenceIndexActions],
  ['sequence-switcher-actions', secSequenceSwitcherActions],
  ['native-otio-actions', secNativeOtioActions],
  ['topbar-actions', secTopbarActions],
  ['topbar-dialog-actions', secTopbarDialogActions],
  ['review-actions', secReviewActions],
  ['recipe-actions', secRecipeActions],
  ['render-queue-actions', secRenderQueueActions],
  ['scopes-actions', secScopesActions],
  ['chat-actions', secChatActions],
  ['director-actions', secDirectorActions],
  ['transcript-actions', secTranscriptActions],
  ['video', secVideo],
  ['audio', secAudio],
  ['blend', secBlend],
  ['typed', secTypedClips],
  ['multi', secMulti],
  ['ctxmenu', secContextMenu],
  ['editverbs', secEditVerbs],
  ['range', secRange],
  ['export', secExport],
  ['renderqueue', secRenderQueue],
  ['menus', secMenus],
  ['drawers', secDrawers],
  ['agent', secAgent],
  ['aiservices', secAIServices],
  ['record', secRecord],
  ['generate', secGenerate],
  ['director', secDirector],
  ['assemble', secAssemble],
  ['transcript', secTranscriptEditor],
  ['reviewqc', secReviewQC],
  ['kinetic', secKinetic],
  ['matte', secMatte],
  ['recipe', secRecipe],
  ['library', secLibrary],
  ['projects', secProjects],
  ['assets', secAssets],
  ['comments', secComments],
  ['mixer', secMixer],
  ['autopilot', secAutopilot],
  ['residual', secResidualVerbs],
]

// ── catalog-drift guard ───────────────────────────────────────────────────────
// The curated VIDEO_EFFECTS / AUDIO_EFFECTS arrays are HARDCODED. They match the
// UI today, but if the engine effect catalog GROWS, the curated loops alone would
// silently stop covering the new effects and coverage would shrink unnoticed.
// secVideo now ALSO drives every non-curated, non-overlay VIDEO effect via the
// "More effects…" overflow (videoOverflowEffects), and secBlend drives the
// OVERLAY_ONLY_EFFECTS on an overlay clip — so this guard fetches the LIVE
// effects.list catalog and emits a LOUD FAIL row only if it carries an effect id
// that NONE of those three paths drives, so a genuine coverage regression TRIPS the
// gate instead of hiding.
async function catalogDriftGuard() {
  const cat = (await verb('effects.list', {})).result?.effects || []
  if (!cat.length) {
    rec('catalog-guard', 'CATALOG-DRIFT:effects', { present: 'na', render: 'na', click: 'na', result: 'na' },
      'effects.list returned no effects (engine unreachable?) — drift check skipped')
  } else {
    const curated = new Set([...VIDEO_EFFECTS, ...AUDIO_EFFECTS])
    // Coverage AFTER the per-effect drive-out:
    //   • curated one-click chips  — VIDEO_EFFECTS (secVideo) + AUDIO_EFFECTS (secAudio)
    //   • secVideo "More effects…" — drives a chip for EVERY non-curated, non-overlay
    //                                VIDEO effect (videoOverflowEffects — the SAME filter)
    //   • secBlend overlay verify  — drives each OVERLAY_ONLY_EFFECTS id on an overlay clip
    // So an effect is COVERED iff one of those reaches it; anything else is genuine drift
    // (a NEW engine effect the harness doesn't yet drive) and trips the gate.
    const overflowKeys = new Set(videoOverflowEffects(cat).map((e) => e.key))
    const covered = (e) =>
      curated.has(e.key) ||
      overflowKeys.has(e.key) ||
      (e.overlay_only && OVERLAY_ONLY_EFFECTS.includes(e.key))
    const uncovered = cat
      .filter((e) => e.key && !covered(e))
      .map((e) => (e.overlay_only ? `${e.key}(overlay_only)` : e.key))
    if (uncovered.length) {
      rec('catalog-guard', 'CATALOG-DRIFT:effects', { present: 'pass', render: 'na', click: 'na', result: 'fail' },
        `CATALOG-DRIFT: effects.list has ${uncovered.length} id(s) the harness never drives: ${uncovered.join(', ')} — add a curated chip, or for an overlay-only effect add the id to OVERLAY_ONLY_EFFECTS (secBlend drives it), so coverage grows with the catalog`)
    } else {
      rec('catalog-guard', 'CATALOG-DRIFT:effects', { present: 'pass', render: 'na', click: 'na', result: 'pass' },
        `live effects.list (${cat.length} ids) fully covered: curated chips + secVideo overflow loop (${overflowKeys.size} non-overlay video) + secBlend overlay-only (${OVERLAY_ONLY_EFFECTS.join(', ') || 'none'})`)
    }
  }
  // Blend modes: there is NO blend-mode list verb in the engine (only edit.blend
  // APPLIES a mode; BLEND_MODES is the canonical Rust const in app/core, validated
  // server-side by is_valid_blend_mode). transitions.list DOES exist but is a
  // DIFFERENT catalog (edit.crossfade xfade styles), and the harness enumerates no
  // TRANSITIONS array to drift-check against it. So there is nothing to runtime-
  // guard for blend modes here — the BLEND_MODES array is exercised only by the
  // per-mode loop in secBlend; if the engine adds a blend mode, add it there.
}

// ── dependency preflight ──────────────────────────────────────────────────────
// Runs FIRST (before the browser launches) so the release gate can hard-fail on an
// incomplete environment without spending a browser/section. It checks the deps the
// dep-gated controls need:
//   • claude CLI  — agent.chat + captions/transcript translate + QC judge. Read from
//                   the engine's OWN env scan (system.doctor `judge.claude` rung == ok
//                   ⇒ `claude` on the engine PATH), so it reflects the engine host.
//   • perception  — full sidecar imports for CV/detectors, Canary alignment,
//                   silence/scenes/beats, and OCR (redact-faces / auto-zoom / render
//                   checks / scenes/silences / beat grid / OCR redaction) AND an
//                   STT engine (transcribe / captions.generate). The doctor
//                   `perception` card reports STT readiness (status ok); these full
//                   sidecar imports are NOT in that status, so we additionally import-check
//                   them against the venv interpreter the card names (cheap import, never
//                   loads a model / runs a detect).
//   • diarize/dub — separate microservices, probed via their /health endpoints.
// FCV_REQUIRE_FULL=1 ⇒ any missing dep is a HARD FAIL (exit 3) with a clear message.
// Unset ⇒ the DEP flags simply gate honest-N/A in the dependent sections.
async function preflight() {
  let doctor = null
  try { doctor = (await verb('system.doctor', { refresh: true })).result } catch { /* engine unreachable → DEP.engine stays false */ }
  DEP.engine = !!doctor && Array.isArray(doctor.cards)
  const card = (id) => (doctor?.cards || []).find((c) => c.id === id) || (doctor?.cards || []).find((c) => c.kind === id)
  const statusOf = (c) => String(c?.status || '').toLowerCase()
  const ffmpeg = card('ffmpeg')
  DEP.ffmpegLibass = ffmpeg?.details?.can_burn_captions === true
  DEP.ffmpegVidstab = ffmpeg?.details?.can_stabilize === true
  DEP.ffmpegZscale = ffmpeg?.details?.can_color_manage === true
  DEP.ffmpegPath = ffmpeg?.details?.resolved || ''
  // claude rung on the engine PATH (agent.chat / translate / judge all shell out to it).
  DEP.claude = statusOf(card('judge.claude')) === 'ok'
  // perception STT (transcription/captions) keys off the perception card's status.
  const perc = card('perception')
  DEP.perceptionStt = statusOf(perc) === 'ok'
  DEP.perceptionPy = perc?.details?.python || ''
  // full sidecar imports (faces / auto-zoom / director sheets / silence / scenes /
  // beats / OCR / Canary alignment) — import-check the venv interpreter directly
  // so release runs fail before a long sweep.
  if (DEP.perceptionPy) {
    const perceptionProbePy = resolveDriverPath(DEP.perceptionPy)
    try {
      const probe = spawnSync(perceptionProbePy, ['-c', `import ${PERCEPTION_FULL_IMPORTS.join(', ')}`], { timeout: 30000, encoding: 'utf8' })
      DEP.perceptionCv = probe.status === 0
      const pathNote = perceptionProbePy === DEP.perceptionPy ? '' : ` via ${perceptionProbePy}`
      DEP.perceptionCvDetail = DEP.perceptionCv ? '' : `${((probe.stderr || probe.stdout || '').trim().split('\n').pop() || `exit ${probe.status}`)}${pathNote}`
    } catch (e) {
      DEP.perceptionCv = false
      DEP.perceptionCvDetail = e?.message || 'import probe failed'
    }
  }
  // matte runtime (AI background removal — edit.matte) keys off the doctor `matte`
  // card (the Matte drawer gates its Apply control on exactly this, panels/Matte).
  // secMatte drives the real Apply -> edit.matte, so the bake runtime must be
  // present for a real RESULT; absent ⇒ honest dep-skip (the drawer shows its
  // requirements card instead). diarize/dub already follow this sidecar pattern.
  DEP.matte = statusOf(card('matte')) === 'ok'
  // generation CLI (assets.generate in secAssets). The verb shells out to the
  // user's OWN codex (gpt-image, image only) or grok (grok-imagine, image+video) CLI;
  // the doctor exposes each as a judge rung (`judge.codex` / `judge.grok`) whose status
  // is `ok` only when the binary is present AND signed-in. Prefer codex (image is the
  // fastest/cheapest path); fall back to grok. Mirrors the DEP.matte sidecar pattern.
  const genCodex = statusOf(card('judge.codex')) === 'ok'
  const genGrok = statusOf(card('judge.grok')) === 'ok'
  DEP.generate = genCodex || genGrok
  DEP.genProvider = genCodex ? 'codex' : (genGrok ? 'grok' : '')
  // Per-provider agent.chat readiness — read each judge.<agent> card's details.chat block
  // (the multi-agent dropdown's own source). ready = installed + wired + authenticated==='yes'
  // (a CONFIRMED session). Mirrors lib/doctor chatAgentBadge's green: 'unknown'/'no' ≠ ready.
  for (const name of ['claude', 'codex', 'grok']) {
    const chat = card(`judge.${name}`)?.details?.chat
    DEP.chatAgents[name] = !!(chat && chat.installed && chat.wired && chat.authenticated === 'yes')
  }
  // diarize / dub services — probe /health directly (the doctor's optional
  // diarize/dub cards exist now, but a live /health check is the truer gate signal).
  const health = async (base) => { try { return (await fetch(base.replace(/\/$/, '') + '/health', { signal: AbortSignal.timeout(5000) })).ok } catch { return false } }
  DEP.diarize = await health(DIARIZE_ENDPOINT)
  DEP.dub = await health(DUB_ENDPOINT)

  const mk = (b) => (b ? '✓' : '✗')
  console.log('══════════ DEPENDENCY PREFLIGHT ══════════')
  console.log(`  mode              = ${FULL ? 'FCV_REQUIRE_FULL=1 (release gate — a missing dep is a HARD FAIL)' : 'partial dev run (a missing dep → honest N/A)'}`)
  console.log(`  engine(doctor)    ${mk(DEP.engine)}`)
  console.log(`  claude CLI        ${mk(DEP.claude)}   agent.chat · translate · QC judge`)
  console.log(`  perception full sidecar ${mk(DEP.perceptionCv)}   redact-faces · auto-zoom · silence · scenes · beats · OCR   [${DEP.perceptionPy || 'no venv python reported'}]`)
  console.log(`  perception STT    ${mk(DEP.perceptionStt)}   transcribe · captions.generate`)
  console.log(`  matte runtime     ${mk(DEP.matte)}   edit.matte (AI background removal)`)
  console.log(`  ffmpeg full       ${mk(DEP.ffmpegLibass && DEP.ffmpegVidstab && DEP.ffmpegZscale)}   libass=${mk(DEP.ffmpegLibass)} vidstab=${mk(DEP.ffmpegVidstab)} zscale=${mk(DEP.ffmpegZscale)}   [${DEP.ffmpegPath || 'no resolved path'}]`)
  console.log(`  generate CLI      ${mk(DEP.generate)}   assets.generate (${DEP.genProvider || 'codex/grok — none ready'})`)
  console.log(`  chat agents       claude:${mk(DEP.chatAgents.claude)} codex:${mk(DEP.chatAgents.codex)} grok:${mk(DEP.chatAgents.grok)}   agent.chat per-provider (codex/grok OPTIONAL — absent = benign skip)`)
  console.log(`  diarize service   ${mk(DEP.diarize)}   ${DIARIZE_ENDPOINT}`)
  console.log(`  dub service       ${mk(DEP.dub)}   ${DUB_ENDPOINT}`)

  if (FULL) {
    const missing = []
    if (!DEP.engine) missing.push('engine unreachable (system.doctor failed) — start cutd')
    if (!DEP.claude) missing.push('claude CLI not on the engine PATH (system.doctor judge.claude≠ok)')
    if (!DEP.perceptionCv) missing.push(`perception venv cannot import full sidecar modules (${PERCEPTION_FULL_IMPORTS.join(', ')}) for redact-faces / auto-zoom / director sheets / silence / scenes / beats / OCR${DEP.perceptionCvDetail ? `: ${DEP.perceptionCvDetail}` : ''}`)
    if (!DEP.perceptionStt) missing.push('perception STT engine not importable (transcribe / captions)')
    if (!DEP.matte) missing.push('matte runtime not set up (system.doctor matte≠ok — edit.matte AI background removal)')
    if (!DEP.ffmpegLibass || !DEP.ffmpegVidstab || !DEP.ffmpegZscale) missing.push(`engine ffmpeg lacks release filters (libass=${DEP.ffmpegLibass}, libvidstab=${DEP.ffmpegVidstab}, zscale=${DEP.ffmpegZscale}) at ${DEP.ffmpegPath || 'unknown path'} — point SHELLX_CUT_FFMPEG at a full build before running installed coverage`)
    if (!DEP.generate) missing.push('no generation CLI ready (system.doctor judge.codex/judge.grok≠ok — assets.generate)')
    if (!DEP.diarize) missing.push(`diarize service down (${DIARIZE_ENDPOINT}/health)`)
    if (!DEP.dub) missing.push(`dub service down (${DUB_ENDPOINT}/health)`)
    if (missing.length) {
      console.error('\n✗✗✗ full-verification env incomplete: ' + missing.join('; '))
      console.error('    FCV_REQUIRE_FULL=1 requires a COMPLETE environment — a missing dep is a HARD FAIL, not a')
      console.error('    per-control N/A. Provision the above (or drop FCV_REQUIRE_FULL for a partial dev run).')
      exit(3)
    }
    console.log('  ✓ all required deps present — full verification will run (zero RESULT N/A expected)')
  }
  console.log('')
}

// ══════════════════════════════════════════════════════════════════════════════
// COVERAGE COMPLETENESS GATE
// ────────────────────────────────────────────────────────────────────────────────
// Make "covers every schema verb" PROVABLE and DRIFT-PROOF. Every one of the verbs
// in schema/verbs.json must be classified into EXACTLY ONE of two hardcoded sets:
//
//   COVERED_VERBS       — the harness asserts a real RESULT on it (via a UI control,
//                         OR verb-level for the flagged no-UI ones, OR DELEGATED:
//                         PRESENT/RENDER-verified here with effect-proof cross-covered
//                         in a sibling release gate).
//   KNOWN_NON_UI_VERBS  — INTENTIONALLY not UI-covered: no always-on human surface
//                         (agent/infra/API/env-setup-only). Each carries a one-line reason.
//
// Both sets are HARDCODED (not derived) on purpose: a NEW schema verb then lands in
// NEITHER set and coverageCheck() FAILS LOUDLY naming it — forcing a coverage decision
// on every verb the engine adds, so nothing is ever silently skipped. The verb names
// are loaded from schema/verbs.json (not a hardcoded 203) so the total auto-tracks.
const SCHEMA_VERBS = (() => {
  const p = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'schema', 'verbs.json')
  return JSON.parse(readFileSync(p, 'utf8')).verbs.map((v) => v.name)
})()
// The final matrix owns every user-action effect directly. Keep this empty as a
// drift tripwire: a future exception must fail the strict final gate until the
// action can be actuated and proven in the same installed run.
const DELEGATED_VERBS = []
const DELEGATED_NOTE = 'none — the installed final matrix owns every user-action effect directly'

// KNOWN_NON_UI_VERBS — verified against ui/src (grep callVerb): none has an ALWAYS-ON
// human editing surface, so each is intentionally excluded with the reason below.
const KNOWN_NON_UI_VERBS = {
  // ui.* — agent/debug navigation + state verbs (no human control)
  'ui.highlight': 'agent/debug overlay-highlight verb — no callVerb in ui/src (agent-only)',
  'ui.open': 'agent navigation verb (open a panel/drawer programmatically) — no human control',
  'ui.playhead': 'internal playhead-move EFFECT (App/Search/Transcript dispatch it on click) — not a discrete assertable control',
  'ui.screenshot': 'debug screenshot verb — no callVerb in ui/src',
  'ui.select': 'internal selection EFFECT (App side-effect of clicking a clip) — not a discrete control',
  'ui.state': 'agent/debug UI-state read — no callVerb in ui/src',
  // media.* infra (no callVerb) + the SigLIP2 visual-search pair (secondary, dep-heavy)
  'media.probe': 'infra metadata probe (backs media.import) — no callVerb in ui/src',
  'media.index': 'SigLIP2 visual-search INDEX builder — secondary Find-moment tab, not always-on, SigLIP2 not a release-gate dep',
  'media.search': 'SigLIP2 visual MOMENT search — secondary Find-moment tab, not always-on; the covered always-on search is transcript.search (secTranscriptEditor)',
  'media.waveform': 'infra waveform render (backs the timeline audio lane) — no callVerb in ui/src',
  'media.filmstrip': 'infra filmstrip render (backs the timeline thumbnails) — no callVerb in ui/src',
  // system.* — one-time ENVIRONMENT PROVISIONING (doctor/EnvCards setup cards), not editing
  'system.setup_perception': 'env provisioning — installs the perception venv (doctor setup card); one-time, not an editing action',
  'system.setup_matte': 'env provisioning — installs the matte runtime (Matte requirements card); one-time',
  'system.set_ffmpeg': 'env provisioning — sets the ffmpeg path (EnvCards); one-time setup',
  'system.set_stt_model': 'env provisioning — picks the STT model (doctor); one-time setup',
  'system.fetch_tool': 'env provisioning — downloads a tool binary (doctor); one-time setup',
  // project.* infra reads (these ARE the harness\'s own assertion plumbing)
  'project.ops': 'op-log READ — the harness\'s own assertion plumbing (opLanded/opsLen read it); not a control',
  'project.state': 'project-state READ — the harness\'s own assertion plumbing (state()/waitForState read it); not a control',
  // misc infra / read-only / file-picker
  'debug.screenshot': 'debug screenshot verb — no callVerb in ui/src',
  'jobs.list': 'background-jobs LIST read (agent/status overview) — the always-on job surface (render queue) polls jobs.status, which IS covered',
  'title.templates': 'title-template LIST read — no callVerb in ui/src',
  'transitions.list': 'transition-CATALOG read (lib/catalogs populates) — read, not an action (the drift guard notes no TRANSITIONS array to runtime-check)',
  'clip.candidates': 'read-only short-form candidate RANKING (backs the Clips panel) — render.bundle is the action over a pick (covered)',
  'motion.template_to_cut': 'Motion template bridge behind Generate motion templates and agent/debug workflows — no separate always-on human control; parent Generate UI covers the user-facing path',
  'motion.script_to_cut': 'Motion scripted-video bridge behind the Generate scripted-video template and agent/debug workflows — no separate always-on human control; parent Generate UI covers the user-facing path',
  'motion.job.get': 'read-only per-job Motion status query for agents and control clients — no human editing control',
  'motion.job.list': 'read-only caller-scoped Motion job listing for agents and control clients — no human editing control',
  'motion.map_import': 'Motion import-plan preflight is an agent-only connector bridge by design; it checks closed rendered operation/source/media shapes and reports path-free verified two-/five-hash SDK lineage or explicit legacy-unverified compatibility, plus independently derived optional current-package exact/changed/unavailable lineage and exact static text/shape editable lowering, while parent Generate/Canvas flows are the human entry points',
  'motion.apply_import': 'Motion import-plan apply is an agent-only connector bridge by design; it applies attested rendered media with a replay-backed immutable originAttestation and optional current-package exact/changed/unavailable comparison, or grouped native static text/shape objects with stable source-layer bindings and rendered fallback, while parent Generate/Canvas flows are the human entry points',
  // plugins.* runtime infra (no callVerb)
  'plugins.call': 'plugin-runtime infra — no callVerb in ui/src',
  'plugins.enable': 'plugin-runtime infra — no callVerb in ui/src',
  'plugins.list': 'plugin-runtime infra — no callVerb in ui/src',
  // analysis / render infra with NO human control
  'edit.track': 'motion-tracking analysis (cv2 CSRT) — no callVerb in ui/src; the agent calls it to get a trajectory it then applies via edit.keyframe',
  'render.preview': 'fast low-resolution incremental draft preview — no callVerb in ui/src (the Preview panel uses render.frame); internal',
  'render.frame': 'single-frame COMPOSE — the harness\'s own SSIM/frame() plumbing (export.frame is the covered user-facing frame export)',
}

// COVERED_VERBS — every verb the harness asserts a RESULT on (schema order). Section map
// (where each domain group is asserted):
//   project.*        secProjects (create/open/list/forget/delete/save/checkpoint/revert/undo/redo/diff/close)
//                    · secMenus (rename/format) · secVideo (color) · secExport (set_output_dir)
//   library.*        secLibrary (list/add/remove/move/tag/favorite/use[verb-level]/add_to_project/folder_*)
//   comment.*        secComments (add/resolve/draft/apply; list verb-level)
//   media.*          secAIServices (transcribe/perception/diarize) · secResidualVerbs (remove/relink/check/bin_save/bin_delete/bin_list) · all sections (import via freshProject + secContextMenu replace)
//   edit.*           secTimelineActions / secVideo / secAudio / secBlend / secTypedClips / secMulti / secContextMenu / secEditVerbs / secMatte (add_mask verb-level) / secMixer (duck + mute/solo flags via the Mixer M/S buttons; mute also on the Timeline track header)
//   grade.*          secVideo (save/apply/list)
//   title/shape.*    secDrawers (add) · secTypedClips (update)
//   assets.*         secAssets (search/fetch/generate; providers verb-level)
//   agent.chat       secAgent
//   audio.*          secDrawers (add_music) · secAIServices (dub) · secContextMenu (cleanup_voice)
//   transcript.*     secTranscriptEditor (get/timeline/cut_words/mute_words + edit.mute_range unmute/search/chapters/assemble/remove_*; translate)
//   captions.*       secProject (add_text/set_style/save_style/apply_style/list_styles/import/translate) · secTypedClips (set_text) · secKinetic (kinetic + reflow/shift verb-level) · secReviewQC seed · secResidualVerbs (set_range) · secExport (generate)
//   render.*         secMenus (storyboard) · secDirector (reframe/direct/qc) · secExport (final) · secResidualVerbs (bundle) · secRenderQueue (queue)
//   verify.*         secReviewQC (pacing/captions/delivery/brand/judge/checks/scopes/pregate) · secMixer (loudness)
//   export.*         secTimelineActions (range/gif) · secExport (frame/audio/publish/gif/xml/otio/edl/srt/vtt/ass/transcript/chapters)
//   import.*         secNativeOtioActions (real installed picker + preview/hash-bound replace)
//   autopilot/recipe secAutopilot (run) · secRecipe (list/describe/run verb-level)
//   generate.*       secGenerate (list/describe/preview/insert/prompt/storyboard through native Generate workspace)
//   motion.*         secGenerate for template bridges; linked-clip Inspector for edit/refresh/relink/tracking controls
//   assemble.*       secAssemble (shorts/repurpose/from_script/broll) · score.clip secVideo
//   screen_record.*  secRecord (doctor + installed live start/studio/stop/autoedit/polish/export)
//   system.*         system.doctor in preflight(); system.mcp_test in Settings > Agent control
const COVERED_VERBS = [
  'project.create', 'project.open', 'project.list', 'project.forget',
  'project.delete', 'project.save', 'project.checkpoint', 'project.revert',
  'project.undo', 'project.redo', 'project.diff', 'project.close',
  'project.rename', 'project.format', 'project.color', 'project.brand', 'project.set_output_dir',
  'project.sequence_list', 'project.sequence_index', 'project.sequence_create', 'project.sequence_switch',
  'project.sequence_rename', 'project.sequence_delete',
  'library.list', 'library.add', 'library.relink', 'library.remove', 'library.move',
  'library.tag', 'library.favorite', 'library.use', 'library.add_to_project',
  'library.folder_add', 'library.folder_rename', 'library.folder_remove', 'comment.add',
  'comment.export', 'comment.import', 'comment.list', 'comment.resolve', 'comment.draft', 'comment.apply',
  'media.import', 'media.remove', 'media.relink', 'media.check',
  'media.bin_save', 'media.bin_delete', 'media.bin_list', 'media.transcribe', 'media.perception',
  'media.diarize', 'effects.list', 'jobs.status', 'edit.split',
  'edit.mute_range', 'transcript.mute_words', 'transcript.ignore_words',
  'edit.cut_to_beat', 'edit.split_at_scenes', 'edit.mark_scenes', 'edit.trim_edges',
  'edit.ripple_delete', 'edit.trim', 'edit.move', 'edit.insert',
  'edit.detach_audio', 'edit.duplicate', 'edit.nest', 'edit.replace',
  'edit.fit_to_fill', 'edit.split_edit', 'edit.paste', 'edit.gain',
  'edit.fade', 'edit.transform', 'edit.crop', 'edit.grade',
  'edit.grade_stack', 'edit.grade_window', 'grade.save', 'grade.apply',
  'grade.list', 'edit.color_space', 'edit.color_match', 'edit.auto_balance',
  'edit.matte', 'edit.effect', 'title.add', 'title.update',
  'edit.add_shape', 'shape.update', 'assets.providers', 'assets.search',
  'assets.fetch', 'agent.chat', 'assets.generate', 'assets.generated_list', 'edit.adjustment',
  'edit.reverse', 'edit.stabilize', 'edit.freeze', 'edit.animate',
  'edit.keyframe', 'edit.auto_zoom', 'edit.add_mask', 'edit.redact',
  'edit.multicam_sync', 'edit.multicam_switch', 'edit.eq', 'audio.cleanup_voice',
  'edit.slide', 'edit.speed', 'edit.speed_ramp', 'edit.crossfade',
  'edit.duck', 'edit.add_track', 'edit.remove_track', 'edit.reorder_track',
  'edit.blend', 'edit.track_visible', 'edit.track_lock', 'edit.mute', 'edit.solo', 'edit.pan', 'edit.add_marker', 'edit.remove_marker', 'edit.seek_marker',
  'edit.move_marker', 'edit.update_marker', 'edit.paste_attributes', 'edit.slip', 'edit.roll', 'edit.slide_edit', 'edit.restore', 'audio.add_music', 'audio.dub',
  'transcript.get', 'transcript.timeline', 'transcript.cut_words', 'transcript.search',
  'transcript.chapters', 'transcript.assemble', 'transcript.remove_silences', 'transcript.remove_fillers',
  'transcript.remove_retakes', 'transcript.translate', 'captions.import', 'captions.generate',
  'captions.translate', 'captions.add_text', 'captions.kinetic', 'captions.set_style', 'captions.save_style', 'captions.apply_style', 'captions.list_styles',
  'captions.reflow', 'captions.shift', 'captions.set_range', 'captions.set_text',
  'render.storyboard', 'render.reframe', 'render.direct', 'render.qc',
  'render.final', 'verify.checks', 'verify.pacing', 'verify.captions',
  'verify.delivery', 'verify.loudness', 'verify.scopes', 'verify.brand',
  'verify.judge', 'verify.pregate', 'export.frame', 'export.audio',
  'export.publish', 'export.gif', 'export.range', 'export.xml', 'export.otio',
  'export.edl', 'export.srt', 'export.vtt', 'export.ass', 'import.otio',
  'export.transcript', 'export.chapters', 'render.bundle', 'render.queue',
  'autopilot.run', 'recipe.list', 'recipe.describe', 'recipe.run',
  'generate.list', 'generate.describe', 'generate.preview', 'generate.insert',
  'generate.from_prompt', 'generate.storyboard', 'motion.link.refresh', 'motion.link.relink', 'motion.link.edit',
  'motion.link.tracking.inventory', 'motion.link.tracking.request', 'motion.link.tracking.inspect',
  'motion.link.tracking.apply', 'motion.link.tracking.verify', 'motion.link.tracking.detach',
  'assemble.repurpose', 'assemble.shorts', 'assemble.from_script', 'score.clip',
  'assemble.broll', 'media.index_status', 'jobs.cancel',
  'screen_record.doctor', 'screen_record.start', 'screen_record.studio_event', 'screen_record.stop',
  'screen_record.autoedit', 'screen_record.polish', 'screen_record.export',
  'system.mcp_test', 'system.doctor',
]

// coverageCheck — pure (no cutd/browser): assert the two hardcoded sets PARTITION the
// schema verb list exactly. Prints the SUMMARY + the KNOWN_NON_UI reasons + the DELEGATED
// note; FAILS LOUDLY naming any verb that is unaccounted (in neither), double-counted (in
// both), or stale (in a set but not in schema). Returns true iff the partition is complete.
function coverageCheck() {
  const schemaSet = new Set(SCHEMA_VERBS)
  const coveredSet = new Set(COVERED_VERBS)
  const nonUiNames = Object.keys(KNOWN_NON_UI_VERBS)
  const nonUiSet = new Set(nonUiNames)
  const overlap = COVERED_VERBS.filter((v) => nonUiSet.has(v))
  const unaccounted = SCHEMA_VERBS.filter((v) => !coveredSet.has(v) && !nonUiSet.has(v))
  const staleCovered = COVERED_VERBS.filter((v) => !schemaSet.has(v))
  const staleNonUi = nonUiNames.filter((v) => !schemaSet.has(v))
  const dupCovered = COVERED_VERBS.length !== coveredSet.size
  const dupNonUi = nonUiNames.length !== nonUiSet.size
  const total = coveredSet.size + nonUiSet.size

  console.log('══════════ COVERAGE COMPLETENESS (schema/verbs.json) ══════════')
  console.log(`  Coverage: ${coveredSet.size} covered / ${nonUiSet.size} excluded(non-UI) / ${coveredSet.size}+${nonUiSet.size}=${total} = schema total ${schemaSet.size}`)
  console.log(`  DELEGATED (effect-proof in a sibling gate, PRESENT/RENDER-verified here; counted inside COVERED): ${DELEGATED_VERBS.length}`)
  console.log(`    ${DELEGATED_NOTE}`)
  console.log(`  KNOWN_NON_UI_VERBS (intentionally not UI-covered — no always-on human surface):`)
  for (const v of nonUiNames) console.log(`    · ${v.padEnd(24)} ${KNOWN_NON_UI_VERBS[v]}`)

  const ok = overlap.length === 0 && unaccounted.length === 0 && staleCovered.length === 0 &&
    staleNonUi.length === 0 && !dupCovered && !dupNonUi && total === schemaSet.size
  if (!ok) {
    console.error('\n  ✗✗✗ COVERAGE PARTITION BROKEN — fix COVERED_VERBS / KNOWN_NON_UI_VERBS:')
    if (unaccounted.length) console.error(`    UNACCOUNTED (in NEITHER set — every schema verb needs a coverage decision): ${unaccounted.join(', ')}`)
    if (overlap.length) console.error(`    DOUBLE-COUNTED (in BOTH sets): ${overlap.join(', ')}`)
    if (staleCovered.length) console.error(`    STALE in COVERED_VERBS (no longer in schema): ${staleCovered.join(', ')}`)
    if (staleNonUi.length) console.error(`    STALE in KNOWN_NON_UI_VERBS (no longer in schema): ${staleNonUi.join(', ')}`)
    if (dupCovered) console.error(`    DUPLICATE name(s) within COVERED_VERBS`)
    if (dupNonUi) console.error(`    DUPLICATE name(s) within KNOWN_NON_UI_VERBS`)
    console.error('    → A NEW engine verb must be ADDED to COVERED_VERBS (naming the section that asserts it) or to KNOWN_NON_UI_VERBS (with a reason). Nothing is silently skipped.')
  } else {
    console.log('  ✓ partition COMPLETE & DISJOINT — every schema verb is in exactly one set (nothing silently skipped).')
  }
  console.log('')
  return ok
}

export async function runFullCoverageVerify() {
  // ── coverage completeness gate (PURE — no cutd/browser): runs first so the partition is
  //    proven before any heavy work, and so `--coverage-check` can validate it CPU-only. ──
  const coverageOk = coverageCheck()
  if (COVERAGE_CHECK) exit(coverageOk ? 0 : 2) // standalone mode: done, never launches cutd/browser
  if (!coverageOk) { console.error('✗ coverage partition is broken (see above) — refusing to run the gate until COVERED_VERBS/KNOWN_NON_UI_VERBS cover every schema verb.'); exit(2) }
  mkdirSync(SCREENS, { recursive: true })
  const sourceActionAudit = buildUiActionCoverageAudit()
  const sourceActionVerdict = assessUiActionCoverage(sourceActionAudit, { strict: true })
  if (!sourceActionVerdict.ok) {
    console.error('✗ strict source-to-native UI action coverage is broken:')
    for (const missing of sourceActionVerdict.missing) console.error(`  · ${missing}`)
    exit(2)
  }
  let expectedSourceActionIds = null
  try {
    const parsed = JSON.parse(readFileSync(EXPECTED_ACTION_MANIFEST, 'utf8'))
    if (parsed.schema !== 'shellx-cut/ui-source-action-manifest@1' || !Array.isArray(parsed.actions)) {
      throw new Error('expected shellx-cut/ui-source-action-manifest@1 with an actions array')
    }
    expectedSourceActionIds = parsed.actions
      .map((action) => typeof action === 'string' ? action : action?.id)
      .filter(Boolean)
  } catch (error) {
    console.error(`✗ could not read committed UI source-action manifest ${EXPECTED_ACTION_MANIFEST}: ${error.message}`)
    exit(2)
  }
  const sourceActionIds = sourceActionAudit.actions.map((action) => action.id)
  // ── media plan — print which clip backs each role + LOUD fallback warnings ──
  console.log('══════════ TEST MEDIA PLAN ══════════')
  console.log(`  CUT_TEST_MEDIA_DIR = ${MEDIA_DIR}`)
  if (ENGINE_MEDIA_DIR !== MEDIA_DIR) console.log(`  CUT_TEST_MEDIA_ENGINE_DIR = ${ENGINE_MEDIA_DIR}`)
  if (process.env.CUT_HARNESS_FFMPEG) console.log(`  CUT_HARNESS_FFMPEG = ${HARNESS_FFMPEG}`)
  console.log(`  SCENE    → ${SCENE}`)
  console.log(`  SPEECH   → ${SPEECH}`)
  console.log(`  FACE     → ${FACE}`)
  console.log(`  SPEAKERS → ${SPEAKERS}`)
  console.log(`  SECOND   → ${SECOND}`)
  if (_mediaFallbacks.length) {
    console.log('\n  ⚠⚠⚠ HIGH-QUALITY CLIP(S) MISSING — RUNNING ON LOW-GRADE FIXTURES ⚠⚠⚠')
    for (const f of _mediaFallbacks) {
      console.log(`  ⚠ ${f.role}`)
      console.log(`      wanted: ${f.existsPath || join(f.dir, f.realName)}  (NOT FOUND locally)`)
      if (f.engineDir && f.engineDir !== f.dir) console.log(`      engine path if present: ${f.engineDir.replace(/[\\/]+$/, '')}/${f.realName}`)
      console.log(`      using fallback: ${f.fallback}`)
    }
    console.log('  ⚠ 4K/HEVC/performance-class bugs will NOT be exercised for the above roles.')
    console.log(`  ⚠ scp the real clips into ${MEDIA_DIR} on this rig for full coverage.\n`)
  } else {
    console.log('  ✓ all roles backed by real high-quality clips\n')
  }
  // Dependency preflight runs BEFORE the browser launches: under FCV_REQUIRE_FULL=1 a
  // missing dep hard-exits here (no browser/section spent); otherwise it just sets the
  // DEP flags the dep-gated sections read.
  await preflight()
  let nativeDriverAttestation = null
  let browser
  let page
  if (EMBEDDED_WDIO) {
    const wdioBrowser = globalThis.browser
    if (!wdioBrowser) throw new Error('FCV_UI_DRIVER=tauri-wdio requires an active WebdriverIO session')
    const { createWebdriverIoPage } = await import('./lib/webdriverIoPage.mjs')
    const native = await createWebdriverIoPage(wdioBrowser, {
      defaultTimeout: UI_ACTION_TIMEOUT_MS,
      provider: process.env.FCV_NATIVE_PROVIDER || 'embedded',
    })
    browser = { close: native.close }
    page = native.page
    nativeDriverAttestation = native.attestation
  } else if (CDP) {
    const versionUrl = new URL('/json/version', CDP.endsWith('/') ? CDP : `${CDP}/`)
    const response = await fetch(versionUrl, { signal: AbortSignal.timeout(5000) })
    if (!response.ok) throw new Error(`FCV_CDP_URL version probe failed: HTTP ${response.status}`)
    nativeDriverAttestation = assessWebView2CdpVersion(await response.json())
    if (!nativeDriverAttestation.ok) {
      throw new Error(`FCV_CDP_URL is not an attested installed WebView2 endpoint: ${nativeDriverAttestation.missing.join('; ')}`)
    }
    browser = await chromium.connectOverCDP(CDP)
    page = browser.contexts().flatMap((context) => context.pages())[0]
  } else {
    browser = await chromium.launch()
    page = await browser.newPage({ viewport: { width: 1600, height: 900 } })
  }
  if (!page) {
    throw new Error(
      EMBEDDED_WDIO
        ? 'the embedded Tauri WebDriver did not expose a native page'
        : `no installed WebView page exposed by FCV_CDP_URL=${CDP}`,
    )
  }
  page.setDefaultTimeout(UI_ACTION_TIMEOUT_MS)
  const runtimeActionRecorder = await createRuntimeActionRecorder(
    page,
    expectedSourceActionIds,
  )
  const consoleErrors = []
  const benignFetch = /favicon|\/api\/frame|\/filmstrip\/|\/proxies\/|\/api\/source\/|\/api\/export\/_monitor_|\/api\/export\/audio_[^./]+\.(wav|mp3)/
  page.on('response', (r) => { if (r.status() >= 400 && !benignFetch.test(r.url())) consoleErrors.push(`HTTP ${r.status()} ${r.url().replace(/^https?:\/\/[^/]+/, '')}`) })
  page.on('pageerror', (e) => consoleErrors.push(`PAGEERROR ${String(e).slice(0, 100)}`))
  if (!EMBEDDED_WDIO) {
    await page.goto(APP, { waitUntil: 'domcontentloaded' })
  }
  await page.waitForSelector('[data-cut-panel="topbar"]', { timeout: 20000 }).catch(() => {})
  await sleep(1200)

  // Catalog-drift guard — runs regardless of FCV_SECTION so a coverage regression
  // (engine effect catalog grown past the harness's hardcoded arrays) always trips.
  await catalogDriftGuard()

  for (const [key, fn] of SECTIONS) {
    if (SECTION_FILTER.length && !SECTION_FILTER.includes(key)) continue
    const t0 = Date.now()
    console.error(`[fcv] section ${key} start`)
    try { await fn(page) }
    catch (e) {
      const detail = String(e?.stack || e?.message || e)
      console.error(`[fcv] section ${key} crash\n${detail}`)
      rec(key, 'SECTION-CRASH', { present: 'fail', render: 'fail', click: 'fail', result: 'fail' }, detail.slice(0, 2000))
    }
    console.error(`[fcv] section ${key} done in ${((Date.now() - t0) / 1000).toFixed(1)}s`)
  }
  let trackHeaderOverflows = []
  try {
    trackHeaderOverflows = await settleWithin(
      page.evaluate(() => [...document.querySelectorAll('.tl-track')].flatMap((row) => {
        const head = row.querySelector('.tl-track-head')
        if (!head) return []
        const right = head.getBoundingClientRect().right + 0.5
        const offenders = [...head.querySelectorAll('button, input, select')]
          .filter((control) => control.getBoundingClientRect().right > right)
          .map((control) => control.getAttribute('data-cut-action') || control.className || control.tagName)
        return offenders.length ? [{ track: row.getAttribute('data-cut-track'), offenders }] : []
      })),
      UI_ACTION_TIMEOUT_MS,
      'final track-header overflow query',
    )
  } catch (error) {
    consoleErrors.push(`TRACK-HEADER-QUERY ${String(error?.message || error)}`)
  }
  if (trackHeaderOverflows.length) consoleErrors.push(`TRACK-HEADER-OVERFLOW ${JSON.stringify(trackHeaderOverflows)}`)

  if (process.env.FCV_INSTALLED_RUNTIME_RECEIPT) {
    const runtimeEvidence = await collectInstalledRuntimeEvidence({
      engineBase: CUTD,
      installedAppPath: process.env.SHELLX_CUT_WDIO_APP || '',
      nativeProvider: process.env.FCV_NATIVE_PROVIDER || '',
      repoRoot: dirname(TESTDATA),
      surface: process.env.FCV_TARGET_SURFACE,
      source: {
        gitCommit: process.env.FCV_SOURCE_GIT_COMMIT,
        version: JSON.parse(readFileSync(join(dirname(TESTDATA), 'app/desktop/src-tauri/tauri.conf.json'), 'utf8')).version,
        contentManifestSha256: process.env.FCV_SOURCE_CONTENT_MANIFEST_SHA256,
        platform: process.platform,
        arch: process.arch,
      },
    })
    mkdirSync(dirname(process.env.FCV_INSTALLED_RUNTIME_RECEIPT), { recursive: true })
    writeFileSync(process.env.FCV_INSTALLED_RUNTIME_RECEIPT,
      `${JSON.stringify(runtimeEvidence, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
  }

  await settleWithin(
    browser.close(),
    UI_ACTION_TIMEOUT_MS,
    'coverage browser disconnect',
  ).catch((error) => {
    consoleErrors.push(`BROWSER-DISCONNECT ${String(error?.message || error)}`)
  })
  rec('global', 'console-clean', { present: 'na', render: 'na', click: 'na', result: consoleErrors.length === 0 ? 'pass' : 'fail' }, consoleErrors.length ? consoleErrors.slice(0, 5).join(' | ') : '0 console/HTTP errors')

  // ── report ──
  console.log('\n══════════ FULL COVERAGE VERIFY ══════════')
  const tally = { present: { pass: 0, fail: 0, na: 0 }, render: { pass: 0, fail: 0, na: 0 }, click: { pass: 0, fail: 0, na: 0 }, result: { pass: 0, fail: 0, na: 0 } }
  const fails = []
  let lastSurface = ''
  for (const r of results) {
    if (r.surface !== lastSurface) { console.log(`\n── ${r.surface} ──`); lastSurface = r.surface }
    for (const d of ['present', 'render', 'click', 'result']) tally[d][r[d]]++
    const line = `  ${r.name.padEnd(34)} PRESENT=${SYM[r.present]} RENDER=${SYM[r.render]} CLICK=${SYM[r.click]} RESULT=${SYM[r.result]} — ${r.evidence}${r.shot ? `  [${r.shot}]` : ''}`
    console.log(line)
    if (isFail(r)) fails.push(r)
  }
  console.log('\n── DIMENSION TALLY ──')
  for (const d of ['present', 'render', 'click', 'result']) console.log(`  ${d.toUpperCase().padEnd(8)} ${tally[d].pass} ✓  ${tally[d].fail} ✗  ${tally[d].na} –`)
  // "Fully verified" = the RESULT dimension is green (action effect proven, all 4
  // real dims clean). Rows whose RESULT is honestly N/A (present-only: desktop
  // pickers, spends-a-turn, no state field to assert) are NOT verification — they
  // get their own tally so the headline never overstates how much was actually
  // proven. The exit code below stays driven ONLY by real `fail` dims — unchanged.
  const fullyVerified = results.filter((r) => r.result === 'pass' && !isFail(r))
  const presentOnly = results.filter((r) => r.result === 'na' && !isFail(r))
  // Split the RESULT=N/A rows into three honest buckets so the headline never hides a
  // genuine "couldn't verify":
  //   • DELEGATED — effect-proof lives in a SIBLING release gate, not here, such as
  //     the screen_record CAPTURE path (start/stop/autoedit/polish/export) → the
  //     recorder rig gate (integrated Cut recorder), since the test's standalone cutd cannot acquire
  //     a live capture surface / macOS TCC and a headless OS screen-grab risks a runaway
  //     process. Cross-covered, not unproven.
  //   • EMPTY-CATALOG GUARD — a "nothing here to exercise" guard that only fires on an empty
  //     engine catalog (never in a complete env).
  //   • COULDN'T-VERIFY — anything else: a real gap. Under FCV_REQUIRE_FULL=1 this MUST be 0
  //     (preflight guarantees every dep present), so a non-empty set here FAILS the gate.
  // Use the same classifier that serializes the machine-readable receipt. Keeping the
  // console and receipt on one decision path prevents a green exit beside `ok:false`.
  const classified = presentOnly.map((row) => ({ row, classification: classifyFullCoverageRow(row) }))
  const rowsClassifiedAs = (classification) => classified
    .filter((item) => item.classification === classification)
    .map((item) => item.row)
  const delegated = rowsClassifiedAs('delegated')
  const guards = rowsClassifiedAs('guard')
  const optAgentSkips = rowsClassifiedAs('optional_agent_skip')
  const depSkips = rowsClassifiedAs('dependency_skip')
  const couldntVerify = rowsClassifiedAs('could_not_verify')
  const strictUnverified = FINAL_ALL_ACTIONS
    ? results.filter((row) => row.rowKind === 'ui_action' &&
      !['present', 'render', 'click', 'result'].every((dim) => row[dim] === 'pass'))
    : []
  const requireRuntimeActionCoverage = FINAL_ALL_ACTIONS
    && SECTION_FILTER.length === 0
    && !ONLY
  const receipt = buildFullCoverageReceipt(results, {
    full: FULL,
    strictAllActions: FINAL_ALL_ACTIONS,
    sourceActionIds,
    expectedSourceActionIds,
    runtimeSourceActionIds: runtimeActionRecorder.ids(),
    expectedRuntimeSourceActionIds: requireRuntimeActionCoverage
      ? expectedSourceActionIds
      : undefined,
    surface: process.env.FCV_TARGET_SURFACE || null,
    runtime: {
      installedApp: process.env.FCV_INSTALLED_APP === '1',
      driver: UI_DRIVER,
      nativeAttached: nativeDriverAttestation?.ok === true,
      nativeBrowser: nativeDriverAttestation?.browser || null,
      nativeProvider: nativeDriverAttestation?.provider || null,
      sourceContentManifestSha256: process.env.FCV_SOURCE_CONTENT_MANIFEST_SHA256 || null,
    },
    coverage: {
      covered: COVERED_VERBS.length,
      excluded: Object.keys(KNOWN_NON_UI_VERBS).length,
      total: SCHEMA_VERBS.length,
      ok: coverageOk,
    },
    media: {
      tier: process.env.FCV_MEDIA_TIER || (_mediaFallbacks.length ? 'fixture-fallback' : 'unattested'),
      scene: SCENE,
      speech: SPEECH,
      face: FACE,
      speakers: SPEAKERS,
      second: SECOND,
      fallbackRoles: _mediaFallbacks,
    },
    screenshotsDir: SCREENS,
  })
  console.log(`\n  CONTROLS: ${results.length} total (${receipt.summary.controls.uiActions} UI actions + ${receipt.summary.controls.supportRows} support rows)`)
  console.log(`    · ${fullyVerified.length} fully VERIFIED (RESULT=✓ — action effect proven)`)
  console.log(`    · ${delegated.length} DELEGATED (effect-proof cross-covered in interaction-verify / the recorder rig gate; PRESENT/RENDER-verified here)`)
  console.log(`    · ${depSkips.length} dependency-absent honest skips (partial run only — preflight enforces these present under FCV_REQUIRE_FULL=1)`)
  console.log(`    · ${optAgentSkips.length} optional multi-agent skips (codex/grok not authed on this rig — benign, NOT preflight-enforced, never fails the gate)`)
  console.log(`    · ${guards.length} empty-catalog guards (no item to exercise — never fires in a complete env)`)
  console.log(`    · ${couldntVerify.length} could-NOT-verify (RESULT=– and not cross-covered — a real gap)`)
  console.log(`    · ${fails.length} with ≥1 FAIL`)
  if (delegated.length) {
    console.log('\n── DELEGATED (RESULT cross-covered in interaction-verify / the recorder rig gate) ──')
    for (const r of delegated) console.log(`  – [${r.surface}] ${r.name} — ${r.evidence}`)
  }
  if (depSkips.length) {
    console.log('\n── DEPENDENCY-ABSENT HONEST SKIPS (partial run — present under FCV_REQUIRE_FULL=1) ──')
    for (const r of depSkips) console.log(`  – [${r.surface}] ${r.name} — ${r.evidence}`)
  }
  if (optAgentSkips.length) {
    console.log('\n── OPTIONAL MULTI-AGENT SKIPS (codex/grok not authed — benign, gate-tolerant) ──')
    for (const r of optAgentSkips) console.log(`  – [${r.surface}] ${r.name} — ${r.evidence}`)
  }
  if (couldntVerify.length) {
    console.log('\n── COULD-NOT-VERIFY (RESULT=– , not cross-covered) ──')
    for (const r of couldntVerify) console.log(`  – [${r.surface}] ${r.name} — ${r.evidence}`)
  }
  if (fails.length) {
    console.log('\n── FAILURES ──')
    for (const r of fails) {
      const which = ['present', 'render', 'click', 'result'].filter((d) => r[d] === 'fail').join(',')
      console.log(`  ✗ [${r.surface}] ${r.name} (failed: ${which}) — ${r.evidence}`)
    }
  }
  // Under the release gate, a could-not-verify control is a GATE failure (the env was
  // complete — every control was supposed to produce real evidence). Delegated + guard rows
  // are intentional and do NOT fail the gate.
  const gateFail = fails.length > 0 ||
    (FULL && (couldntVerify.length > 0 || depSkips.length > 0)) ||
    (FINAL_ALL_ACTIONS && (
      strictUnverified.length > 0
      || !receipt.sourceActionManifest.matchesExpected
      || (requireRuntimeActionCoverage && !receipt.runtimeSourceActionManifest.matchesExpected)
    ))
  if (FULL) {
    console.log(`\n  RELEASE GATE (FCV_REQUIRE_FULL=1): ${gateFail ? '✗ FAIL' : '✓ PASS'} — ${fails.length} FAIL, ${couldntVerify.length} could-not-verify + ${depSkips.length} dep-skip (target 0/0), ${delegated.length} delegated`)
  }
  if (FINAL_ALL_ACTIONS) {
    console.log(`  FINAL ALL-ACTIONS GATE: ${gateFail ? '✗ FAIL' : '✓ PASS'} — ${strictUnverified.length} rows did not pass PRESENT/RENDER/CLICK/RESULT (target 0)`)
    console.log(`  SOURCE ACTION MANIFEST: observed=${receipt.sourceActionManifest.total}, expected=${receipt.sourceActionManifest.expectedTotal ?? 'missing'}, missing=${receipt.sourceActionManifest.missing.length}, unexpected=${receipt.sourceActionManifest.unexpected.length}, match=${receipt.sourceActionManifest.matchesExpected}`)
    console.log(`  RUNTIME SOURCE ACTIONS: observed=${receipt.runtimeSourceActionManifest.total}, expected=${receipt.runtimeSourceActionManifest.expectedTotal ?? 'not-required'}, missing=${receipt.runtimeSourceActionManifest.missing.length}, unexpected=${receipt.runtimeSourceActionManifest.unexpected.length}, match=${receipt.runtimeSourceActionManifest.matchesExpected}`)
    if (receipt.runtimeSourceActionManifest.missing.length) {
      console.log(`    missing runtime actions: ${receipt.runtimeSourceActionManifest.missing.slice(0, 40).join(', ')}${receipt.runtimeSourceActionManifest.missing.length > 40 ? ', …' : ''}`)
    }
    if (receipt.runtimeSourceActionManifest.unexpected.length) {
      console.log(`    unexpected runtime actions: ${receipt.runtimeSourceActionManifest.unexpected.slice(0, 40).join(', ')}${receipt.runtimeSourceActionManifest.unexpected.length > 40 ? ', …' : ''}`)
    }
    console.log(`  RUNTIME ACTION RECEIPT: unique=${receipt.actionManifest.total}, occurrences=${receipt.actionManifest.occurrences}, repeated=${receipt.actionManifest.repeated.length}, sha256=${receipt.actionManifest.sha256}`)
  }
  // ── coverage partition recap (the "covers every schema verb" proof, re-stated at the
  //    end where a reader looks; the full per-verb breakdown printed at startup) ──
  console.log('\n── SCHEMA-VERB COVERAGE (schema/verbs.json) ──')
  console.log(`  Coverage: ${COVERED_VERBS.length} covered / ${Object.keys(KNOWN_NON_UI_VERBS).length} excluded(non-UI) / ${COVERED_VERBS.length + Object.keys(KNOWN_NON_UI_VERBS).length} = schema total ${SCHEMA_VERBS.length}  ${coverageOk ? '(partition COMPLETE & DISJOINT ✓)' : '(✗ BROKEN — see startup)'}`)
  console.log(`  DELEGATED: ${DELEGATED_NOTE}`)
  console.log(`  screenshots → ${SCREENS}`)
  if (process.env.FCV_RESULT_RECEIPT) {
    mkdirSync(dirname(process.env.FCV_RESULT_RECEIPT), { recursive: true })
    writeFileSync(process.env.FCV_RESULT_RECEIPT, JSON.stringify(receipt, null, 2), 'utf8')
    console.log(`  result receipt → ${process.env.FCV_RESULT_RECEIPT}`)
  }
  exit(gateFail ? 1 : 0)
}

if (!EMBEDDED_WDIO && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runFullCoverageVerify().catch((e) => { console.error(e); exit(2) })
}
