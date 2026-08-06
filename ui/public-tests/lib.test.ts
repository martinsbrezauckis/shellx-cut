// ui/public-tests/lib.test.ts — pure-function unit tests (run: `npm run test:lib`).
//
// Role: deterministic checks for the load-bearing string/URL helpers that the
// Playwright sweep can't isolate (and that only misbehave on platforms the dev
// box isn't). No DOM, no network — imports the REAL modules via tsx so a
// regression in the shipped code fails here.
//
// Tiny zero-dependency harness (no vitest): a failing case prints and flips the
// exit code. Add cases here when you fix a pure helper.

import { existsSync, readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { resolveMediaRole } from '../../scripts/lib/cross-host-media.mjs'
import { resolveCoverageAppUrl } from './lib/fullCoverageAppUrl.mjs'
import { createProjectWithRetry } from './lib/fullCoverageProjectTransition.mjs'
import { createRuntimeActionRecorder } from './lib/fullCoverageRuntimeActionRecorder.mjs'
import { parseSsimAll } from './lib/fullCoverageVisual.mjs'
import { exportUrl, mediaClipTimelineDurationMs, UI_OPEN_PANELS, type ClipEffect } from '../src/lib/client'
import { baseVideoTrackId, isTrackLocked } from '../src/lib/layerStack'
import { libraryIdFromAssetHash, mediaBasename } from '../src/lib/mediaPath'
import { offlineAssetView, offlineMediaMaps } from '../src/lib/offlineMedia'
import { sourceMsAtTimelinePosition, timelineMsAtSourcePosition } from '../src/lib/mediaTime'
import { resolveCommentTime } from '../src/lib/commentAnchors'
import { cardLabel, isFfmpegMissing, STT_MODELS, type DoctorCard } from '../src/lib/doctor'
import { cutManualFeatureUrl } from '../src/lib/manual'
import { outputDirectoryForPath } from '../src/lib/exportDestination'
import {
  FIXED_KEY_ACTIONS,
  KEY_ACTIONS,
  bindingFromEvent,
  conflictsFor,
  matchesFixedAction,
} from '../src/lib/keymap'
import { planAssetInsertAtPlayhead, planTimelineAssetDrop, trackEndMs } from '../src/lib/placement'
import { assetReadiness, mediaCapabilitiesFromDoctor, summarizeMediaReadiness } from '../src/panels/Assets/mediaReadiness'
import { libraryMembershipBatches } from '../src/panels/Assets/libraryMembership'
import { libraryDetailItem } from '../src/panels/Library/model'
import { nextLibraryItemIndex } from '../src/panels/Library/useLibraryKeyboardNavigation'
import { laidToEditorialMs, layoutTrack, linkedSiblings, planLinkedSplit, sourceTimelineOccurrences, trackRows, trackSeams, type LaidItem } from '../src/panels/Timeline/layout'
import { adjacentGapSlot } from '../src/panels/Timeline/ClipContextMenuModel'
import { trackOrderStatus, trackReorderTargetIndex } from '../src/panels/Timeline/trackControlsModel'
import { planRippleTrimAtPlayhead, sourceTrimAtTimelinePosition } from '../src/panels/Timeline/rippleTrim'
import { timelineEditFailureMessage } from '../src/panels/Timeline/editFeedback'
import { userActionFailureDetail } from '../src/lib/userActionFeedback'
import { COMMANDS } from '../src/palette/commands'
import { EXPORT_OPTIONS } from '../src/topbar/model'
import { preferredProjectLeftTab, shouldReturnToProjectsAfterResync } from '../src/app/model'
import {
  availableProjectName,
  isSupportedMediaPath,
  projectNameFromMediaPath,
  supportedMediaKind,
} from '../src/lib/projectBootstrap'
import { UI_SURFACES, uiSurface } from '../src/app/uiSurfaceRegistry'
import { mcpConfigText, type AgentDiscovery } from '../src/lib/agentControl'
import { trackAuditionExportError } from '../src/components/trackAuditionModel'
import {
  effectParameterSummary,
  moveClipEffect,
  toggleClipEffect,
} from '../src/panels/Inspector/effectChainModel'
import {
  audioCleanupSummary,
  duckingSummary,
  stabilizationReadiness,
  videoColorSummary,
  videoEffectsSummary,
  videoMotionSummary,
  videoPrivacySummary,
} from '../src/panels/Inspector/inspectorTaskModel'
import type { InspectorMediaClip } from '../src/panels/Inspector/model'
import {
  defaultTrackingAnalysisId,
  normalizedTrackingRegion,
  trackingModelForMode,
  trackingVerificationLabel,
} from '../src/panels/Inspector/motionTrackingModel'
import { activeCutSpans } from '../src/panels/Review/shared'
import { activeBaseAssetId, activeVideo, previewFrameMs } from '../src/panels/Preview/model'
import { resolveCaptions, resolveOverlays, shouldUseLivePreviewSurface } from '../src/panels/Preview/composite'
import {
  chatAttachmentLabel,
  chatAttachmentOptions,
  MAX_CHAT_ATTACHMENTS,
  toggleChatAttachment,
} from '../src/panels/AgentChat/attachmentModel'
import {
  AGENT_PROMPT_CATEGORIES,
  AGENT_PROMPT_LIBRARY,
  AGENT_QUICK_PROMPTS,
} from '../src/panels/AgentChat/promptLibrary'
import { recipeNeedsPreview } from '../src/panels/Recipes/model'
import {
  SETTINGS_CATEGORY_IDS,
  searchSettings,
} from '../src/panels/Environment/settingsModel'
import {
  buildShortcutSettingsRows,
  filterShortcutSettingsRows,
  shortcutSettingsCounts,
} from '../src/panels/Environment/keymapSettingsModel'

const audioSync = await import('../src/panels/Preview/audioSync').catch(() => null)

let failures = 0
function eq(actual: unknown, expected: unknown, label: string): void {
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  if (!ok) failures++
  // eslint-disable-next-line no-console
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}`)
  if (!ok) {
    // eslint-disable-next-line no-console
    console.log(`        got=${JSON.stringify(actual)}\n        want=${JSON.stringify(expected)}`)
  }
}

// --- Full-coverage SSIM evidence parsing -----------------------------------
{
  eq(parseSsimAll('SSIM Y:-0.25 U:0.1 V:0.2 All:-0.125 (0.1)'), -0.125, 'SSIM evidence accepts a valid negative score from inverted frames')
  eq(parseSsimAll('SSIM All:1.25e-3 (0.1)'), 0.00125, 'SSIM evidence accepts scientific notation')
  eq(parseSsimAll('ffmpeg failed before producing a score'), null, 'SSIM evidence rejects output without an All score')
}

// --- Installed/native conditional fixture origin ---------------------------
{
  const configured = await resolveCoverageAppUrl({
    evaluate: async () => { throw new Error('configured URL must not inspect the page') },
  }, 'http://localhost:5173/project?old=1#stale')
  eq(configured, 'http://localhost:5173/project', 'Conditional fixtures strip query and hash from a configured app URL')

  const installed = await resolveCoverageAppUrl({
    evaluate: async () => 'tauri://localhost/project?mock=1#fixture',
  })
  eq(installed, 'tauri://localhost/project', 'Conditional fixtures retain the installed Tauri origin')
}

// --- Native full-coverage project transition retry -------------------------
{
  let now = 0
  const requests: Array<{ name?: string }> = []
  const pending = {
    ok: false,
    error: { code: 'job_cancel_pending', message: 'worker still draining' },
  }
  const result = await createProjectWithRetry({
    verb: async (_name: string, args: { name?: string }) => {
      requests.push(args)
      return requests.length < 3
        ? pending
        : { ok: true, result: { path: '/fixture/retry.cutproj' } }
    },
    name: 'fcv_retry_fixture',
    settings: { width: 1280, height: 720, fps: 30 },
    timeoutMs: 1_000,
    retryDelayMs: 100,
    sleepFn: async (ms: number) => { now += ms },
    nowFn: () => now,
  })
  eq(result.attempts, 3, 'Project transition retries only the temporary job drain refusal')
  eq(result.response.result?.path, '/fixture/retry.cutproj', 'Project transition returns the successful create response')
  eq(requests.every((request) => request.name === 'fcv_retry_fixture'), true, 'Project transition keeps one collision-safe name across retries')

  const fatal = await createProjectWithRetry({
    verb: async () => ({ ok: false, error: { code: 'invalid_args' } }),
    name: 'fcv_fatal_fixture',
    settings: {},
    sleepFn: async () => { throw new Error('fatal project errors must not sleep') },
  })
  eq(fatal.attempts, 1, 'Project transition does not retry unrelated failures')
  eq(fatal.response.error?.code, 'invalid_args', 'Project transition preserves unrelated project errors')

  let timeoutMessage = ''
  now = 0
  try {
    await createProjectWithRetry({
      verb: async () => pending,
      name: 'fcv_timeout_fixture',
      settings: {},
      timeoutMs: 100,
      retryDelayMs: 40,
      sleepFn: async (ms: number) => { now += ms },
      nowFn: () => now,
    })
  } catch (error) {
    timeoutMessage = String(error instanceof Error ? error.message : error)
  }
  eq(timeoutMessage.includes('after 4 attempts and 100 ms'), true, 'Project transition timeout reports bounded retry evidence')
}

// --- Task-oriented Inspector summaries and setup truth ---------------------
{
  const clip = {
    id: 'clip-inspector',
    asset: 'asset-inspector',
    src_in_ms: 0,
    src_out_ms: 10_000,
  } as InspectorMediaClip
  eq(videoMotionSummary(clip), {
    label: 'Stabilization and auto zoom',
    tone: 'neutral',
  }, 'Inspector motion summary explains the available task when inactive')
  eq(videoMotionSummary({
    ...clip,
    stabilize: { enabled: true },
    keyframes: [
      { param: 'scale', points: [{ t_ms: 0, value: 1 }] },
      { param: 'scale', points: [{ t_ms: 500, value: 1.1 }] },
    ],
  }), {
    label: 'Stabilized · 2 zoom keyframes',
    tone: 'active',
  }, 'Inspector motion summary reports applied state while collapsed')
  eq(videoColorSummary({
    ...clip,
    grade: { contrast: 1.2 },
    grade_stack: [{ saturation: 1.1 }],
    grade_windows: [{
      window: { shape: 'rect', points: [[0.1, 0.1], [0.9, 0.9]] },
      grade: { exposure: 0.2 },
    }],
    input_color_space: 'srgb',
  }), {
    label: 'Grade applied · 1 layer · 1 window · SRGB',
    tone: 'active',
  }, 'Inspector Color summary exposes grade layers and windows while collapsed')
  eq(videoEffectsSummary({ ...clip, effects: [{ type: 'invert' }] }, 'screen'), {
    label: '1 effect · screen blend',
    tone: 'active',
  }, 'Inspector Effects summary exposes effect count and blend while collapsed')
  eq(videoPrivacySummary({ ...clip, mask: { enabled: true }, matte: { tier: 'rvm' } }), {
    label: 'Redaction applied · Background removed',
    tone: 'active',
  }, 'Inspector Privacy summary exposes applied redaction and matte state')
  eq(audioCleanupSummary({ ...clip, effects: [{ type: 'denoise' }], eq: { high_pass_hz: 80 } }), {
    label: '1 effect · EQ applied',
    tone: 'active',
  }, 'Inspector Audio effects summary exposes applied cleanup state')
  eq(duckingSummary(null), {
    label: 'Needs a second audio track',
    tone: 'warning',
  }, 'Inspector ducking summary names its missing prerequisite')
  eq(stabilizationReadiness(null), {
    ready: false,
    reason: 'Checking the installed video tools…',
  }, 'Inspector stabilization waits honestly for environment truth')
  eq(stabilizationReadiness({
    schema: 'cut-doctor-v1',
    scanned_at: '2026-07-28T00:00:00Z',
    os: 'test',
    arch: 'test',
    app_version: 'test',
    essential_ok: true,
    cards: [{
      id: 'ffmpeg',
      kind: 'tool',
      status: 'ok',
      details: { can_stabilize: true },
    }],
  }), {
    ready: true,
    reason: null,
  }, 'Inspector stabilization enables only from verified installed capability')
}

// --- Bounded cross-surface Library membership ------------------------------
{
  const ids = Array.from({ length: 1_001 }, (_, index) => `id-${index}`)
  eq(
    libraryMembershipBatches(ids).map((batch) => batch.length),
    [500, 500, 1],
    'Library membership splits 1,001 exact ids at the public 500-id cap',
  )
  eq(
    libraryMembershipBatches(['late-page', 'first-page', 'late-page'], 2),
    [['late-page', 'first-page']],
    'Library membership deduplicates without dropping an id beyond page one',
  )
  eq(nextLibraryItemIndex(100, 50, 'ArrowUp'), 49, 'Library ArrowUp moves to the previous media row')
  eq(nextLibraryItemIndex(100, 50, 'ArrowDown'), 51, 'Library ArrowDown moves to the next media row')
  eq(nextLibraryItemIndex(100, 50, 'Home'), 0, 'Library Home moves to the first media row')
  eq(nextLibraryItemIndex(100, 50, 'End'), 99, 'Library End moves to the last media row')
  eq(nextLibraryItemIndex(100, 0, 'ArrowUp'), 0, 'Library row navigation stops at the first boundary')
  eq(nextLibraryItemIndex(100, 99, 'ArrowDown'), 99, 'Library row navigation stops at the last boundary')

  const detailItems = [{ id: 'a' }, { id: 'b' }]
  eq(libraryDetailItem(detailItems, [], 'b')?.id, 'b', 'Library details follow keyboard or pointer focus')
  eq(libraryDetailItem(detailItems, [detailItems[0]], 'b')?.id, 'a', 'Library single selection owns the details pane')
  eq(libraryDetailItem(detailItems, detailItems, 'b'), null, 'Library multi-selection keeps item details unambiguous')
}

// --- Searchable cross-sequence clip/marker index ---------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(root, 'ui/src')
  const schema = readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')
  const dispatch = readFileSync(resolve(root, 'app/server/src/dispatch.rs'), 'utf8')
  const registry = readFileSync(resolve(root, 'app/server/src/registry.rs'), 'utf8')
  const client = readFileSync(resolve(srcRoot, 'lib/client.ts'), 'utf8')
  const results = readFileSync(resolve(srcRoot, 'lib/clientResults.ts'), 'utf8')
  const layout = readFileSync(resolve(srcRoot, 'layout/useLayout.ts'), 'utf8')
  const app = readFileSync(resolve(srcRoot, 'App.tsx'), 'utf8')
  const leftPanel = readFileSync(resolve(srcRoot, 'panels/LeftPanel/index.tsx'), 'utf8')
  const panel = readFileSync(resolve(srcRoot, 'panels/SequenceIndex/index.tsx'), 'utf8')
  const verify = readFileSync(resolve(here, 'verify-sequence-index.mjs'), 'utf8')

  eq(schema.includes('"name": "project.sequence_index"'), true, 'schema exposes the read-only cross-sequence index')
  eq(dispatch.includes('"project.sequence_index" => project_sequence_index'), true, 'server dispatch routes project.sequence_index')
  eq(registry.includes('"project.sequence_index"'), true, 'verb registry exposes project.sequence_index')
  eq(client.includes("'project.sequence_index': { query?: string; kind?: 'all' | 'clip' | 'marker'"), true, 'typed client pins Sequence Index filters')
  eq(results.includes("'project.sequence_index': SequenceIndexResult"), true, 'typed client pins Sequence Index rows')
  eq(layout.includes("export type FindSurface = 'find-media' | 'find-moment' | 'sequence-index'"), true, 'layout persists Sequence Index as a Find surface')
  eq(layout.includes("findSurfaceValue === 'sequence-index'"), true, 'layout restores persisted Sequence Index state')
  eq(uiSurface('sequence-index')?.action, { kind: 'find', surface: 'sequence-index' }, 'ui.open can reveal Sequence Index')
  eq(leftPanel.includes('data-cut-find-tab="sequence-index"'), true, 'Find rail exposes the Sequence tab')
  eq(leftPanel.includes('<SequenceIndex project={project} onProjectChanged={onProjectChanged} />'), true, 'Find rail mounts the native Sequence Index panel')
  eq(leftPanel.includes("document.addEventListener('cut:open-source-monitor', revealSourceMonitor)"), true, 'Source requests reveal the mounted Assets source monitor')
  eq(leftPanel.includes("document.removeEventListener('cut:open-source-monitor', revealSourceMonitor)"), true, 'Source-monitor routing listener is removed on unmount')
  eq(panel.includes("callVerb('project.sequence_index'"), true, 'Sequence Index loads through the public verb contract')
  eq(panel.includes('data-cut-sequence-index-status'), true, 'Sequence Index exposes live issue and track-state filters')
  eq(panel.includes('sequenceIndexCsv') && panel.includes('data-cut-sequence-index-copy'), true, 'Sequence Index exposes bounded path-light CSV handoff')
  eq(panel.includes('loadGeneration.current') && panel.includes('generation !== loadGeneration.current'), true, 'Sequence Index ignores stale filter responses')
  eq(panel.includes("callVerb('project.sequence_switch'"), true, 'Sequence Index can open an inactive sequence')
  eq(panel.includes("callVerb('ui.playhead'"), true, 'Sequence Index navigation moves the playhead')
  eq(verify.includes("full source path is absent from Sequence Index DOM"), true, 'browser gate protects path-light index rendering')
  eq(verify.includes("Source action reveals the source monitor instead of opening it behind Find"), true, 'browser gate proves source results become visible')
  eq(verify.includes("inactive-sequence result switches sequence and seeks"), true, 'browser gate proves cross-sequence navigation')
}

eq(mediaClipTimelineDurationMs({ src_in_ms: 1000, src_out_ms: 5000, speed: 2 }), 2000, '2x media clip timeline duration is half its source span')
eq(SETTINGS_CATEGORY_IDS.length, 10, 'Settings has a deliberate ten-destination information architecture')
eq(searchSettings('MCP').map((result) => result.category.id), ['agent-control'], 'Settings search routes MCP to Agent control')
eq(searchSettings('keyboard').map((result) => result.category.id), ['editing'], 'Settings search routes keyboard to Editing')
{
  const executable = 'C:\\Program Files\\ShellX Cut\\cutd.exe'
  const discovery = {
    schema: 'shellx-cut/agent-docs/2',
    mcp_client_config: {
      mcpServers: {
        'shellx-cut': { command: executable, args: ['mcp'] },
      },
    },
  } as unknown as AgentDiscovery
  const config = JSON.parse(mcpConfigText(discovery))
  eq(config.mcpServers['shellx-cut'].command, executable, 'Agent control copies the exact installed MCP executable')
  eq(config.mcpServers['shellx-cut'].args, ['mcp'], 'Agent control copies proxy mode instead of standalone state')
}
{
  const commentsChord = {
    key: 'C',
    ctrlKey: true,
    metaKey: false,
    altKey: false,
    shiftKey: true,
  } as KeyboardEvent
  eq(bindingFromEvent(commentsChord), 'Ctrl+Shift+C', 'Key binding capture preserves Shift in modified letter chords')
  eq(matchesFixedAction(commentsChord, 'comments.toggle'), true, 'Fixed Comments chord matches Ctrl+Shift+C')
  eq(KEY_ACTIONS.some((action) => action.id.startsWith('recording.')), false, 'Hard-coded recording keys are not remappable')
  eq(FIXED_KEY_ACTIONS.filter((action) => action.id.startsWith('recording.')).length, 5, 'Fixed shortcut table owns all recording keys')
  eq(
    conflictsFor('F9', 'timeline.split').some((action) => action.id === 'recording.toggle'),
    true,
    'Remap conflict detection reserves the native F9 recording key',
  )
  const bindings: Record<string, string> = {
    'preview.playPause': 'F9',
    'timeline.split': 'Alt+S',
  }
  const changed = new Set(['preview.playPause', 'timeline.split'])
  const rows = buildShortcutSettingsRows(
    (id) => bindings[id] ?? KEY_ACTIONS.find((action) => action.id === id)?.def ?? '',
    (id) => changed.has(id),
  )
  eq(
    shortcutSettingsCounts(rows),
    { commands: KEY_ACTIONS.length + FIXED_KEY_ACTIONS.length, changed: 2, conflicts: 2 },
    'Shortcut Settings summary counts commands, changes, and both sides of a fixed-key conflict',
  )
  eq(
    filterShortcutSettingsRows(rows, 'split', 'timeline', 'changed').map((row) => row.id),
    ['timeline.split'],
    'Shortcut Settings composes text, group, and Changed filters',
  )
  eq(
    filterShortcutSettingsRows(rows, '', 'all', 'conflicts').map((row) => row.id),
    ['preview.playPause', 'recording.toggle'],
    'Shortcut Settings conflict filter includes editable and fixed owners',
  )
}
eq(chatAttachmentLabel('C:\\media\\hero.mov', 'a1'), 'hero.mov', 'Chat attachment labels Windows project assets by filename')
eq(chatAttachmentLabel('/media/interview.wav', 'a2'), 'interview.wav', 'Chat attachment labels POSIX project assets by filename')
eq(
  chatAttachmentOptions({ assets: { z: { path: '/b/zed.mov' }, a: { path: '/a/alpha.mov' } } } as never),
  [{ id: 'a', label: 'alpha.mov' }, { id: 'z', label: 'zed.mov' }],
  'Chat attachment options are stable and sorted without exposing paths to the request',
)
eq(toggleChatAttachment(['a1'], 'a2'), ['a1', 'a2'], 'Chat attachments add a registered asset ID')
eq(toggleChatAttachment(['a1', 'a2'], 'a1'), ['a2'], 'Chat attachments toggle an existing asset ID off')
eq(
  toggleChatAttachment(Array.from({ length: MAX_CHAT_ATTACHMENTS }, (_, i) => `a${i}`), 'extra'),
  Array.from({ length: MAX_CHAT_ATTACHMENTS }, (_, i) => `a${i}`),
  'Chat attachments stop at the per-turn limit',
)
const schemaVerbNames = new Set(
  (JSON.parse(readFileSync(new URL('../../schema/verbs.json', import.meta.url), 'utf8')) as { verbs: Array<{ name: string }> })
    .verbs.map((verb) => verb.name),
)
eq(AGENT_PROMPT_LIBRARY.length, 8, 'Agent prompt library has a deliberate catalog-size tripwire')
eq(AGENT_PROMPT_CATEGORIES, ['Polish', 'Repurpose', 'Speech', 'Review'], 'Agent prompt library groups common outcomes for scanning')
eq(AGENT_QUICK_PROMPTS.map((preset) => preset.label).sort(), ['Dub to Latvian', 'Label speakers', 'Repurpose as shorts'], 'Agent prompt library preserves the three quick actions')
eq(new Set(AGENT_PROMPT_LIBRARY.map((preset) => preset.id)).size, AGENT_PROMPT_LIBRARY.length, 'Agent prompt library IDs are unique')
eq(AGENT_PROMPT_LIBRARY.every((preset) => preset.prompt.trim().length > 0), true, 'Agent prompt library never ships an empty request')
eq(AGENT_PROMPT_LIBRARY.every((preset) => preset.verbs.every((verb) => schemaVerbNames.has(verb))), true, 'Agent prompt library maps only to real schema verbs')
eq(recipeNeedsPreview({ stages: [{ id: 'bundle', verb: 'render.bundle', args: {}, gate: null, await_job: true }] }), true, 'Social bundle recipes require an exact plan preview')
eq(recipeNeedsPreview({ stages: [{ id: 'publish', verb: 'export.publish', args: {}, gate: null, await_job: true }] }), true, 'Platform export recipes require an exact plan preview')
eq(recipeNeedsPreview({ stages: [{ id: 'read', verb: 'media.transcribe', args: {}, gate: null, await_job: true }] }), false, 'Read and analysis-only recipe stages do not require a mutation preview')
eq(mediaClipTimelineDurationMs({ src_in_ms: 1000, src_out_ms: 5000, speed: 0.5 }), 8000, '0.5x media clip timeline duration is twice its source span')
const linkedMotionItem = layoutTrack({
  id: 'v1', kind: 'video', clips: [{
    id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 1000,
    motion_link: {
      schema: 'shellx-cut/motion-link@1', clipId: 'c1', assetId: 'a1', motionSourceId: 'pkg:motion',
      packageId: 'pkg', motionId: 'motion', sourceRevision: 'a'.repeat(64), planPath: '/tmp/plan.json',
      mode: 'rendered_media', state: 'linked-current', render: { path: '/tmp/render.mp4', sha256: 'b'.repeat(64), byteLength: 10, artifactHandleId: 'artifact-1' },
      fallbackPath: '/tmp/render.mp4',
    },
  }],
} as never)[0]
eq(linkedMotionItem.motionLink?.clipId, 'c1', 'Timeline layout preserves linked Motion clip identity')
eq(linkedMotionItem.motionLink?.state, 'linked-current', 'Timeline layout preserves linked Motion status')

const rippleTrimItems = [
  { id: 'v-a', kind: 'video', trackId: 'v1', startMs: 0, durMs: 5000, label: 'a1', asset: 'a1', srcInMs: 0, srcOutMs: 5000 },
  { id: 'v-b', kind: 'video', trackId: 'v1', startMs: 5000, durMs: 5000, label: 'a2', asset: 'a2', srcInMs: 5000, srcOutMs: 10000 },
  { id: 'a-a', kind: 'audio', trackId: 'a1t', startMs: 0, durMs: 5000, label: 'a1', asset: 'a1', srcInMs: 0, srcOutMs: 5000 },
] as never[]
eq(
  planRippleTrimAtPlayhead(rippleTrimItems, ['v-a'], 1500, 'start'),
  {
    clipId: 'v-a', trackId: 'v1', side: 'start', rangeMs: [0, 1500], seekMs: 0,
    operation: 'trim', trim: { src_in_ms: 1500 },
  },
  'Q plans a selected clip start trim from the playhead and closes the head gap',
)
eq(
  planRippleTrimAtPlayhead(rippleTrimItems, ['v-a'], 1500, 'end'),
  {
    clipId: 'v-a', trackId: 'v1', side: 'end', rangeMs: [1500, 5000], seekMs: 1500,
    operation: 'trim', trim: { src_out_ms: 1500 },
  },
  'W plans a selected clip end trim from the playhead',
)
eq(
  planRippleTrimAtPlayhead(rippleTrimItems, ['a-a'], 1500, 'start')?.clipId,
  'a-a',
  'Ripple trim honors an explicitly selected audio clip under the playhead',
)
eq(
  planRippleTrimAtPlayhead(rippleTrimItems, ['v-b'], 1500, 'start')?.clipId,
  'v-a',
  'Ripple trim falls back to the active program clip when selection is elsewhere',
)
eq(
  planRippleTrimAtPlayhead(rippleTrimItems, [], 5000, 'start')?.operation,
  'delete',
  'Q at a clip boundary removes the entire preceding clip',
)
eq(
  planRippleTrimAtPlayhead(rippleTrimItems, [], 5000, 'end')?.clipId,
  'v-b',
  'W at a clip boundary targets the following clip',
)
eq(planRippleTrimAtPlayhead(rippleTrimItems, [], 11_000, 'end'), null, 'Ripple trim is a no-op in a timeline gap')

// --- Timeline dual time-base (laid vs EDITORIAL) -----------------------------
// Red-proofs for the dual-surface harness's live P1 catch: after a crossfade
// shortens the LAID layout, dispatching laid coordinates targets boundaries the
// engine (which keys cumulative-track verbs on EDITORIAL time — the plain
// clip-duration sum) rejects as not_found. Exact live numbers reproduced:
// 400ms crossfade at editorial 2000 → the next seam is editorial 3642, but the
// pre-fix UI dispatched laid 3242.
const xfadeTrackItems = layoutTrack({
  id: 'v1', kind: 'video', clips: [
    { id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 2000 },
    { id: 'c2', asset: 'a1', src_in_ms: 2000, src_out_ms: 3642, xfade_in_ms: 400, xfade_kind: 'dissolve' },
    { id: 'c3', asset: 'a2', src_in_ms: 0, src_out_ms: 2400 },
  ],
} as never)
eq(xfadeTrackItems.map((i) => i.startMs), [0, 1600, 3242], 'Laid starts rewind by the crossfade overlap (draw/pointer space)')
eq(xfadeTrackItems.map((i) => i.editorialStartMs), [0, 2000, 3642], 'Editorial starts are crossfade-independent (the engine cursor space)')
const xfadeSeams = trackSeams(xfadeTrackItems)
eq(xfadeSeams.map((s) => s.atMs), [2000, 3642], 'Seam.atMs carries the EDITORIAL boundary (pre-fix bug: dispatched laid 3242 where the engine cut is 3642)')
eq(xfadeSeams.map((s) => s.laidMs), [2000, 3242], 'Seam.laidMs stays the drawn boundary in render space')
eq(laidToEditorialMs(xfadeTrackItems, 3100), 3500, 'laid→editorial maps a within-clip position through the upstream overlap')
eq(laidToEditorialMs(xfadeTrackItems, 1800), 1800, 'laid→editorial resolves an overlap-covered position into the LEFT clip tail (identity before any upstream crossfade)')
eq(laidToEditorialMs(xfadeTrackItems, 5700), 6100, 'laid→editorial extends past the last clip by the overshoot')
eq(laidToEditorialMs([], 1234), 1234, 'laid→editorial is identity on an empty track')

// --- Linked A/V split planning (demo-v2 P2) ---------------------------------
// Razor/split and ripple-delete through the UI must land on the insert-created
// linked audio half too — a video-only cut leaves the audio uncut, drifting
// out of alignment with a stale total duration.
const linkedPairItems = [
  { id: 'v-1', kind: 'video', trackId: 'v1', startMs: 0, editorialStartMs: 0, durMs: 6042, label: 'clip', asset: 'clip', srcInMs: 0, srcOutMs: 6042 },
  { id: 'a-1', kind: 'audio', trackId: 'a1t', startMs: 0, editorialStartMs: 0, durMs: 6042, label: 'clip', asset: 'clip', srcInMs: 0, srcOutMs: 6042 },
  { id: 'v-o', kind: 'video', trackId: 'v2', startMs: 0, editorialStartMs: 0, durMs: 6042, label: 'other', asset: 'other', srcInMs: 0, srcOutMs: 6042 },
] as LaidItem[]
eq(linkedSiblings(linkedPairItems[0], linkedPairItems).map((i) => i.id), ['a-1'], 'linkedSiblings resolves the exact insert-placed audio counterpart (engine resolve_linked_media criteria)')
eq(linkedSiblings(linkedPairItems[2], linkedPairItems), [], 'linkedSiblings never matches a different asset')
eq(
  linkedSiblings({ ...linkedPairItems[0], srcInMs: 100 } as LaidItem, linkedPairItems),
  [],
  'linkedSiblings requires the same source window — a re-trimmed half is no longer an exact counterpart',
)
eq(
  planLinkedSplit(linkedPairItems[0], linkedPairItems, 2000, () => false),
  { kind: 'ok', targets: [{ track: 'v1', atMs: 2000, clipId: 'v-1' }, { track: 'a1t', atMs: 2000, clipId: 'a-1' }] },
  'A UI split cuts the video clip AND its linked audio half (the demo-v2 P2 razor fix)',
)
eq(
  planLinkedSplit(linkedPairItems[0], linkedPairItems, 2000, (trackId) => trackId === 'a1t'),
  { kind: 'locked', trackId: 'a1t' },
  'Split refuses (not half-splits) when the linked half sits on a locked track',
)
eq(
  planLinkedSplit(
    linkedPairItems[0],
    [...linkedPairItems, { ...linkedPairItems[1], id: 'a-dup', trackId: 'a2t' } as LaidItem],
    2000,
    () => false,
  ),
  { kind: 'ambiguous', candidates: 2 },
  'Split refuses on multiple exact counterparts rather than guessing (engine ambiguity policy)',
)
// Per-track editorial conversion: the pair shares one LAID span, but each
// track carries its own editorial cursor (here v1 has a 400ms crossfade
// upstream, a1t does not) — the same cut lands at different editorial at_ms.
const divergedClockPair = [
  { id: 'v-lead', kind: 'video', trackId: 'v1', startMs: 0, editorialStartMs: 0, durMs: 2000, label: 'lead', asset: 'lead', srcInMs: 0, srcOutMs: 2000 },
  { id: 'v-2', kind: 'video', trackId: 'v1', startMs: 1600, editorialStartMs: 2000, durMs: 1642, label: 'clip', asset: 'clip', srcInMs: 2000, srcOutMs: 3642 },
  { id: 'a-lead', kind: 'audio', trackId: 'a1t', startMs: 0, editorialStartMs: 0, durMs: 1600, label: 'leadA', asset: 'leadA', srcInMs: 0, srcOutMs: 1600 },
  { id: 'a-2', kind: 'audio', trackId: 'a1t', startMs: 1600, editorialStartMs: 1600, durMs: 1642, label: 'clip', asset: 'clip', srcInMs: 2000, srcOutMs: 3642 },
] as LaidItem[]
eq(
  planLinkedSplit(divergedClockPair[1], divergedClockPair, 2400, () => false),
  { kind: 'ok', targets: [{ track: 'v1', atMs: 2800, clipId: 'v-2' }, { track: 'a1t', atMs: 2400, clipId: 'a-2' }] },
  'Linked split converts the shared laid cut through EACH track’s own editorial cursor',
)

// adjacentGapSlot must hand edit.fit_to_fill the gap's EDITORIAL slot — the
// laid position understates it after an upstream crossfade (pre-fix: 2600).
const gapAfterXfadeItems = layoutTrack({
  id: 'v1', kind: 'video', clips: [
    { id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 2000 },
    { id: 'c2', asset: 'a1', src_in_ms: 2000, src_out_ms: 3000, xfade_in_ms: 400, xfade_kind: 'dissolve' },
    { kind: 'gap', duration_ms: 1000 },
    { id: 'c3', asset: 'a2', src_in_ms: 0, src_out_ms: 500 },
  ],
} as never)
eq(
  adjacentGapSlot(gapAfterXfadeItems[1], gapAfterXfadeItems),
  { track: 'v1', at_ms: 3000, duration_ms: 1000 },
  'adjacentGapSlot reports the gap in EDITORIAL time (fit_to_fill cursor space)',
)
const reverseFastItem = {
  id: 'rev', kind: 'video', trackId: 'v1', startMs: 1000, durMs: 2000,
  label: 'a3', asset: 'a3', srcInMs: 0, srcOutMs: 4000, speed: 2, reverse: true,
} as never
eq(
  sourceTrimAtTimelinePosition(reverseFastItem, 'start', 1500),
  { src_out_ms: 3000 },
  'Reverse 2x start trim adjusts the high source edge with scaled timeline time',
)
eq(
  sourceTrimAtTimelinePosition(reverseFastItem, 'end', 1500),
  { src_in_ms: 3000 },
  'Reverse 2x end trim adjusts the low source edge with scaled timeline time',
)
eq(sourceMsAtTimelinePosition(reverseFastItem, 1500), 3000, 'Shared media clock maps reverse 2x timeline time to source time')
eq(
  timelineMsAtSourcePosition({ ...reverseFastItem, reverse: false }, 3000),
  2500,
  'Forward media clock maps a 2x source timestamp back to timeline time',
)
eq(mediaBasename('C:\\Media\\Interview take 2.mov'), 'Interview take 2.mov', 'Media labels use readable Windows filenames')
eq(mediaBasename('/media/interview/take-2.mov'), 'take-2.mov', 'Media labels use readable POSIX filenames')
eq(libraryIdFromAssetHash(`sha256:${'A'.repeat(64)}`), 'aaaaaaaaaaaaaaaa', 'Project asset hashes map to content-addressed Library ids')
eq(libraryIdFromAssetHash('sample:weak-hash'), null, 'Non-SHA media hashes do not produce false Library matches')
eq(
  timelineEditFailureMessage({
    ok: false,
    error: { code: 'GUARDRAIL', message: 'Linked audio is locked.', cause: 'a1', suggested_action: 'Unlock track a2.' },
  }, 'Move failed.'),
  'Linked audio is locked. Unlock track a2.',
  'Timeline edit failures include the engine recovery action',
)
eq(
  userActionFailureDetail('edit.redact', {
    ok: false,
    error: {
      code: 'capability_missing',
      message: 'Face detection is unavailable.',
      cause: 'The perception model is not installed.',
      suggested_action: 'Install captions and transcription support.',
    },
  }, 'Could not blur faces.'),
  {
    message: 'Face detection is unavailable. Install captions and transcription support.',
    setupSurface: 'settings-ai-transcription',
  },
  'Capability failures include a direct route to the relevant setup surface',
)
eq(
  userActionFailureDetail('edit.gain', {
    ok: false,
    error: { code: 'guardrail', message: 'Track is locked.', cause: 'a1t' },
  }, 'Could not change the level.'),
  { message: 'Track is locked.', setupSurface: undefined },
  'Ordinary edit failures do not misdirect the user to capability setup',
)
eq(
  userActionFailureDetail('edit.gain', { ok: true, result: {} }, 'Could not change the level.'),
  null,
  'Successful human actions do not publish false failure notices',
)
eq(defaultTrackingAnalysisId('clip hero / 01'), 'clip-hero-01-track', 'Motion tracking creates safe stable analysis ids')
eq(trackingModelForMode('point'), 'translation', 'Point tracking selects translation')
eq(trackingModelForMode('planar'), 'homography', 'Planar tracking selects homography')
eq(
  normalizedTrackingRegion({ x: 25, y: 20, width: 50, height: 40 }),
  { x: 0.25, y: 0.2, width: 0.5, height: 0.4 },
  'Motion tracking converts frame-percent seed regions to normalized values',
)
eq(
  normalizedTrackingRegion({ x: 75, y: 20, width: 50, height: 40 }),
  null,
  'Motion tracking refuses seed regions outside the frame',
)
eq(
  trackingVerificationLabel({ attached: true, current: true, reasons: [] }),
  'Verified: stabilization and source are current.',
  'Motion tracking verification reports a current attachment clearly',
)

function cssBlock(css: string, selector: string): string {
  const start = css.indexOf(`${selector} {`)
  if (start < 0) return ''
  const bodyStart = css.indexOf('{', start)
  const bodyEnd = css.indexOf('}', bodyStart)
  return bodyEnd < 0 ? '' : css.slice(bodyStart + 1, bodyEnd)
}

function python310ForProbe(root: string): string {
  const venvDir = process.platform === 'win32'
    ? resolve(root, 'app/perception/py/.venv/Scripts')
    : resolve(root, 'app/perception/py/.venv/bin')
  const candidates = [
    process.env.SHELLX_CUT_PYTHON,
    resolve(venvDir, 'python3.13'),
    resolve(venvDir, 'python3.12'),
    resolve(venvDir, 'python3.11'),
    resolve(venvDir, 'python3.10'),
    process.platform === 'win32' ? resolve(venvDir, 'python.exe') : resolve(venvDir, 'python'),
    'python3.13',
    'python3.12',
    'python3.11',
    'python3.10',
    'python3',
  ].filter((value): value is string => !!value)

  for (const candidate of candidates) {
    const check = spawnSync(candidate, ['-c', 'import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)'], {
      encoding: 'utf8',
    })
    if (check.status === 0) return candidate
  }
  return process.env.SHELLX_CUT_PYTHON || 'python3'
}

// --- exportUrl: path → /api/export/<rel> ------------------------------------
// Regression: on Windows the engine returns native paths
// (backslashes + the `\\?\` extended-length prefix). The old `lastIndexOf
// ('exports/')` missed → the whole drive path leaked into the URL → 404 → the
// preview audio monitor was SILENT while the real render had sound. These cases
// lock the cross-platform normalization.
eq(exportUrl('/home/u/p.cutproj/exports/audio.mp3'), '/api/export/audio.mp3', 'posix absolute path')
eq(exportUrl('exports/audio.mp3'), '/api/export/audio.mp3', 'project-relative path')
eq(
  exportUrl('\\\\?\\C:\\Users\\Example\\Documents\\ShellX Cut Projects\\screen.cutproj\\exports\\audio.mp3'),
  '/api/export/audio.mp3',
  'windows extended-length (\\\\?\\) representative user path',
)

// --- exportUrl: exports written OUTSIDE the project (0.6.105/0.6.106 P1) -----
// Once the user picks a default export folder, every default-named export lands
// outside the project. Folding those into the project-relative shape produced
// either a URL the engine cannot resolve (404 → dead in-app playback) or, when
// the chosen folder's own path contained an `exports/` segment, a BARE NAME
// that resolved to a stale same-named file inside the project — the app then
// played the WRONG export silently. The chosen folder decides the shape.
eq(
  exportUrl('/home/u/Deliveries/render_007.mp4', '/home/u/Deliveries'),
  '/api/export-file?path=%2Fhome%2Fu%2FDeliveries%2Frender_007.mp4',
  'an export in the chosen output folder gets the exact-file URL',
)
eq(
  exportUrl('/home/u/Videos/exports/render_007.mp4', '/home/u/Videos/exports'),
  '/api/export-file?path=%2Fhome%2Fu%2FVideos%2Fexports%2Frender_007.mp4',
  'a chosen folder that itself contains exports/ no longer collapses to a bare name',
)
eq(
  exportUrl('/home/u/p.cutproj/exports/audio.mp3', '/home/u/Deliveries'),
  '/api/export/audio.mp3',
  'an export inside the project keeps the portable project-relative URL',
)
eq(
  exportUrl('/home/u/Deliveries-evil/render.mp4', '/home/u/Deliveries'),
  '/api/export-file?path=%2Fhome%2Fu%2FDeliveries-evil%2Frender.mp4',
  'the output-folder test compares whole segments, not a string prefix',
)
eq(
  exportUrl('/home/u/SaveAs/one-off.mp4', null),
  '/api/export-file?path=%2Fhome%2Fu%2FSaveAs%2Fone-off.mp4',
  'an absolute path outside any exports/ segment names the exact file (the engine fences it)',
)
eq(
  exportUrl('audio.mp3', null),
  '/api/export/audio.mp3',
  'a bare relative path stays project-relative',
)
eq(
  exportUrl('C:\\Users\\Example\\Deliveries\\render_007.mp4', 'c:\\users\\example\\deliveries'),
  '/api/export-file?path=C%3A%5CUsers%5CExample%5CDeliveries%5Crender_007.mp4',
  'windows: case-insensitive folder match, and the RAW native path is what gets sent',
)
eq(
  exportUrl('\\\\?\\C:\\Users\\Example\\Deliveries\\render.mp4', 'C:\\Users\\Example\\Deliveries'),
  '/api/export-file?path=%5C%5C%3F%5CC%3A%5CUsers%5CExample%5CDeliveries%5Crender.mp4',
  'windows: the \\\\?\\ prefix survives into the query (the engine resolves it natively)',
)

eq(
  preferredProjectLeftTab({ assets: {} } as any),
  'assets',
  'An empty project opens the local Assets workflow',
)

eq(
  preferredProjectLeftTab({
    assets: {
      a1: { path: '/clips/talk.mp4', hash: 'sha256:test', transcript: 'receipts/a1.words.json' },
    },
  } as any),
  'transcript',
  'A project with transcript data opens the Transcript workflow',
)
eq(
  shouldReturnToProjectsAfterResync(undefined),
  false,
  'A superseded project refresh does not close the active workspace',
)
eq(
  shouldReturnToProjectsAfterResync(null),
  true,
  'An authoritative no-project refresh returns to Projects',
)
eq(
  shouldReturnToProjectsAfterResync({} as any),
  false,
  'An open project refresh preserves the active workspace',
)
eq(supportedMediaKind('C:\\clips\\Launch.MP4'), 'video', 'drop bootstrap recognizes Windows video paths')
eq(supportedMediaKind('/clips/poster.webp'), 'image', 'drop bootstrap recognizes still images')
eq(isSupportedMediaPath('/notes/readme.txt'), false, 'drop bootstrap refuses non-media files')
eq(projectNameFromMediaPath('C:\\clips\\Launch: final?.mp4'), 'Launch final', 'drop bootstrap sanitizes a portable project name')
eq(projectNameFromMediaPath('/clips/CON.mov'), 'CON project', 'drop bootstrap avoids Windows reserved folder names')
eq(
  availableProjectName('Launch', ['launch', 'Launch 2']),
  'Launch 3',
  'drop bootstrap chooses a case-insensitive free project name',
)
eq(
  activeCutSpans([{
    op_id: 'op_ignore',
    status: 'applied',
    verb: 'transcript.ignore_words',
    args: { asset: 'a1', word_range: [3, 4] },
    effects: [{ asset: 'a1', word_range: [3, 4] }],
  } as any]),
  [],
  'Non-destructive transcript ignores are not projected as removed words',
)
eq(
  activeCutSpans([{
    op_id: 'op_cut',
    status: 'applied',
    verb: 'transcript.cut_words',
    args: { asset: 'a1', word_range: [3, 4] },
  } as any]),
  [{ opId: 'op_cut', asset: 'a1', wordRange: [3, 4], rationale: undefined }],
  'Destructive transcript cuts remain projected as removed words',
)
eq(trackAuditionExportError({ ok: true, result: { path: '/tmp/track.mp3' } }), null, 'Track audition accepts a rendered stem path')
eq(
  trackAuditionExportError({ ok: true, result: {} }),
  'Track audio export returned no playable file.',
  'Track audition reports an empty successful export',
)
eq(
  trackAuditionExportError({ ok: false, error: { code: 'render_failed', message: 'No audio stream', cause: 'test' } }),
  'No audio stream',
  'Track audition surfaces the engine error message',
)

const denoiseEffect: ClipEffect = { type: 'denoise', amount: 0.5 }
const compressorEffect: ClipEffect = { type: 'compressor', amount: 0.7 }
const projectedEffectChain = toggleClipEffect(toggleClipEffect([], denoiseEffect), compressorEffect)
eq(
  projectedEffectChain.map((effect) => effect.type),
  ['denoise', 'compressor'],
  'Effect-chain projections preserve rapid additions instead of replacing the first effect',
)
eq(
  moveClipEffect(projectedEffectChain, 1, -1).map((effect) => effect.type),
  ['compressor', 'denoise'],
  'Effect-chain reorder moves one effect without losing its neighbor',
)
eq(
  effectParameterSummary(compressorEffect),
  'amount 0.7',
  'Effect-chain rows expose stored parameter values',
)
eq(
  exportUrl('C:\\Users\\Example\\proj.cutproj\\exports\\sub\\mix.mp3'),
  '/api/export/sub/mix.mp3',
  'windows drive path with a nested exports subdir',
)
eq(
  exportUrl('/proj/exports/a b+c.mp3'),
  '/api/export/a%20b%2Bc.mp3',
  'special characters are percent-encoded per segment',
)

eq(
  trackEndMs({
    tracks: [{
      id: 'v1',
      kind: 'video',
      clips: [
        { kind: 'gap', duration_ms: 1000 },
        { id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 4000, speed: 2 },
        { id: 'c2', asset: 'a2', src_in_ms: 0, src_out_ms: 3000, xfade_in_ms: 500 },
      ],
    }],
  } as any, 'v1'),
  5500,
  'trackEndMs uses realized timeline end: gaps, retime, and crossfade overlap',
)

eq(
  planAssetInsertAtPlayhead({ asset: 'a2', kind: 'video', at_ms: 5000 }),
  {
    asset: 'a2',
    kind: 'video',
    at_ms: 5000,
    ripple: true,
    rationale: 'add a2 to the base timeline at 5.00s',
  },
  'Assets Insert defaults to the base timeline instead of creating a new overlay track',
)

eq(
  sourceTimelineOccurrences({
    tracks: [
      {
        id: 'v1', kind: 'video', clips: [
          { kind: 'gap', duration_ms: 10_000 },
          { id: 'normal', asset: 'a2', src_in_ms: 2000, src_out_ms: 5500 },
        ],
      },
      {
        id: 'v2', kind: 'video', clips: [
          { kind: 'gap', duration_ms: 5000 },
          { id: 'reverse-fast', asset: 'a2', src_in_ms: 2000, src_out_ms: 6000, speed: 2, reverse: true },
        ],
      },
    ],
  } as any, 'a2', 3000),
  [
    { clipId: 'reverse-fast', trackId: 'v2', atMs: 6500 },
    { clipId: 'normal', trackId: 'v1', atMs: 11_000 },
  ],
  'Visual-search source hits resolve every trimmed/delayed/speed/reverse timeline occurrence',
)

eq(
  sourceTimelineOccurrences({
    tracks: [{
      id: 'v1', kind: 'video', clips: [
        { id: 'ramped', asset: 'a2', src_in_ms: 0, src_out_ms: 5000, speed_ramp: { points: [] } },
      ],
    }],
  } as any, 'a2', 2000),
  [],
  'Visual-search source mapping refuses approximate timeline jumps for variable-speed ramps',
)

eq(
  planTimelineAssetDrop({
    asset: 'a2',
    kind: 'video',
    at_ms: 7000,
    target: { id: 'v1', kind: 'video', kindIndex: 0 },
  }),
  {
    asset: 'a2',
    kind: 'video',
    at_ms: 7000,
    ripple: true,
    rationale: 'drop a2 into the base timeline at 7.00s',
  },
  'Dropping video on the base video track inserts into the story track',
)

eq(
  planTimelineAssetDrop({
    asset: 'a2',
    kind: 'video',
    at_ms: 7000,
    target: { id: 'v2', kind: 'video', kindIndex: 1 },
  }),
  {
    asset: 'a2',
    kind: 'video',
    at_ms: 7000,
    ripple: false,
    videoTrack: 'v2',
    newAudioTrack: true,
    rationale: 'place a2 on overlay track v2 at 7.00s',
  },
  'Dropping video on an existing overlay track places it on top without rippling the base timeline',
)

eq(
  planTimelineAssetDrop({
    asset: 'a2',
    kind: 'video',
    at_ms: 7000,
    overlay: true,
    target: null,
  }),
  {
    asset: 'a2',
    kind: 'video',
    at_ms: 7000,
    ripple: false,
    createTrackKind: 'video',
    useCreatedTrackFor: 'video',
    newAudioTrack: true,
    rationale: 'place a2 on a new overlay track at 7.00s',
  },
  'Alt-drop requests an explicit new overlay track',
)

eq(
  planTimelineAssetDrop({
    asset: 'a2',
    kind: 'video',
    at_ms: 7000,
    target: { id: 'v2', kind: 'video', kindIndex: 1, locked: true },
  }),
  null,
  'Timeline asset drops refuse locked target tracks',
)

eq(
  trackRows({
    tracks: [
      { id: 'v1', kind: 'video', clips: [], visible: false, locked: true },
      { id: 'a1t', kind: 'audio', clips: [], locked: false },
    ],
    markers: [],
  } as any).map((r) => ({ id: r.id, visible: r.visible, locked: r.locked })),
  [
    { id: 'v1', visible: false, locked: true },
    { id: 'a1t', visible: true, locked: false },
  ],
  'Timeline row layout carries visibility and lock state for gestures/drop targets',
)

eq(
  trackOrderStatus([
    { id: 'v1', kind: 'video' },
    { id: 'v2', kind: 'video' },
    { id: 'v3', kind: 'video' },
    { id: 'a1t', kind: 'audio' },
  ], 'v2'),
  { index: 1, count: 3, canMoveBack: true, canMoveForward: true },
  'Timeline track controls expose same-kind z-order state for overlays',
)

eq(
  {
    back: trackReorderTargetIndex([
      { id: 'v1', kind: 'video' },
      { id: 'v2', kind: 'video' },
      { id: 'v3', kind: 'video' },
      { id: 'a1t', kind: 'audio' },
    ], 'v2', 'back'),
    forward: trackReorderTargetIndex([
      { id: 'v1', kind: 'video' },
      { id: 'v2', kind: 'video' },
      { id: 'v3', kind: 'video' },
      { id: 'a1t', kind: 'audio' },
    ], 'v2', 'forward'),
    blocked: trackReorderTargetIndex([
      { id: 'v1', kind: 'video' },
      { id: 'a1t', kind: 'audio' },
    ], 'v1', 'back'),
  },
  { back: 0, forward: 2, blocked: null },
  'Timeline track controls dispatch group-relative reorder indexes',
)

const layerPreviewProject = {
  settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48_000 },
  assets: {
    base: { path: '/media/base.mp4' },
    overlay: { path: '/media/overlay.mp4' },
  },
  caption_styles: {},
  tracks: [
    { id: 'v-empty', kind: 'video', clips: [], visible: true, locked: false },
    { id: 'a1t', kind: 'audio', clips: [], visible: true, locked: false },
    { id: 'v-base', kind: 'video', visible: true, locked: false, clips: [
      { id: 'base-clip', asset: 'base', src_in_ms: 0, src_out_ms: 1000 },
    ] },
    { id: 'v-overlay', kind: 'video', visible: true, locked: false, clips: [
      { id: 'overlay-clip', asset: 'overlay', src_in_ms: 0, src_out_ms: 2000, transform: { x: 0.5, y: 0.5, scale: 0.5, opacity: 1 } },
    ] },
    { id: 'cap1', kind: 'caption', visible: true, locked: false, clips: [
      { id: 'caption-1', text: 'Layer caption', range_ms: [0, 2000] },
    ] },
  ],
} as any

eq(baseVideoTrackId(layerPreviewProject.tracks), 'v-base', 'Layer stack ignores empty video tracks when resolving the base canvas')
eq(isTrackLocked(layerPreviewProject.tracks, 'v-overlay'), false, 'Layer stack reports an editable unlocked track')
layerPreviewProject.tracks.find((track: any) => track.id === 'v-overlay').locked = true
eq(isTrackLocked(layerPreviewProject.tracks, 'v-overlay'), true, 'Layer stack reports a locked track')
const offlineMaps = offlineMediaMaps([
  { asset: 'base', path: '/media/base.mp4', exists: false, referenced: 1 },
  { asset: 'overlay', path: '/media/overlay.mp4', exists: true, modified_ms: 1234, referenced: 1 },
])
eq([...offlineMaps.offlineAssetIds], ['base'], 'Shared offline-media model keeps the missing asset ids')
eq([...offlineMaps.modifiedMs], [['overlay', 1234]], 'Shared offline-media model keeps modification times only when available')
eq(
  offlineAssetView(layerPreviewProject, offlineMaps.offlineAssetIds, 'base'),
  { id: 'base', label: 'base.mp4', path: '/media/base.mp4', kind: 'media' },
  'Shared offline-media view exposes a path-light user label',
)
eq(activeBaseAssetId(layerPreviewProject, 500), 'base', 'Preview resolves an offline base asset without opening its source')
eq(activeVideo(layerPreviewProject, 500, new Set())?.trackId, 'v-base', 'Live preview plays the renderer-defined base track')
eq(activeVideo(layerPreviewProject, 500, new Set())?.clipId, 'base-clip', 'Live preview identity follows the active clip, not only its source URL')
const retimedPreviewProject = structuredClone(layerPreviewProject)
const retimedBase = retimedPreviewProject.tracks.find((track: any) => track.id === 'v-base').clips[0]
retimedBase.src_in_ms = 1000
retimedBase.src_out_ms = 5000
retimedBase.speed = 2
eq(activeVideo(retimedPreviewProject, 500, new Set())?.srcMs, 2000, 'Live preview seek honors a clip speed factor')
retimedBase.reverse = true
eq(activeVideo(retimedPreviewProject, 500, new Set())?.srcMs, 4000, 'Live preview seek honors reverse playback with speed')
eq(shouldUseLivePreviewSurface(true, true, 0), false, 'Paused composed preview uses the exact engine frame')
eq(shouldUseLivePreviewSurface(true, true, 1), true, 'Forward composed playback uses the responsive live composite')
eq(shouldUseLivePreviewSurface(true, true, -1), true, 'Reverse composed playback uses the responsive live composite')
eq(shouldUseLivePreviewSurface(true, false, 0), true, 'Source preview keeps a playable video mounted while paused')
eq(shouldUseLivePreviewSurface(false, true, 1), false, 'Composed playback without a playable base stays on frame rendering')
eq(activeVideo(layerPreviewProject, 1500, new Set()), null, 'Live preview keeps a base gap instead of promoting an active overlay')
eq(previewFrameMs(7615, 7615, 1000 / 30), 7581, 'Paused playback requests the last representable frame at the half-open content end')
eq(previewFrameMs(8000, 7615, 1000 / 30), 8000, 'Seeking beyond content keeps the real black timeline gap')
eq(
  resolveOverlays(layerPreviewProject, 500, 'v-base', new Set()).overlays.map((layer) => layer.trackId),
  ['v-overlay'],
  'Live preview resolves visible video tracks above the base in stack order',
)
layerPreviewProject.tracks.find((track: any) => track.id === 'v-overlay').visible = false
eq(resolveOverlays(layerPreviewProject, 500, 'v-base', new Set()).overlays, [], 'Live preview excludes hidden overlay tracks')
layerPreviewProject.tracks.find((track: any) => track.id === 'v-base').visible = false
eq(activeVideo(layerPreviewProject, 500, new Set()), null, 'Live preview keeps a hidden base as black instead of promoting an overlay')
layerPreviewProject.tracks.find((track: any) => track.id === 'cap1').visible = false
eq(resolveCaptions(layerPreviewProject, 500), [], 'Live preview excludes hidden caption tracks')

eq(
  assetReadiness({
    id: 'a1',
    path: '/clips/phone4k.mov',
    probe: { kind: 'video', width: 3840, height: 2160, fps: 59.94 },
    offline: false,
    used: 1,
  }),
  {
    level: 'large-source',
    label: '4K source',
    hint: 'This clip may stutter during preview. Turn on proxies for future imports, or re-import it with proxies if playback lags.',
    needsAction: true,
    badges: [{ label: '4K source', tone: 'warn', title: 'High-resolution source playback can stutter without a proxy.' }],
  },
  'Large 4K source-only clips get an actionable readiness warning',
)

eq(
  assetReadiness({
    id: 'a2',
    path: '/clips/proxied.mp4',
    proxy: 'proxies/a2.mp4',
    probe: { kind: 'video', width: 3840, height: 2160, fps: 60 },
    offline: false,
    used: 0,
  }),
  {
    level: 'proxy-ready',
    label: 'Proxy ready',
    hint: 'Smooth editing media is available. Final export still uses the original source.',
    needsAction: false,
    badges: [{ label: 'Proxy ready', tone: 'good', title: 'Smooth editing media is available for this clip.' }],
  },
  'Proxied video clips are marked ready instead of source-risk',
)

eq(
  summarizeMediaReadiness([
    {
      id: 'missing',
      path: '/moved/source.mp4',
      probe: { kind: 'video', width: 1920, height: 1080 },
      offline: true,
      used: 2,
    },
    {
      id: 'heavy',
      path: '/clips/phone4k.mov',
      probe: { kind: 'video', width: 3840, height: 2160 },
      offline: false,
      used: 0,
    },
    {
      id: 'ready',
      path: '/clips/proxy.mp4',
      proxy: 'proxies/ready.mp4',
      probe: { kind: 'video', width: 3840, height: 2160 },
      offline: false,
      used: 1,
    },
  ]),
  {
    total: 3,
    videos: 3,
    offline: 1,
    usedOffline: 1,
    sourceOnly: 1,
    heavySource: 1,
    proxyReady: 1,
    filmstripMissing: 3,
    needsAction: 2,
    firstOffline: 'missing',
    level: 'missing',
    title: 'Editing limited · 1 source missing',
    hint: 'Relink missing files before preview or export.',
    analysis: 'unknown',
    dimensions: {
      source: {
        state: 'partial',
        value: '2/3',
        detail: 'Relink missing source files before preview or export.',
      },
      edit: {
        state: 'partial',
        value: '2/3',
        detail: 'Only available source files can be edited.',
      },
      proxy: {
        state: 'partial',
        value: '1/2',
        detail: 'Some available videos use source playback.',
      },
      speech: {
        state: 'unknown',
        value: 'Unverified',
        detail: 'Speech analysis capability has not been verified.',
      },
      perception: {
        state: 'unknown',
        value: 'Unverified',
        detail: 'Perception analysis capability has not been verified.',
      },
      services: {
        state: 'unknown',
        value: 'Not reported',
        detail: 'Optional service cards are not available in the current doctor report.',
      },
    },
  },
  'Media Health summary counts missing, source-risk, and proxy-ready clips separately',
)

eq(
  mediaCapabilitiesFromDoctor({
    cards: [
      { id: 'perception', kind: 'perception', status: 'degraded', details: { stt_ready: false } },
      { id: 'dub', kind: 'service', status: 'ok', details: {} },
      { id: 'diarize', kind: 'service', status: 'unknown', details: {} },
    ],
  } as any),
  {
    speech: 'unavailable',
    perception: 'ready',
    optionalServicesReady: 1,
    optionalServicesTotal: 2,
  },
  'Media Health derives speech, perception, and optional-service capabilities independently',
)

{
  const health = summarizeMediaReadiness([
    {
      id: 'analyzed',
      path: '/clips/analyzed.mp4',
      proxy: 'proxies/analyzed.mp4',
      perception: 'receipts/analyzed.perception.json',
      probe: { kind: 'video', width: 1920, height: 1080 },
      offline: false,
      used: 1,
    },
  ], {
    speech: 'unavailable',
    perception: 'ready',
    optionalServicesReady: 1,
    optionalServicesTotal: 2,
  })
  eq(
    {
      title: health.title,
      analysis: health.analysis,
      edit: health.dimensions.edit,
      speech: health.dimensions.speech,
      perception: health.dimensions.perception,
      services: health.dimensions.services,
    },
    {
      title: 'Editing ready · analysis partial',
      analysis: 'partial',
      edit: {
        state: 'ready',
        value: '1/1',
        detail: 'Every asset is available for editing.',
      },
      speech: {
        state: 'unavailable',
        value: 'Unavailable',
        detail: 'Speech analysis tools are not available on this machine.',
      },
      perception: {
        state: 'ready',
        value: '1/1',
        detail: 'Perception analysis is ready for every available audio and video asset.',
      },
      services: {
        state: 'partial',
        value: '1/2',
        detail: 'Some optional media services are reachable.',
      },
    },
    'Editing readiness stays ready while speech is unavailable and perception is complete',
  )
}

{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const placement = readFileSync(resolve(srcRoot, 'lib/placement.ts'), 'utf8')
  const manual = readFileSync(resolve(srcRoot, 'lib/manual.ts'), 'utf8')
  const videoToolsSetup = readFileSync(resolve(srcRoot, 'lib/videoToolsSetup.ts'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const icons = readFileSync(resolve(srcRoot, 'icons/registry.ts'), 'utf8')
  const app = readFileSync(resolve(srcRoot, 'App.tsx'), 'utf8')
  const appDrawerStack = readFileSync(resolve(srcRoot, 'app/AppDrawerStack.tsx'), 'utf8')
  const appSurfaceEvents = readFileSync(resolve(srcRoot, 'app/useAppSurfaceEvents.ts'), 'utf8')
  const appRightRail = readFileSync(resolve(srcRoot, 'app/AppRightRail.tsx'), 'utf8')
  const commandPalette = readFileSync(resolve(srcRoot, 'palette/CommandPalette.tsx'), 'utf8')
  const paletteCommands = readFileSync(resolve(srcRoot, 'palette/commands.ts'), 'utf8')
  const assetsPanel = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const previewPanel = readFileSync(resolve(srcRoot, 'panels/Preview/index.tsx'), 'utf8')
  const environmentPanel = readFileSync(resolve(srcRoot, 'panels/Environment/index.tsx'), 'utf8')
  const reviewPanel = readFileSync(resolve(srcRoot, 'panels/Review/index.tsx'), 'utf8')
  const scopesPanel = existsSync(resolve(srcRoot, 'panels/Review/Scopes.tsx'))
    ? readFileSync(resolve(srcRoot, 'panels/Review/Scopes.tsx'), 'utf8')
    : ''
  const recipesPanel = readFileSync(resolve(srcRoot, 'panels/Recipes/index.tsx'), 'utf8')
  const recipesCatalog = JSON.parse(readFileSync(resolve(root, 'schema/recipes.json'), 'utf8')) as {
    recipes: Array<{
      name: string
      stages: Array<{ id: string; verb: string; args?: Record<string, unknown> }>
    }>
  }
  const recipeByName = Object.fromEntries(recipesCatalog.recipes.map((recipe) => [recipe.name, recipe]))

  eq(placement.includes('const linkedAudioRipple = false'), true, 'Linked audio insert does not ripple a second time after the primary video opens the gap')
  eq(manual.includes("CUT_MANUAL_URL = 'https://docs.theshellx.com/manual/cut/'"), true, 'Cut stores the canonical online manual URL in app source')
  eq(cutManualFeatureUrl('cut.left.media_health'), 'https://docs.theshellx.com/manual/cut/?feature=cut.left.media_health', 'Cut manual helper deep-links to a feature id')
  eq(cutManualFeatureUrl('cut.preview.ffmpeg_setup'), 'https://docs.theshellx.com/manual/cut/?feature=cut.preview.ffmpeg_setup', 'FFmpeg setup actions deep-link to the exact manual feature id')
  eq(isFfmpegMissing({ cards: [{ id: 'ffmpeg', status: 'missing' }], essential_ok: false } as any), true, 'Missing FFmpeg card triggers setup notices')
  eq(isFfmpegMissing({ cards: [{ id: 'ffmpeg', status: 'unknown' }], essential_ok: true } as any), false, 'Unverified FFmpeg does not show the missing-tool setup notice')
  eq(isFfmpegMissing({ cards: [{ id: 'ffmpeg', status: 'degraded' }], essential_ok: true } as any), false, 'Degraded-but-present FFmpeg does not block preview/export as missing')
  eq(topbar.includes('data-cut-manual-link'), true, 'Topbar exposes an in-app Manual link')
  eq(topbar.includes('openCutManual()'), true, 'Topbar Manual button opens the canonical manual URL')
  eq(icons.includes('manual: BookOpen'), true, 'Manual button uses the icon registry instead of an inline glyph')
  eq(assetsPanel.includes('data-cut-media-health'), true, 'Assets exposes a compact Media Health summary')
  eq(assetsPanel.includes('data-cut-media-health-relink-first'), true, 'Media Health has a one-click relink path for the first missing asset')
  eq(assetsPanel.includes('data-cut-media-health-advanced'), true, 'Media Health hides technical details behind an advanced disclosure')
  eq(assetsPanel.includes('data-cut-media-health-dimension-state'), true, 'Media Health exposes machine-readable dimension states')
  for (const dimension of ['source', 'edit', 'proxy', 'speech', 'perception', 'services']) {
    eq(assetsPanel.includes(`['${dimension}'`), true, `Media Health renders the ${dimension} readiness dimension`)
  }
  eq(assetsPanel.includes('Adds the asset on the base timeline'), true, 'Assets Insert tooltip matches base-timeline semantics')
  eq(app.includes("cut:local-highlight"), true, 'App accepts local palette highlights through the shared highlight overlay')
  eq(appSurfaceEvents.includes("cut:open-environment"), true, 'Palette and help events can open Settings > Environment')
  eq(appSurfaceEvents.includes("cut:refresh-doctor"), true, 'Preview setup notices can request an Environment re-scan after FFmpeg install')
  eq(previewPanel.includes('data-cut-preview-ffmpeg-setup'), true, 'Preview shows a setup notice when FFmpeg is missing')
  eq(previewPanel.includes('data-cut-preview-install-ffmpeg'), true, 'Preview FFmpeg notice has a direct setup action')
  eq(videoToolsSetup.includes("id: 'settings-video-performance'"), true, 'FFmpeg setup opens the exact Settings category that owns its card')
  eq(previewPanel.includes('openVideoToolsSettings'), true, 'Preview uses the shared exact-category FFmpeg setup route')
  eq(topbar.includes('const openVideoToolsSetup = openVideoToolsSettings'), true, 'Topbar missing-FFmpeg action uses the exact-category setup route')
  eq(app.includes("setEnvCategory('overview')"), true, 'Ordinary Settings navigation still opens the overview')
  eq(previewPanel.includes('data-cut-preview-ffmpeg-guide'), true, 'Preview FFmpeg notice links directly to the FFmpeg setup guide')
  eq(previewPanel.includes('data-cut-preview-ffmpeg-recheck'), true, 'Preview FFmpeg notice has a re-check action after install')
  eq(videoToolsSetup.includes("cut.preview.ffmpeg_setup"), true, 'Preview FFmpeg notice uses the preview-specific manual anchor')
  eq(topbar.includes('data-cut-export-ffmpeg-setup'), true, 'Topbar shows an export setup notice when FFmpeg is missing')
  eq(topbar.includes('data-cut-export-install-ffmpeg'), true, 'Topbar export FFmpeg notice can open Settings')
  eq(topbar.includes('data-cut-export-ffmpeg-guide'), true, 'Topbar export FFmpeg notice links to the FFmpeg guide')
  eq(topbar.includes('data-cut-export-ffmpeg-recheck'), true, 'Topbar export FFmpeg notice can re-check after install')
  eq(topbar.includes('exportNeedsFfmpeg'), true, 'Topbar guards render/video export actions when FFmpeg is missing')
  eq(videoToolsSetup.includes("cut.preview.ffmpeg_setup"), true, 'Topbar export FFmpeg notice uses the preview-specific manual anchor')
  eq(environmentPanel.includes('data-cut-setup-path'), true, 'First-run wizard shows a plain-language setup path')
  eq(environmentPanel.includes('data-cut-setup-manual'), true, 'First-run wizard links to the online setup manual')
  eq(environmentPanel.includes("openCutManual('cut.preview.ffmpeg_setup')"), true, 'First-run wizard setup guide opens the FFmpeg preview setup manual anchor')
  eq(reviewPanel.includes("type ReviewTab = 'ops' | 'receipts' | 'qc' | 'scopes' | 'diff'"), true, 'Review has a dedicated Scopes tab in the main review rail')
  eq(reviewPanel.includes('data-cut-review-tab={t}') && reviewPanel.includes("'scopes'"), true, 'Review scopes tab has a stable selector')
  eq(appRightRail.includes('reviewTabRequest') && appRightRail.includes('cut:open-review-tab'), true, 'Review Scopes open requests survive a collapsed rail')
  eq(uiSurface('scopes')?.action, { kind: 'review', tab: 'scopes' }, 'ui.open scopes routes to the Review Scopes tab')
  eq(uiSurface('review')?.action, { kind: 'review', tab: 'ops' }, 'ui.open review opens a collapsed Review rail on Ops')
  eq(paletteCommands.includes('cut.review.scopes') && paletteCommands.includes('Video scopes'), true, 'Command palette exposes the Review Scopes surface')
  eq(scopesPanel.includes("callVerb('verify.scopes'"), true, 'Scopes panel runs the verify.scopes engine verb')
  eq(scopesPanel.includes('data-cut-scopes'), true, 'Scopes panel exposes a stable root selector')
  eq(scopesPanel.includes('data-cut-action="scopes-run"'), true, 'Scopes panel exposes a stable run action')
  eq(scopesPanel.includes('data-cut-scopes-kind={kind}') && scopesPanel.includes("vectorscope: 'Vectorscope'"), true, 'Scopes panel can request vectorscope images')
  eq(scopesPanel.includes('data-cut-scopes-kind={kind}') && scopesPanel.includes("waveform: 'Waveform'"), true, 'Scopes panel can request waveform images')
  eq(scopesPanel.includes('data-cut-scopes-kind={kind}') && scopesPanel.includes("histogram: 'Histogram'"), true, 'Scopes panel can request histogram images')
  eq(scopesPanel.includes('data-cut-scopes-images'), true, 'Scopes panel displays generated scope image links when requested')
  eq(scopesPanel.includes('clipped highlights') && scopesPanel.includes('crushed shadows'), true, 'Scopes panel translates engine flags into user-facing color warnings')
  for (const name of ['first-project', 'edit-for-clarity', 'phone-clip-cleanup', 'social-short-bundle', 'area-privacy-mask', 'add-captions', 'youtube-export', 'tiktok-export']) {
    eq(recipeByName[name] != null, true, `Built-in recipe catalog includes ${name}`)
  }
  eq(recipesCatalog.recipes.length >= 10, true, 'Recipe catalog covers a first edit and common specialist workflows')
  eq(recipeByName['first-project']?.stages.map((stage) => stage.verb), ['media.transcribe', 'media.perception', 'transcript.remove_silences', 'captions.generate', 'render.final'], 'First-project recipe measures pauses before the conservative cut, captions, and reviewed render')
  eq(recipeByName['edit-for-clarity']?.params.intensity?.enum, ['calm', 'natural', 'jumpy'], 'Edit for clarity exposes a three-level intensity control')
  eq(recipeByName['edit-for-clarity']?.stages.map((stage) => stage.verb), ['media.transcribe', 'media.perception', 'transcript.remove_retakes', 'transcript.remove_fillers', 'transcript.remove_silences'], 'Edit for clarity packages the complete reviewable speech cleanup pass')
  eq(recipesPanel.includes('data-cut-recipe-starter'), true, 'Recipe drawer identifies the first-project workflow as the starting point')
  eq(recipesPanel.includes('data-cut-recipe-sample'), true, 'First-project detail can create the bundled sample project')
  eq(recipesPanel.includes("starter: 'first-edit'") && recipesPanel.includes("proxy: false"), true, 'Bundled sample follows project.create then normal fast media.import')
  eq(appDrawerStack.includes('onProjectSwitched={onProjectSwitched}'), true, 'Recipe sample creation uses the app project-switch reset after opening its project')
  eq(recipeByName['phone-clip-cleanup']?.stages.some((stage) => stage.verb === 'audio.cleanup_voice'), true, 'Phone cleanup recipe includes voice cleanup')
  eq(recipeByName['social-short-bundle']?.stages.some((stage) => stage.verb === 'render.bundle'), true, 'Social short recipe maps to render.bundle')
  eq(recipeByName['social-short-bundle']?.description.includes('selected timeline window'), false, 'Social short recipe does not claim an unthreaded selected range')
  eq(recipeByName['area-privacy-mask']?.stages.some((stage) => stage.verb === 'edit.add_mask'), true, 'Privacy recipe maps to edit.add_mask')
  eq(recipeByName['add-captions']?.stages.map((stage) => stage.verb), ['media.transcribe', 'captions.generate'], 'Caption recipe maps to transcript then captions')
  eq(recipeByName['youtube-export']?.stages.some((stage) => stage.verb === 'export.publish' && stage.args?.platform === 'youtube'), true, 'YouTube recipe maps to export.publish youtube')
  eq(recipeByName['tiktok-export']?.stages.some((stage) => stage.verb === 'export.publish' && stage.args?.platform === 'tiktok'), true, 'TikTok recipe maps to export.publish tiktok')
  eq(recipesPanel.includes('recipeNeedsPreview'), true, 'Recipe drawer requires a preview before edit or delivery recipes run')
  eq(recipesPanel.includes("from './model'"), true, 'Recipe drawer uses the tested preview policy helper')
  eq(recipesPanel.includes("view === 'detail' ? manifest?.title ?? 'Recipe' : 'Recipes'"), true, 'Recipe detail keeps the selected workflow name visible')
  eq(recipesPanel.includes("case 'transcript.remove_retakes': return 'Remove repeated takes'"), true, 'Recipe drawer names retake cleanup in user language')
  eq(recipesPanel.includes("option === 'jumpy') return 'Tight'"), true, 'Recipe drawer labels tight pacing without exposing the internal jumpy enum')
  eq(recipesPanel.includes('data-cut-recipe-preview-required'), true, 'Recipe drawer explains when preview is required before running')
  eq(recipesPanel.includes("!report.status.startsWith('completed')"), true, 'Recipe warning completion is not rendered as a stopped stage')
  eq(recipesPanel.includes("case 'render.bundle': return 'Make social versions'"), true, 'Recipe drawer labels render.bundle in user language')
  eq(recipesPanel.includes("case 'edit.add_mask': return 'Mask selected area'"), true, 'Recipe drawer labels edit.add_mask in user language')
  eq(recipesPanel.includes("case 'export.publish': return 'Export for platform'"), true, 'Recipe drawer labels export.publish in user language')
  eq(commandPalette.includes('data-cut-command-manual'), true, 'Command palette rows carry manual feature ids for docs-aware search')
  eq(COMMANDS.some((c) => c.id === 'media-health' && c.manualId === 'cut.left.media_health' && c.highlight?.selector === '[data-cut-media-health]'), true, 'Palette can find and highlight Media Health')
  eq(COMMANDS.some((c) => c.id === 'ffmpeg-setup' && c.manualId === 'cut.preview.ffmpeg_setup' && c.highlight?.selector === '[data-cut-env-card="ffmpeg"]'), true, 'Palette can find and highlight FFmpeg setup')
  eq(COMMANDS.some((c) => c.id === 'proxy-imports' && c.manualId === 'cut.left.proxies' && c.highlight?.selector === '[data-cut-proxy-toggle]'), true, 'Palette can find and highlight proxy import controls')
}

// --- Preview monitor audio sync --------------------------------------------
// WebKit sits a few hundred milliseconds away from the video clock during normal
// playback. Treating that as hard drift causes repeated <audio>.currentTime
// writes, which can sound like the monitor audio is toggling on/off on macOS.
eq(
  typeof audioSync?.monitorAudioResyncTarget,
  'function',
  'monitor audio sync exposes a pure resync decision helper',
)
if (audioSync?.monitorAudioResyncTarget) {
  eq(
    audioSync.monitorAudioResyncTarget({
      audioTimeS: 10.0,
      playheadMs: 10340,
      nowMs: 5000,
      lastResyncAtMs: 0,
    }),
    null,
    'monitor audio does not reseek for WebKit steady drift around 340ms',
  )
  eq(
    audioSync.monitorAudioResyncTarget({
      audioTimeS: 10.0,
      playheadMs: 10900,
      nowMs: 5000,
      lastResyncAtMs: 0,
    }),
    10.9,
    'monitor audio reseeks when normal drift is clearly audible',
  )
  eq(
    audioSync.monitorAudioResyncTarget({
      audioTimeS: 10.0,
      playheadMs: 10900,
      nowMs: 5400,
      lastResyncAtMs: 5000,
    }),
    null,
    'monitor audio does not repeatedly reseek inside the cooldown window',
  )
  eq(
    audioSync.monitorAudioResyncTarget({
      audioTimeS: 10.0,
      playheadMs: 12600,
      nowMs: 5400,
      lastResyncAtMs: 5000,
    }),
    12.6,
    'monitor audio still force-corrects extreme drift inside the cooldown window',
  )
}

eq(
  resolveCommentTime({
    tracks: [{
      id: 'v1',
      kind: 'video',
      clips: [
        { id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 4000 },
        { id: 'c2', asset: 'a1', src_in_ms: 0, src_out_ms: 5000 },
      ],
    }],
  } as any, {
    id: 'cm1',
    at_ms: 6000,
    end_ms: 7000,
    text: 'tighten this',
    author: 'client',
    status: 'open',
    ts: '2026-07-01T00:00:00Z',
    anchor: { track_id: 'v1', clip_id: 'c2', offset_ms: 1000 },
  }),
  { atMs: 5000, endMs: 6000, status: 'anchored' },
  'resolveCommentTime follows the anchored clip after upstream ripple edits',
)

eq(
  resolveCommentTime({
    tracks: [{ id: 'v1', kind: 'video', clips: [{ id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 4000 }] }],
  } as any, {
    id: 'cm1',
    at_ms: 6000,
    text: 'tighten this',
    author: 'client',
    status: 'open',
    ts: '2026-07-01T00:00:00Z',
    anchor: { track_id: 'v1', clip_id: 'c2', offset_ms: 1000 },
  }),
  { atMs: 6000, status: 'stale' },
  'resolveCommentTime reports stale when the anchored clip was deleted',
)

// --- client URL helpers ownership ------------------------------------------
// Keep these path/URL helpers out of the already-large typed verb contract.
// client.ts remains the compatibility export point, but the implementation must
// live in a focused helper so future path fixes are narrow and testable.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const clientPath = resolve(here, '../src/lib/client.ts')
  const modelPath = resolve(here, '../src/lib/clientModel.ts')
  const resultsPath = resolve(here, '../src/lib/clientResults.ts')
  const urlsPath = resolve(here, '../src/lib/clientUrls.ts')
  const mediaPath = resolve(here, '../src/lib/clientMedia.ts')
  const catalogsPath = resolve(here, '../src/lib/catalogs.ts')
  const libraryPath = resolve(here, '../src/panels/Library/index.tsx')
  const libraryCssPath = resolve(here, '../src/panels/Library/library.css')
  const leftPanelPath = resolve(here, '../src/panels/LeftPanel/index.tsx')
  const clipsPath = resolve(here, '../src/panels/Clips/index.tsx')
  const statusbarPath = resolve(here, '../src/statusbar/index.tsx')
  const receiptsPath = resolve(here, '../src/panels/Review/Receipts.tsx')
  const client = readFileSync(clientPath, 'utf8')
  const model = existsSync(modelPath) ? readFileSync(modelPath, 'utf8') : ''
  const results = existsSync(resultsPath) ? readFileSync(resultsPath, 'utf8') : ''
  const urls = existsSync(urlsPath) ? readFileSync(urlsPath, 'utf8') : ''
  const media = existsSync(mediaPath) ? readFileSync(mediaPath, 'utf8') : ''
  const catalogs = existsSync(catalogsPath) ? readFileSync(catalogsPath, 'utf8') : ''
  const library = existsSync(libraryPath) ? readFileSync(libraryPath, 'utf8') : ''
  const libraryCss = existsSync(libraryCssPath) ? readFileSync(libraryCssPath, 'utf8') : ''
  const leftPanel = existsSync(leftPanelPath) ? readFileSync(leftPanelPath, 'utf8') : ''
  const clips = existsSync(clipsPath) ? readFileSync(clipsPath, 'utf8') : ''
  const statusbar = existsSync(statusbarPath) ? readFileSync(statusbarPath, 'utf8') : ''
  const receipts = existsSync(receiptsPath) ? readFileSync(receiptsPath, 'utf8') : ''
  eq(existsSync(modelPath), true, 'client shared model types have their own module')
  eq(client.includes("from './clientModel'"), true, 'client.ts imports/re-exports shared model types from the model module')
  eq(client.includes('export interface ProjectSettings'), false, 'client.ts no longer owns project model interfaces inline')
  eq(client.includes('export type Clip ='), false, 'client.ts no longer owns the clip union inline')
  eq(client.includes('export interface Waveform'), false, 'client.ts no longer owns media display result types inline')
  eq(client.includes('export function isIdentityTransform('), false, 'client.ts no longer owns transform helper logic inline')
  eq(model.includes('export interface ProjectSettings'), true, 'clientModel owns project settings')
  eq(model.includes('export type Clip ='), true, 'clientModel owns the clip union')
  eq(model.includes('export interface Waveform'), true, 'clientModel owns media display result types')
  eq(model.includes('export function isIdentityTransform('), true, 'clientModel owns transform helper logic')
  eq(existsSync(resultsPath), true, 'client typed result payloads have their own module')
  eq(client.includes("from './clientResults'"), true, 'client.ts imports/re-exports typed result payloads from the result module')
  eq(client.includes('export interface VerbResults'), false, 'client.ts no longer owns the VerbResults map inline')
  eq(client.includes('export interface GenerateTemplateSummary'), false, 'client.ts no longer owns Generate result contracts inline')
  eq(client.includes('export interface LibraryListResult'), false, 'client.ts no longer owns Library result contracts inline')
  eq(client.includes('export interface ColorMatchStats'), false, 'client.ts no longer owns color-analysis result contracts inline')
  eq(results.includes('export interface VerbResults'), true, 'clientResults owns the VerbResults map')
  eq(results.includes('export interface PregateRisk'), true, 'clientResults owns pregate risk contracts')
  eq(results.includes('export interface PregateReport'), true, 'clientResults owns pregate report contracts')
  eq(results.includes('export interface GenerateTemplateSummary'), true, 'clientResults owns Generate result contracts')
  eq(results.includes('export interface LibraryListResult'), true, 'clientResults owns Library result contracts')
  eq(results.includes('export interface ColorMatchStats'), true, 'clientResults owns color-analysis result contracts')
  eq(existsSync(urlsPath), true, 'client URL helpers have their own module')
  eq(client.includes("from './clientUrls'"), true, 'client.ts imports/re-exports URL helpers from the helper module')
  eq(client.includes('export function sourceUrl('), false, 'client.ts no longer owns sourceUrl inline')
  eq(client.includes('export function exportUrl('), false, 'client.ts no longer owns exportUrl inline')
  eq(client.includes('export function frameUrl('), false, 'client.ts no longer owns frameUrl inline')
  eq(urls.includes('export const API_BASE'), true, 'clientUrls owns the shared API base')
  eq(urls.includes('export function sourceUrl('), true, 'clientUrls owns sourceUrl')
  eq(urls.includes('export function exportUrl('), true, 'clientUrls owns exportUrl')
  eq(urls.includes('export function frameUrl('), true, 'clientUrls owns frameUrl')
  eq(existsSync(mediaPath), true, 'client media helpers have their own module')
  eq(client.includes("from './clientMedia'"), true, 'client.ts re-exports media preview helpers from the helper module')
  eq(client.includes('const waveformCache'), false, 'client.ts no longer owns waveform cache inline')
  eq(client.includes('const windowThumbCache'), false, 'client.ts no longer owns windowed thumbnail cache inline')
  eq(client.includes('export function getWaveform('), false, 'client.ts no longer owns getWaveform inline')
  eq(client.includes('export function getWindowThumbs('), false, 'client.ts no longer owns getWindowThumbs inline')
  eq(client.includes('export interface WindowThumbs'), false, 'client.ts no longer owns WindowThumbs inline')
  eq(media.includes('const waveformCache'), true, 'clientMedia owns waveform cache')
  eq(media.includes('const windowThumbCache'), true, 'clientMedia owns windowed thumbnail cache')
  eq(media.includes('export function getWaveform('), true, 'clientMedia owns getWaveform')
  eq(media.includes('export function getWindowThumbs('), true, 'clientMedia owns getWindowThumbs')
  eq(media.includes('export interface WindowThumbs'), true, 'clientMedia owns WindowThumbs')
  eq(media.includes("type Waveform } from './client'"), false, 'clientMedia does not import display result types through the verb client')
  eq(media.includes("import type { Waveform } from './clientModel'"), true, 'clientMedia imports display result types from the pure model module')
  eq(media.includes('if (!r.ok) {') && media.includes('waveformCache.delete(key)'), true, 'getWaveform drops failed verb responses from the cache so transient errors retry')
  eq(catalogs.includes('effectsPromise = null') && catalogs.includes('transitionsPromise = null'), true, 'catalog helpers clear one-shot failure promises so a later popover can retry')
  eq(library.includes('setPosterFail(new Set())') && library.includes('[items]'), true, 'Library clears poster failure cache after a reload/items refresh')
  eq(leftPanel.includes('data-cut-left-tab="find"') && leftPanel.includes("onClick={() => onTab('find')}"), true, 'Find tab has a real click handler instead of a dead active-looking control')
  eq(cssBlock(libraryCss, '.lb-list-head').includes('display: none'), false, 'Library list header is not hidden with display:none')
  eq(library.includes('lb-list-h--sort'), true, 'Library list keeps a visible name-sort header control')
  eq(clips.includes('return next.size ? next : new Set([p])'), false, 'Clips format chips do not silently reselect the last chip')
  eq(clips.includes('platforms.has(p) && platforms.size === 1'), true, 'Clips refuses the last format deselect before state becomes empty')
  eq(clips.includes("setErr('Choose at least one platform.')"), true, 'Clips surfaces a visible message when no bundle format would remain')
  eq(clips.includes('exportUrl,') && clips.includes('function exportUrl(') === false, true, 'Clips uses the cross-platform shared export URL mapper')
  // Export URL construction contract (0.6.105/0.6.106 P1). One mapper decides
  // the shape, and it needs the chosen export folder to do it — a second private
  // copy in a panel is how Receipts ended up dropping the download link for
  // every render delivered outside the project.
  eq(urls.includes("export const EXPORT_OUTPUT_DIR_STORAGE_KEY = 'cut.outputDir'"), true, 'clientUrls owns the chosen-export-folder key it needs to pick a URL shape')
  eq(urls.includes('/api/export-file?path='), true, 'clientUrls can name an export outside the project by exact path')
  eq(receipts.includes('sharedExportUrl'), true, 'Receipts maps render outputs through the shared export URL mapper')
  eq(receipts.includes("lastIndexOf('/exports/')"), false, 'Receipts no longer owns a private exports-only URL mapper')
  eq(clips.includes('data-cut-package-status=') && clips.includes('Package needs review') && clips.includes('Package blocked'), true, 'Clips labels publish-package readiness honestly')
  eq(clips.includes('data-cut-bundle-manifest') && clips.includes('data-cut-bundle-issues'), true, 'Clips exposes the hashed package manifest and structured issues')
  eq(statusbar.includes('fixActions.length > 0') && statusbar.includes('unmeasured'), true, 'Status bar separates measured failures from unmeasured receipt checks')
  eq(receipts.includes("status') === 'unmeasured'") && receipts.includes('UNMEASURED'), true, 'Receipt UI renders instrumentation gaps as unmeasured rather than failed content')
  eq(receipts.includes('allWaived') && receipts.includes("kind: 'pending'"), true, 'All-waived receipt verdict renders as a non-green pending/waived state')
}

// --- full-coverage media paths: WSL verifier → Windows installed engine -----
// Installed Windows runs need two paths for the same file: one path the local
// Node harness can existsSync() (/mnt/c/...) and one path the Windows engine can
// import (C:/...). Without this split the harness silently fell back to repo
// fixtures even though real media existed in Windows Downloads.
{
  const role = resolveMediaRole({
    localDir: '/mnt/c/Users/Example/Downloads',
    engineDir: 'C:/Users/Example/Downloads',
    realName: 'podcast_2speakers.mp4',
    fallback: '/repo/testdata/two_faces.mp4',
    role: 'SPEAKERS',
    exists: (path: string) => path === '/mnt/c/Users/Example/Downloads/podcast_2speakers.mp4',
  })
  eq(role.path, 'C:/Users/Example/Downloads/podcast_2speakers.mp4', 'media role returns the engine-import path')
  eq(role.existsPath, '/mnt/c/Users/Example/Downloads/podcast_2speakers.mp4', 'media role records the local existence path')
  eq(role.fallbackUsed, false, 'media role does not fall back when the local mapped file exists')
}

// --- Vite dev proxy: generated media routes must not fall through to SPA ----
// Generate previews and rendered media come back as root-relative cutd routes
// such as /frames/<file>.png. In dev, Vite must proxy them to cutd; otherwise
// the browser receives index.html inside an <img> and naturalWidth stays 0.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const vite = readFileSync(resolve(root, 'ui/vite.config.ts'), 'utf8')

  eq(vite.includes('const CUTD_TARGET ='), true, 'vite config has a shared cutd proxy target')
  for (const route of ['/api', '/frames', '/filmstrip', '/proxies']) {
    eq(vite.includes(`'${route}': cutdProxy`), true, `vite proxies ${route} to cutd`)
  }
}

// --- full-coverage media resolver ownership ---------------------------------
// The exhaustive verifier must keep real-media role mapping in a small helper:
// Windows/macOS/remote runs need local existence checks to resolve separately
// from the engine import path, and fallback roles must be logged centrally.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const fcv = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const mediaHelper = resolve(root, 'ui/public-tests/lib/fullCoverageMedia.mjs')
  const mediaHelperText = existsSync(mediaHelper) ? readFileSync(mediaHelper, 'utf8') : ''

  eq(existsSync(mediaHelper), true, 'full-coverage media resolver lives in a helper module')
  eq(fcv.includes("from './lib/fullCoverageMedia.mjs'"), true, 'full-coverage harness imports the media resolver helper')
  eq(fcv.includes('createFullCoverageMedia({'), true, 'full-coverage harness binds the media resolver with dirs/testdata')
  eq(fcv.includes('const _mediaFallbacks = []'), false, 'full-coverage harness does not own fallback role storage inline')
  eq(fcv.includes('function media(realName'), false, 'full-coverage harness does not own media role resolution inline')
  eq(mediaHelperText.includes('resolveMediaRole'), true, 'media resolver helper delegates role mapping to cross-host-media')
  eq(mediaHelperText.includes('fallbacks.push'), true, 'media resolver helper records fallback roles for release evidence')
  eq(mediaHelperText.includes('engineMediaDir'), true, 'media resolver helper preserves separate engine import paths')
}

// --- cut-media render configuration ownership --------------------------------
// render.rs should stay focused on graph/render execution. Presets, output
// formats, platform specs, and RenderOptions are pure configuration and belong
// in a smaller sibling module.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const renderPath = resolve(root, 'app/media/src/render.rs')
  const renderOptionsPath = resolve(root, 'app/media/src/render/options.rs')
  const render = readFileSync(renderPath, 'utf8')
  const options = existsSync(renderOptionsPath) ? readFileSync(renderOptionsPath, 'utf8') : ''

  eq(render.includes('mod options;'), true, 'render.rs registers the render options submodule')
  eq(render.includes('pub use options::{'), true, 'render.rs re-exports render configuration types for API compatibility')
  for (const token of [
    'pub struct RenderPreset',
    'pub const PRESET_NAMES',
    'pub const FORMAT_NAMES',
    'pub struct PlatformSpec',
    'pub fn platform_spec',
    'pub struct RenderOptions',
    'impl RenderOptions',
  ]) {
    eq(render.includes(token), false, `render.rs no longer owns ${token}`)
    eq(options.includes(token), true, `render/options.rs owns ${token}`)
  }
  eq(options.includes('pub fn output_geometry'), true, 'render/options.rs owns output geometry resolution')
}

// --- full-coverage transcript preconditions ---------------------------------
// Downstream translation/caption/export checks need actual WORDS, not merely an
// asset.transcript pointer. Long installed runs can produce 0-word STT receipts
// after earlier imports, so the harness must seed a deterministic non-empty
// transcript before asserting non-STT surfaces such as transcript.translate and
// captions.generate.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const fcv = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')

  eq(fcv.includes('function writeSeededTranscript'), true, 'full-coverage harness can write a deterministic non-empty transcript fixture')
  eq(fcv.includes('async function ensureNonEmptyTranscript'), true, 'full-coverage harness has a non-empty transcript precondition helper')
  eq(fcv.includes("const projectCtx = await freshProject(page, 'proj')"), true, 'project-scope coverage keeps the created project path for transcript seeding')
  eq(fcv.includes("await ensureNonEmptyTranscript(page, projectCtx.projectPath, projectCtx.assetId"), true, 'translate-transcript coverage seeds/links non-empty words when STT is empty')
  eq(fcv.includes("const projectCtx = await freshProject(page, 'export', SPEECH)"), true, 'export coverage keeps the created project path for caption transcript seeding')
  eq(fcv.includes("await ensureNonEmptyTranscript(page, projectCtx.projectPath, expAsset"), true, 'export coverage ensures captions.generate sees non-empty transcript words')
}

// --- human view-state callbacks do not self-relay through the agent API -----
// Human playhead/selection gestures commit locally, then useUiStatePublisher
// sends the observable snapshot. Relaying ui.playhead/ui.select back to the
// same UI after the local update produces a correct applied:false conflict.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const app = readFileSync(resolve(here, '../src/App.tsx'), 'utf8')
  const seek = app.slice(app.indexOf('const onSeek = useCallback'), app.indexOf('const onSelect = useCallback'))
  const select = app.slice(app.indexOf('const onSelect = useCallback'), app.indexOf('const onCutWords = useCallback'))

  eq(seek.includes('setPlayheadMs(Math.max(0, Math.round(atMs)))'), true, 'human seek commits a normalized local playhead')
  eq(seek.includes("callVerb('ui.playhead'"), false, 'human seek does not relay ui.playhead back into the same UI')
  eq(select.includes('setSelectedClipIds(clipIds)'), true, 'human selection commits local selected clips')
  eq(select.includes("callVerb('ui.select'"), false, 'human selection does not relay ui.select back into the same UI')
  eq(app.includes('const uiStateRef = useUiStatePublisher({'), true, 'local view state remains published for agent observation')
}

// --- ui.open panel contract: schema enum === client UI_OPEN_PANELS ----------
// Drift tripwire: the runtime relays any panel string to
// the UI, so the schema enum is the only contract agents/generated tools see.
// It had drifted — runtime handled matte/shape/stock/find-media/search/find-
// moment but the schema advertised only 6. client.ts UI_OPEN_PANELS is now the
// single source of truth (App.tsx's ui.open switch handles exactly these); this
// asserts schema/verbs.json's enum matches it, so a future panel add can't land
// in App.tsx without also updating the published contract.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const verbsPath = resolve(here, '../../schema/verbs.json')
  const verbs = JSON.parse(readFileSync(verbsPath, 'utf8')) as {
    verbs: Array<{ name: string; args?: { properties?: { panel?: { enum?: string[] } } } }>
  }
  const uiOpen = verbs.verbs.find((v) => v.name === 'ui.open')
  const schemaEnum = uiOpen?.args?.properties?.panel?.enum ?? []
  eq([...schemaEnum].sort(), [...UI_OPEN_PANELS].sort(), 'ui.open: schema enum === client UI_OPEN_PANELS')
  eq(new Set(UI_SURFACES.map((surface) => surface.id)).size, UI_SURFACES.length, 'shared UI surface ids are unique')
  eq(
    UI_SURFACES.every((surface) => surface.selector.length > 0 && surface.humanRoute.length > 0),
    true,
    'every shared UI surface has a stable selector and human route',
  )
  eq(
    UI_SURFACES.filter((surface) => !('action' in surface)).every((surface) => surface.agentOnlyReason),
    true,
    'every human-only dialog records why ui.open is not its control route',
  )
  for (const panel of ['assets', 'generate', 'generate-prompt', 'generate-storyboard', 'generate-media', 'projects'] as const) {
    eq(UI_OPEN_PANELS.includes(panel), true, `ui.open includes left-sidebar ${panel} tab`)
  }
  eq(UI_OPEN_PANELS.includes('library'), true, 'ui.open includes the dedicated Library workspace')
  eq(UI_OPEN_PANELS.includes('scopes'), true, 'ui.open includes the Review Scopes tab')
  {
    const here = dirname(fileURLToPath(import.meta.url))
    const surfaceEvents = readFileSync(resolve(here, '../src/app/useAppSurfaceEvents.ts'), 'utf8')
    const showEditor = surfaceEvents.indexOf('const showEditor = () =>')
    const actionSwitch = surfaceEvents.indexOf('switch (action.kind)')
    eq(showEditor >= 0, true, 'ui.open has a shared local helper for closing Environment/Wizard transient surfaces')
    eq(
      showEditor >= 0 && showEditor < actionSwitch,
      true,
      'ui.open closes Environment/Wizard after handling environment routes and before opening normal panels',
    )
    eq(
      surfaceEvents.slice(showEditor, actionSwitch).includes('setWizardOpen(false)')
        && surfaceEvents.slice(showEditor, actionSwitch).includes('setEnvOpen(false)'),
      true,
      'ui.open normal panel navigation dismisses the Environment drawer instead of leaving it over the target panel',
    )
    eq(
      uiSurface('wizard')?.action.kind === 'overlay'
        && uiSurface('environment')?.action.kind === 'settings',
      true,
      'ui.open wizard/environment routes remain explicit before normal-panel dismissal',
    )
  }
}

// --- project.delete client/schema contract ---------------------------------
// project.delete is deliberately not an op and accepts only one index/path
// selector. The typed client once advertised `rationale`, but strict runtime
// validation rejected it. Compare the concrete TypeScript fields to the schema
// so generated UI callers cannot be offered another wire-invalid argument.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const client = readFileSync(resolve(here, '../src/lib/client.ts'), 'utf8')
  const verbs = JSON.parse(readFileSync(resolve(here, '../../schema/verbs.json'), 'utf8')) as {
    verbs: Array<{ name: string; args?: { properties?: Record<string, unknown> } }>
  }
  const typedBody = client.match(/'project\.delete':\s*\{([^}]*)\}/)?.[1] ?? ''
  const typedFields = [...typedBody.matchAll(/([A-Za-z_][A-Za-z0-9_]*)\??\s*:/g)]
    .map((match) => match[1])
    .sort()
  const schemaFields = Object.keys(
    verbs.verbs.find((verb) => verb.name === 'project.delete')?.args?.properties ?? {},
  ).sort()
  eq(typedFields, schemaFields, 'project.delete: typed client fields match strict schema')
}

// --- ui.highlight manual/demo dismissal ------------------------------------
// Public docs and guided demos can use duration_ms:0 so a user has time to find
// the named control. That must never leave them stuck with an overlay they
// cannot close from the app itself.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const overlay = readFileSync(resolve(here, '../src/HighlightOverlay.tsx'), 'utf8')
  const css = readFileSync(resolve(here, '../src/highlight.css'), 'utf8')
  const verbs = JSON.parse(readFileSync(resolve(here, '../../schema/verbs.json'), 'utf8')) as {
    verbs: Array<{ name: string; description?: string }>
  }
  const uiHighlight = verbs.verbs.find((v) => v.name === 'ui.highlight')
  const reference = readFileSync(resolve(here, '../../skill/shellx-cut/reference.md'), 'utf8')
  eq(overlay.includes("aria-label=\"Close highlight\""), true, 'ui.highlight chip exposes a close button')
  eq(overlay.includes('data-cut-highlight-close'), true, 'ui.highlight close button has a stable data-cut selector')
  eq(
    overlay.includes("e.key === 'Escape'") && overlay.includes('onClear()'),
    true,
    'ui.highlight can be dismissed with Escape',
  )
  eq(cssBlock(css, '.hl-chip').includes('pointer-events: auto'), true, 'ui.highlight chip accepts pointer events for close')
  eq(css.includes('[data-cut-highlight-close]'), true, 'ui.highlight close button has dedicated CSS')
  eq(
    Boolean(uiHighlight?.description?.includes('close button') && uiHighlight.description.includes('Escape')),
    true,
    'ui.highlight schema documents manual dismissal',
  )
  eq(
    reference.includes('close button') && reference.includes('Escape'),
    true,
    'ui.highlight skill reference documents manual dismissal',
  )
}

// --- user-facing empty copy must match current IA ---------------------------
// Projects and Library are the topbar entry points now. There is no "+ New"
// topbar button, so stale empty states send fresh users to a non-existent
// control.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const files = [
    'panels/Transcript/index.tsx',
    'panels/Preview/index.tsx',
    'panels/Timeline/index.tsx',
    'panels/Shape/index.tsx',
    'panels/Title/index.tsx',
  ]
  const offenders = files.filter((rel) => readFileSync(resolve(srcRoot, rel), 'utf8').includes('+ New'))
  eq(offenders, [], 'empty states do not reference removed "+ New" topbar control')
}

// --- production build must split large editor surfaces ----------------------
// The app shell is now broad enough that one monolithic Vite chunk makes risky
// edits slower and keeps triggering the production bundle warning. Keep the
// split explicit so future large panels do not silently collapse back into the
// entry chunk. Optional surfaces use React.lazy at the app boundary; Vite keeps
// third-party code in a vendor chunk.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const viteConfig = readFileSync(resolve(here, '../vite.config.ts'), 'utf8')
  const app = readFileSync(resolve(here, '../src/App.tsx'), 'utf8')
  const appWorkspacePath = resolve(here, '../src/app/AppWorkspace.tsx')
  const appWorkspace = existsSync(appWorkspacePath) ? readFileSync(appWorkspacePath, 'utf8') : ''
  const appRightRailPath = resolve(here, '../src/app/AppRightRail.tsx')
  const appRightRail = existsSync(appRightRailPath) ? readFileSync(appRightRailPath, 'utf8') : ''
  const theme = readFileSync(resolve(here, '../src/theme.css'), 'utf8')
  for (const expected of ['manualChunks', 'vendor', '/node_modules/']) {
    eq(viteConfig.includes(expected), true, `Vite config declares production split control: ${expected}`)
  }
  for (const expected of [
    "const EnvironmentPanel = lazy(() =>",
    "const StartWizard = lazy(() =>",
  ]) {
    eq(app.includes(expected), true, `App declares lazy surface split: ${expected}`)
  }
  for (const expected of [
    "const Comments = lazy(() => import('../panels/Comments'))",
    "const RecordWorkspace = lazy(() => import('../panels/Record'))",
    "const LeftPanel = lazy(() => import('../panels/LeftPanel'))",
  ]) {
    eq(appWorkspace.includes(expected), true, `App workspace declares lazy surface split: ${expected}`)
  }
  for (const expected of [
    "const Inspector = lazy(() => import('../panels/Inspector'))",
    "const GradeDrawer = lazy(() => import('../panels/Grade'))",
    "const MixerDrawer = lazy(() => import('../panels/Mixer'))",
    "const AgentChat = lazy(() => import('../panels/AgentChat'))",
  ]) {
    eq(appRightRail.includes(expected), true, `App right rail declares lazy surface split: ${expected}`)
  }
  eq(app.includes('<Suspense fallback={<SurfaceLoading />}'), true, 'App keeps lazy panel fallbacks local to the mounted surface')
  eq(appWorkspace.includes('<Suspense fallback={<SurfaceLoading />}'), true, 'App workspace keeps lazy panel fallbacks local to the mounted surface')
  eq(appRightRail.includes('<Suspense fallback={<SurfaceLoading />}'), true, 'App right rail keeps lazy panel fallbacks local to the mounted surface')
  eq(theme.includes('.app__loading'), true, 'App lazy-loading fallback has stable layout styling')
  const topbar = readFileSync(resolve(here, '../src/topbar/index.tsx'), 'utf8')
  const topbarCss = readFileSync(resolve(here, '../src/topbar/topbar.css'), 'utf8')
  eq(topbar.includes('className="tb-gpu-label"'), true, 'Topbar wraps GPU label so compact CSS can hide the text without clipping it')
  eq(topbarCss.includes('.tb-gpu-label'), true, 'Topbar CSS targets the GPU label directly in compact layouts')
  eq(topbarCss.includes('.tb-gpu span:not(.tb-gpu-dot)'), false, 'Topbar compact GPU style does not rely on a selector that misses raw text nodes')
}

// --- App shell: pure editor helpers stay outside the mounted shell ----------
// The app shell already owns connection, layout, and cross-panel orchestration.
// Keep pure clip/generate helpers in a small app model module so App.tsx does
// not keep absorbing unrelated utility logic as new surfaces are wired.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const appPath = resolve(here, '../src/App.tsx')
  const appModelPath = resolve(here, '../src/app/model.ts')
  const appLayoutControllerPath = resolve(here, '../src/app/useAppLayoutController.ts')
  const appClipboardControllerPath = resolve(here, '../src/app/useAppClipboardController.ts')
  const appKeyboardControllerPath = resolve(here, '../src/app/useAppKeyboardController.ts')
  const appSurfaceEventsPath = resolve(here, '../src/app/useAppSurfaceEvents.ts')
  const appWorkspacePath = resolve(here, '../src/app/AppWorkspace.tsx')
  const appDrawerStackPath = resolve(here, '../src/app/AppDrawerStack.tsx')
  const appRightRailPath = resolve(here, '../src/app/AppRightRail.tsx')
  const commandPalettePath = resolve(here, '../src/palette/CommandPalette.tsx')
  const domHelperPath = resolve(here, '../src/lib/dom.ts')
  const blockingOverlayPath = resolve(here, '../src/components/overlay/useBlockingOverlay.ts')
  const app = readFileSync(appPath, 'utf8')
  const appModel = existsSync(appModelPath) ? readFileSync(appModelPath, 'utf8') : ''
  const appLayoutController = existsSync(appLayoutControllerPath) ? readFileSync(appLayoutControllerPath, 'utf8') : ''
  const appClipboardController = existsSync(appClipboardControllerPath) ? readFileSync(appClipboardControllerPath, 'utf8') : ''
  const appKeyboardController = existsSync(appKeyboardControllerPath) ? readFileSync(appKeyboardControllerPath, 'utf8') : ''
  const appSurfaceEvents = existsSync(appSurfaceEventsPath) ? readFileSync(appSurfaceEventsPath, 'utf8') : ''
  const appWorkspace = existsSync(appWorkspacePath) ? readFileSync(appWorkspacePath, 'utf8') : ''
  const appDrawerStack = existsSync(appDrawerStackPath) ? readFileSync(appDrawerStackPath, 'utf8') : ''
  const appRightRail = existsSync(appRightRailPath) ? readFileSync(appRightRailPath, 'utf8') : ''
  const layoutModule = readFileSync(resolve(here, '../src/layout/useLayout.ts'), 'utf8')
  const theme = readFileSync(resolve(here, '../src/theme.css'), 'utf8')
  const commandPalette = existsSync(commandPalettePath) ? readFileSync(commandPalettePath, 'utf8') : ''
  const domHelper = existsSync(domHelperPath) ? readFileSync(domHelperPath, 'utf8') : ''
  const blockingOverlay = existsSync(blockingOverlayPath) ? readFileSync(blockingOverlayPath, 'utf8') : ''

  eq(existsSync(appModelPath), true, 'App pure model helper module exists')
  eq(
    `${app}\n${appClipboardController}\n${appSurfaceEvents}`.includes("from './app/model'") ||
      `${appClipboardController}\n${appSurfaceEvents}`.includes("from './model'"),
    true,
    'App shell/controllers import pure model helpers',
  )
  for (const inline of [
    'const clamp =',
    'interface ClipSnapshot',
    'function snapshotClip',
    'function pasteTargetTrack',
    'function normalizeGenerateTab',
  ]) {
    eq(app.includes(inline), false, `App shell no longer owns inline helper: ${inline}`)
  }
  for (const exported of [
    'export const clamp',
    'export interface ClipSnapshot',
    'export function snapshotClip',
    'export function pasteTargetTrack',
    'export function normalizeGenerateTab',
    'export function preferredProjectLeftTab',
  ]) {
    eq(appModel.includes(exported), true, `App model exports helper: ${exported}`)
  }
  eq(existsSync(appLayoutControllerPath), true, 'App layout controller has its own hook module')
  eq(app.includes("from './app/useAppLayoutController'"), true, 'App shell imports the layout controller hook')
  eq(app.includes('useAppLayoutController(selectedClipIds)'), true, 'App shell delegates editor layout control to the hook')
  for (const inline of [
    "import { useLayout } from './layout/useLayout'",
    'const MIN_TRANSCRIPT_PX',
    'const MIN_PREVIEW_PX',
    'const MIN_TIMELINE_PX',
    'const middleRef = useRef<HTMLDivElement>(null)',
    'const mainRef = useRef<HTMLDivElement>(null)',
    'const splitRef = useRef<HTMLDivElement>(null)',
    'const dragSplit = useCallback(',
    'const [splitW, setSplitW] = useState(0)',
    'new ResizeObserver',
    'const txWidth =',
    'const dragTimeline = useCallback(',
    'const dragRail = useCallback(',
  ]) {
    eq(app.includes(inline), false, `App shell no longer owns layout controller detail: ${inline}`)
  }
  for (const expected of [
    'export function useAppLayoutController',
    "import { useLayout } from '../layout/useLayout'",
    'const MIN_TRANSCRIPT_PX',
    'const MIN_PREVIEW_PX',
    'const MIN_TIMELINE_PX',
    'const middleRef = useRef<HTMLDivElement>(null)',
    'const mainRef = useRef<HTMLDivElement>(null)',
    'const splitRef = useRef<HTMLDivElement>(null)',
    'const dragSplit = useCallback(',
    'const [splitW, setSplitW] = useState(0)',
    'new ResizeObserver',
    'const txWidth =',
    'const dragTimeline = useCallback(',
    'const dragRail = useCallback(',
  ]) {
    eq(appLayoutController.includes(expected), true, `App layout controller owns detail: ${expected}`)
  }
  eq(appLayoutController.includes('hadSelRef'), false, 'App layout controller does not auto-open the right rail on first clip selection')
  eq(appLayoutController.includes('selectedClipIds.length > 0'), false, 'Clip selection preserves full-width timeline until the user opens tools')
  eq(appRightRail.includes("target.closest('[data-cut-timeline-automation]')"), true, 'Overlay rail outside-click handling preserves timeline automation interactions')
  eq(appRightRail.includes("[data-cut-timeline-automation-menu]:not([hidden])"), true, 'Overlay rail Escape handling lets the automation menu close first')
  eq(existsSync(appClipboardControllerPath), true, 'App clipboard controller has its own hook module')
  eq(app.includes("from './app/useAppClipboardController'"), true, 'App shell imports the clipboard controller hook')
  eq(app.includes('preferredProjectLeftTab(nextProject)'), true, 'Explicit project switches choose Assets or Transcript from project content')
  eq(app.includes("leftTab: preferredProjectLeftTab(nextProject)"), true, 'Project switching does not leave the Projects tab active')
  eq(appModel.includes("return hasTranscript ? 'transcript' : 'assets'"), true, 'Project navigation policy keeps Find deliberate')
  eq(layoutModule.includes("leftTab: 'projects'"), true, 'Fresh layouts default to Projects before project-local tools')
  eq(app.includes('useAppClipboardController({'), true, 'App shell delegates clipboard control to the hook')
  eq(app.includes('clearClipboard()'), true, 'App shell clears the delegated clipboard on project switches')
  for (const inline of [
    'type Track',
    'type ClipSnapshot',
    'pasteTargetTrack',
    'snapshotClip',
    'const clipboardRef = useRef',
    'const [clipboardHasContent, setClipboardHasContent] = useState(false)',
    'const liveRef = useRef({ project, playheadMs, selectedClipIds })',
    'liveRef.current = { project, playheadMs, selectedClipIds }',
    'const copyClip = useCallback(',
    'const cutClip = useCallback(',
    'const pasteClip = useCallback(',
    'const clipTimelineDur =',
    "callVerb('edit.ripple_delete'",
    "callVerb('edit.paste'",
    'const onClipKey =',
    "e.key.toLowerCase()",
    "window.addEventListener('keydown', onClipKey)",
  ]) {
    eq(app.includes(inline), false, `App shell no longer owns clipboard controller detail: ${inline}`)
  }
  for (const expected of [
    'export function useAppClipboardController',
    'type Track',
    'type ClipSnapshot',
    'pasteTargetTrack',
    'snapshotClip',
    'const clipboardRef = useRef',
    'const [clipboardHasContent, setClipboardHasContent] = useState(false)',
    'const liveRef = useRef({ project, playheadMs, selectedClipIds })',
    'liveRef.current = { project, playheadMs, selectedClipIds }',
    'const copyClip = useCallback(',
    'const cutClip = useCallback(',
    'const pasteClip = useCallback(',
    'const clearClipboard = useCallback(',
    'const clipTimelineDur =',
    "callVerb('edit.ripple_delete'",
    "callVerb('edit.paste'",
    'const onClipKey =',
    "e.key.toLowerCase()",
    "window.addEventListener('keydown', onClipKey)",
  ]) {
    eq(appClipboardController.includes(expected), true, `App clipboard controller owns detail: ${expected}`)
  }
  eq(existsSync(appKeyboardControllerPath), true, 'App keyboard controller has its own hook module')
  eq(app.includes("from './app/useAppKeyboardController'"), true, 'App shell imports the keyboard controller hook')
  eq(app.includes('useAppKeyboardController({'), true, 'App shell delegates global keyboard control to the hook')
  for (const inline of [
    'const onKey = (e: KeyboardEvent)',
    "e.key === '\\\\'",
    "e.key === ']'",
    "e.key === 'r' || e.key === 'R'",
    'railPinned: true',
    'document.querySelector<HTMLElement>(\'[data-cut-panel="review"]\')?.focus()',
    "window.addEventListener('keydown', onKey)",
    'const onUndoRedo = (e: KeyboardEvent)',
    "const isZ = e.key === 'z' || e.key === 'Z'",
    "const isY = e.key === 'y' || e.key === 'Y'",
    "window.addEventListener('keydown', onUndoRedo)",
  ]) {
    eq(app.includes(inline), false, `App shell no longer owns keyboard controller detail: ${inline}`)
  }
  for (const expected of [
    'export function useAppKeyboardController',
    'interface AppKeyboardControllerArgs',
    "import { shouldIgnoreGlobalShortcut } from '../lib/dom'",
    "import { matchesFixedAction } from '../lib/keymap'",
    'const onKey = (e: KeyboardEvent)',
    "e.key === '\\\\'",
    "matchesFixedAction(e, 'comments.toggle')",
    "e.key === 'r' || e.key === 'R'",
    'document.querySelector<HTMLElement>(\'[data-cut-panel="review"]\')?.focus()',
    "window.addEventListener('keydown', onKey)",
    'const onUndoRedo = (e: KeyboardEvent)',
    "const isZ = e.key === 'z' || e.key === 'Z'",
    "const isY = e.key === 'y' || e.key === 'Y'",
    "window.addEventListener('keydown', onUndoRedo)",
  ]) {
    eq(appKeyboardController.includes(expected), true, `App keyboard controller owns detail: ${expected}`)
  }
  eq(existsSync(domHelperPath), true, 'shared DOM keyboard helper module exists')
  for (const expected of [
    'export function isEditableTarget',
    'export function isBlockingOverlayActive',
    'export function shouldIgnoreGlobalShortcut',
    "target.tagName === 'INPUT'",
    "target.tagName === 'SELECT'",
    "target.tagName === 'TEXTAREA'",
    'target.isContentEditable',
  ]) {
    eq(domHelper.includes(expected), true, `shared DOM helper owns editable-target detail: ${expected}`)
  }
  eq(existsSync(blockingOverlayPath), true, 'shared blocking-overlay contract exists')
  for (const expected of [
    'export function useBlockingOverlay',
    'dataset.cutBlockingOverlay',
    'entry.element.inert = true',
    "entry.element.setAttribute('aria-hidden', 'true')",
    "event.key === 'Escape'",
    "event.key !== 'Tab'",
    'opener.focus({ preventScroll: true })',
  ]) {
    eq(blockingOverlay.includes(expected), true, `blocking overlay owns accessibility behavior: ${expected}`)
  }
  for (const expected of [
    'import { isEditableTarget } from "../lib/dom"',
    'const onKey = (e: KeyboardEvent)',
    'if (isEditableTarget(e.target)) return',
    'e.preventDefault()',
    'setOpen((o) => !o)',
  ]) {
    eq(commandPalette.includes(expected), true, `Command palette Ctrl/Cmd-K guard exists: ${expected}`)
  }
  eq(existsSync(appSurfaceEventsPath), true, 'App surface event bridge has its own hook module')
  eq(app.includes("from './app/useAppSurfaceEvents'"), true, 'App shell imports the surface event bridge')
  eq(app.includes('useAppSurfaceEvents({'), true, 'App shell delegates document surface events to the hook')
  for (const inline of [
    'const onOpenReceipts = () =>',
    'railPinned: true',
    'const onOpenComment = (e: Event)',
    "const onKinetic = () => openSurface('kinetic')",
    'const onOpenChat = (e: Event)',
    'const onOpenDrawer = (e: Event)',
    "document.addEventListener('cut:open-receipts'",
    "document.addEventListener('cut:open-comment'",
    "document.addEventListener('cut:open-kinetic'",
    "document.addEventListener('cut:open-grade'",
    "document.addEventListener('cut:open-layer'",
    "document.addEventListener('cut:open-matte'",
    "document.addEventListener('cut:open-stock'",
    "document.addEventListener('cut:open-shape'",
    "document.addEventListener('cut:open-search'",
    "document.addEventListener('cut:open-generate'",
    "document.addEventListener('cut:open-chat'",
    "document.addEventListener('cut:open-drawer'",
    'normalizeGenerateTab(requestedTab)',
    'agentChatPromptSeq.current += 1',
  ]) {
    eq(app.includes(inline), false, `App shell no longer owns surface event detail: ${inline}`)
  }
  for (const expected of [
    'export function useAppSurfaceEvents',
    'interface AppSurfaceEventsArgs',
    'const onOpenReceipts = () =>',
    'const onOpenComment = (e: Event)',
    "const onKinetic = () => openSurface('kinetic')",
    'const onOpenChat = (e: Event)',
    'const onOpenDrawer = (e: Event)',
    "document.addEventListener('cut:open-receipts'",
    "document.addEventListener('cut:open-comment'",
    "document.addEventListener('cut:open-kinetic'",
    "document.addEventListener('cut:open-grade'",
    "document.addEventListener('cut:open-layer'",
    "document.addEventListener('cut:open-matte'",
    "document.addEventListener('cut:open-stock'",
    "document.addEventListener('cut:open-shape'",
    "document.addEventListener('cut:open-search'",
    "document.addEventListener('cut:open-generate'",
    "document.addEventListener('cut:open-chat'",
    "document.addEventListener('cut:open-drawer'",
    'normalizeGenerateTab(requestedTab)',
    'agentChatPromptSeq.current += 1',
  ]) {
    eq(appSurfaceEvents.includes(expected), true, `App surface event bridge owns detail: ${expected}`)
  }
  eq(existsSync(appWorkspacePath), true, 'App editor workspace has its own component module')
  eq(app.includes("from './app/AppWorkspace'"), true, 'App shell imports the workspace component')
  eq(app.includes('<AppWorkspace'), true, 'App shell renders the workspace component')
  for (const inline of [
    "import { Icon } from './icons'",
    "import Divider from './layout/Divider'",
    "import Preview from './panels/Preview'",
    "import Timeline from './panels/Timeline'",
    "const Comments = lazy(() => import('./panels/Comments'))",
    "const RecordWorkspace = lazy(() => import('./panels/Record'))",
    "const LeftPanel = lazy(() => import('./panels/LeftPanel'))",
    'data-cut-action="expand-left"',
    '<Comments',
    '<RecordWorkspace',
    '<LeftPanel',
    '<Preview',
    '<Timeline',
  ]) {
    eq(app.includes(inline), false, `App shell no longer owns workspace detail: ${inline}`)
  }
  for (const expected of [
    'export default function AppWorkspace',
    "import { Icon } from '../icons'",
    "import Divider from '../layout/Divider'",
    "import Preview from '../panels/Preview'",
    "import Timeline from '../panels/Timeline'",
    "const Comments = lazy(() => import('../panels/Comments'))",
    "const RecordWorkspace = lazy(() => import('../panels/Record'))",
    "const LeftPanel = lazy(() => import('../panels/LeftPanel'))",
    'data-cut-action="expand-left"',
    '<Comments',
    '<RecordWorkspace',
    '<LeftPanel',
    '<Preview',
    '<Timeline',
  ]) {
    eq(appWorkspace.includes(expected), true, `App workspace owns detail: ${expected}`)
  }
  eq(existsSync(appDrawerStackPath), true, 'App modal drawer stack has its own component module')
  eq(app.includes("from './app/AppDrawerStack'"), true, 'App shell imports the drawer-stack component')
  eq(app.includes('type Drawer ='), false, 'App shell no longer owns the drawer union inline')
  eq(app.includes('<AppDrawerStack'), true, 'App shell renders the drawer-stack component')
  for (const inline of [
    "const MusicBed = lazy(() => import('./panels/MusicBed'))",
    "const MatteDrawer = lazy(() => import('./panels/Matte'))",
    "const ShapeDrawer = lazy(() => import('./panels/Shape'))",
    "const LayerDrawer = lazy(() => import('./panels/Layer'))",
    "const KineticDrawer = lazy(() => import('./panels/Kinetic'))",
    "const ClipsDrawer = lazy(() => import('./panels/Clips'))",
    "const AutopilotDrawer = lazy(() => import('./panels/Autopilot'))",
    "const AssembleDrawer = lazy(() => import('./panels/Assemble'))",
    "const RecipesDrawer = lazy(() => import('./panels/Recipes'))",
    "const MaskDrawer = lazy(() => import('./panels/Mask'))",
    "const TitleDrawer = lazy(() => import('./panels/Title'))",
    '<MusicBed',
    '<TitleDrawer',
    '<KineticDrawer',
    '<MatteDrawer',
    '<ShapeDrawer',
    '<LayerDrawer',
    '<ClipsDrawer',
    '<AutopilotDrawer',
    '<RecipesDrawer',
    '<MaskDrawer',
    '<AssembleDrawer',
  ]) {
    eq(app.includes(inline), false, `App shell no longer owns drawer stack detail: ${inline}`)
  }
  for (const expected of [
    'export type AppDrawer',
    'export default function AppDrawerStack',
    "const MusicBed = lazy(() => import('../panels/MusicBed'))",
    "const MatteDrawer = lazy(() => import('../panels/Matte'))",
    "const ShapeDrawer = lazy(() => import('../panels/Shape'))",
    "const LayerDrawer = lazy(() => import('../panels/Layer'))",
    "const KineticDrawer = lazy(() => import('../panels/Kinetic'))",
    "const ClipsDrawer = lazy(() => import('../panels/Clips'))",
    "const AutopilotDrawer = lazy(() => import('../panels/Autopilot'))",
    "const AssembleDrawer = lazy(() => import('../panels/Assemble'))",
    "const RecipesDrawer = lazy(() => import('../panels/Recipes'))",
    "const MaskDrawer = lazy(() => import('../panels/Mask'))",
    "const TitleDrawer = lazy(() => import('../panels/Title'))",
    '<MusicBed',
    '<TitleDrawer',
    '<KineticDrawer',
    '<MatteDrawer',
    '<ShapeDrawer',
    '<LayerDrawer',
    '<ClipsDrawer',
    '<AutopilotDrawer',
    '<RecipesDrawer',
    '<MaskDrawer',
    '<AssembleDrawer',
  ]) {
    eq(appDrawerStack.includes(expected), true, `App drawer-stack owns detail: ${expected}`)
  }
  eq(appDrawerStack.includes("| 'stock'"), false, 'AppDrawer union excludes Stock after it moved to the left Find surface')
  eq(appDrawerStack.includes("| 'search'"), false, 'AppDrawer union excludes Search after it moved to the left Find surface')
  eq(existsSync(appRightRailPath), true, 'App right rail has its own component module')
  eq(app.includes("from './app/AppRightRail'"), true, 'App shell imports the right-rail component')
  eq(app.includes('<AppRightRail'), true, 'App shell renders the right-rail component')
  eq(app.includes('data-cut-right-tabs'), false, 'App shell no longer owns right-tab selector inline')
  eq(app.includes('data-cut-right-tab'), false, 'App shell no longer owns right-tab button selectors inline')
  eq(app.includes("import Review from './panels/Review'"), false, 'App shell no longer imports Review directly')
  eq(app.includes("const Inspector = lazy(() => import('./panels/Inspector'))"), false, 'App shell no longer lazy-loads Inspector directly')
  eq(app.includes("const AgentChat = lazy(() => import('./panels/AgentChat'))"), false, 'App shell no longer lazy-loads Agent Chat directly')
  eq(app.includes("const GradeDrawer = lazy(() => import('./panels/Grade'))"), false, 'App shell no longer lazy-loads Grade directly')
  eq(app.includes("const MixerDrawer = lazy(() => import('./panels/Mixer'))"), false, 'App shell no longer lazy-loads Mixer directly')
  eq(app.includes('<Review'), false, 'App shell no longer renders Review inline')
  eq(app.includes('<AgentChat project={project} prefill={agentChatPrefill} />'), false, 'App shell no longer renders Agent Chat inline')
  for (const expected of [
    'export default function AppRightRail',
    "import Review from '../panels/Review'",
    "const Inspector = lazy(() => import('../panels/Inspector'))",
    "const AgentChat = lazy(() => import('../panels/AgentChat'))",
    "const GradeDrawer = lazy(() => import('../panels/Grade'))",
    "const MixerDrawer = lazy(() => import('../panels/Mixer'))",
    'data-cut-right-tabs',
    'data-cut-right-tab',
    'data-cut-rail-overlay',
    'data-cut-rail-pin',
    'data-cut-rail-close',
    '<Review',
    '{railPinned && (',
    '<AgentChat project={project} prefill={agentChatPrefill} />',
  ]) {
    eq(appRightRail.includes(expected), true, `App right rail owns detail: ${expected}`)
  }
  for (const expected of [
    'railPinned: boolean',
    'railPinned: false',
    "railPinned: 'railPinned' in p && p.railPinned === true",
  ]) {
    eq(layoutModule.includes(expected), true, `Layout state owns right-rail pin persistence: ${expected}`)
  }
  for (const expected of [
    '.app__rail--overlay',
    '.app__rail--pinned',
    '.app__rail-controls',
    '.app__rail--overlay .app__inspector',
  ]) {
    eq(theme.includes(expected), true, `Theme styles contextual right-rail layout: ${expected}`)
  }
}

// --- Topbar: keep menu constants and result helpers out of the JSX shell -----
{
  const here = dirname(fileURLToPath(import.meta.url))
  const topbarPath = resolve(here, '../src/topbar/index.tsx')
  const topbarModelPath = resolve(here, '../src/topbar/model.ts')
  const topbarJobsPath = resolve(here, '../src/topbar/useTopbarJobs.ts')
  const topbarDismissPath = resolve(here, '../src/topbar/useTopbarDismissibleMenu.ts')
  const topbarPreflightPath = resolve(here, '../src/topbar/PreflightWarning.tsx')
  const timelineGlobalToolsPath = resolve(here, '../src/panels/Timeline/TimelineGlobalTools.tsx')
  const clientResultsPath = resolve(here, '../src/lib/clientResults.ts')
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')
  const topbar = readFileSync(topbarPath, 'utf8')
  const topbarModel = existsSync(topbarModelPath) ? readFileSync(topbarModelPath, 'utf8') : ''
  const topbarJobs = existsSync(topbarJobsPath) ? readFileSync(topbarJobsPath, 'utf8') : ''
  const topbarDismiss = existsSync(topbarDismissPath) ? readFileSync(topbarDismissPath, 'utf8') : ''
  const topbarPreflight = existsSync(topbarPreflightPath) ? readFileSync(topbarPreflightPath, 'utf8') : ''
  const timelineGlobalTools = existsSync(timelineGlobalToolsPath) ? readFileSync(timelineGlobalToolsPath, 'utf8') : ''
  const results = readFileSync(clientResultsPath, 'utf8')

  eq(existsSync(topbarModelPath), true, 'Topbar model helper module exists')
  eq(topbar.includes("from './model'"), true, 'Topbar imports menu constants from the model module')
  for (const inline of [
    'const WORKSPACE_MODES',
    'const EXPORT_OPTIONS',
    'const ASYNC_RENDER_IDS',
    'const PRESETS',
    'type Preset =',
    'const PROFILES',
    'type Profile =',
    'const ASPECTS',
    'type Aspect =',
    'const REFRAME_PRESETS',
    'type ReframePreset =',
    'const FORMATS',
    'type FileFormat =',
    'const FORMAT_LABELS',
    'const LOUDNESS',
    'type Loudness =',
    'const LOUDNESS_LABELS',
    'function selectedOption',
  ]) {
    eq(topbar.includes(inline), false, `Topbar JSX shell no longer owns model detail: ${inline}`)
  }
  for (const expected of [
    'export const WORKSPACE_MODES',
    'export const EXPORT_OPTIONS',
    'export const ASYNC_RENDER_IDS',
    'export const PRESETS',
    'export type Preset',
    'export const PROFILES',
    'export type Profile',
    'export const ASPECTS',
    'export type Aspect',
    'export const REFRAME_PRESETS',
    'export type ReframePreset',
    'export const FORMATS',
    'export type FileFormat',
    'export const FORMAT_LABELS',
    'export const LOUDNESS',
    'export type Loudness',
    'export const LOUDNESS_LABELS',
    'export function selectedOption',
  ]) {
    eq(topbarModel.includes(expected), true, `Topbar model owns detail: ${expected}`)
  }
  eq(topbarModel.includes('toolResultMessage'), false, 'Topbar model no longer owns global timeline tool feedback')
  eq(timelineGlobalTools.includes('function toolResultMessage'), true, 'Timeline global tools own their feedback helper')
  eq(existsSync(topbarJobsPath), true, 'Topbar jobs hook module exists')
  eq(topbar.includes("from './useTopbarJobs'"), true, 'Topbar imports the jobs hook')
  eq(topbar.includes('useTopbarJobs()'), true, 'Topbar delegates live job tracking to the hook')
  for (const inline of [
    'interface JobView',
    'const [jobs, setJobs]',
    "callVerb('jobs.list'",
    'events.onStatus',
    'events.subscribe((ev)',
    'setJobs((prev)',
    'const jobList = Object.values(jobs)',
    'isRenderBlockingJobKind',
  ]) {
    eq(topbar.includes(inline), false, `Topbar JSX shell no longer owns jobs detail: ${inline}`)
  }
  for (const expected of [
    'interface JobView',
    'export function isRenderBlockingJobKind',
    'export function useTopbarJobs',
    "callVerb('jobs.list'",
    'events.onStatus',
    'events.subscribe((ev)',
    'setJobs((prev)',
    'const jobList = Object.values(jobs)',
    'const renderRunning = jobList.some((j) => isRenderBlockingJobKind(j.kind))',
    'return { jobList, renderRunning }',
  ]) {
    eq(topbarJobs.includes(expected), true, `Topbar jobs hook owns detail: ${expected}`)
  }
  eq(existsSync(topbarDismissPath), true, 'Topbar dismissible-menu hook module exists')
  eq(topbar.includes("from './useTopbarDismissibleMenu'"), true, 'Topbar imports the dismissible-menu hook')
  for (const call of [
    'useTopbarDismissibleMenu(menuRef, menuOpen, setMenuOpen)',
    'useTopbarDismissibleMenu(renderRef, renderOptsOpen, setRenderOptsOpen)',
  ]) {
    eq(topbar.includes(call), true, `Topbar delegates menu dismissal: ${call}`)
  }
  eq(topbar.includes('data-cut-find-btn'), false, 'Topbar no longer carries a redundant Find/Search launcher')
  eq(topbar.includes('findOpen') || topbar.includes('findRef') || topbar.includes('onOpenFind'), false, 'Topbar no longer owns Find menu state')
  for (const inline of [
    'const onDown = (e: MouseEvent)',
    'document.addEventListener(\'mousedown\', onDown)',
    'document.removeEventListener(\'mousedown\', onDown)',
  ]) {
    eq(topbar.includes(inline), false, `Topbar JSX shell no longer owns dismissible-menu detail: ${inline}`)
  }
  for (const expected of [
    'export function useTopbarDismissibleMenu',
    'const onDown = (e: MouseEvent)',
    'const onKey = (e: KeyboardEvent)',
    "if (e.key === 'Escape') setOpen(false)",
    "document.addEventListener('mousedown', onDown)",
    "document.addEventListener('keydown', onKey)",
    "document.removeEventListener('mousedown', onDown)",
    "document.removeEventListener('keydown', onKey)",
  ]) {
    eq(topbarDismiss.includes(expected), true, `Topbar dismissible-menu hook owns detail: ${expected}`)
  }
  eq(existsSync(topbarPreflightPath), true, 'Topbar pre-render warning component exists')
  eq(topbar.includes("import PreflightWarning from './PreflightWarning'"), true, 'Topbar imports the preflight warning component')
  eq(topbar.includes("callVerb('verify.pregate'"), true, 'Topbar runs verify.pregate before video render/export')
  eq(topbar.includes('pendingPreflight'), true, 'Topbar stores a pending preflight action for user confirmation')
  eq(topbar.includes('<PreflightWarning'), true, 'Topbar renders the preflight warning component')
  eq(topbarPreflight.includes('data-cut-pregate-warning'), true, 'Preflight warning has a stable root selector')
  eq(topbarPreflight.includes('data-cut-pregate-risk'), true, 'Preflight warning lists pregate risks')
  eq(topbarPreflight.includes('data-cut-pregate-details'), true, 'Preflight warning keeps detailed pregate diagnostics collapsible')
  eq(topbarPreflight.includes('data-cut-pregate-continue'), true, 'Preflight warning allows non-blocking warnings to continue')
  eq(topbarPreflight.includes('data-cut-pregate-blocked'), true, 'Preflight warning marks severe pregate risks as blocked')
  eq(topbarPreflight.includes('data-cut-pregate-guide'), true, 'Preflight warning links to a manual guide')
  eq(topbarPreflight.includes('data-cut-pregate-guide-feature'), true, 'Preflight warning exposes the chosen manual feature id')
  eq(topbarPreflight.includes('const RISK_COPY'), true, 'Preflight warning maps raw pregate risk kinds to plain labels')
  eq(topbarPreflight.includes("title: 'Black ending'"), true, 'Preflight warning names empty_tail as a black ending')
  eq(topbarPreflight.includes("title: 'Silent export'"), true, 'Preflight warning names silent_output in user language')
  eq(topbarPreflight.includes("title: 'Black border'"), true, 'Preflight warning names uniform_border in user language')
  eq(topbarPreflight.includes("risk.kind.replace(/_/g, ' ')"), false, 'Preflight warning does not show raw risk ids by default')
  eq(topbarPreflight.includes('report.perception_assets ?? 0'), true, 'Preflight warning treats perception_assets as a backend count')
  eq(results.includes('perception_assets?: number'), true, 'PregateReport types perception_assets as the backend count')
  eq(topbarPreflight.includes("openCutManual('cut.export.preflight')"), true, 'Preflight warning uses the export preflight manual anchor')
  eq(fullCoverage.includes('verify.pregate(preflight warning)'), true, 'Full coverage verifier records pregate as a UI preflight surface')
  eq(fullCoverage.includes('verify.pregate(verb-level'), false, 'Full coverage verifier no longer claims pregate is verb-level only')
  const pregateCrosscheck = fullCoverage.slice(
    fullCoverage.indexOf("rec(S, 'verify.pregate(preflight warning)'"),
    fullCoverage.indexOf("rec(S, 'verify.pregate(preflight warning)'") + 260,
  )
  eq(pregateCrosscheck.includes("rowKind: 'support'"), true, 'Pregate verb cross-check cannot impersonate the separately actuated topbar action')
}

// --- Topbar: platform publishes honor the footage QC profile ------------------
// The 2026-08-06 demo-shoot P3: export.publish had no way to choose the footage
// profile, so silent screen-demo publishes always ran the talking_head battery
// and failed caption_presence/lufs on the receipt. The Export menu now shares
// the Render menu's Footage choice and forwards it into export.publish.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const topbar = readFileSync(resolve(here, '../src/topbar/index.tsx'), 'utf8')

  // BEHAVIORAL: drive the REAL publish run functions through a captured fetch —
  // the verb name and the exact JSON body are the wire contract under test.
  const realFetch = globalThis.fetch
  const publishCalls: Array<{ url: string; body: Record<string, unknown> }> = []
  globalThis.fetch = (async (url: unknown, init?: { body?: string }) => {
    publishCalls.push({ url: String(url), body: JSON.parse(init?.body ?? '{}') as Record<string, unknown> })
    return { json: async () => ({ ok: true, result: {} }) }
  }) as typeof fetch
  try {
    const publish = EXPORT_OPTIONS.filter((option) => option.group === 'publish')
    eq(publish.map((option) => option.id), ['pub_youtube', 'pub_tiktok', 'pub_reels', 'pub_x'], 'Publish group carries the four platform entries')
    const tiktok = publish.find((option) => option.id === 'pub_tiktok')
    // Selected silent_screen_demo → export.publish is called WITH the profile.
    await tiktok?.run(undefined, 'silent_screen_demo')
    eq(publishCalls[0]?.url.endsWith('/api/verb/export.publish'), true, 'Publish run posts the export.publish verb')
    eq(publishCalls[0]?.body, { platform: 'tiktok', profile: 'silent_screen_demo' }, 'Chosen footage profile is forwarded to export.publish')
    // 'auto' maps to undefined at the call site → NO profile key on the wire
    // (the engine default + auto-detect proposal, today's behavior).
    await tiktok?.run(undefined)
    eq(publishCalls[1]?.body, { platform: 'tiktok' }, 'Auto profile omits the profile key entirely')
    eq('profile' in (publishCalls[1]?.body ?? {}), false, 'Auto profile never sends profile: undefined/null')
    // Save As path still composes with the profile (path + profile together).
    await tiktok?.run('/tmp/t.mp4', 'silent_screen_demo')
    eq(publishCalls[2]?.body, { platform: 'tiktok', profile: 'silent_screen_demo', path: '/tmp/t.mp4' }, 'Profile and explicit path compose on one publish call')
    // The plain Video entry hits render.final directly — same shared choice
    // (the select sits in the same menu; a render-backed entry ignoring it
    // would be a silently-dead control).
    const video = EXPORT_OPTIONS.find((option) => option.id === 'video')
    await video?.run(undefined, 'silent_screen_demo')
    eq(publishCalls[3]?.url.endsWith('/api/verb/render.final'), true, 'Video export posts render.final')
    eq(publishCalls[3]?.body, { preset: 'standard', profile: 'silent_screen_demo' }, 'Video export forwards the chosen footage profile to render.final')
    await video?.run(undefined)
    eq(publishCalls[4]?.body, { preset: 'standard' }, 'Video export with auto profile omits the profile key')
  } finally {
    globalThis.fetch = realFetch
  }

  // WIRING: the Export menu renders the shared Footage select and the call site
  // maps 'auto' → undefined for the publish group only.
  eq(topbar.includes('data-cut-export-profile'), true, 'Export menu owns a stable Footage-profile selector')
  eq(topbar.includes("opt.run(explicitPath, profile === 'auto' ? undefined : profile)"), true, 'Publish exports forward the shared footage profile (auto = omit)')
  eq((topbar.match(/onChange=\{\(e\) => setProfile\(selectedOption\(PROFILES, e\.target\.value, profile\)\)\}/g) || []).length >= 2, true, 'Render and Export menus drive ONE shared profile state')
}

// --- Environment cards: visible labels are user-outcome first ----------------
// The Environment tab is for non-specialist users. Default card titles and model
// options must not lead with implementation stack names; raw IDs and deep setup
// terms belong in Advanced details.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const mk = (id: string, kind: DoctorCard['kind'] = 'tool'): DoctorCard => ({
    id,
    kind,
    status: 'ok',
    details: {},
  })
  const labels = [
    cardLabel(mk('ffmpeg')).title,
    cardLabel(mk('ffprobe')).title,
    cardLabel(mk('perception', 'perception')).title,
    cardLabel(mk('matte', 'matte')).title,
    cardLabel(mk('matte_premium', 'matte')).title,
    cardLabel(mk('dub', 'service')).title,
    cardLabel(mk('diarize', 'service')).title,
    cardLabel(mk('gpu-encode')).title,
    ...STT_MODELS.map((m) => m.label),
  ]
  const banned = ['Perception (Python)', 'FFmpeg', 'FFprobe', 'whisperx', 'onnx-asr', 'venv']
  const offenders = labels.filter((label) =>
    banned.some((term) => label.toLowerCase().includes(term.toLowerCase())),
  )
  eq(offenders, [], 'Environment visible labels hide implementation names')
  eq(labels.every((label) => label.length <= 58), true, 'Environment visible labels stay compact')
  const envCards = readFileSync(resolve(srcRoot, 'panels/Environment/EnvCards.tsx'), 'utf8')
  const envCardRow = readFileSync(resolve(srcRoot, 'panels/Environment/EnvCardRow.tsx'), 'utf8')
  const serviceRuntime = readFileSync(resolve(srcRoot, 'panels/Environment/ServiceRuntime.tsx'), 'utf8')
  const transcript = readFileSync(resolve(srcRoot, 'panels/Transcript/index.tsx'), 'utf8')
  const transcriptSetupPath = resolve(srcRoot, 'panels/Transcript/TranscriptSetupCard.tsx')
  const transcriptSetup = existsSync(transcriptSetupPath) ? readFileSync(transcriptSetupPath, 'utf8') : ''
  const transcriptModelPath = resolve(srcRoot, 'panels/Transcript/model.ts')
  const transcriptModel = existsSync(transcriptModelPath) ? readFileSync(transcriptModelPath, 'utf8') : ''
  const transcriptAssetWordsPath = resolve(srcRoot, 'panels/Transcript/AssetWords.tsx')
  const transcriptAssetWords = existsSync(transcriptAssetWordsPath) ? readFileSync(transcriptAssetWordsPath, 'utf8') : ''
  const transcriptReelTrayPath = resolve(srcRoot, 'panels/Transcript/ReelTray.tsx')
  const transcriptReelTray = existsSync(transcriptReelTrayPath) ? readFileSync(transcriptReelTrayPath, 'utf8') : ''
  const transcriptCss = readFileSync(resolve(srcRoot, 'panels/Transcript/transcript.css'), 'utf8')
  const client = readFileSync(resolve(srcRoot, 'lib/client.ts'), 'utf8')
  const clientModel = readFileSync(resolve(srcRoot, 'lib/clientModel.ts'), 'utf8')
  const clientResults = readFileSync(resolve(srcRoot, 'lib/clientResults.ts'), 'utf8')
  const schema = readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')
  const transcriptDispatch = readFileSync(resolve(root, 'app/server/src/dispatch.rs'), 'utf8')
  const speechTextDispatch = readFileSync(resolve(root, 'app/server/src/dispatch/speech_text.rs'), 'utf8')
  eq(envCards.includes('label="Captions and transcription"'), true, 'Environment groups perception setup as captions and transcription')
  eq(envCards.includes('label="Perception"'), false, 'Environment group labels do not expose perception as the user-facing category')
  eq(envCardRow.includes('Install captions'), true, 'Environment perception setup action uses non-specialist install wording')
  eq(envCardRow.includes('Set up perception'), false, 'Environment perception setup action does not expose perception jargon')
  eq(transcriptSetup.includes('Install captions'), true, 'Transcript missing-transcription setup action uses install captions wording')
  eq(`${transcript}\n${transcriptSetup}`.includes('Set up perception'), false, 'Transcript missing-transcription setup action does not expose perception jargon')
  eq(existsSync(transcriptSetupPath), true, 'Transcript setup card has its own component module')
  eq(transcript.includes("from './TranscriptSetupCard'"), true, 'Transcript imports the setup-card component module')
  eq(transcript.includes('data-cut-perception-setup'), false, 'Transcript panel no longer owns setup-card markup inline')
  eq(transcriptSetup.includes('data-cut-perception-setup'), true, 'Transcript setup-card component owns the setup card selector')
  eq(transcriptSetup.includes('data-cut-action="setup-perception"'), true, 'Transcript setup-card component preserves the install action selector')
  eq(transcriptSetup.includes('Install captions'), true, 'Transcript setup-card component owns the non-specialist install copy')
  eq(existsSync(transcriptModelPath), true, 'Transcript pure model helpers have their own module')
  eq(transcript.includes("from './model'"), true, 'Transcript panel imports model helpers')
  eq(transcript.includes('function timelineEntriesFrom'), false, 'Transcript panel no longer owns timeline result parsing inline')
  eq(transcript.includes('function searchResultFrom'), false, 'Transcript panel no longer owns search result parsing inline')
  eq(transcript.includes('function chaptersOf'), false, 'Transcript panel no longer owns chapter result parsing inline')
  eq(transcript.includes('function reelSnippet'), false, 'Transcript panel no longer owns reel snippet formatting inline')
  eq(transcriptModel.includes('export function timelineEntriesFrom'), true, 'Transcript model owns timeline result parsing')
  eq(transcriptModel.includes('export function searchResultFrom'), true, 'Transcript model owns search result parsing')
  eq(transcriptModel.includes('export function chaptersOf'), true, 'Transcript model owns chapter result parsing')
  eq(transcriptModel.includes('export function reelSnippet'), true, 'Transcript model owns reel snippet formatting')
  eq(transcriptModel.includes('export interface Sel'), true, 'Transcript model owns word-selection shape')
  eq(transcriptModel.includes('export interface ReelSpan'), true, 'Transcript model owns reel-tray shape')
  eq(existsSync(transcriptAssetWordsPath), true, 'Transcript source word renderer has its own component module')
  eq(transcript.includes("from './AssetWords'"), true, 'Transcript panel imports the source word renderer component')
  eq(transcript.includes('function AssetWords'), false, 'Transcript panel no longer owns source word renderer inline')
  eq(transcript.includes('const FILLER_WORDS'), false, 'Transcript panel no longer owns source-word filler styling internals')
  eq(transcriptAssetWords.includes('export default function AssetWords'), true, 'Transcript source word renderer module exports the component')
  eq(transcriptAssetWords.includes('data-cut-transcript-empty'), true, 'Transcript source word renderer owns source empty-state selector')
  eq(transcriptAssetWords.includes('data-cut-action="restore"'), true, 'Transcript source word renderer owns restore selector')
  eq(transcriptAssetWords.includes('tx-word--filler'), true, 'Transcript source word renderer owns filler-word styling')
  eq(transcriptAssetWords.includes('data-cut-word'), true, 'Transcript source word renderer owns word selectors')
  eq(transcript.includes("runUserVerb('transcript.ignore_words'"), true, 'Transcript toolbar dispatches transcript.ignore_words with visible failure feedback')
  eq(transcript.includes('data-cut-action="ignore-words"'), true, 'Transcript toolbar exposes Ignore words')
  eq(transcript.includes('data-cut-action="unignore-words"'), true, 'Transcript toolbar exposes Unignore words')
  eq(transcript.includes('project?.transcript_ignores'), true, 'Transcript panel reads project transcript ignores')
  eq(transcript.includes('setIgnoreOverride(r.result?.transcript_ignores ?? [])'), true, 'Transcript ignore styling converges from the successful mutation response')
  eq(transcript.includes("ev.op.verb === 'project.undo' || ev.op.verb === 'project.redo'"), true, 'Transcript ignore override yields to history navigation')
  eq(transcriptAssetWords.includes('data-cut-word-ignored'), true, 'Transcript source words expose ignored selectors')
  eq(transcriptAssetWords.includes('tx-word--ignored'), true, 'Transcript source words render ignored words distinctly')
  eq(transcriptCss.includes('.tx-word--ignored'), true, 'Transcript CSS styles ignored words separately from muted words')
  eq(
    /title="[^"]*(transcript\.ignore_words|transcript\.mute_words|edit\.mute_range|transcript\.chapters)[^"]*"/.test(transcript),
    false,
    'Transcript toolbar titles stay user-facing and do not expose raw debug verb names',
  )
  eq(client.includes("'transcript.ignore_words'"), true, 'Typed client exposes transcript.ignore_words args')
  eq(clientModel.includes('export interface TranscriptIgnore'), true, 'Client model exposes transcript ignore state')
  eq(clientModel.includes('transcript_ignores?: TranscriptIgnore[]'), true, 'Project model carries transcript ignores')
  eq(clientResults.includes("'transcript.ignore_words'"), true, 'Typed client exposes transcript.ignore_words result')
  eq(schema.includes('"name": "transcript.ignore_words"'), true, 'Schema exposes transcript.ignore_words')
  eq(transcriptDispatch.includes('"transcript.ignore_words" =>'), true, 'Dispatch routes transcript.ignore_words')
  eq(transcriptDispatch.includes('speech_text::transcript_ignore_words'), true, 'Dispatch routes transcript.ignore_words through speech_text module')
  eq(speechTextDispatch.includes('split_word_range_by_ignores'), true, 'Assembly splits selected ranges around ignored transcript words')
  eq(`${transcriptDispatch}\n${speechTextDispatch}`.includes('transcript_word_ignored(&transcript_ignores'), true, 'Captions/timeline harvesting skip ignored transcript words')
  eq(existsSync(transcriptReelTrayPath), true, 'Transcript reel tray has its own component module')
  eq(transcript.includes("from './ReelTray'"), true, 'Transcript panel imports the reel tray component')
  eq(transcript.includes('data-cut-reel=""'), false, 'Transcript panel no longer owns reel tray markup inline')
  eq(transcript.includes('data-cut-action="assemble-reel"'), false, 'Transcript panel no longer owns Assemble reel selector inline')
  eq(transcriptReelTray.includes('export default function ReelTray'), true, 'Transcript reel tray module exports the component')
  eq(transcriptReelTray.includes('data-cut-reel=""'), true, 'Transcript reel tray component owns the tray selector')
  eq(transcriptReelTray.includes('data-cut-action="assemble-reel"'), true, 'Transcript reel tray component owns Assemble reel selector')
  eq(transcriptReelTray.includes('data-cut-action="reel-clear"'), true, 'Transcript reel tray component owns Clear selector')
  eq(transcriptReelTray.includes('data-cut-action="reel-remove"'), true, 'Transcript reel tray component owns Remove span selector')
  eq(cardLabel(mk('dub', 'service')), { title: 'AI dubbing', role: 'Re-voice translated speech as a new audio track' }, 'Dub service label is compact and outcome-led')
  eq(cardLabel(mk('diarize', 'service')), { title: 'Speaker labels', role: 'Mark who speaks when in a transcript' }, 'Diarize service label is compact and outcome-led')
  eq(serviceRuntime.includes("const runtimeReady = card.status === 'ok'"), true, 'Environment service runtime ready pill follows card status, not stale reachable details')
  eq(serviceRuntime.includes("card.details?.reachable === true"), false, 'Environment service runtime does not show green ready beside degraded setup copy')
  eq(
    envCardRow.includes("card.status === 'ok' && card.details?.hardware_available === true"),
    true,
    'Environment GPU chip requires both OK status and detected hardware before showing green GPU',
  )
  eq(
    envCardRow.includes("return card.details?.hardware_available === true\n      ? { label: 'GPU', cls: 'env-st--ok' }"),
    false,
    'Environment GPU chip does not ignore degraded card status',
  )
  eq(
    STT_MODELS.map((m) => m.label),
    ['Parakeet v3', 'Canary-1B-v2 + MMS_FA', 'Whisper large-v3'],
    'Environment STT picker uses compact model names',
  )
  const serverRoot = resolve(here, '../../app/server/src')
  const doctor = readFileSync(resolve(serverRoot, 'doctor.rs'), 'utf8')
  const dispatch = readFileSync(resolve(serverRoot, 'dispatch.rs'), 'utf8')
  const dispatchTestsPath = resolve(serverRoot, 'dispatch/tests.rs')
  const dispatchTests = readFileSync(dispatchTestsPath, 'utf8')
  const systemToolTestsPath = resolve(serverRoot, 'dispatch/tests/system_tools.rs')
  const systemToolTests = existsSync(systemToolTestsPath) ? readFileSync(systemToolTestsPath, 'utf8') : ''
  const screenRecordTestsPath = resolve(serverRoot, 'dispatch/tests/screen_record.rs')
  const screenRecordTests = existsSync(screenRecordTestsPath) ? readFileSync(screenRecordTestsPath, 'utf8') : ''
  const renderVerifyTestsPath = resolve(serverRoot, 'dispatch/tests/render_verify.rs')
  const renderVerifyTests = existsSync(renderVerifyTestsPath) ? readFileSync(renderVerifyTestsPath, 'utf8') : ''
  const recipeRunnerTestsPath = resolve(serverRoot, 'dispatch/tests/recipe_runner.rs')
  const recipeRunnerTests = existsSync(recipeRunnerTestsPath) ? readFileSync(recipeRunnerTestsPath, 'utf8') : ''
  const mainRs = readFileSync(resolve(serverRoot, 'main.rs'), 'utf8')
  eq(doctor.includes('Install captions'), true, 'Doctor hints use the current captions install wording')
  eq(doctor.includes('Set up perception'), false, 'Doctor hints do not send users to perception jargon')
  const serviceCardsPath = resolve(serverRoot, 'doctor/service_cards.rs')
  const serviceCards = existsSync(serviceCardsPath) ? readFileSync(serviceCardsPath, 'utf8') : ''
  eq(existsSync(serviceCardsPath), true, 'Doctor optional service cards live in a helper module')
  eq(doctor.includes('fn service_card('), false, 'doctor.rs does not own optional service card construction inline')
  eq(doctor.includes('service_cards::dub_card()'), true, 'doctor.rs routes Dub card through service_cards helper')
  eq(doctor.includes('service_cards::diarize_card()'), true, 'doctor.rs routes Diarize card through service_cards helper')
  eq(serviceCards.includes('fn service_reachable'), true, 'service_cards helper owns optional service reachability probing')
  eq(serviceCards.includes('CardStatus::Unknown'), true, 'service_cards helper preserves neutral optional-service status')
  const ffmpegSettingsPath = resolve(serverRoot, 'ffmpeg_settings.rs')
  const ffmpegSettings = existsSync(ffmpegSettingsPath) ? readFileSync(ffmpegSettingsPath, 'utf8') : ''
  eq(existsSync(ffmpegSettingsPath), true, 'server FFmpeg settings handler lives in a focused module')
  eq(mainRs.includes('mod ffmpeg_settings;'), true, 'server main compiles the FFmpeg settings module')
  eq(dispatch.includes('async fn system_set_ffmpeg'), false, 'dispatch no longer owns the system.set_ffmpeg handler body')
  eq(
    dispatch.includes('"system.set_ffmpeg"')
      && dispatch.includes('crate::ffmpeg_settings::system_set_ffmpeg(state, args)')
      && dispatch.includes('.await')
      && dispatch.includes('.into()'),
    true,
    'dispatch delegates system.set_ffmpeg to the FFmpeg settings module',
  )
  eq(ffmpegSettings.includes('pub(crate) async fn system_set_ffmpeg'), true, 'FFmpeg settings module owns the public handler')
  eq(ffmpegSettings.includes('cut_media::hwencode::probe_ffmpeg_caps'), true, 'FFmpeg settings module validates a picked ffmpeg executable')
  eq(ffmpegSettings.includes('cut_media::toolpath::write_override_setting'), true, 'FFmpeg settings module persists the chosen ffmpeg override')
  eq(ffmpegSettings.includes('"restart_required": true'), true, 'FFmpeg settings module preserves the restart-required result contract')
  eq(dispatchTests.includes('mod system_tools;'), true, 'dispatch tests register system tool tests as a submodule')
  eq(existsSync(systemToolTestsPath), true, 'dispatch system tool tests live in a focused submodule')
  eq(dispatchTests.includes('async fn system_doctor_returns_cards_without_a_project'), false, 'dispatch tests file no longer owns system.doctor test bodies inline')
  eq(dispatchTests.includes('async fn system_fetch_tool_full_path_against_local_fixture'), false, 'dispatch tests file no longer owns system.fetch_tool fixture inline')
  eq(systemToolTests.includes('async fn system_doctor_returns_cards_without_a_project'), true, 'system tool test module owns system.doctor coverage')
  eq(systemToolTests.includes('async fn system_fetch_tool_full_path_against_local_fixture'), true, 'system tool test module owns full fetch-tool fixture coverage')
  eq(dispatchTests.includes('mod screen_record;'), true, 'dispatch tests register screen-record tests as a submodule')
  eq(existsSync(screenRecordTestsPath), true, 'screen-record tests live in a focused submodule')
  eq(dispatchTests.includes('async fn screen_record_stop_rejects_path_like_capture_id'), false, 'dispatch tests file no longer owns screen-record stop test bodies inline')
  eq(dispatchTests.includes('async fn f16_polish_places_system_audio'), false, 'dispatch tests file no longer owns screen-record polish fixture inline')
  eq(screenRecordTests.includes('async fn screen_record_stop_rejects_path_like_capture_id'), true, 'screen-record test module owns stop validation coverage')
  eq(screenRecordTests.includes('async fn f16_polish_places_system_audio'), true, 'screen-record test module owns polish system-audio coverage')
  eq(dispatchTests.includes('mod render_verify;'), true, 'dispatch tests register render/verify tests as a submodule')
  eq(existsSync(renderVerifyTestsPath), true, 'render/verify tests live in a focused submodule')
  eq(dispatchTests.includes('async fn render_queue_rejects_empty_jobs'), false, 'dispatch tests file no longer owns render.queue test bodies inline')
  eq(dispatchTests.includes('async fn verify_pregate_wiring_no_project_then_empty_pass'), false, 'dispatch tests file no longer owns verify.pregate wiring coverage inline')
  eq(dispatchTests.includes('async fn render_final_dry_run_plans_without_encoding'), false, 'dispatch tests file no longer owns render.final dry-run coverage inline')
  eq(dispatchTests.includes('async fn jobs_cancel_aborts_active_job_through_dispatch'), false, 'dispatch tests file no longer owns jobs.cancel dispatch coverage inline')
  eq(renderVerifyTests.includes('async fn render_queue_rejects_empty_jobs'), true, 'render/verify test module owns render.queue validation coverage')
  eq(renderVerifyTests.includes('async fn verify_pregate_wiring_no_project_then_empty_pass'), true, 'render/verify test module owns verify.pregate wiring coverage')
  eq(renderVerifyTests.includes('async fn render_final_dry_run_plans_without_encoding'), true, 'render/verify test module owns render.final dry-run coverage')
  eq(renderVerifyTests.includes('async fn jobs_cancel_aborts_active_job_through_dispatch'), true, 'render/verify test module owns jobs.cancel dispatch coverage')
  eq(dispatchTests.includes('mod recipe_runner;'), true, 'dispatch tests register recipe runner tests as a submodule')
  eq(existsSync(recipeRunnerTestsPath), true, 'recipe runner tests live in a focused submodule')
  eq(dispatchTests.includes('fn marker_test_recipe'), false, 'dispatch tests file no longer owns recipe runner fixtures inline')
  eq(dispatchTests.includes('async fn recipe_run_drives_stages_checkpoints_and_reverts'), false, 'dispatch tests file no longer owns recipe.run checkpoint coverage inline')
  eq(dispatchTests.includes('async fn recipe_dry_run_plans_without_mutating'), false, 'dispatch tests file no longer owns recipe dry-run coverage inline')
  eq(dispatchTests.includes('async fn recipe_list_describe_are_pure_reads'), false, 'dispatch tests file no longer owns recipe pure-read coverage inline')
  eq(recipeRunnerTests.includes('fn marker_test_recipe'), true, 'recipe runner test module owns marker recipe fixtures')
  eq(recipeRunnerTests.includes('async fn recipe_run_drives_stages_checkpoints_and_reverts'), true, 'recipe runner test module owns recipe.run checkpoint coverage')
  eq(recipeRunnerTests.includes('async fn recipe_dry_run_plans_without_mutating'), true, 'recipe runner test module owns dry-run coverage')
  eq(recipeRunnerTests.includes('async fn recipe_list_describe_are_pure_reads'), true, 'recipe runner test module owns pure-read recipe coverage')
  const smartBinTestsPath = resolve(serverRoot, 'dispatch/tests/smart_bins.rs')
  const smartBinTests = existsSync(smartBinTestsPath) ? readFileSync(smartBinTestsPath, 'utf8') : ''
  eq(dispatchTests.includes('mod smart_bins;'), true, 'dispatch tests register smart-bin tests as a submodule')
  eq(existsSync(smartBinTestsPath), true, 'smart-bin tests live in a focused submodule')
  eq(
    smartBinTests.includes('async fn smart_bins_filter_resolution_offline_and_modified_time'),
    true,
    'smart-bin test module owns resolution/offline/date coverage',
  )
  const envCss = readFileSync(resolve(srcRoot, 'panels/Environment/environment.css'), 'utf8')
  eq(
    /\.env-advanced-row dd\s*{[^}]*overflow-wrap:\s*anywhere/s.test(envCss) &&
      /\.env-advanced-row dd\s*{[^}]*white-space:\s*normal/s.test(envCss),
    true,
    'Environment Advanced details wrap long diagnostics instead of clipping them',
  )
}

// --- Assets smart bins: release UI exposes concrete criteria ---------------
// Smart bins need to be visible to non-specialist editors as normal filters:
// type, name, unused, 4K, missing media, and recent files. The saved-bin call
// must persist those criteria rather than collapsing them into an opaque item.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const assetsPanel = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const client = readFileSync(resolve(srcRoot, 'lib/client.ts'), 'utf8')

  for (const selector of [
    'data-cut-asset-resolution-filter',
    'data-cut-asset-offline-filter',
    'data-cut-asset-recent-filter',
  ]) {
    eq(assetsPanel.includes(selector), true, `Assets panel exposes ${selector}`)
  }
  for (const criterion of ['min_width', 'min_height', 'offline', 'modified_after_ms']) {
    eq(assetsPanel.includes(criterion), true, `Assets save-bin payload includes ${criterion}`)
    eq(client.includes(`${criterion}?:`), true, `typed media.bin_save args include ${criterion}`)
  }
  eq(assetsPanel.includes('(media.bin_save)'), false, 'save-bin tooltip does not expose raw debug verb names')
  eq(assetsPanel.includes('(media.relink)'), false, 'relink tooltip does not expose raw debug verb names')
}

// --- Offline media is one shared state across Assets, Timeline, and Preview --
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const context = readFileSync(resolve(srcRoot, 'app/OfflineMediaContext.tsx'), 'utf8')
  const assets = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const clip = readFileSync(resolve(srcRoot, 'panels/Timeline/ClipView.tsx'), 'utf8')
  const timelineCss = readFileSync(resolve(srcRoot, 'panels/Timeline/timeline.css'), 'utf8')
  const preview = readFileSync(resolve(srcRoot, 'panels/Preview/index.tsx'), 'utf8')
  const offlinePreview = readFileSync(resolve(srcRoot, 'panels/Preview/PreviewOffline.tsx'), 'utf8')
  const transcriptCss = readFileSync(resolve(srcRoot, 'panels/Transcript/transcript.css'), 'utf8')

  eq(context.includes("callVerb('media.check', {})"), true, 'Offline media provider owns the shared filesystem check')
  eq(assets.includes('useOfflineMedia()'), true, 'Assets consumes the shared offline-media provider')
  eq(clip.includes('data-cut-timeline-offline'), true, 'Timeline clips expose a labelled offline state')
  eq(clip.includes('data-cut-action="timeline-relink-offline"'), true, 'Timeline offline clips expose Relink')
  eq(
    timelineCss.includes('z-index: calc(var(--z-playhead) + 1);') && timelineCss.includes('overflow: visible;'),
    true,
    'Narrow offline clips preserve temporal width while their recovery control wins over seam handles',
  )
  eq(preview.includes('posterFailed ?'), true, 'Preview replaces a failed poster instead of leaving a broken image')
  eq(offlinePreview.includes('data-cut-preview-offline'), true, 'Preview exposes a labelled offline placeholder')
  eq(offlinePreview.includes('data-cut-action="preview-relink-offline"'), true, 'Preview offline placeholders expose Relink')
  eq(transcriptCss.includes('z-index: calc(var(--z-dropdown) + 1);'), true, 'Transcript Tools stays above body recovery and range-selection controls')
}

// --- Library/Projects copy: visible actions must explain outcomes ------------
// Keep implementation words out of the main UI. Library should say it stores a
// managed copy; Projects should explain delete vs remove-from-list without
// exposing .cutproj internals to non-specialist users.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const library = readFileSync(resolve(srcRoot, 'panels/Library/index.tsx'), 'utf8')
  const projects = readFileSync(resolve(srcRoot, 'panels/Projects/index.tsx'), 'utf8')
  const projectsCss = readFileSync(resolve(srcRoot, 'panels/Projects/projects.css'), 'utf8')
  const libraryOffenders = [
    'Make portable',
    'Embedded into the library (portable)',
    'Embedded in the library (portable)',
    'Portable copy',
  ].filter((term) => library.includes(term))
  const projectOffenders = [
    "project's .cutproj folder",
    '.cutproj from disk',
    'use ✕',
    '✕',
    '🗑',
  ].filter((term) => projects.includes(term))
  eq(libraryOffenders, [], 'Library visible copy avoids portable jargon')
  eq(projectOffenders, [], 'Projects visible copy avoids file-format jargon and text glyph controls')
}

// --- Review surface: review controls must be discoverable and human-worded ----
// Keep the Review/OpsFeed selective-undo control available for
// keyboard-selected rows, and keep user-facing labels out of internal
// engine/history terms. The adjacent guards lock already-normalized IA labels so
// the same stale-copy drift does not re-enter Assets, Comments, Keymap, or export
// settings while this area is being touched.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const app = readFileSync(resolve(srcRoot, 'App.tsx'), 'utf8')
  const agentChat = readFileSync(resolve(srcRoot, 'panels/AgentChat/index.tsx'), 'utf8')
  const opsFeed = readFileSync(resolve(srcRoot, 'panels/Review/OpsFeed.tsx'), 'utf8')
  const reviewCss = readFileSync(resolve(srcRoot, 'panels/Review/review.css'), 'utf8')
  const assets = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const comments = readFileSync(resolve(srcRoot, 'panels/Comments/index.tsx'), 'utf8')
  const keymap = readFileSync(resolve(srcRoot, 'KeymapOverlay.tsx'), 'utf8')
  const keymapSource = readFileSync(resolve(srcRoot, 'lib/keymap.ts'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const appRightRail = readFileSync(resolve(srcRoot, 'app/AppRightRail.tsx'), 'utf8')
  const appSurfaceEvents = readFileSync(resolve(srcRoot, 'app/useAppSurfaceEvents.ts'), 'utf8')

  eq(opsFeed.includes('>blocks rebase<'), false, 'OpsFeed dependent badge avoids engine rebase jargon')
  eq(opsFeed.includes('blocked by later edits'), true, 'OpsFeed dependent badge explains the user outcome')
  eq(
    reviewCss.includes('.rr-op--focused .rr-op__actions') &&
      reviewCss.includes('.rr-op:focus-within .rr-op__actions'),
    true,
    'OpsFeed actions are visible for focused and keyboard-interactive rows',
  )
  eq(opsFeed.includes('tabIndex={0}'), true, 'OpsFeed rows can receive keyboard focus')
  eq(assets.includes('Add at playhead'), true, 'Assets primary action says Add at playhead')
  eq(comments.includes('Comments{'), true, 'Comments rail title is explicit')
  eq(comments.includes('data-cut-comment-disclosure'), true, 'Comments rows expose a named disclosure control')
  eq(comments.includes('cm__row-caret'), true, 'Comments disclosure uses a visible caret affordance')
  eq(comments.includes('aria-expanded={selected}'), true, 'Comments disclosure exposes expanded state')
  eq(
    keymapSource.includes("id: 'recording.toggle'") && keymapSource.includes("binding: 'F9'") &&
      keymapSource.includes('export const FIXED_KEY_ACTIONS'),
    true,
    'Central keymap lists F9 honestly as a fixed recorder shortcut',
  )
  eq(
    keymapSource.indexOf("id: 'recording.toggle'") > keymapSource.indexOf('export const FIXED_KEY_ACTIONS'),
    true,
    'Recorder shortcuts are excluded from the remappable action table',
  )
  eq(keymap.includes('fixedRows()'), true, 'Keymap overlay derives fixed Recording shortcuts from the central keymap')
  eq(topbar.includes('Aspect / reframe'), true, 'Topbar export settings label aspect controls plainly')
  eq(topbar.includes('File format'), true, 'Topbar export settings label format controls plainly')
  eq(appSurfaceEvents.includes("cut:open-chat"), true, 'App exposes an event to open Agent Chat')
  eq(app.includes('agentChatPrefill'), true, 'App preserves Agent Chat prefill while the lazy chat tab loads')
  eq(appSurfaceEvents.includes('setAgentChatPrefill({ prompt, nonce:'), true, 'App hands chat prompts through state instead of a timing-only event')
  eq(
    `${app}\n${appRightRail}`.includes('<AgentChat project={project} prefill={agentChatPrefill} />'),
    true,
    'App passes pending prompts into Agent Chat',
  )
  eq(`${app}\n${appSurfaceEvents}`.includes("document.dispatchEvent(new CustomEvent('cut:agent-chat-prompt'"), false, 'App does not drop lazy chat prompts through a fixed timeout dispatch')
  eq(agentChat.includes("cut:agent-chat-prompt"), true, 'Agent Chat accepts external prompt prefill events')
  eq(agentChat.includes('prefill?.nonce'), true, 'Agent Chat applies app-level prefill handoffs')
  eq(agentChat.includes("attachments: turnAttachments.length > 0"), true, 'Agent Chat sends only selected registered asset IDs')
  eq(agentChat.includes('data-cut-chat-turn-attachment'), true, 'Agent Chat keeps attachment receipts on the user turn')
  eq(agentChat.includes('data-cut-chat-prompt-library'), true, 'Agent Chat exposes the curated prompt library')
  eq(agentChat.includes('data-cut-chat-prompt-verbs'), true, 'Agent Chat keeps each curated prompt mapping inspectable')
  eq(agentChat.includes('setPromptLibraryOpen(false)'), true, 'Choosing a curated prompt closes the prompt menu')
  eq(agentChat.includes('void loadAgents(true)\n  }, [loadAgents])'), true, 'Agent Chat refreshes agent readiness on mount instead of showing stale cached ready')
  eq(comments.includes('res?.failed_verb'), false, 'Comments apply failure flash does not interpolate undefined failed_verb')
  eq(comments.includes('res?.checkpoint'), false, 'Comments apply success/failure flash does not interpolate undefined checkpoint')
}

// --- Keep resolved user-visible homes stable ----------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const inspector = readFileSync(resolve(srcRoot, 'panels/Inspector/index.tsx'), 'utf8')
  const projectCaptionsPath = resolve(srcRoot, 'panels/Inspector/ProjectCaptionsSection.tsx')
  const projectCaptions = existsSync(projectCaptionsPath) ? readFileSync(projectCaptionsPath, 'utf8') : ''
  const kinetic = readFileSync(resolve(srcRoot, 'panels/Kinetic/index.tsx'), 'utf8')
  const transcript = readFileSync(resolve(srcRoot, 'panels/Transcript/index.tsx'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')

  eq(existsSync(projectCaptionsPath), true, 'Inspector project captions section has its own module')
  eq(inspector.includes("from './ProjectCaptionsSection'"), true, 'Inspector imports the project captions component module')
  eq(inspector.includes('<ProjectCaptionsSection'), true, 'Inspector renders the project captions component')
  eq(inspector.includes('data-cut-inspector-group="captions"'), false, 'Inspector panel no longer owns project captions markup inline')
  eq(projectCaptions.includes('data-cut-inspector-group="captions"'), true, 'Project captions module owns the group selector')
  eq(projectCaptions.includes('data-cut-caption-text'), true, 'Project captions module owns the caption text selector')
  eq(projectCaptions.includes('data-cut-caption-import'), true, 'Project captions module owns the subtitle import selector')
  eq(projectCaptions.includes('data-cut-action="translate-captions"'), true, 'Project captions module owns caption translation action')
  eq(projectCaptions.includes('data-cut-action="translate-transcript"'), true, 'Project captions module owns transcript translation action')
  eq(projectCaptions.includes("callVerb('captions.add_text'"), true, 'Project captions module dispatches captions.add_text')
  eq(projectCaptions.includes("callVerb('captions.import'"), true, 'Project captions module dispatches captions.import')
  eq(projectCaptions.includes("callVerb('captions.translate'"), true, 'Project captions module dispatches captions.translate')
  eq(projectCaptions.includes("callVerb('transcript.translate'"), true, 'Project captions module dispatches transcript.translate')
  eq(projectCaptions.includes('caption look (captions.save_style)'), false, 'Caption style save tooltip does not expose captions.save_style')
  eq(projectCaptions.includes('caption look (captions.apply_style'), false, 'Caption style apply tooltip does not expose captions.apply_style')
  eq(projectCaptions.includes('Update the built-in caption style for this position (captions.set_style)'), false, 'Caption style setter tooltip does not expose captions.set_style')
  eq(projectCaptions.includes('Save the current color, size, and position as a reusable caption look'), true, 'Caption style save tooltip uses plain language')
  eq(projectCaptions.includes('Apply this look to every caption in the project'), true, 'Caption style apply tooltip uses plain language')
  eq(kinetic.includes('captions.kinetic).</p>'), false, 'Kinetic captions drawer subtitle does not expose captions.kinetic')
  eq(transcript.includes('title="captions.kinetic'), false, 'Transcript animate tooltip does not expose captions.kinetic')
  eq(fullCoverage.includes("name: 'caption-composer-add'"), true, 'Full coverage drives the project-scope caption composer')
  eq(topbar.includes('data-cut-director-open'), true, 'Topbar exposes a visible Director launcher')
  eq(topbar.includes("directorOpen && aspect !== 'project'"), true, 'Director modal stays gated to reframe aspects')
  eq(fullCoverage.includes("name: 'director-open'"), true, 'Full coverage opens the Director launcher')
}

// --- Selected-clip regression: high-value edits need visible homes ------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const inspector = readFileSync(resolve(srcRoot, 'panels/Inspector/index.tsx'), 'utf8')
  const clipActionsPath = resolve(srcRoot, 'panels/Inspector/ClipActionsSection.tsx')
  const clipActions = existsSync(clipActionsPath) ? readFileSync(clipActionsPath, 'utf8') : ''
  const clipActionsHookPath = resolve(srcRoot, 'panels/Inspector/useInspectorClipActions.ts')
  const clipActionsHook = existsSync(clipActionsHookPath) ? readFileSync(clipActionsHookPath, 'utf8') : ''
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')

  eq(clipActions.includes('data-cut-inspector-quick-actions'), true, 'Inspector exposes selected-clip quick actions')
  eq(clipActions.includes('data-cut-inspector-replace-asset'), true, 'Inspector offers a selected-clip replacement asset picker')
  eq(clipActions.includes('data-cut-inspector-action="replace-source"'), true, 'Inspector exposes selected-clip Replace source action')
  eq(clipActionsHook.includes("callVerb('edit.replace'"), true, 'Inspector selected-clip Replace dispatches edit.replace')
  eq(clipActionsHook.includes('target_clip: sel.clip.id'), true, 'Inspector Replace targets the selected clip id')
  eq(clipActions.includes('data-cut-inspector-action="detach-audio"'), true, 'Inspector exposes selected-clip Detach audio action')
  eq(clipActionsHook.includes("callVerb('edit.detach_audio'"), true, 'Inspector selected-clip Detach audio dispatches edit.detach_audio')
  eq(fullCoverage.includes("name: 'inspector-replace-source'"), true, 'Full coverage drives Inspector Replace source')
  eq(fullCoverage.includes("name: 'inspector-detach-audio'"), true, 'Full coverage drives Inspector Detach audio')
  eq(existsSync(clipActionsHookPath), true, 'Inspector Clip actions controller has its own hook module')
  eq(inspector.includes("from './useInspectorClipActions'"), true, 'Inspector imports the Clip actions controller hook')
  eq(inspector.includes('useInspectorClipActions({'), true, 'Inspector delegates Clip actions controller state to the hook')
  eq(existsSync(clipActionsPath), true, 'Inspector Clip actions section has its own module')
  eq(inspector.includes("from './ClipActionsSection'"), true, 'Inspector imports the Clip actions component module')
  eq(inspector.includes('<ClipActionsSection'), true, 'Inspector renders the Clip actions component')
  eq(clipActions.includes('data-cut-inspector-quick-actions'), true, 'Clip actions module owns the quick-actions group selector')
  eq(clipActions.includes('data-cut-inspector-replace-asset'), true, 'Clip actions module owns the replacement asset picker selector')
  eq(clipActions.includes('data-cut-inspector-action="replace-source"'), true, 'Clip actions module owns the Replace source action selector')
  eq(clipActions.includes('data-cut-inspector-action="detach-audio"'), true, 'Clip actions module owns the Detach audio action selector')
  eq(inspector.includes('data-cut-inspector-quick-actions'), false, 'Inspector panel no longer owns Clip actions markup inline')
  eq(inspector.includes('const [replaceAssetId'), false, 'Inspector panel no longer owns replacement asset state inline')
  eq(inspector.includes('const [clipActionNote'), false, 'Inspector panel no longer owns quick-action note state inline')
  eq(inspector.includes('const runReplaceSource = useCallback'), false, 'Inspector panel no longer owns Replace source controller inline')
  eq(inspector.includes('const runDetachAudio = useCallback'), false, 'Inspector panel no longer owns Detach audio controller inline')
  eq(clipActionsHook.includes('const [replaceAssetId'), true, 'Inspector Clip actions hook owns replacement asset state')
  eq(clipActionsHook.includes('const [clipActionNote'), true, 'Inspector Clip actions hook owns quick-action note state')
  eq(clipActionsHook.includes('replacementCandidates(project'), true, 'Inspector Clip actions hook derives compatible replacement assets')
}

// --- Inspector color/grade/window controllers stay out of the panel shell ----
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const inspector = readFileSync(resolve(srcRoot, 'panels/Inspector/index.tsx'), 'utf8')
  const colorHookPath = resolve(srcRoot, 'panels/Inspector/useInspectorColorControls.ts')
  const colorHook = existsSync(colorHookPath) ? readFileSync(colorHookPath, 'utf8') : ''
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')

  eq(existsSync(colorHookPath), true, 'Inspector color/grade/window controller has its own hook module')
  eq(inspector.includes("from './useInspectorColorControls'"), true, 'Inspector imports the Color controls controller hook')
  eq(inspector.includes('useInspectorColorControls({'), true, 'Inspector delegates color/grade/window controller state to the hook')
  for (const inline of [
    'const [colorBusy',
    'const [colorNote',
    'const [presets',
    'const [selPreset',
    'const [saveName',
    'const [stackLayer',
    'const [winRegion',
    'const [winLook',
    'const setProjectSpace',
    'const setClipInputSpace',
    'const reloadPresets',
    'const saveLook',
    'const applyLook',
    'const stackBase',
    'const addStackLayer',
    'const removeStackLayer',
    'const clipWindows',
    'const addWindow',
    'const removeWindow',
    'const clearWindows',
  ]) {
    eq(inspector.includes(inline), false, `Inspector panel no longer owns color controller detail inline: ${inline}`)
  }
  for (const expected of [
    "callVerb('project.color'",
    "callVerb('edit.color_space'",
    "callVerb('grade.list'",
    "callVerb('grade.save'",
    "callVerb('grade.apply'",
    "callVerb('edit.grade_stack'",
    "callVerb('edit.grade_window'",
    'const [colorBusy',
    'const [colorNote',
    'const [presets',
    'const [selPreset',
    'const [saveName',
    'const [stackLayer',
    'const [winRegion',
    'const [winLook',
    'const clipWindows',
  ]) {
    eq(colorHook.includes(expected), true, `Inspector color controls hook owns: ${expected}`)
  }
  eq(colorHook.includes('remove_index: idx'), true, 'Inspector removes one power window with the atomic contract')
  eq(colorHook.includes('rebuild windows (remove one)'), false, 'Inspector never clears and rebuilds the power-window stack to remove one')
  for (const expected of [
    "name: 'color-working'",
    "name: 'color-output'",
    "name: 'color-input'",
    "name: 'grade-save'",
    "name: 'grade-apply'",
    "name: 'grade-stack-add'",
    "name: 'grade-stack-remove'",
    "name: 'grade-window-add'",
    "name: 'grade-window-remove'",
  ]) {
    eq(fullCoverage.includes(expected), true, `Full coverage drives Inspector color/grade/window action: ${expected}`)
  }
}

// --- Inspector auto-video controllers stay out of the panel shell ------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const inspector = readFileSync(resolve(srcRoot, 'panels/Inspector/index.tsx'), 'utf8')
  const autoHookPath = resolve(srcRoot, 'panels/Inspector/useInspectorAutoVideoControls.ts')
  const autoHook = existsSync(autoHookPath) ? readFileSync(autoHookPath, 'utf8') : ''
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')

  eq(existsSync(autoHookPath), true, 'Inspector auto-video controller has its own hook module')
  eq(inspector.includes("from './useInspectorAutoVideoControls'"), true, 'Inspector imports the auto-video controller hook')
  eq(inspector.includes('useInspectorAutoVideoControls({'), true, 'Inspector delegates auto-video controller state to the hook')
  for (const inline of [
    'const [matchRef',
    'const [zoomIntensity',
    'const [adjLook',
    'const [autoNote',
    'const [autoBusy',
    'const refCandidates',
    'const autoBalance',
    'const colorMatch',
    'const autoZoom',
    'const addAdjustment',
  ]) {
    eq(inspector.includes(inline), false, `Inspector panel no longer owns auto-video detail inline: ${inline}`)
  }
  for (const expected of [
    "callVerb('edit.auto_balance'",
    "callVerb('edit.color_match'",
    "callVerb('edit.auto_zoom'",
    "callVerb('edit.adjustment'",
    'layoutTrack(track)',
    'const refCandidates',
    'const [matchRef',
    'const [zoomIntensity',
    'const [adjLook',
    'const [autoNote',
    'const [autoBusy',
  ]) {
    eq(autoHook.includes(expected), true, `Inspector auto-video hook owns: ${expected}`)
  }
  for (const expected of [
    "name: 'auto-balance'",
    "name: 'auto-zoom'",
    "name: 'adjustment'",
    "name: 'edit.color_match(Inspector Match)'",
  ]) {
    eq(fullCoverage.includes(expected), true, `Full coverage drives Inspector auto-video action: ${expected}`)
  }
}

// --- Color analysis receipts state their v1 sampling limits -----------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '..', '..')
  const dispatch = [
    readFileSync(resolve(root, 'app/server/src/dispatch.rs'), 'utf8'),
    readFileSync(resolve(root, 'app/server/src/dispatch/edit_tools.rs'), 'utf8'),
    readFileSync(resolve(root, 'app/server/src/dispatch/edit_tools/visual.rs'), 'utf8'),
  ].join('\n')
  const verbs = readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')
  const reference = readFileSync(resolve(root, 'skill/shellx-cut/reference.md'), 'utf8')

  eq(
    dispatch.includes('"sampled_at_s"') && dispatch.includes('"analysis_note"') && dispatch.includes('single representative mid-clip frame'),
    true,
    'Color match/auto-balance results expose sampled time and v1 analysis limits',
  )
  eq(
    verbs.includes('sampled_at_s') && verbs.includes('analysis_note'),
    true,
    'Color match/auto-balance schema result contracts document sampled time and analysis note',
  )
  eq(
    reference.includes('sampled_at_s') && reference.includes('analysis_note'),
    true,
    'Color match/auto-balance skill reference documents sampled time and analysis note',
  )
}

// --- Fit-to-fill source-window contract stays documented ----------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '..', '..')
  const coreEdit = readFileSync(resolve(root, 'app/core/src/edit.rs'), 'utf8')
  const verbs = readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')
  const reference = readFileSync(resolve(root, 'skill/shellx-cut/reference.md'), 'utf8')

  eq(
    coreEdit.includes('source range must stay inside the probed asset duration'),
    true,
    'fit_to_fill core rejects source windows beyond the probed asset duration',
  )
  eq(
    verbs.includes('Explicit source windows must stay inside the probed asset duration.'),
    true,
    'fit_to_fill schema documents source-window probe bounds',
  )
  eq(
    reference.includes('must stay inside the probed asset duration'),
    true,
    'fit_to_fill skill reference documents source-window probe bounds',
  )
}

// --- Inspector has one commit home per selected-clip verb --------------------
// Speed / Retime owns speed, reverse, freeze, and speed-ramp. The Picture tool
// group may keep deep-drawer launchers and adjacent one-shot repairs, but it must
// not reintroduce duplicate speed/reverse commit controls.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const inspector = readFileSync(resolve(srcRoot, 'panels/Inspector/index.tsx'), 'utf8')
  const speedSectionPath = resolve(srcRoot, 'panels/Inspector/SpeedSection.tsx')
  const speedSection = existsSync(speedSectionPath) ? readFileSync(speedSectionPath, 'utf8') : ''
  const engagementSectionPath = resolve(srcRoot, 'panels/Inspector/EngagementSection.tsx')
  const engagementSection = existsSync(engagementSectionPath) ? readFileSync(engagementSectionPath, 'utf8') : ''
  const grade = readFileSync(resolve(srcRoot, 'panels/Grade/index.tsx'), 'utf8')
  const gradeInspectorGate = readFileSync(resolve(here, 'verify-grade-inspector.mjs'), 'utf8')

  eq(existsSync(speedSectionPath), true, 'Inspector Speed / Retime component has its own module')
  eq(inspector.includes("from './SpeedSection'"), true, 'Inspector imports the Speed / Retime component module')
  eq(inspector.includes('function SpeedSection'), false, 'Inspector panel no longer defines Speed / Retime inline')
  eq(inspector.includes('<SpeedSection'), true, 'Inspector renders the Speed / Retime component')
  eq(speedSection.includes('propKey="speed"'), true, 'Speed / Retime module owns the speed input selector')
  eq(speedSection.includes('data-cut-prop="speed-reverse"'), true, 'Speed / Retime module owns the Reverse toggle')
  eq(speedSection.includes('defaultCollapsed'), true, 'Speed / Retime starts collapsed as a secondary clip adjustment')
  eq(inspector.includes('data-cut-inspector-speed='), false, 'Inspector no longer has duplicate quick speed buttons')
  eq(inspector.includes('data-cut-inspector-action="reverse"'), false, 'Inspector no longer has a duplicate Picture reverse button')
  eq(gradeInspectorGate.includes('data-cut-prop-input="speed"'), true, 'Grade/Inspector gate uses the Speed / Retime speed input')
  eq(gradeInspectorGate.includes('data-cut-prop="speed-reverse"'), true, 'Grade/Inspector gate uses the Speed / Retime Reverse toggle')
  eq(gradeInspectorGate.includes('data-cut-inspector-speed='), false, 'Grade/Inspector gate does not depend on removed duplicate speed buttons')
  eq(gradeInspectorGate.includes('data-cut-inspector-action="reverse"'), false, 'Grade/Inspector gate does not depend on removed duplicate reverse button')
  eq(gradeInspectorGate.includes('seedDuckPerception'), true, 'Grade/Inspector gate seeds deterministic duck perception')
  eq(gradeInspectorGate.includes('duck-probe'), false, 'Grade/Inspector gate does not wait on live perception with duck probes')
  eq(gradeInspectorGate.includes('v1 perception never ready'), false, 'Grade/Inspector gate does not fail on live perception readiness text')
  eq(existsSync(engagementSectionPath), true, 'Inspector Engagement section has its own module')
  eq(inspector.includes("from './EngagementSection'"), true, 'Inspector imports the Engagement component module')
  eq(inspector.includes('function EngagementSection'), false, 'Inspector panel no longer defines Engagement inline')
  eq(inspector.includes('<EngagementSection'), true, 'Inspector renders the Engagement component')
  eq(engagementSection.includes('data-cut-inspector-group="engagement"'), true, 'Engagement module owns the group selector')
  eq(engagementSection.includes('data-cut-action="score-clip"'), true, 'Engagement module owns the score action selector')
  eq(engagementSection.includes("callVerb('score.clip'"), true, 'Engagement module dispatches score.clip')
  eq(
    engagementSection.includes('sectionKey="engagement"') && engagementSection.includes('defaultCollapsed'),
    true,
    'Engagement starts collapsed as an analysis-only inspector section',
  )
  eq(grade.includes('data-cut-grade-lut-pick'), true, 'Grade LUT has a native .cube picker')
  eq(grade.includes('data-cut-grade-lut-advanced'), true, 'Grade LUT manual path is inside an Advanced section')
  eq(grade.includes('placeholder="/full/path/to/look.cube"'), false, 'Grade LUT does not lead with a raw absolute-path placeholder')
  eq(gradeInspectorGate.includes('data-cut-grade-lut-advanced'), true, 'Grade/Inspector gate opens the Advanced LUT path fallback')
}

// --- Inspector selected overlay editors stay in owned modules ----------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const inspector = readFileSync(resolve(srcRoot, 'panels/Inspector/index.tsx'), 'utf8')
  const captionEditPath = resolve(srcRoot, 'panels/Inspector/CaptionEditSection.tsx')
  const titleEditPath = resolve(srcRoot, 'panels/Inspector/TitleEditSection.tsx')
  const shapeEditPath = resolve(srcRoot, 'panels/Inspector/ShapeEditSection.tsx')
  const transformSectionPath = resolve(srcRoot, 'panels/Inspector/TransformSection.tsx')
  const croppingSectionPath = resolve(srcRoot, 'panels/Inspector/CroppingSection.tsx')
  const volumeSectionPath = resolve(srcRoot, 'panels/Inspector/VolumeSection.tsx')
  const fadesSectionPath = resolve(srcRoot, 'panels/Inspector/FadesSection.tsx')
  const inspectorModelPath = resolve(srcRoot, 'panels/Inspector/model.ts')
  const clipPropHookPath = resolve(srcRoot, 'components/inspector/useClipProp.ts')
  const captionEdit = existsSync(captionEditPath) ? readFileSync(captionEditPath, 'utf8') : ''
  const titleEdit = existsSync(titleEditPath) ? readFileSync(titleEditPath, 'utf8') : ''
  const shapeEdit = existsSync(shapeEditPath) ? readFileSync(shapeEditPath, 'utf8') : ''
  const transformSection = existsSync(transformSectionPath) ? readFileSync(transformSectionPath, 'utf8') : ''
  const croppingSection = existsSync(croppingSectionPath) ? readFileSync(croppingSectionPath, 'utf8') : ''
  const volumeSection = existsSync(volumeSectionPath) ? readFileSync(volumeSectionPath, 'utf8') : ''
  const fadesSection = existsSync(fadesSectionPath) ? readFileSync(fadesSectionPath, 'utf8') : ''
  const inspectorModel = existsSync(inspectorModelPath) ? readFileSync(inspectorModelPath, 'utf8') : ''
  const clipPropHook = existsSync(clipPropHookPath) ? readFileSync(clipPropHookPath, 'utf8') : ''

  eq(existsSync(captionEditPath), true, 'Inspector caption edit section has its own module')
  eq(existsSync(titleEditPath), true, 'Inspector title edit section has its own module')
  eq(existsSync(shapeEditPath), true, 'Inspector shape edit section has its own module')
  eq(existsSync(transformSectionPath), true, 'Inspector Transform section has its own module')
  eq(existsSync(croppingSectionPath), true, 'Inspector Cropping section has its own module')
  eq(existsSync(volumeSectionPath), true, 'Inspector Volume section has its own module')
  eq(existsSync(fadesSectionPath), true, 'Inspector Fades section has its own module')
  eq(inspector.includes("from './CaptionEditSection'"), true, 'Inspector imports the caption edit component module')
  eq(inspector.includes("from './TitleEditSection'"), true, 'Inspector imports the title edit component module')
  eq(inspector.includes("from './ShapeEditSection'"), true, 'Inspector imports the shape edit component module')
  eq(inspector.includes("from './TransformSection'"), true, 'Inspector imports the Transform component module')
  eq(inspector.includes("from './CroppingSection'"), true, 'Inspector imports the Cropping component module')
  eq(inspector.includes("from './VolumeSection'"), true, 'Inspector imports the Volume component module')
  eq(inspector.includes("from './FadesSection'"), true, 'Inspector imports the Fades component module')
  eq(inspector.includes('function CaptionEditSection'), false, 'Inspector panel no longer defines caption edit inline')
  eq(inspector.includes('function TitleEditSection'), false, 'Inspector panel no longer defines title edit inline')
  eq(inspector.includes('function ShapeEditSection'), false, 'Inspector panel no longer defines shape edit inline')
  eq(inspector.includes('function TransformSection'), false, 'Inspector panel no longer defines Transform inline')
  eq(inspector.includes('function CroppingSection'), false, 'Inspector panel no longer defines Cropping inline')
  eq(inspector.includes('function VolumeSection'), false, 'Inspector panel no longer defines Volume inline')
  eq(inspector.includes('function FadesSection'), false, 'Inspector panel no longer defines Fades inline')
  eq(captionEdit.includes('data-cut-inspector-group="caption-edit"'), true, 'Caption edit module owns the group selector')
  eq(captionEdit.includes('data-cut-caption-edit-text'), true, 'Caption edit module owns the caption textarea selector')
  eq(captionEdit.includes("callVerb('captions.set_text'"), true, 'Caption edit module dispatches captions.set_text')
  eq(titleEdit.includes('data-cut-inspector-group="title-edit"'), true, 'Title edit module owns the group selector')
  eq(titleEdit.includes('data-cut-title-edit-text'), true, 'Title edit module owns the title textarea selector')
  eq(titleEdit.includes("callVerb('title.update'"), true, 'Title edit module dispatches title.update')
  eq(shapeEdit.includes('data-cut-inspector-group="shape-edit"'), true, 'Shape edit module owns the group selector')
  eq(shapeEdit.includes('data-cut-shape-edit-label'), true, 'Shape edit module owns the shape label selector')
  eq(shapeEdit.includes("callVerb('shape.update'"), true, 'Shape edit module dispatches shape.update')
  eq(transformSection.includes('sectionKey="transform"'), true, 'Transform module owns the Transform inspector section')
  eq(transformSection.includes('useClipTransform'), true, 'Transform module composes the clip transform hook')
  eq(clipPropHook.includes("callVerb('edit.transform'"), true, 'clip prop hook dispatches edit.transform')
  eq(croppingSection.includes('sectionKey="cropping"'), true, 'Cropping module owns the Cropping inspector section')
  eq(croppingSection.includes('defaultCollapsed'), true, 'Cropping starts collapsed to keep primary Transform controls in the first viewport')
  eq(croppingSection.includes('useClipCrop'), true, 'Cropping module composes the clip crop hook')
  eq(clipPropHook.includes("callVerb('edit.crop'"), true, 'clip prop hook dispatches edit.crop')
  eq(volumeSection.includes('sectionKey="volume"'), true, 'Volume module owns the Volume inspector section')
  eq(volumeSection.includes("runUserVerb('edit.gain'"), true, 'Volume module dispatches edit.gain with visible failure feedback')
  eq(fadesSection.includes('sectionKey="fades"'), true, 'Fades module owns the Fades inspector section')
  eq(fadesSection.includes('defaultCollapsed'), true, 'Fades starts collapsed as a secondary clip adjustment')
  eq(fadesSection.includes("callVerb('edit.fade'"), true, 'Fades module dispatches edit.fade')
  eq(existsSync(inspectorModelPath), true, 'Inspector model helpers have their own module')
  eq(inspector.includes("from './model'"), true, 'Inspector imports the model helper module')
  for (const inline of [
    'const VIDEO_EFFECTS',
    'const AUDIO_EFFECTS',
    'const EQ_PRESETS',
    'const BLEND_MODES',
    'const REDACT_PRESETS',
    'const CLEANUP_STRENGTHS',
    'const CAPTION_POSITIONS',
    'const ADJ_LOOKS',
    'const COLOR_SPACES',
    'const GRADE_STACK_LAYERS',
    'const TRANSLATE_LANGS',
    'function clipEffectsOf',
    'function sourceDims',
    'function replacementCandidates',
    'function toGradeLayer',
    'function gradeSummary',
  ]) {
    eq(inspector.includes(inline), false, `Inspector panel no longer owns model detail inline: ${inline}`)
  }
  for (const expected of [
    'export const VIDEO_EFFECTS',
    'export const AUDIO_EFFECTS',
    'export const EQ_PRESETS',
    'export const BLEND_MODES',
    'export const REDACT_PRESETS',
    'export const CLEANUP_STRENGTHS',
    'export const CAPTION_POSITIONS',
    'export const ADJ_LOOKS',
    'export const COLOR_SPACES',
    'export const GRADE_STACK_LAYERS',
    'export const TRANSLATE_LANGS',
    'export function clipEffectsOf',
    'export function sourceDims',
    'export function replacementCandidates',
    'export function toGradeLayer',
    'export function gradeSummary',
  ]) {
    eq(inspectorModel.includes(expected), true, `Inspector model owns detail: ${expected}`)
  }
}

// --- Environment/Library readability: no clipped setup explanations ----------
// Model-backed services are real features (audio.dub/media.diarize), so the
// Environment panel must present them as model-runtime cards with an Agent Chat
// affordance, not as raw endpoint text. Library controls must remain visible and
// wrap in the narrow left rail instead of hiding actions behind hover or clipping
// the top controls into one line.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const dispatch = readFileSync(resolve(here, '../../app/server/src/dispatch.rs'), 'utf8')
  const translationHandlers = [
    readFileSync(resolve(here, '../../app/server/src/dispatch/captions.rs'), 'utf8'),
    readFileSync(resolve(here, '../../app/server/src/dispatch/speech_text.rs'), 'utf8'),
    readFileSync(resolve(here, '../../app/server/src/dub.rs'), 'utf8'),
  ].join('\n')
  const translate = readFileSync(resolve(here, '../../app/server/src/translate.rs'), 'utf8')

  eq(translate.includes('pub(crate) async fn run_translation'), true, 'translate module owns translation backend orchestration')
  eq(translate.includes('async fn run_translation_once'), true, 'translate module owns translation backend failover internals')
  eq(translate.includes('async fn run_translation_cli_agent'), true, 'translate module owns CLI-agent translation spawning')
  eq(translate.includes('available_cli_agents()'), true, 'translate orchestration uses the CLI agent availability ladder')
  eq(dispatch.includes('async fn run_translation('), false, 'dispatch no longer owns translation backend orchestration')
  eq(dispatch.includes('async fn run_translation_once'), false, 'dispatch no longer owns translation backend failover internals')
  eq(dispatch.includes('async fn run_translation_cli_agent'), false, 'dispatch no longer owns CLI-agent translation spawning')
  eq(translationHandlers.includes('crate::translate::run_translation('), true, 'translation handlers delegate caption/dub translation to the translate module')
}

{
  const here = dirname(fileURLToPath(import.meta.url))
  const dispatch = readFileSync(resolve(here, '../../app/server/src/dispatch.rs'), 'utf8')
  const dub = readFileSync(resolve(here, '../../app/server/src/dub.rs'), 'utf8')
  const diarize = readFileSync(resolve(here, '../../app/server/src/diarize.rs'), 'utf8')

  eq(dub.includes('pub(crate) async fn audio_dub'), true, 'dub module owns the audio.dub verb handler')
  eq(
    dispatch.includes('"audio.dub" => crate::dub::audio_dub(state, args, actor).await.into()'),
    true,
    'dispatch delegates audio.dub to the dub handler module',
  )
  eq(
    /(?:^|\n)(?:pub\(crate\)\s+)?async fn audio_dub/.test(dispatch),
    false,
    'dispatch no longer owns the audio.dub handler body',
  )
  eq(
    dub.includes('crate::translate::run_translation('),
    true,
    'dub handler keeps the CLI-first translation path inside the dub module',
  )
  eq(diarize.includes('pub(crate) async fn media_diarize'), true, 'diarize module owns the media.diarize verb handler')
  eq(
    dispatch.includes('"media.diarize" => crate::diarize::media_diarize(state, args).await.into()'),
    true,
    'dispatch delegates media.diarize to the diarize handler module',
  )
  eq(dispatch.includes('async fn media_diarize'), false, 'dispatch no longer owns the media.diarize handler body')
  eq(diarize.includes('pub(crate) fn merge_diarization'), true, 'diarize module owns perception-report speaker merge')
  eq(diarize.includes('cut_perception::apply_diarization'), true, 'diarize module applies speaker turns to words')
  eq(dispatch.includes('fn merge_diarization'), false, 'dispatch no longer owns diarization report merge logic')
  eq(diarize.includes('merge_diarization(&receipts2'), true, 'diarize handler delegates report merge inside the diarize module')
  eq(dispatch.includes('crate::diarize::merge_diarization('), false, 'dispatch no longer calls diarization report merge directly')
}

{
  const here = dirname(fileURLToPath(import.meta.url))
  const dispatch = readFileSync(resolve(here, '../../app/server/src/dispatch.rs'), 'utf8')
  const screenRecord = readFileSync(resolve(here, '../../app/server/src/screen_record.rs'), 'utf8')
  const screenRecordPolish = readFileSync(resolve(here, '../../app/server/src/screen_record/polish.rs'), 'utf8')
  const screenRecordStudio = readFileSync(resolve(here, '../../app/server/src/screen_record_studio.rs'), 'utf8')

  for (const fnName of ['screen_record_doctor', 'screen_record_start']) {
    eq(
      screenRecord.includes(`pub(crate) async fn ${fnName}`),
      true,
      `screen_record lifecycle owns the ${fnName} verb handler`,
    )
  }
  for (const fnName of ['screen_record_autoedit', 'screen_record_export']) {
    eq(
      screenRecordPolish.includes(`pub(crate) async fn ${fnName}`),
      true,
      `screen_record polish module owns the ${fnName} verb handler`,
    )
    eq(
      screenRecord.includes(fnName),
      true,
      `screen_record facade re-exports the ${fnName} verb handler`,
    )
  }
  for (const fnName of [
    'screen_record_doctor',
    'screen_record_start',
    'screen_record_autoedit',
    'screen_record_export',
  ]) {
    eq(
      new RegExp(`(?:^|\\n)(?:pub\\(crate\\)\\s+)?async fn ${fnName}`).test(dispatch),
      false,
      `dispatch no longer owns the ${fnName} handler body`,
    )
  }
  eq(
    /"screen_record\.doctor"\s*=>\s*crate::screen_record::screen_record_doctor\(args\)\s*\.await\s*\.into\(\)/s.test(dispatch),
    true,
    'dispatch delegates screen_record.doctor to the screen_record module',
  )
  eq(
    /"screen_record\.start"\s*=>\s*crate::screen_record::screen_record_start\(state,\s*args\)\s*\.await\s*\.into\(\)/s.test(dispatch),
    true,
    'dispatch delegates screen_record.start to the screen_record module',
  )
  eq(
    screenRecordStudio.includes('pub(crate) async fn screen_record_studio_event'),
    true,
    'screen_record_studio module owns the screen_record.studio_event verb handler',
  )
  eq(
    /"screen_record\.studio_event"\s*=>\s*\{\s*crate::screen_record_studio::screen_record_studio_event\(state,\s*args\)\s*\.await\s*\.into\(\)\s*\}/s.test(dispatch),
    true,
    'dispatch delegates screen_record.studio_event to the Studio metadata module',
  )
  eq(
    /(?:^|\n)(?:pub\(crate\)\s+)?async fn screen_record_studio_event/.test(dispatch),
    false,
    'dispatch no longer owns the screen_record.studio_event handler body',
  )
  eq(
    /"screen_record\.autoedit"\s*=>\s*crate::screen_record::screen_record_autoedit\(state,\s*args\)\s*\.await\s*\.into\(\)/s.test(dispatch),
    true,
    'dispatch delegates screen_record.autoedit to the screen_record module',
  )
  eq(
    /"screen_record\.export"\s*=>\s*crate::screen_record::screen_record_export\(state,\s*args\)\s*\.await\s*\.into\(\)/s.test(dispatch),
    true,
    'dispatch delegates screen_record.export to the screen_record module',
  )
  eq(screenRecord.includes('pub(crate) fn screen_record_cache_dir'), true, 'screen_record module owns the cache directory helper')
  eq(screenRecord.includes('pub(crate) fn validate_screen_record_capture_id'), true, 'screen_record module owns capture-id validation')
  eq(screenRecordStudio.includes('pub(crate) fn apply_studio_events_to_plan'), true, 'Studio module owns event-to-plan patching')
  eq(screenRecordPolish.includes('apply_studio_events_to_plan'), true, 'screen_record.autoedit applies Studio metadata to the polished plan')
  eq(screenRecordPolish.includes('pub(crate) fn plan_cache_tag'), true, 'screen_record polish module owns polish cache tagging')
  eq(screenRecord.includes('plan_cache_tag'), true, 'screen_record facade re-exports polish cache tagging')
  eq(dispatch.includes('fn screen_record_cache_dir'), false, 'dispatch no longer owns screen-record cache directory logic')
  eq(dispatch.includes('fn validate_screen_record_capture_id'), false, 'dispatch no longer owns capture-id validation')
  eq(dispatch.includes('fn plan_cache_tag'), false, 'dispatch no longer owns screen-record polish cache tagging')
}

{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const dispatch = readFileSync(resolve(here, '../../app/server/src/dispatch.rs'), 'utf8')
  const envCards = readFileSync(resolve(srcRoot, 'panels/Environment/EnvCards.tsx'), 'utf8')
  const envCardRowPath = resolve(srcRoot, 'panels/Environment/EnvCardRow.tsx')
  const envCardRow = existsSync(envCardRowPath) ? readFileSync(envCardRowPath, 'utf8') : ''
  const envServicePath = resolve(srcRoot, 'panels/Environment/ServiceRuntime.tsx')
  const envServiceRuntime = existsSync(envServicePath) ? readFileSync(envServicePath, 'utf8') : ''
  const envSttPath = resolve(srcRoot, 'panels/Environment/SttModelControl.tsx')
  const envSttControl = existsSync(envSttPath) ? readFileSync(envSttPath, 'utf8') : ''
  const envPanel = readFileSync(resolve(srcRoot, 'panels/Environment/index.tsx'), 'utf8')
  const settingsShell = readFileSync(resolve(srcRoot, 'panels/Environment/SettingsShell.tsx'), 'utf8')
  const settingsSections = readFileSync(resolve(srcRoot, 'panels/Environment/SettingsCategoryContent.tsx'), 'utf8')
  const envCss = readFileSync(resolve(srcRoot, 'panels/Environment/environment.css'), 'utf8')
  const app = readFileSync(resolve(srcRoot, 'App.tsx'), 'utf8')
  const appWorkspace = readFileSync(resolve(srcRoot, 'app/AppWorkspace.tsx'), 'utf8')
  const appRightRail = readFileSync(resolve(srcRoot, 'app/AppRightRail.tsx'), 'utf8')
  const appSurfaceEvents = readFileSync(resolve(srcRoot, 'app/useAppSurfaceEvents.ts'), 'utf8')
  const appKeyboardController = readFileSync(resolve(srcRoot, 'app/useAppKeyboardController.ts'), 'utf8')
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')
  const leftPanel = readFileSync(resolve(srcRoot, 'panels/LeftPanel/index.tsx'), 'utf8')
  const stock = readFileSync(resolve(srcRoot, 'panels/Stock/index.tsx'), 'utf8')
  const drawerCss = readFileSync(resolve(srcRoot, 'panels/drawer.css'), 'utf8')
  const assemble = readFileSync(resolve(srcRoot, 'panels/Assemble/index.tsx'), 'utf8')
  const timeline = readFileSync(resolve(srcRoot, 'panels/Timeline/index.tsx'), 'utf8')
  const timelineToolbarPath = resolve(srcRoot, 'panels/Timeline/TimelineToolbar.tsx')
  const timelineToolbar = existsSync(timelineToolbarPath) ? readFileSync(timelineToolbarPath, 'utf8') : ''
  const timelineGlobalToolsPath = resolve(srcRoot, 'panels/Timeline/TimelineGlobalTools.tsx')
  const timelineGlobalTools = existsSync(timelineGlobalToolsPath) ? readFileSync(timelineGlobalToolsPath, 'utf8') : ''
  const timelineAutomationMenuPath = resolve(srcRoot, 'panels/Timeline/TimelineAutomationMenu.tsx')
  const timelineAutomationMenu = existsSync(timelineAutomationMenuPath) ? readFileSync(timelineAutomationMenuPath, 'utf8') : ''
  const timelineSaveActionsPath = resolve(srcRoot, 'panels/Timeline/TimelineSaveActions.tsx')
  const timelineSaveActions = existsSync(timelineSaveActionsPath) ? readFileSync(timelineSaveActionsPath, 'utf8') : ''
  const timelineRangeSavesPath = resolve(srcRoot, 'panels/Timeline/useTimelineRangeSaves.ts')
  const timelineRangeSaves = existsSync(timelineRangeSavesPath) ? readFileSync(timelineRangeSavesPath, 'utf8') : ''
  const timelineAssetDropPath = resolve(srcRoot, 'panels/Timeline/useTimelineAssetDrop.ts')
  const timelineAssetDrop = existsSync(timelineAssetDropPath) ? readFileSync(timelineAssetDropPath, 'utf8') : ''
  const assetCardDragPath = resolve(srcRoot, 'lib/useAssetCardDrag.ts')
  const assetCardDrag = existsSync(assetCardDragPath) ? readFileSync(assetCardDragPath, 'utf8') : ''
  const assetsPanel = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const timelineWaveformPath = resolve(srcRoot, 'panels/Timeline/WaveformCanvas.tsx')
  const timelineWaveform = existsSync(timelineWaveformPath) ? readFileSync(timelineWaveformPath, 'utf8') : ''
  const timelineClipViewPath = resolve(srcRoot, 'panels/Timeline/ClipView.tsx')
  const timelineClipView = existsSync(timelineClipViewPath) ? readFileSync(timelineClipViewPath, 'utf8') : ''
  const motionLinkSectionPath = resolve(srcRoot, 'panels/Inspector/MotionLinkSection.tsx')
  const motionLinkSection = existsSync(motionLinkSectionPath) ? readFileSync(motionLinkSectionPath, 'utf8') : ''
  const timelineTrackControlsPath = resolve(srcRoot, 'panels/Timeline/TrackControls.tsx')
  const timelineTrackControls = existsSync(timelineTrackControlsPath) ? readFileSync(timelineTrackControlsPath, 'utf8') : ''
  const trackAuditionPath = resolve(srcRoot, 'components/TrackAuditionButton.tsx')
  const trackAudition = existsSync(trackAuditionPath) ? readFileSync(trackAuditionPath, 'utf8') : ''
  const inspectorSectionPath = resolve(srcRoot, 'components/inspector/InspectorSection.tsx')
  const inspectorSection = existsSync(inspectorSectionPath) ? readFileSync(inspectorSectionPath, 'utf8') : ''
  const propertyRowPath = resolve(srcRoot, 'components/inspector/PropertyRow.tsx')
  const propertyRow = existsSync(propertyRowPath) ? readFileSync(propertyRowPath, 'utf8') : ''
  const timelineSpeedControlPath = resolve(srcRoot, 'panels/Timeline/SpeedControl.tsx')
  const timelineSpeedControl = existsSync(timelineSpeedControlPath) ? readFileSync(timelineSpeedControlPath, 'utf8') : ''
  const timelineCrossfadePopoverPath = resolve(srcRoot, 'panels/Timeline/CrossfadePopover.tsx')
  const timelineCrossfadePopover = existsSync(timelineCrossfadePopoverPath) ? readFileSync(timelineCrossfadePopoverPath, 'utf8') : ''
  const timelineDuckStripPath = resolve(srcRoot, 'panels/Timeline/DuckStrip.tsx')
  const timelineDuckStrip = existsSync(timelineDuckStripPath) ? readFileSync(timelineDuckStripPath, 'utf8') : ''
  const timelineMarkerContextMenuPath = resolve(srcRoot, 'panels/Timeline/MarkerContextMenu.tsx')
  const timelineMarkerContextMenu = existsSync(timelineMarkerContextMenuPath) ? readFileSync(timelineMarkerContextMenuPath, 'utf8') : ''
  const timelineTrimPopoverPath = resolve(srcRoot, 'panels/Timeline/TrimPopover.tsx')
  const timelineTrimPopover = existsSync(timelineTrimPopoverPath) ? readFileSync(timelineTrimPopoverPath, 'utf8') : ''
  const timelineRulerPath = resolve(srcRoot, 'panels/Timeline/TimelineRuler.tsx')
  const timelineRuler = existsSync(timelineRulerPath) ? readFileSync(timelineRulerPath, 'utf8') : ''
  const timelineOverlaysPath = resolve(srcRoot, 'panels/Timeline/TimelineOverlays.tsx')
  const timelineOverlays = existsSync(timelineOverlaysPath) ? readFileSync(timelineOverlaysPath, 'utf8') : ''
  const timelineEmptyStatePath = resolve(srcRoot, 'panels/Timeline/TimelineEmptyState.tsx')
  const timelineEmptyState = existsSync(timelineEmptyStatePath) ? readFileSync(timelineEmptyStatePath, 'utf8') : ''
  const timelineGestureHudPath = resolve(srcRoot, 'panels/Timeline/TimelineGestureHud.tsx')
  const timelineGestureHud = existsSync(timelineGestureHudPath) ? readFileSync(timelineGestureHudPath, 'utf8') : ''
  const timelineGuidesPath = resolve(srcRoot, 'panels/Timeline/TimelineGuides.tsx')
  const timelineGuides = existsSync(timelineGuidesPath) ? readFileSync(timelineGuidesPath, 'utf8') : ''
  const timelineSeamHandlesPath = resolve(srcRoot, 'panels/Timeline/TimelineSeamHandles.tsx')
  const timelineSeamHandles = existsSync(timelineSeamHandlesPath) ? readFileSync(timelineSeamHandlesPath, 'utf8') : ''
  const timelineTrackRowPath = resolve(srcRoot, 'panels/Timeline/TimelineTrackRow.tsx')
  const timelineTrackRow = existsSync(timelineTrackRowPath) ? readFileSync(timelineTrackRowPath, 'utf8') : ''
  const timelineClipContextModelPath = resolve(srcRoot, 'panels/Timeline/ClipContextMenuModel.ts')
  const timelineClipContextModel = existsSync(timelineClipContextModelPath) ? readFileSync(timelineClipContextModelPath, 'utf8') : ''
  const timelineClipContextMenuPath = resolve(srcRoot, 'panels/Timeline/ClipContextMenu.tsx')
  const timelineClipContextMenu = existsSync(timelineClipContextMenuPath) ? readFileSync(timelineClipContextMenuPath, 'utf8') : ''
  const timelineClipActionsPath = resolve(srcRoot, 'panels/Timeline/useTimelineClipActions.ts')
  const timelineClipActions = existsSync(timelineClipActionsPath) ? readFileSync(timelineClipActionsPath, 'utf8') : ''
  const timelineRippleTrimPath = resolve(srcRoot, 'panels/Timeline/rippleTrim.ts')
  const timelineRippleTrim = existsSync(timelineRippleTrimPath) ? readFileSync(timelineRippleTrimPath, 'utf8') : ''
  const keymap = readFileSync(resolve(srcRoot, 'lib/keymap.ts'), 'utf8')
  const timelineWindowedThumbsPath = resolve(srcRoot, 'panels/Timeline/useWindowedThumbnails.ts')
  const timelineWindowedThumbs = existsSync(timelineWindowedThumbsPath) ? readFileSync(timelineWindowedThumbsPath, 'utf8') : ''
  const preview = readFileSync(resolve(srcRoot, 'panels/Preview/index.tsx'), 'utf8')
  const previewLayersPath = resolve(srcRoot, 'panels/Preview/PreviewLayers.tsx')
  const previewLayers = existsSync(previewLayersPath) ? readFileSync(previewLayersPath, 'utf8') : ''
  const previewModelPath = resolve(srcRoot, 'panels/Preview/model.ts')
  const previewModel = existsSync(previewModelPath) ? readFileSync(previewModelPath, 'utf8') : ''
  const previewContainBoxPath = resolve(srcRoot, 'panels/Preview/useContainBox.ts')
  const previewContainBox = existsSync(previewContainBoxPath) ? readFileSync(previewContainBoxPath, 'utf8') : ''
  const previewTransportPath = resolve(srcRoot, 'panels/Preview/PreviewTransport.tsx')
  const previewTransport = existsSync(previewTransportPath) ? readFileSync(previewTransportPath, 'utf8') : ''
  const previewExactReviewPath = resolve(srcRoot, 'panels/Preview/PreviewExactReview.tsx')
  const previewExactReview = existsSync(previewExactReviewPath) ? readFileSync(previewExactReviewPath, 'utf8') : ''
  const previewExportActionsPath = resolve(srcRoot, 'panels/Preview/usePreviewExportActions.ts')
  const previewExportActions = existsSync(previewExportActionsPath) ? readFileSync(previewExportActionsPath, 'utf8') : ''
  const previewMonitorBadgesPath = resolve(srcRoot, 'panels/Preview/PreviewMonitorBadges.tsx')
  const previewMonitorBadges = existsSync(previewMonitorBadgesPath) ? readFileSync(previewMonitorBadgesPath, 'utf8') : ''
  const previewEmptyStatePath = resolve(srcRoot, 'panels/Preview/PreviewEmptyState.tsx')
  const previewEmptyState = existsSync(previewEmptyStatePath) ? readFileSync(previewEmptyStatePath, 'utf8') : ''
  const libraryPanel = readFileSync(resolve(srcRoot, 'panels/Library/index.tsx'), 'utf8')
  const libraryCss = readFileSync(resolve(srcRoot, 'panels/Library/library.css'), 'utf8')
  const libraryModelPath = resolve(srcRoot, 'panels/Library/model.ts')
  const libraryModel = existsSync(libraryModelPath) ? readFileSync(libraryModelPath, 'utf8') : ''
  const libraryActionsPath = resolve(srcRoot, 'panels/Library/LibraryActions.tsx')
  const libraryActions = existsSync(libraryActionsPath) ? readFileSync(libraryActionsPath, 'utf8') : ''
  const libraryPosterPath = resolve(srcRoot, 'panels/Library/LibraryPoster.tsx')
  const libraryPoster = existsSync(libraryPosterPath) ? readFileSync(libraryPosterPath, 'utf8') : ''
  const libraryTagsPath = resolve(srcRoot, 'panels/Library/LibraryTags.tsx')
  const libraryTags = existsSync(libraryTagsPath) ? readFileSync(libraryTagsPath, 'utf8') : ''
  const libraryCardPath = resolve(srcRoot, 'panels/Library/LibraryCard.tsx')
  const libraryCard = existsSync(libraryCardPath) ? readFileSync(libraryCardPath, 'utf8') : ''
  const libraryRowPath = resolve(srcRoot, 'panels/Library/LibraryRow.tsx')
  const libraryRow = existsSync(libraryRowPath) ? readFileSync(libraryRowPath, 'utf8') : ''

  eq(
    dispatch.includes('Command::new("python3")') || dispatch.includes("Command::new('python3')"),
    false,
    'server adapters do not spawn bare python3, which opens macOS Command Line Tools prompts',
  )
  const libraryBulkBarPath = resolve(srcRoot, 'panels/Library/LibraryBulkBar.tsx')
  const libraryBulkBar = existsSync(libraryBulkBarPath) ? readFileSync(libraryBulkBarPath, 'utf8') : ''
  const libraryFoldersPath = resolve(srcRoot, 'panels/Library/LibraryFolders.tsx')
  const libraryFolders = existsSync(libraryFoldersPath) ? readFileSync(libraryFoldersPath, 'utf8') : ''
  const libraryCollectionsPath = resolve(srcRoot, 'panels/Library/LibraryCollections.tsx')
  const libraryCollections = existsSync(libraryCollectionsPath) ? readFileSync(libraryCollectionsPath, 'utf8') : ''
  const libraryFiltersPath = resolve(srcRoot, 'panels/Library/LibraryFilters.tsx')
  const libraryFilters = existsSync(libraryFiltersPath) ? readFileSync(libraryFiltersPath, 'utf8') : ''
  const libraryContextMenusPath = resolve(srcRoot, 'panels/Library/LibraryContextMenus.tsx')
  const libraryContextMenus = existsSync(libraryContextMenusPath) ? readFileSync(libraryContextMenusPath, 'utf8') : ''
  const libraryWorkspacePath = resolve(srcRoot, 'panels/Library/LibraryWorkspace.tsx')
  const libraryWorkspace = existsSync(libraryWorkspacePath) ? readFileSync(libraryWorkspacePath, 'utf8') : ''
  const libraryDetailsPath = resolve(srcRoot, 'panels/Library/LibraryDetails.tsx')
  const libraryDetails = existsSync(libraryDetailsPath) ? readFileSync(libraryDetailsPath, 'utf8') : ''
  const libraryPlacementPath = resolve(srcRoot, 'panels/Library/libraryPlacement.ts')
  const libraryPlacement = existsSync(libraryPlacementPath) ? readFileSync(libraryPlacementPath, 'utf8') : ''
  const generateCss = readFileSync(resolve(srcRoot, 'panels/GenerateTemplates/generateTemplates.css'), 'utf8')
  const generateTemplates = readFileSync(resolve(srcRoot, 'panels/GenerateTemplates/index.tsx'), 'utf8')
  const generateTemplatePanelPath = resolve(srcRoot, 'panels/GenerateTemplates/TemplatePanel.tsx')
  const generateTemplatePanel = existsSync(generateTemplatePanelPath) ? readFileSync(generateTemplatePanelPath, 'utf8') : ''
  const generatePromptPath = resolve(srcRoot, 'panels/GenerateTemplates/PromptPanel.tsx')
  const generatePrompt = existsSync(generatePromptPath) ? readFileSync(generatePromptPath, 'utf8') : ''
  const generateStoryboardPath = resolve(srcRoot, 'panels/GenerateTemplates/StoryboardPanel.tsx')
  const generateStoryboard = existsSync(generateStoryboardPath) ? readFileSync(generateStoryboardPath, 'utf8') : ''
  const layout = readFileSync(resolve(srcRoot, 'layout/useLayout.ts'), 'utf8')
  const doctorRs = readFileSync(resolve(here, '../../app/server/src/doctor.rs'), 'utf8')
  const httpRs = readFileSync(resolve(here, '../../app/server/src/http.rs'), 'utf8')
  const tauriConf = readFileSync(resolve(here, '../../app/desktop/src-tauri/tauri.conf.json'), 'utf8')
  const macInfoPlist = readFileSync(resolve(here, '../../app/desktop/src-tauri/Info.plist'), 'utf8')
  const macSystemAudio = readFileSync(resolve(here, '../../app/recorder/record-capture/src/mac_systemaudio.mm'), 'utf8')
  const desktopLib = readFileSync(resolve(here, '../../app/desktop/src-tauri/src/lib.rs'), 'utf8')
  const windowsUiux = readFileSync(resolve(here, '../../scripts/windows/cdp-cut-verify-0650-uiux.mjs'), 'utf8')
  const startHereForAgent = readFileSync(resolve(here, '../../START_HERE_FOR_AGENT.txt'), 'utf8')
  const envServicesVerifyPath = resolve(here, 'environment-services-verify.mjs')
  const envServicesVerify = existsSync(envServicesVerifyPath) ? readFileSync(envServicesVerifyPath, 'utf8') : ''

  eq(existsSync(envSttPath), true, 'Environment STT model control has its own module')
  eq(envCardRow.includes("from './SttModelControl'"), true, 'Environment card-row imports the STT model control module')
  eq(envCards.includes('data-cut-env-stt-control'), false, 'EnvCards no longer owns STT selector markup inline')
  eq(envCards.includes('Caption model:'), false, 'EnvCards no longer owns STT selector copy inline')
  eq(envSttControl.includes('data-cut-env-stt-control'), true, 'STT model control module owns the control selector')
  eq(envSttControl.includes('data-cut-env-stt-model'), true, 'STT model control module owns the model selector')
  eq(envSttControl.includes('data-cut-env-stt-advanced'), true, 'STT model control module owns model details disclosure')
  eq(envSttControl.includes('nemo-canary'), true, 'STT model control module keeps Canary visible in copy')
  eq(settingsShell.includes('data-cut-settings-categories'), true, 'Settings exposes labelled category navigation')
  eq(settingsShell.includes('data-cut-settings-search'), true, 'Settings exposes a dedicated setting search')
  eq(settingsSections.includes('Video & performance') && settingsSections.includes('AI & transcription'), true, 'Settings sections describe grouped setup in user-facing language')
  eq(`${envPanel}\n${settingsShell}\n${settingsSections}`.includes('Tools, perception, services'), false, 'Settings copy does not expose perception jargon')
  eq(`${envCards}\n${envCardRow}`.includes('Install video processing so imports, previews, and exports work.'), true, 'Environment ffmpeg hint explains what Install enables')
  eq(doctorRs.includes('click Install to download it automatically'), true, 'Doctor missing-ffmpeg hint uses human Install copy')
  eq(existsSync(envCardRowPath), true, 'Environment card row has its own component module')
  eq(envCards.includes("from './EnvCardRow'"), true, 'EnvCards imports the card-row component module')
  eq(envCards.includes('function CardRow'), false, 'EnvCards no longer owns capability row markup inline')
  eq(envCards.includes('function statusChip'), false, 'EnvCards no longer owns status-chip logic inline')
  eq(envCards.includes('function compactFact'), false, 'EnvCards no longer owns compact fact logic inline')
  eq(envCards.includes('function compactHint'), false, 'EnvCards no longer owns hint copy inline')
  eq(envCards.includes('function advancedRows'), false, 'EnvCards no longer owns advanced diagnostic rows inline')
  eq(envCards.includes('data-cut-env-card='), false, 'EnvCards no longer owns environment card selectors inline')
  eq(envCards.includes('data-cut-env-download'), false, 'EnvCards no longer owns install action selectors inline')
  eq(envCards.includes('data-cut-env-setup-perception'), false, 'EnvCards no longer owns perception setup selectors inline')
  eq(envCards.includes('data-cut-env-setup-matte'), false, 'EnvCards no longer owns matte setup selectors inline')
  eq(envCards.includes('data-cut-env-ffmpeg-control'), false, 'EnvCards no longer owns ffmpeg override controls inline')
  eq(envCards.includes('<SttModelControl'), false, 'EnvCards no longer composes STT control directly inside the group coordinator')
  eq(envCardRow.includes('function statusChip'), true, 'Environment card-row component owns status-chip logic')
  eq(envCardRow.includes('function compactFact'), true, 'Environment card-row component owns compact fact logic')
  eq(envCardRow.includes('function compactHint'), true, 'Environment card-row component owns hint copy')
  eq(envCardRow.includes('function advancedRows'), true, 'Environment card-row component owns advanced diagnostic rows')
  eq(envCardRow.includes('data-cut-env-card='), true, 'Environment card-row component owns card selectors')
  eq(envCardRow.includes('data-cut-env-download'), true, 'Environment card-row component owns install action selectors')
  eq(envCardRow.includes('data-cut-env-setup-perception'), true, 'Environment card-row component owns perception setup selectors')
  eq(envCardRow.includes('data-cut-env-setup-matte'), true, 'Environment card-row component owns matte setup selectors')
  eq(envCardRow.includes('data-cut-env-ffmpeg-control'), true, 'Environment card-row component owns ffmpeg override controls')
  eq(envCardRow.includes('<ServiceRuntimeActions'), true, 'Environment card-row component composes service runtime actions')
  eq(envCardRow.includes('<ServiceRuntimeDetail'), true, 'Environment card-row component composes service runtime details')
  eq(envCardRow.includes('<SttModelControl'), true, 'Environment card-row component composes STT model controls')
  eq(existsSync(envServicePath), true, 'Environment service runtime UI has its own module')
  eq(envCardRow.includes("from './ServiceRuntime'"), true, 'Environment card-row imports the service runtime module')
  eq(envCards.includes('function serviceInfo'), false, 'EnvCards no longer owns Dub/Diarize service metadata inline')
  eq(envCards.includes('data-cut-env-service-chat'), false, 'EnvCards no longer owns service action selectors inline')
  eq(envCards.includes('env-service-card'), false, 'EnvCards no longer owns service model-card markup inline')
  eq(existsSync(libraryModelPath), true, 'Library pure model helpers have their own module')
  eq(libraryPanel.includes("from './model'"), true, 'Library panel imports pure model helpers')
  eq(libraryPanel.includes('type TypeFilter ='), false, 'Library panel no longer owns the type filter model inline')
  eq(libraryPanel.includes('type SortKey ='), false, 'Library panel no longer owns the sort model inline')
  eq(libraryPanel.includes('type ViewMode ='), false, 'Library panel no longer owns the view model inline')
  eq(libraryPanel.includes('const SORT_KEYS'), false, 'Library panel no longer owns sort constants inline')
  eq(libraryPanel.includes('const TYPE_TABS'), false, 'Library panel no longer owns type tab constants inline')
  eq(libraryPanel.includes('function sortKeyFromInput'), false, 'Library panel no longer owns sort input parsing inline')
  eq(libraryPanel.includes('function shortDur'), false, 'Library panel no longer owns duration formatting inline')
  eq(libraryPanel.includes('function posterSrc'), false, 'Library panel no longer owns poster URL formatting inline')
  eq(libraryModel.includes('export type TypeFilter'), true, 'Library model module owns the type filter model')
  eq(libraryModel.includes('export type SortKey'), true, 'Library model module owns the sort model')
  eq(libraryModel.includes('export type ViewMode'), true, 'Library model module owns the view model')
  eq(libraryModel.includes('export const SORT_KEYS'), true, 'Library model module owns sort constants')
  eq(libraryModel.includes('export const TYPE_TABS'), true, 'Library model module owns type tab constants')
  eq(libraryModel.includes('export function sortKeyFromInput'), true, 'Library model module owns sort input parsing')
  eq(libraryModel.includes('export function shortDur'), true, 'Library model module owns duration formatting')
  eq(libraryModel.includes('export function posterSrc'), true, 'Library model module owns poster URL formatting')
  eq(existsSync(libraryActionsPath), true, 'Library item action cluster has its own component module')
  eq(`${libraryCard}\n${libraryRow}`.includes("from './LibraryActions'"), true, 'Library item surfaces import the item action component')
  eq(libraryPanel.includes('const renderActions ='), false, 'Library panel no longer owns item action markup inline')
  eq(libraryActions.includes('data-cut-library-toproject'), true, 'Library action component owns Add to project selector')
  eq(libraryActions.includes('data-cut-library-move'), true, 'Library action component owns Move selector')
  eq(libraryActions.includes('data-cut-library-tagbtn'), true, 'Library action component owns Tag selector')
  eq(libraryActions.includes('data-cut-library-portable'), true, 'Library action component owns Keep a copy selector')
  eq(libraryActions.includes('data-cut-library-remove'), true, 'Library action component owns Remove selector')
  eq(libraryActions.includes('Keep a copy'), true, 'Library action component keeps the managed-copy wording')
  eq(libraryActions.includes('+ Project'), false, 'Library item actions avoid shorthand project labels')
  eq(libraryActions.includes('Add to project'), true, 'Library item actions use the same human label as the context menu')
  eq(libraryPanel.includes("useState<ViewMode>('list')"), true, 'Library defaults to compact list view in its dedicated workspace')
  eq(libraryPanel.includes('⊞ Browse files'), false, 'Library browse action does not use a text glyph')
  eq(libraryPanel.includes('<Icon name="import" size={14}'), true, 'Library browse action uses the shared icon system')
  eq(existsSync(libraryPosterPath), true, 'Library poster renderer has its own component module')
  eq(`${libraryCard}\n${libraryRow}`.includes("from './LibraryPoster'"), true, 'Library item surfaces import the poster component')
  eq(libraryPanel.includes('function KindGlyph'), false, 'Library panel no longer owns poster fallback glyph inline')
  eq(libraryPanel.includes('const renderPoster ='), false, 'Library panel no longer owns poster rendering inline')
  eq(libraryPoster.includes('lb-thumb-img'), true, 'Library poster component owns thumbnail image rendering')
  eq(libraryPoster.includes('lb-thumb-glyph'), true, 'Library poster component owns fallback glyph rendering')
  eq(libraryPoster.includes('posterSrc(item)'), true, 'Library poster component uses the shared poster URL helper')
  eq(libraryPoster.includes('draggable={false}'), true, 'Library poster images do not start an accidental native WebView drag')
  eq(`${libraryCard}\n${libraryRow}`.includes('onMouseDown='), false, 'Library removes its unreachable sidebar-only timeline drag lane')
  eq(existsSync(libraryTagsPath), true, 'Library tag editor/filter chips have their own component module')
  eq(`${libraryCard}\n${libraryRow}`.includes("from './LibraryTags'"), true, 'Library item surfaces import the tag component')
  eq(libraryPanel.includes('const renderTags ='), false, 'Library panel no longer owns tag editor markup inline')
  eq(libraryPanel.includes('data-cut-library-taginput'), false, 'Library panel no longer owns tag input selector inline')
  eq(libraryPanel.includes('data-cut-library-tags'), false, 'Library panel no longer owns tag-list selector inline')
  eq(libraryPanel.includes('data-cut-library-tag='), false, 'Library panel no longer owns tag chip selector inline')
  eq(libraryTags.includes('data-cut-library-taginput'), true, 'Library tag component owns tag input selector')
  eq(libraryTags.includes('data-cut-library-tags'), true, 'Library tag component owns tag-list selector')
  eq(libraryTags.includes('data-cut-library-tag'), true, 'Library tag component owns tag chip selector')
  eq(existsSync(libraryCardPath), true, 'Library grid card has its own component module')
  eq(libraryPanel.includes("from './LibraryCard'"), true, 'Library panel imports the grid card component')
  eq(libraryPanel.includes('const renderCard ='), false, 'Library panel no longer owns grid card markup inline')
  eq(libraryCard.includes('data-cut-library-card'), true, 'Library card component owns card selector')
  eq(libraryCard.includes('data-cut-library-select'), true, 'Library card component owns grid select selector')
  eq(libraryCard.includes('{selected ? <Icon name="check"'), true, 'Library grid select shows a check only after selection')
  eq(libraryCard.includes('data-cut-library-fav'), true, 'Library card component owns grid favorite selector')
  eq(libraryCard.includes('<LibraryPoster'), true, 'Library card component composes the poster component')
  eq(libraryCard.includes('<LibraryTags'), true, 'Library card component composes the tag component')
  eq(libraryCard.includes('<LibraryActions'), true, 'Library card component composes the action component')
  eq(existsSync(libraryRowPath), true, 'Library list row has its own component module')
  eq(libraryPanel.includes("from './LibraryRow'"), true, 'Library panel imports the list row component')
  eq(libraryPanel.includes('const renderRow ='), false, 'Library panel no longer owns list row markup inline')
  eq(libraryRow.includes('data-cut-library-card'), true, 'Library row component owns row card selector')
  eq(libraryRow.includes('data-cut-library-select'), true, 'Library row component owns row select selector')
  eq(libraryRow.includes('{selected ? <Icon name="check"'), true, 'Library row select shows a check only after selection')
  eq(libraryRow.includes('data-cut-library-fav'), true, 'Library row component owns row favorite selector')
  eq(libraryRow.includes('<LibraryPoster'), true, 'Library row component composes the poster component')
  eq(libraryRow.includes('<LibraryTags'), true, 'Library row component composes the tag component')
  eq(libraryRow.includes('<LibraryActions'), true, 'Library row component composes the action component')
  eq(existsSync(libraryBulkBarPath), true, 'Library bulk action bar has its own component module')
  eq(libraryPanel.includes("from './LibraryBulkBar'"), true, 'Library panel imports the bulk action bar component')
  eq(libraryPanel.includes('data-cut-library-bulkbar'), false, 'Library panel no longer owns bulk action bar selector inline')
  eq(libraryBulkBar.includes('data-cut-library-bulkbar'), true, 'Library bulk action bar component owns bulkbar selector')
  eq(libraryBulkBar.includes('data-cut-library-bulk-tag'), true, 'Library bulk action bar component owns bulk tag selector')
  eq(libraryBulkBar.includes('data-cut-library-bulk-taginput'), true, 'Library bulk action bar component owns bulk tag input selector')
  eq(libraryBulkBar.includes('data-cut-library-bulk-move'), true, 'Library bulk action bar component owns bulk move selector')
  eq(libraryBulkBar.includes('data-cut-library-bulk-toproject'), true, 'Library bulk action bar component owns bulk project selector')
  eq(libraryBulkBar.includes('data-cut-library-bulk-remove'), true, 'Library bulk action bar component owns bulk remove selector')
  eq(libraryBulkBar.includes('data-cut-library-bulk-clear'), true, 'Library bulk action bar component owns bulk clear selector')
  eq(libraryBulkBar.includes('+ Project'), false, 'Library bulk actions avoid shorthand project labels')
  eq(libraryBulkBar.includes('Add to project'), true, 'Library bulk actions match item action wording')
  eq(existsSync(libraryFoldersPath), true, 'Library folder strip has its own component module')
  eq(libraryPanel.includes("from './LibraryFolders'"), true, 'Library panel imports the folder strip component')
  eq(libraryPanel.includes('data-cut-library-folders'), false, 'Library panel no longer owns folder strip selector inline')
  eq(libraryFolders.includes('data-cut-library-folders'), true, 'Library folder strip component owns folder strip selector')
  eq(libraryFolders.includes('data-cut-library-folder='), true, 'Library folder strip component owns folder chip selectors')
  eq(libraryFolders.includes('data-cut-library-folder-rename'), true, 'Library folder strip component owns folder rename input selector')
  eq(libraryFolders.includes('data-cut-library-folder-rename-btn'), true, 'Library folder strip component owns visible rename button selector')
  eq(libraryFolders.includes('data-cut-library-newfolder'), true, 'Library folder strip component owns new-folder selector')
  eq(existsSync(libraryCollectionsPath), true, 'Library collection rail has its own component module')
  eq(libraryPanel.includes("from './LibraryCollections'"), true, 'Library workspace imports the collection rail')
  eq(libraryCollections.includes('data-cut-library-collection='), true, 'Library collection rail exposes stable action selectors')
  for (const collection of ['all', 'recent', 'favorites', 'missing']) {
    eq(libraryCollections.includes(`id: '${collection}'`), true, `Library collection rail exposes ${collection}`)
  }
  eq(libraryCollections.includes('data-cut-library-collection-tag='), true, 'Library collection rail exposes navigable tags')
  eq(existsSync(libraryFiltersPath), true, 'Library filter toolbar has its own component module')
  eq(libraryPanel.includes("from './LibraryFilters'"), true, 'Library panel imports the filter toolbar component')
  eq(libraryPanel.includes('data-cut-library-tabs'), false, 'Library panel no longer owns type-tab selector inline')
  eq(libraryPanel.includes('data-cut-library-sort'), false, 'Library panel no longer owns sort selector inline')
  eq(libraryPanel.includes('data-cut-library-search'), false, 'Library panel no longer owns search selector inline')
  eq(libraryPanel.includes('data-cut-library-tagfilter'), false, 'Library panel no longer owns active tag-filter selector inline')
  eq(libraryFilters.includes('data-cut-library-tabs'), true, 'Library filter toolbar component owns type-tab selector')
  eq(libraryFilters.includes('data-cut-library-tab='), true, 'Library filter toolbar component owns type-tab item selectors')
  eq(libraryFilters.includes('data-cut-library-sort'), true, 'Library filter toolbar component owns sort selector')
  eq(libraryFilters.includes('data-cut-library-search'), true, 'Library filter toolbar component owns search selector')
  eq(libraryFilters.includes('data-cut-library-tagfilter'), true, 'Library filter toolbar component owns active tag-filter selector')
  eq(existsSync(libraryContextMenusPath), true, 'Library context menus have their own component module')
  eq(libraryPanel.includes("from './LibraryContextMenus'"), true, 'Library panel imports the context menu component')
  eq(libraryPanel.includes('data-cut-library-folder-menu'), false, 'Library panel no longer owns folder context menu markup inline')
  eq(libraryPanel.includes('data-cut-library-card-menu'), false, 'Library panel no longer owns card context menu markup inline')
  eq(libraryContextMenus.includes('data-cut-library-folder-menu'), true, 'Library context menu component owns folder menu selector')
  eq(libraryContextMenus.includes('data-cut-library-card-menu'), true, 'Library context menu component owns card menu selector')
  eq(libraryContextMenus.includes('data-cut-library-folder-ctx'), true, 'Library context menu component owns folder menu actions')
  eq(libraryContextMenus.includes('data-cut-library-card-ctx'), true, 'Library context menu component owns card menu actions')
  eq(libraryContextMenus.includes('Add to project'), true, 'Library context menu component keeps action wording aligned')
  eq(envServiceRuntime.includes('data-cut-env-service-chat'), true, 'Service runtime module owns Agent Chat action selectors')
  eq(envServiceRuntime.includes('data-cut-env-service-connect'), true, 'Service runtime module owns Connect action selectors')
  eq(envServiceRuntime.includes('data-cut-env-service-rescan'), true, 'Service runtime module owns Re-scan action selectors')
  eq(envServiceRuntime.includes('data-cut-env-service-setup'), true, 'Service runtime module owns setup disclosure selectors')
  eq(envServiceRuntime.includes('setupPrompt'), true, 'Service runtime cards expose an Agent Chat setup prompt for disconnected services')
  eq(envServiceRuntime.includes('Help me connect OmniVoice TTS'), true, 'Dub card has a non-specialist Agent Chat connection prompt')
  eq(envServiceRuntime.includes('Help me connect Sortformer v2'), true, 'Diarize card has a non-specialist Agent Chat connection prompt')
  eq(envServiceRuntime.includes('OmniVoice TTS'), true, 'Service runtime module owns Dub runtime copy')
  eq(envServiceRuntime.includes('Sortformer v2'), true, 'Service runtime module owns Diarize runtime copy')
  eq(envServiceRuntime.includes('data-cut-env-service-chat'), true, 'Environment service cards offer Agent Chat actions')
  eq(envServiceRuntime.includes('data-cut-env-service-setup'), true, 'Environment service cards offer a setup/help action, not only prose')
  eq(envServiceRuntime.includes('data-cut-env-service-primary'), true, 'Environment service cards expose a compact primary action in the card action column')
  eq(envServiceRuntime.includes('data-cut-env-service-connect'), true, 'Environment service cards expose a Connect action instead of only explanatory text')
  eq(envServiceRuntime.includes('data-cut-env-service-rescan'), true, 'Environment service cards keep re-scan as a secondary service action')
  eq(envServiceRuntime.includes('data-cut-env-service-requirement'), true, 'Environment service cards expose the external-runtime requirement in the compact card')
  eq(envServiceRuntime.includes('External runtime required'), true, 'Environment service cards make clear the heavy model runtime is external')
  eq(envServiceRuntime.includes('Connector included'), true, 'Environment service cards make clear the app-side connector is already bundled')
  eq(envServiceRuntime.includes('Connect service'), true, 'Environment service setup action names the thing being connected')
  eq(envServiceRuntime.includes('Ask Agent for help'), true, 'Environment advanced service help names the Agent handoff clearly')
  eq(
    envCardRow.indexOf('<ServiceRuntimeDetail') >= 0
      && envCardRow.indexOf('<AdvancedDetails card={card}') >= 0
      && envCardRow.indexOf('<ServiceRuntimeDetail') < envCardRow.indexOf('<AdvancedDetails card={card}'),
    true,
    'Environment service setup block renders before generic Advanced details so connection steps are not buried behind diagnostics',
  )
  eq(envServiceRuntime.includes('onOpenSetup(card.id)'), true, 'Environment service Connect action opens the setup details')
  eq(envCardRow.includes('serviceRuntimeRequirement(card)'), false, 'Environment service cards do not repeat connection status in the fact column')
  for (const status of ['Ready', 'Needs attention', 'Needs setup', 'Optional', 'Check again']) {
    eq(envCardRow.includes(`label: '${status}'`), true, `Environment uses the canonical ${status} status`)
  }
  for (const staleStatus of ["label: 'OK'", "label: 'DEGRADED'", "label: 'MISSING'", 'CAN\'T VERIFY']) {
    eq(envCardRow.includes(staleStatus), false, `Environment removes stale status token ${staleStatus}`)
  }
  eq(`${envCards}\n${envServiceRuntime}`.includes('Model runtime not connected'), false, 'Environment service fact avoids long repeated runtime wording')
  eq(windowsUiux.includes('data-cut-env-service-connect'), true, 'Windows UIUX harness checks service Connect actions')
  eq(windowsUiux.includes('data-cut-env-service-rescan'), true, 'Windows UIUX harness checks service Re-scan actions')
  eq(windowsUiux.includes('environment-dub-connect-opens-setup'), true, 'Windows UIUX harness verifies Dub Connect opens setup steps')
  eq(windowsUiux.includes('environment-dub-chat-prefills-setup'), true, 'Windows UIUX harness verifies Dub service Chat handoff')
  eq(windowsUiux.includes('environment-diarize-chat-prefills-setup'), true, 'Windows UIUX harness verifies Diarize service Chat handoff')
  eq(windowsUiux.includes('environment-service-copy-no-gpu-host'), true, 'Windows UIUX harness rejects stale GPU-host service copy in installed builds')
  eq(existsSync(envServicesVerifyPath), true, 'Lightweight Environment service runtime verifier exists for unsigned batch checks')
  for (const expected of [
    '[data-cut-env-card="dub"]',
    '[data-cut-env-card="diarize"]',
    '[data-cut-env-service-connect="dub"]',
    '[data-cut-env-service-chat="dub"]',
    '[data-cut-env-service-connect="diarize"]',
    '[data-cut-env-service-chat="diarize"]',
    'data-cut-env-service-requirement',
    'environment-service-no-major-overflow',
  ]) {
    eq(envServicesVerify.includes(expected), true, `Environment service runtime verifier covers ${expected}`)
  }
  eq(envServiceRuntime.includes('Connection steps'), true, 'Environment service cards use compact connection-step disclosure wording')
  eq(envServiceRuntime.includes('env-service-card'), true, 'Environment service cards use compact model-runtime card layout')
  eq(envServiceRuntime.includes('data-cut-env-service-powered-by'), true, 'Environment service cards name the verb powered by the runtime')
  eq(envServiceRuntime.includes('data-cut-env-service-connector'), true, 'Environment service cards separate packaged connector state from service state')
  eq(envServiceRuntime.includes('data-cut-env-service-runtime'), true, 'Environment service cards show external service reachability as a compact status')
  eq(envServiceRuntime.includes('External service'), true, 'Environment service cards use user-facing runtime wording instead of raw endpoint-first copy')
  eq(envServiceRuntime.includes('OmniVoice TTS'), true, 'Dub card names the OmniVoice model runtime')
  eq(envServiceRuntime.includes('Sortformer v2'), true, 'Diarize card names the Sortformer model runtime')
  eq(cssBlock(envCss, '.env-service-card').includes('grid-template-columns'), true, 'Environment service cards use a stable grid, not a cramped prose row')
  eq(cssBlock(envCss, '.env-service-metrics').includes('flex-wrap: wrap'), true, 'Environment service card metrics wrap instead of clipping')
  eq(cssBlock(envCss, '.env-row-action').includes('flex-wrap: wrap'), true, 'Environment row actions wrap instead of widening the Settings drawer')
  eq(cssBlock(envCss, '.env-row-action').includes('min-width: 0'), true, 'Environment row action column can shrink inside the drawer grid')
  eq(cssBlock(envCss, '.env-btn--sm').includes('white-space: normal'), true, 'Environment compact action labels can wrap instead of clipping')
  eq(envCss.includes('@media (max-width: 640px)') && envCss.includes('grid-template-columns: auto minmax(0, 1fr);'), true, 'Environment rows collapse to a stable two-column layout in the narrow Settings drawer')
  eq(doctorRs.includes('"runner_available"'), true, 'Doctor service cards report runner wiring')
  eq(doctorRs.includes('"model"'), true, 'Doctor service cards report the model/runtime name')
  eq(doctorRs.includes('configured_sidecar_python()'), true, 'Doctor passive Environment scan uses configured/app-managed Python only')
  eq(doctorRs.includes('fn perception_card() -> Card {\n    let (python, _script) = cut_perception::sidecar_paths();'), false, 'Perception card does not probe bare python3/python on launch')
  eq(doctorRs.includes('fn matte_card() -> Card {\n    let (perc_py, _script) = cut_perception::sidecar_paths();'), false, 'Matte card does not probe bare python3/python on launch')
  eq(tauriConf.includes('"../../../skill/shellx-cut": "agent-docs/skill/shellx-cut"'), true, 'Desktop bundle ships the ShellX Cut agent skill')
  eq(tauriConf.includes('"../../../schema/verbs.json": "agent-docs/schema/verbs.json"'), true, 'Desktop bundle ships the verb schema for fresh-machine agents')
  eq(tauriConf.includes('"../../../docs/public/FEATURE_CHANGE_WORKFLOW.md": "agent-docs/docs/public/FEATURE_CHANGE_WORKFLOW.md"'), true, 'Desktop bundle ships the feature/debug surface workflow doc')
  eq(tauriConf.includes('"../../../docs/public/SHELLX_MOTION_BOUNDARY.md": "agent-docs/docs/public/SHELLX_MOTION_BOUNDARY.md"'), true, 'Desktop bundle ships the ShellX Motion boundary doc')
  eq(tauriConf.includes('"../../../START_HERE_FOR_AGENT.txt": "agent-docs/START_HERE_FOR_AGENT.txt"'), true, 'Desktop bundle ships the fresh-machine agent start-here note')
  eq(tauriConf.includes('"../../../AGENTS.md": "agent-docs/AGENTS.md"'), true, 'Desktop bundle ships repo agent rules')
  eq(tauriConf.includes('"../../../README.md": "agent-docs/README.md"'), true, 'Desktop bundle ships public product docs')
  eq(tauriConf.includes('"../../../docs/public/FEATURES.md": "agent-docs/docs/public/FEATURES.md"'), true, 'Desktop bundle ships the public-safe feature inventory for fresh-machine agents')
  eq(tauriConf.includes('"../../../docs/public/DEBUG_API.md": "agent-docs/docs/public/DEBUG_API.md"'), true, 'Desktop bundle ships the Debug API operator reference')
  eq(startHereForAgent.includes('docs/public/FEATURES.md'), true, 'Start-here note points fresh-machine agents at the public-safe feature inventory')
  eq(desktopLib.includes('SHELLX_CUT_AGENT_DOCS_DIR'), true, 'Desktop shell tells cutd where bundled agent docs live')
  eq(desktopLib.includes('detect_with_resources(&exe_dir, &resource_dir)'), true, 'Desktop tool detection checks the packaged Resources directory on macOS')
  eq(httpRs.includes('.route("/agent", get(get_agent_info))'), true, 'cutd exposes /api/agent as a fresh-install agent discovery endpoint')
  eq(httpRs.includes('.route("/agent-doc/*path", get(serve_agent_doc))'), true, 'cutd exposes packaged skill/reference docs over loopback')
  eq(httpRs.includes('START_HERE_FOR_AGENT.txt'), true, '/api/agent points fresh agents at the start-here note')
  eq(httpRs.includes('docs/public/FEATURES.md'), true, '/api/agent exposes the public-safe feature inventory through the bundled-doc allowlist')
  eq(httpRs.includes('{"id": "debug-api", "path": "docs/public/DEBUG_API.md"'), true, '/api/agent advertises the bundled Debug API operator reference')
  eq(httpRs.includes('docs/public/SHELLX_MOTION_BOUNDARY.md'), true, '/api/agent exposes the Motion boundary doc through the bundled-doc allowlist')
  eq(httpRs.includes('skill/shellx-cut/SKILL.md'), true, '/api/agent points agents at the ShellX Cut skill')
  eq(httpRs.includes('skill/shellx-cut/craft/INDEX.md'), true, '/api/agent points agents at the complete craft guide index')
  eq(httpRs.includes('schema/verbs.json'), true, '/api/agent points agents at the live verb schema docs')
  eq(macInfoPlist.includes('<key>NSAudioCaptureUsageDescription</key>'), true, 'macOS bundle declares Core Audio process-tap usage text')
  eq(macSystemAudio.includes('kAudioTapPropertyUID'), true, 'macOS system audio resolves the HAL tap UID')
  eq(macSystemAudio.includes('kAudioAggregateDevicePropertyTapList'), true, 'macOS system audio attaches the HAL tap through the aggregate tap-list property')
  eq(macSystemAudio.includes('kAudioAggregateDeviceTapListKey'), false, 'macOS system audio does not attach a description UUID through aggregate composition')
  eq(macInfoPlist.includes('<key>LSRequiresCarbon</key>') && macInfoPlist.includes('<false/>'), true, 'macOS bundle overrides stale LSRequiresCarbon=true')
  eq(/tunnel|GPU host|remote GPU box/i.test(envCards), false, 'Environment service UI copy avoids tunnel/GPU-host setup language')
  eq(/tunnel|GPU host|remote GPU box/i.test(doctorRs), false, 'Doctor service hints avoid tunnel/GPU-host setup language')
  eq(cssBlock(envCss, '.env-row-title').includes('white-space: nowrap'), false, 'Environment titles can wrap instead of clipping')
  eq(cssBlock(envCss, '.env-row-title').includes('overflow-wrap'), true, 'Environment titles wrap long labels')
  eq(cssBlock(envCss, '.env-row-role').includes('white-space: nowrap'), false, 'Environment role text can wrap')
  eq(cssBlock(envCss, '.env-fact').includes('text-overflow'), false, 'Environment fact chips do not ellipsize setup text')
  eq(cssBlock(envCss, '.env-fact').includes('white-space: nowrap'), false, 'Environment fact chips can wrap long install/runtime text')
  eq(cssBlock(envCss, '.env-fact').includes('overflow-wrap'), true, 'Environment fact chips break long version tokens safely')
  eq(cssBlock(envCss, '.env-ff-path').includes('text-overflow'), false, 'Environment configured paths and model ids do not ellipsize')
  eq(cssBlock(envCss, '.env-ff-path').includes('white-space: nowrap'), false, 'Environment configured paths and model ids can wrap')
  eq(cssBlock(envCss, '.env-ff-path').includes('overflow-wrap'), true, 'Environment configured paths and model ids break long tokens safely')
  eq(cssBlock(envCss, '.env-ff-note').includes('line-height'), true, 'Environment explanatory notes have readable line height')
  eq(cssBlock(libraryCss, '.lb-actions').includes('opacity: 0'), false, 'Library item actions are not hover-only')
  eq(cssBlock(libraryCss, '.lb-chip-edit').includes('opacity: 0'), false, 'Library folder rename affordance is not hover-only')
  eq(cssBlock(libraryCss, '.lb-select').includes('opacity: 0'), false, 'Library grid select affordance is not hover-only')
  eq(cssBlock(libraryCss, '.lb-card').includes('border-radius: 10px'), false, 'Library grid cards use the shared compact radius scale')
  eq(libraryCss.includes('.lb-add {\n  display: flex;\n  flex-wrap: wrap;'), true, 'Library add controls retain a wrapping compact fallback')
  eq(libraryCss.includes('.lb-toolbar { display: flex; flex-wrap: wrap;'), true, 'Library filter controls retain a wrapping compact fallback')
  eq(cssBlock(libraryCss, '.lb-name').includes('white-space: normal'), true, 'Library card names wrap instead of clipping')
  eq(cssBlock(libraryCss, '.lb-name').includes('overflow-wrap: anywhere'), true, 'Library card names can break long filenames')
  eq(cssBlock(libraryCss, '.lb-row-name').includes('white-space: normal'), true, 'Library list names wrap instead of clipping')
  eq(existsSync(previewLayersPath), true, 'Preview overlay/caption layers have their own component module')
  eq(preview.includes("from './PreviewLayers'"), true, 'Preview imports the overlay/caption layers module')
  eq(preview.includes('function OverlayVideo'), false, 'Preview panel no longer defines overlay video rendering inline')
  eq(preview.includes('function CaptionText'), false, 'Preview panel no longer defines caption text rendering inline')
  eq(preview.includes('data-cut-overlay='), false, 'Preview panel no longer owns overlay video selectors inline')
  eq(preview.includes('data-cut-caption='), false, 'Preview panel no longer owns caption selectors inline')
  eq(preview.includes('events.subscribe'), false, 'Preview panel does not use raw subscribe naming for event-bus listeners')
  eq(preview.includes('events.onEvent'), true, 'Preview panel uses the semantic event-bus listener alias')
  eq(previewLayers.includes('function OverlayVideo'), true, 'Preview layers module owns overlay video rendering')
  eq(previewLayers.includes('function CaptionText'), true, 'Preview layers module owns caption text rendering')
  eq(previewLayers.includes('data-cut-overlay='), true, 'Preview layers module owns overlay track selector')
  eq(previewLayers.includes('data-cut-overlay-clip'), true, 'Preview layers module owns overlay clip selector')
  eq(previewLayers.includes('data-cut-caption='), true, 'Preview layers module owns caption selector')
  eq(previewLayers.includes('gradeFilter(layer.grade)'), true, 'Preview layers module preserves overlay grade filtering')
  eq(previewLayers.includes('overlayBoxStyle'), true, 'Preview layers module preserves overlay transform styling')
  eq(existsSync(previewModelPath), true, 'Preview playback model helpers have their own module')
  eq(existsSync(previewContainBoxPath), true, 'Preview contain-box hook has its own module')
  eq(preview.includes("from './model'"), true, 'Preview panel imports playback model helpers')
  eq(preview.includes("from './useContainBox'"), true, 'Preview panel imports the contain-box hook')
  eq(preview.includes('function activeVideo'), false, 'Preview panel no longer owns active-video resolution inline')
  eq(preview.includes('function videoFrameCallbacks'), false, 'Preview panel no longer owns video-frame callback adapter inline')
  eq(preview.includes('function useContainBox'), false, 'Preview panel no longer owns contain-box hook inline')
  eq(previewModel.includes('export function activeVideo'), true, 'Preview model owns active-video resolution')
  eq(previewModel.includes('export function videoFrameCallbacks'), true, 'Preview model owns video-frame callback adapter')
  eq(previewModel.includes('export const RVFC_SUPPORTED'), true, 'Preview model owns requestVideoFrameCallback feature detection')
  eq(previewModel.includes('export type ActiveVideo'), true, 'Preview model owns active-video type')
  eq(previewContainBox.includes('export function useContainBox'), true, 'Preview contain-box module owns the hook')
  eq(existsSync(previewTransportPath), true, 'Preview transport bar has its own component module')
  eq(preview.includes("from './PreviewTransport'"), true, 'Preview panel imports the transport component')
  eq(preview.includes('data-cut-transport'), false, 'Preview panel no longer owns transport markup inline')
  eq(preview.includes('data-cut-action="snapshot-frame"'), false, 'Preview panel no longer owns snapshot-frame selector inline')
  eq(preview.includes('data-cut-action="render-section"'), false, 'Preview panel no longer owns render-section selector inline')
  eq(preview.includes('data-cut-audio-toggle'), false, 'Preview panel no longer owns audio-toggle selector inline')
  eq(preview.includes('data-cut-quality-toggle'), false, 'Preview panel no longer owns composed-toggle selector inline')
  eq(previewTransport.includes('export default function PreviewTransport'), true, 'Preview transport module exports the component')
  eq(previewTransport.includes('data-cut-transport'), true, 'Preview transport component owns the transport selector')
  eq(previewTransport.includes('data-cut-action="snapshot-frame"'), true, 'Preview transport component owns the snapshot-frame selector')
  eq(previewTransport.includes('data-cut-action="render-section"'), true, 'Preview transport component owns the render-section selector')
  eq(previewTransport.includes('data-cut-audio-toggle'), true, 'Preview transport component owns the audio-toggle selector')
  eq(previewTransport.includes('data-cut-quality-toggle'), true, 'Preview transport component owns the composed-toggle selector')
  eq(previewTransport.includes('<MasterMeter'), true, 'Preview transport component owns master meter placement')
  eq(existsSync(previewExactReviewPath), true, 'Preview exact-review overlay has its own component module')
  eq(preview.includes("from './PreviewExactReview'"), true, 'Preview panel imports the exact-review component')
  eq(preview.includes('data-cut-exact'), false, 'Preview panel no longer owns exact-review overlay selectors inline')
  eq(preview.includes('data-cut-action="save-section"'), false, 'Preview panel no longer owns save-section selector inline')
  eq(preview.includes('data-cut-action="exit-exact"'), false, 'Preview panel no longer owns exit-exact selector inline')
  eq(previewExactReview.includes('export default function PreviewExactReview'), true, 'Preview exact-review module exports the component')
  eq(previewExactReview.includes('data-cut-exact'), true, 'Preview exact-review component owns overlay selector')
  eq(previewExactReview.includes('data-cut-exact-video'), true, 'Preview exact-review component owns exact video selector')
  eq(previewExactReview.includes('data-cut-action="save-section"'), true, 'Preview exact-review component owns save-section selector')
  eq(previewExactReview.includes('data-cut-action="exit-exact"'), true, 'Preview exact-review component owns exit-exact selector')
  eq(previewExactReview.includes('timecode(exact.rangeMs[0])'), true, 'Preview exact-review component preserves range timecode display')
  eq(existsSync(previewExportActionsPath), true, 'Preview export actions have their own hook module')
  eq(preview.includes("from './usePreviewExportActions'"), true, 'Preview panel imports the export actions hook')
  eq(preview.includes('const snapFrame = useCallback'), false, 'Preview panel no longer owns snapshot action inline')
  eq(preview.includes('const sectionRange = useCallback'), false, 'Preview panel no longer owns rendered-section range calculation inline')
  eq(preview.includes('const renderSection = useCallback'), false, 'Preview panel no longer owns rendered-section export action inline')
  eq(preview.includes('const saveSection = useCallback'), false, 'Preview panel no longer owns save-rendered-section action inline')
  eq(previewExportActions.includes('export function usePreviewExportActions'), true, 'Preview export actions hook exports the action bundle')
  eq(previewExportActions.includes("callVerb('export.frame'"), true, 'Preview export actions hook dispatches frame snapshots')
  eq(previewExportActions.includes("callVerb('export.range'"), true, 'Preview export actions hook dispatches rendered-section exports')
  eq(previewExportActions.includes("callVerb('media.import'"), true, 'Preview export actions hook dispatches Save to Assets imports')
  eq(previewExportActions.includes('layoutTrack('), true, 'Preview export actions hook preserves selected-clip range calculation')
  eq(existsSync(previewMonitorBadgesPath), true, 'Preview monitor badges have their own component module')
  eq(preview.includes("from './PreviewMonitorBadges'"), true, 'Preview panel imports the monitor-badges component')
  eq(preview.includes('data-cut-source-chip'), false, 'Preview panel no longer owns source-chip selector inline')
  eq(preview.includes('data-cut-overlay-cap'), false, 'Preview panel no longer owns overlay-cap selector inline')
  eq(preview.includes('data-cut-spinner'), false, 'Preview panel no longer owns spinner selector inline')
  eq(preview.includes('data-cut-proxy-chip'), false, 'Preview panel no longer owns proxy-chip selector inline')
  eq(previewMonitorBadges.includes('export default function PreviewMonitorBadges'), true, 'Preview monitor-badges module exports the component')
  eq(previewMonitorBadges.includes('data-cut-source-chip'), true, 'Preview monitor-badges component owns source-chip selector')
  eq(previewMonitorBadges.includes('data-cut-overlay-cap'), true, 'Preview monitor-badges component owns overlay-cap selector')
  eq(previewMonitorBadges.includes('data-cut-spinner'), true, 'Preview monitor-badges component owns spinner selector')
  eq(previewMonitorBadges.includes('data-cut-proxy-chip'), true, 'Preview monitor-badges component owns proxy-chip selector')
  eq(previewMonitorBadges.includes('PROXY RENDERING'), true, 'Preview monitor-badges component preserves proxy-build copy')
  eq(existsSync(previewEmptyStatePath), true, 'Preview empty/import state has its own component module')
  eq(preview.includes("from './PreviewEmptyState'"), true, 'Preview panel imports the empty-state component')
  eq(preview.includes('data-cut-import-cta'), false, 'Preview panel no longer owns import CTA selector inline')
  eq(preview.includes('Import media to begin'), false, 'Preview panel no longer owns import CTA copy inline')
  eq(previewEmptyState.includes('export default function PreviewEmptyState'), true, 'Preview empty-state module exports the component')
  eq(previewEmptyState.includes('data-cut-import-cta'), true, 'Preview empty-state component owns import CTA selector')
  eq(previewEmptyState.includes('Import media to begin'), true, 'Preview empty-state component preserves import CTA copy')
  eq(previewEmptyState.includes('Create a project in Projects to begin'), true, 'Preview empty-state component preserves no-project copy')
  eq(assemble.includes("{ id: 'shorts', label: 'Short ranges'"), true, 'Assemble shorts mode names the actual range-finding result')
  eq(assemble.includes("label: 'Auto-shorts'"), false, 'Assemble shorts mode no longer overpromises auto-generated shorts')
  eq(assemble.includes('Find short-worthy ranges'), true, 'Assemble shorts primary action describes range finding')
  eq(assemble.includes('Materialize a short from its range'), true, 'Assemble shorts copy explains the materialize step')
  eq(existsSync(timelineToolbarPath), true, 'Timeline toolbar has its own component module')
  eq(existsSync(timelineGlobalToolsPath), true, 'Timeline global tools have their own component module')
  eq(existsSync(timelineAutomationMenuPath), true, 'Timeline automation menu has its own component module')
  eq(timeline.includes("from './TimelineToolbar'"), true, 'Timeline imports the toolbar component module')
  eq(timeline.includes('data-cut-timeline-toolbar'), false, 'Timeline panel no longer owns toolbar selector markup inline')
  eq(timeline.includes('data-cut-tc-readout'), false, 'Timeline panel no longer owns timecode readout markup inline')
  eq(timeline.includes('data-cut-tool="razor"'), false, 'Timeline panel no longer owns razor tool markup inline')
  eq(timeline.includes('data-cut-action="sync-by-audio"'), false, 'Timeline panel no longer owns sync action markup inline')
  eq(timeline.includes('data-cut-action="open-matte"'), false, 'Timeline panel no longer owns contextual matte action markup inline')
  eq(timelineToolbar.includes('data-cut-timeline-toolbar'), true, 'Timeline toolbar component owns the toolbar selector')
  eq(timelineToolbar.includes("from './TimelineAutomationMenu'"), true, 'Timeline toolbar imports the automation menu component')
  eq(timelineToolbar.includes('<TimelineAutomationMenu'), true, 'Timeline toolbar renders one automation menu beside direct edit tools')
  eq(timelineAutomationMenu.includes("from './TimelineGlobalTools'"), true, 'Timeline automation menu owns the global tools cluster')
  eq(timelineAutomationMenu.includes('<TimelineGlobalTools'), true, 'Timeline automation menu renders global timeline tools')
  eq(timelineGlobalTools.includes('data-cut-timeline-tools'), true, 'Timeline global tools expose a scoped group selector')
  eq(timelineGlobalTools.includes('data-cut-tool={tool.id}'), true, 'Timeline global tools bind each action id to the data-cut-tool selector')
  for (const toolId of ['trim_edges', 'split_scenes', 'mark_scenes']) {
    eq(timelineGlobalTools.includes(`id: '${toolId}'`), true, `Timeline global tools expose ${toolId}`)
  }
  eq(timelineToolbar.includes('data-cut-tc-readout'), true, 'Timeline toolbar component owns the timecode readout')
  eq(timelineToolbar.includes('data-cut-tool="razor"'), true, 'Timeline toolbar component owns the razor tool selector')
  eq(timelineAutomationMenu.includes('data-cut-action="sync-by-audio"'), true, 'Timeline automation menu owns the sync action selector')
  eq(timelineAutomationMenu.includes('data-cut-timeline-automation-trigger'), true, 'Timeline automation menu exposes a stable trigger selector')
  eq(timelineAutomationMenu.includes('data-cut-timeline-automation>'), true, 'Timeline automation menu exposes a root interaction boundary')
  eq(timelineAutomationMenu.includes('aria-haspopup="menu"'), true, 'Timeline automation trigger exposes menu semantics')
  eq(timelineAutomationMenu.includes("['ArrowDown', 'ArrowUp', 'Home', 'End']"), true, 'Timeline automation menu supports keyboard item navigation')
  eq(timelineAutomationMenu.includes("querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()"), true, 'Timeline automation menu focuses its first enabled command')
  eq(timelineToolbar.includes('data-cut-action="open-matte"'), true, 'Timeline toolbar component owns the contextual matte action')
  eq(timelineToolbar.includes('data-cut-action="add-video-track"'), true, 'Timeline toolbar exposes a central add-video-track command')
  eq(timelineToolbar.includes('data-cut-action="add-audio-track"'), true, 'Timeline toolbar exposes a central add-audio-track command')
  eq(timelineToolbar.includes('data-cut-action="ripple-trim-start"'), true, 'Timeline toolbar exposes ripple trim start')
  eq(timelineToolbar.includes('data-cut-action="ripple-trim-end"'), true, 'Timeline toolbar exposes ripple trim end')
  eq(timelineClipActions.includes("callVerb('edit.add_track'"), true, 'Timeline action hook dispatches the public add-track verb')
  eq(timelineClipActions.includes("callVerb('edit.trim'"), true, 'Timeline action hook dispatches linked playhead trims')
  // Delete cleanup scopes to the tracks the delete actually touched — the
  // `ranges` map holds exactly the selection plus its linked A/V halves.
  eq(timelineClipActions.includes('cleanupEmptyTracks(new Set([...ranges.values()].map((range) => range.track)))'), true, 'Delete cleanup only considers tracks touched by that delete')
  eq(timelineClipActions.includes('if (!candidates.has(t.id)) continue'), true, 'Unrelated empty user-created tracks survive delete cleanup')
  eq(existsSync(timelineRippleTrimPath), true, 'Playhead-side ripple trim has a pure planning module')
  eq(timelineRippleTrim.includes('planRippleTrimAtPlayhead'), true, 'Ripple trim planner owns active-clip selection and boundary rules')
  eq(keymap.includes("id: 'timeline.rippleTrimStart'") && keymap.includes("def: 'Q'"), true, 'Q defaults to ripple trim start')
  eq(keymap.includes("id: 'timeline.rippleTrimEnd'") && keymap.includes("def: 'W'"), true, 'W defaults to ripple trim end')
  eq(timelineToolbar.includes('<SpeedControl'), true, 'Timeline toolbar component composes the speed control')
  eq(timelineToolbar.includes('<TimelineSaveActions'), true, 'Timeline toolbar component composes the save/GIF actions')
  eq(existsSync(timelineSaveActionsPath), true, 'Timeline save/GIF action cluster has its own component module')
  eq(
    timeline.includes("from './TimelineSaveActions'") || timelineToolbar.includes("from './TimelineSaveActions'"),
    true,
    'Timeline render tree imports the save/GIF action component',
  )
  eq(timeline.includes('data-cut-action="save-range"'), false, 'Timeline panel no longer owns save-range selector inline')
  eq(timelineSaveActions.includes('data-cut-action="save-range"'), true, 'Timeline save action component owns save-range selector')
  eq(timelineSaveActions.includes('data-cut-action="save-gif"'), true, 'Timeline save action component owns GIF selector')
  eq(timelineSaveActions.includes('data-cut-save-note'), true, 'Timeline save action component owns save-note selector')
  eq(timelineSaveActions.includes('Render the selected timeline span as a reusable asset'), true, 'Timeline save-span tooltip leads with the reusable result')
  eq(existsSync(timelineRangeSavesPath), true, 'Timeline range/GIF save controller has its own hook module')
  eq(timeline.includes("from './useTimelineRangeSaves'"), true, 'Timeline panel imports the range/GIF save hook')
  eq(timeline.includes("callVerb('export.range'"), false, 'Timeline panel no longer owns Save to Assets export dispatch inline')
  eq(timeline.includes("callVerb('export.gif'"), false, 'Timeline panel no longer owns GIF export dispatch inline')
  eq(timeline.includes('const [savingRange'), false, 'Timeline panel no longer owns save-range status state inline')
  eq(timelineRangeSaves.includes("callVerb('export.range'"), true, 'Timeline save hook dispatches export.range')
  eq(timelineRangeSaves.includes("callVerb('export.gif'"), true, 'Timeline save hook dispatches export.gif')
  eq(timelineRangeSaves.includes('first 30s'), true, 'Timeline save hook preserves the GIF 30s cap feedback')
  eq(timelineRangeSaves.includes('Save to Assets'), true, 'Timeline save hook preserves shared Assets wording')
  eq(existsSync(timelineAssetDropPath), true, 'Timeline asset drag/drop controller has its own hook module')
  eq(timeline.includes("from './useTimelineAssetDrop'"), true, 'Timeline panel imports the asset drag/drop hook')
  eq(timeline.includes('function assetDragDetailFrom'), false, 'Timeline panel no longer owns asset drag payload parsing inline')
  eq(timeline.includes('document.addEventListener(ASSET_DRAG_MOVE'), false, 'Timeline panel no longer binds asset drag event listeners inline')
  eq(timelineAssetDrop.includes('function assetDragDetailFrom'), true, 'Timeline asset-drop hook owns asset drag payload parsing')
  eq(timelineAssetDrop.includes('document.addEventListener(ASSET_DRAG_MOVE'), true, 'Timeline asset-drop hook listens for asset drag moves')
  eq(timelineAssetDrop.includes('document.addEventListener(ASSET_DRAG_DROP'), true, 'Timeline asset-drop hook listens for asset drops')
  eq(timelineAssetDrop.includes("callVerb('edit.add_track'"), true, 'Timeline asset-drop hook creates a new line for dropped media')
  eq(timelineAssetDrop.includes('placeLinkedAV'), true, 'Timeline asset-drop hook preserves linked audio/video placement')
  eq(timelineAssetDrop.includes('duration_ms: durMs'), true, 'Timeline asset-drop hook preserves still-image duration insertion')
  eq(existsSync(assetCardDragPath), true, 'Assets has a dedicated drag controller')
  eq(assetsPanel.includes("from '../../lib/useAssetCardDrag'"), true, 'Assets uses the shared drag controller')
  eq(libraryPanel.includes("from '../../lib/useAssetCardDrag'"), false, 'Dedicated Library workspace no longer carries unreachable timeline-drag code')
  eq(assetCardDrag.includes('setPointerCapture'), true, 'Asset drag captures the initiating pointer when the WebView supports it')
  eq(assetCardDrag.includes("window.addEventListener('pointercancel'"), true, 'Asset drag cancels cleanly when macOS interrupts the pointer')
  eq(assetCardDrag.includes("window.addEventListener('mousedown'"), false, 'Asset drag does not install duplicate global mouse-down handlers')
  eq(assetCardDrag.includes("window.addEventListener('mousemove'"), true, 'Asset drag provides a macOS mouse-event compatibility lane')
  eq(existsSync(timelineWaveformPath), true, 'Timeline waveform canvas has its own component module')
  eq(timelineClipView.includes("from './WaveformCanvas'"), true, 'Timeline clip renderer imports the waveform canvas component')
  eq(timeline.includes('const WaveformCanvas ='), false, 'Timeline panel no longer defines the waveform canvas inline')
  eq(timeline.includes('data-cut-waveform='), false, 'Timeline panel no longer owns waveform selector markup inline')
  eq(timelineWaveform.includes('data-cut-waveform='), true, 'Timeline waveform component owns the waveform selector')
  eq(timelineWaveform.includes('getWaveform(asset)'), true, 'Timeline waveform component owns the waveform fetch')
  eq(existsSync(timelineClipViewPath), true, 'Timeline clip renderer has its own component module')
  eq(
    timeline.includes("from './ClipView'") || timelineTrackRow.includes("from './ClipView'"),
    true,
    'Timeline render tree imports the clip renderer component',
  )
  eq(timeline.includes('const ClipView ='), false, 'Timeline panel no longer defines the clip renderer inline')
  eq(timeline.includes('data-cut-clip={item.id}'), false, 'Timeline panel no longer owns clip selector markup inline')
  eq(timeline.includes('data-cut-trim={`${item.id}:l`}'), false, 'Timeline panel no longer owns trim selector markup inline')
  eq(timelineClipView.includes('data-cut-clip={item.id}'), true, 'Timeline clip renderer owns the clip selector')
  eq(timelineClipView.includes('data-cut-clip-film={item.id}'), true, 'Timeline clip renderer owns filmstrip selector markup')
  eq(timelineClipView.includes('data-cut-trim={`${item.id}:l`}'), true, 'Timeline clip renderer owns left trim selector markup')
  eq(timelineClipView.includes('<WaveformCanvas'), true, 'Timeline clip renderer composes the waveform component')
  eq(timelineClipView.includes('data-cut-motion-link={item.id}'), true, 'Timeline clip renderer exposes linked Motion identity')
  eq(timelineClipView.includes('data-cut-motion-state={item.motionLink.state}'), true, 'Timeline clip renderer exposes linked Motion state')
  eq(existsSync(motionLinkSectionPath), true, 'Linked Motion inspector has its own component module')
  eq(motionLinkSection.includes('data-cut-inspector-group="motion-link"'), true, 'Linked Motion inspector exposes its stable group selector')
  eq(motionLinkSection.includes('data-cut-motion-status'), true, 'Linked Motion inspector exposes visible status')
  eq(motionLinkSection.includes("callVerb('motion.link.refresh'"), true, 'Linked Motion inspector wires verified rerender')
  eq(motionLinkSection.includes("callVerb('motion.link.relink'"), true, 'Linked Motion inspector wires identity-checked relink')
  eq(motionLinkSection.includes("callVerb('motion.link.edit'"), true, 'Linked Motion inspector wires Canvas package intake')
  eq(motionLinkSection.includes('data-cut-motion-refresh='), true, 'Linked Motion refresh has a stable selector')
  eq(motionLinkSection.includes('data-cut-motion-relink='), true, 'Linked Motion relink has a stable selector')
  eq(motionLinkSection.includes('data-cut-motion-edit='), true, 'Edit in Motion has a stable selector')
  eq(motionLinkSection.includes('rain, water, snow'), true, 'Linked Motion inspector explains environment ownership')
  eq(motionLinkSection.includes('curves, and keyframes'), true, 'Edit in Motion explains its authoring scope')
  eq(existsSync(timelineWindowedThumbsPath), true, 'Timeline windowed thumbnail cache has its own hook module')
  eq(timeline.includes("from './useWindowedThumbnails'"), true, 'Timeline panel imports the windowed thumbnail hook')
  eq(timeline.includes('getWindowThumbs'), false, 'Timeline panel no longer owns windowed thumbnail fetches inline')
  eq(timelineWindowedThumbs.includes('getWindowThumbs'), true, 'Timeline windowed thumbnail hook owns thumbnail fetches')
  eq(timelineWindowedThumbs.includes('WIN_ACTIVATE_PXPS'), true, 'Timeline windowed thumbnail hook owns zoom activation constants')
  eq(existsSync(timelineTrackControlsPath), true, 'Timeline track controls have their own component module')
  eq(
    timeline.includes("from './TrackControls'") || timelineTrackRow.includes("from './TrackControls'"),
    true,
    'Timeline render tree imports the track controls component module',
  )
  eq(timeline.includes('function GainControl'), false, 'Timeline panel no longer defines the gain control inline')
  eq(timeline.includes('function MuteButton'), false, 'Timeline panel no longer defines the mute control inline')
  eq(timeline.includes('function ListenButton'), false, 'Timeline panel no longer defines the listen control inline')
  eq(timeline.includes('function KindIcon'), false, 'Timeline panel no longer defines track kind icons inline')
  eq(timeline.includes('data-cut-action="set-gain"'), false, 'Timeline panel no longer owns gain selector markup inline')
  eq(timeline.includes('data-cut-action="toggle-mute"'), false, 'Timeline panel no longer owns mute selector markup inline')
  eq(timeline.includes('data-cut-action="track-listen"'), false, 'Timeline panel no longer owns listen selector markup inline')
  eq(timelineTrackControls.includes('data-cut-action="set-gain"'), true, 'Timeline track controls own gain selector markup')
  eq(timelineTrackControls.includes('data-cut-action="toggle-mute"'), true, 'Timeline track controls own mute selector markup')
  eq(timelineTrackControls.includes('data-cut-action="toggle-solo"'), true, 'Timeline track controls own solo selector markup')
  eq(timelineTrackControls.includes('data-cut-action="toggle-track-visibility"'), true, 'Timeline track controls own visibility selector markup')
  eq(timelineTrackControls.includes('data-cut-action="toggle-track-lock"'), true, 'Timeline track controls own lock selector markup')
  eq(timelineTrackControls.includes('data-cut-action="set-pan"'), true, 'Timeline track controls own pan selector markup')
  eq(timelineTrackControls.includes('<TrackAuditionButton'), true, 'Timeline track controls compose the shared listen control')
  eq(timelineTrackControls.includes('data-cut-action="track-send-back"'), true, 'Timeline track controls own send-back selector markup')
  eq(timelineTrackControls.includes('data-cut-action="track-bring-forward"'), true, 'Timeline track controls own bring-forward selector markup')
  eq(/runUserVerb\(\s*'edit\.gain'/.test(timelineTrackControls), true, 'Timeline track controls dispatch edit.gain with visible failure feedback')
  eq(/runUserVerb\(\s*'edit\.mute'/.test(timelineTrackControls), true, 'Timeline track controls dispatch edit.mute with visible failure feedback')
  eq(/runUserVerb\(\s*'edit\.solo'/.test(timelineTrackControls), true, 'Timeline track controls dispatch edit.solo with visible failure feedback')
  eq(timelineTrackControls.includes("runUserVerb('edit.track_visible'"), true, 'Timeline track controls dispatch edit.track_visible with visible failure feedback')
  eq(timelineTrackControls.includes("runUserVerb('edit.track_lock'"), true, 'Timeline track controls dispatch edit.track_lock with visible failure feedback')
  eq(timelineTrackControls.includes("runUserVerb('edit.pan'"), true, 'Timeline track controls dispatch edit.pan with visible failure feedback')
  eq(timelineTrackControls.includes("runUserVerb('edit.reorder_track'"), true, 'Timeline track controls dispatch edit.reorder_track with visible failure feedback')
  eq(existsSync(trackAuditionPath), true, 'Timeline and Mixer share a track audition controller')
  eq(trackAudition.includes('data-cut-action="track-listen"'), true, 'Shared listen control preserves the timeline selector')
  eq(trackAudition.includes("callVerb('export.audio'"), true, 'Shared listen control dispatches export.audio')
  eq(trackAudition.includes('const baseUrl = exportUrl(path)'), true, 'Shared listen control translates exported stems to preview URLs')
  eq(trackAudition.includes('activeAudition?.stop()'), true, 'Starting any Listen control stops the prior track globally')
  eq(trackAudition.includes('request !== requestRef.current'), true, 'Shared listen control ignores stale export and playback completions')
  eq(trackAudition.includes('audio.onended = () =>'), true, 'Shared listen control returns to idle when playback ends')
  eq(trackAudition.includes('data-cut-audition-error={error || undefined}'), true, 'Shared listen control exposes visible error state evidence')
  eq(trackAudition.includes("cached?.revision === revisionKey"), true, 'Shared listen control can retry a ready stem after autoplay rejection')
  eq(fullCoverage.includes("actionId: 'track-listen'"), true, 'Native full coverage owns the shared track Listen action')
  eq(fullCoverage.includes('args?.track === speechTrackId'), true, 'Track Listen coverage proves the selected track reaches export.audio')
  eq(fullCoverage.includes("args?.rationale === 'timeline per-track listen'"), true, 'Track Listen coverage proves the timeline surface reaches export.audio')
  eq(fullCoverage.includes("probe._auditionState === 'error'"), true, 'Track Listen coverage exercises the ready-stem retry after autoplay rejection')
  eq(fullCoverage.includes("probe._auditionState === 'playing'"), true, 'Track Listen coverage requires actual playback state')
  eq(fullCoverage.includes("data-cut-audition-state') === 'idle'"), true, 'Track Listen coverage requires Stop to restore idle state')
  eq(propertyRow.includes('data-cut-prop-keyframe'), false, 'Property rows do not expose unreachable future keyframe controls')
  eq(propertyRow.includes('aria-label={`Reset ${label} to ${dflt}${unit ?? \'\'}`}'), true, 'Property reset has an accessible name')
  eq(inspectorSection.includes('aria-pressed={!bypassed}'), true, 'Section Bypass exposes its active state')
  eq(inspectorSection.includes('aria-label={bypassed ? `Enable ${title}` : `Bypass ${title}`}'), true, 'Section Bypass names both outcomes')
  eq(inspectorSection.includes("{bypassed ? '○' : '●'}"), true, 'Section Bypass renders visibly distinct enabled and bypassed states')
  eq(inspectorSection.includes('aria-label={`Reset ${title}`}'), true, 'Section Reset has an accessible name')
  for (const actionId of ['prop-slider', 'prop-reset', 'section-bypass', 'section-reset']) {
    eq(fullCoverage.includes(`actionId: '${actionId}'`), true, `Native full coverage owns ${actionId}`)
  }
  eq(fullCoverage.includes("args.x === 0.55"), true, 'Property slider coverage proves its release value reaches edit.transform')
  eq(fullCoverage.includes("args.x === 0 && args.y === 0 && args.scale === 1 && args.opacity === 1"), true, 'Section reset coverage proves the identity transform reaches the engine')
  for (const actionId of [
    'effect-on',
    'effect-chain-move-down',
    'effect-chain-move-up',
    'effect-chain-remove',
    'inspector-open-music',
    'inspector-redact-mode',
    'redact-draw',
    'speed-ramp-preset',
    'speed-ramp-clear',
    'shape-edit-kind',
    'grade-window-clear',
  ]) {
    eq(fullCoverage.includes(`actionId: '${actionId}'`), true, `Native full coverage owns ${actionId}`)
  }
  eq(fullCoverage.includes("waitForEffectChain(['compressor', 'denoise'])"), true, 'Effect-chain Move down coverage proves the new engine order')
  eq(fullCoverage.includes("waitForEffectChain(['denoise', 'compressor'])"), true, 'Effect-chain Move up coverage proves the restored engine order')
  eq(fullCoverage.includes("waitForEffectChain(['denoise'])"), true, 'Effect-chain Remove coverage proves the shortened engine chain')
  eq(fullCoverage.includes("args.points.length === 0"), true, 'Speed-ramp Clear coverage proves an empty curve reaches the engine')
  eq(fullCoverage.includes("args.enabled === false"), true, 'Power-window Clear all coverage proves disabled windows reach the engine')
  eq(fullCoverage.includes("args?.shape === 'ellipse'"), true, 'Shape-kind coverage proves the selected geometry reaches shape.update')
  eq(timelineTrackRow.includes('<TrackVisibilityButton'), true, 'Timeline track row exposes visual-track visibility in the header')
  eq(timelineTrackRow.includes('<TrackLockButton'), true, 'Timeline track row exposes lock state in the header')
  eq(timelineTrackRow.includes('<PanControl'), true, 'Timeline track row exposes audio pan in the header')
  eq(timelineTrackRow.includes('data-cut-track-locked'), true, 'Timeline track rows expose lock state for debug inspection')
  eq(timelineTrackRow.includes('data-cut-track-visible'), true, 'Timeline track rows expose visibility state for debug inspection')
  eq(timeline.includes('isTrackLocked'), true, 'Timeline gesture handlers check locked-track state before mutating')
  eq(timelineTrackRow.includes('data-cut-locked'), true, 'Timeline track row exposes locked gesture state to selectors')
  eq(timelineTrackRow.includes('<TrackOrderControls'), true, 'Timeline track row composes track z-order controls')
  eq(timelineTrackRow.includes('<SoloButton'), true, 'Timeline track row exposes solo in the timeline header')
  eq(timelineTrackRow.includes("{track.kind === 'audio' && ("), true, 'Timeline mute/solo controls are limited to rendered audio tracks')
  eq(existsSync(timelineSpeedControlPath), true, 'Timeline speed control has its own component module')
  eq(
    timeline.includes("from './SpeedControl'") || timelineToolbar.includes("from './SpeedControl'"),
    true,
    'Timeline render tree imports the speed control component module',
  )
  eq(timeline.includes('function SpeedControl'), false, 'Timeline panel no longer defines the speed control inline')
  eq(timeline.includes('data-cut-speed-control'), false, 'Timeline panel no longer owns speed control selector markup inline')
  eq(timeline.includes('data-cut-speed-input'), false, 'Timeline panel no longer owns speed input selector markup inline')
  eq(timeline.includes('data-cut-action={`speed-${p}`}'), false, 'Timeline panel no longer owns speed preset selector markup inline')
  eq(timelineSpeedControl.includes('data-cut-speed-control'), true, 'Timeline speed control module owns speed control selector markup')
  eq(timelineSpeedControl.includes('data-cut-speed-input'), true, 'Timeline speed control module owns speed input selector markup')
  eq(timelineSpeedControl.includes('data-cut-action="speed-preset"'), true, 'Timeline speed control module owns speed preset selector markup')
  eq(timelineSpeedControl.includes('min={0.25}'), true, 'Timeline speed control keeps the lower speed guard')
  eq(timelineSpeedControl.includes('max={4}'), true, 'Timeline speed control keeps the upper speed guard')
  eq(existsSync(timelineCrossfadePopoverPath), true, 'Timeline crossfade popover has its own component module')
  eq(timeline.includes("from './CrossfadePopover'"), true, 'Timeline imports the crossfade popover component module')
  eq(timeline.includes('function CrossfadePopover'), false, 'Timeline panel no longer defines the crossfade popover inline')
  eq(timeline.includes('data-cut-xfade-pop'), false, 'Timeline panel no longer owns crossfade popover selector markup inline')
  eq(timeline.includes('data-cut-action="apply-xfade"'), false, 'Timeline panel no longer owns crossfade apply selector markup inline')
  eq(timeline.includes('data-cut-action="clear-xfade"'), false, 'Timeline panel no longer owns crossfade clear selector markup inline')
  eq(timelineCrossfadePopover.includes('data-cut-xfade-pop'), true, 'Timeline crossfade popover owns popover selector')
  eq(timelineCrossfadePopover.includes('data-cut-xfade-input'), true, 'Timeline crossfade popover owns duration input selector')
  eq(timelineCrossfadePopover.includes('data-cut-xfade-style'), true, 'Timeline crossfade popover owns transition style selector')
  eq(timelineCrossfadePopover.includes('data-cut-action="apply-xfade"'), true, 'Timeline crossfade popover owns apply action selector')
  eq(timelineCrossfadePopover.includes('data-cut-action="clear-xfade"'), true, 'Timeline crossfade popover owns clear action selector')
  eq(timelineCrossfadePopover.includes('getTransitionsCatalog()'), true, 'Timeline crossfade popover fetches the transition catalog')
  eq(existsSync(timelineDuckStripPath), true, 'Timeline duck envelope strip has its own component module')
  eq(
    timeline.includes("from './DuckStrip'") || timelineTrackRow.includes("from './DuckStrip'"),
    true,
    'Timeline render tree imports the duck envelope component module',
  )
  eq(timeline.includes('const DuckStrip'), false, 'Timeline panel no longer defines the duck envelope inline')
  eq(timeline.includes('data-cut-duck'), false, 'Timeline panel no longer owns duck envelope selector markup inline')
  eq(timelineDuckStrip.includes('data-cut-duck'), true, 'Timeline duck envelope component owns the duck selector')
  eq(timelineDuckStrip.includes('tl-duck-label'), true, 'Timeline duck envelope component owns the duck readout label')
  eq(timelineDuckStrip.includes('msToPx'), true, 'Timeline duck envelope component keeps the time-to-pixel mapping')
  eq(existsSync(timelineMarkerContextMenuPath), true, 'Timeline marker context menu has its own component module')
  eq(timeline.includes("from './MarkerContextMenu'"), true, 'Timeline imports the marker context menu component module')
  eq(timeline.includes('data-cut-marker-menu'), false, 'Timeline panel no longer owns marker context menu selector markup inline')
  eq(timeline.includes('data-cut-marker-ctx-backdrop'), false, 'Timeline panel no longer owns marker context backdrop selector markup inline')
  eq(timeline.includes('data-cut-marker-ctx="seek"'), false, 'Timeline panel no longer owns marker seek action selector inline')
  eq(timeline.includes('data-cut-marker-ctx="delete"'), false, 'Timeline panel no longer owns marker delete action selector inline')
  eq(timelineMarkerContextMenu.includes('data-cut-marker-menu'), true, 'Timeline marker context menu owns the menu selector')
  eq(timelineMarkerContextMenu.includes('data-cut-marker-ctx-backdrop'), true, 'Timeline marker context menu owns the backdrop selector')
  eq(timelineMarkerContextMenu.includes('data-cut-marker-ctx="seek"'), true, 'Timeline marker context menu owns the seek selector')
  eq(timelineMarkerContextMenu.includes('data-cut-marker-ctx="delete"'), true, 'Timeline marker context menu owns the delete selector')
  eq(timelineMarkerContextMenu.includes('clampMenu(el, menu.x, menu.y)'), true, 'Timeline marker context menu clamps itself into the native viewport')
  eq(existsSync(timelineTrimPopoverPath), true, 'Timeline trim popover has its own component module')
  eq(timelineTrimPopover.includes('clampPopover(el, x, y)'), true, 'Timeline trim popover clamps itself into the native viewport')
  eq(existsSync(timelineRulerPath), true, 'Timeline ruler has its own component module')
  eq(timeline.includes("from './TimelineRuler'"), true, 'Timeline imports the ruler component module')
  eq(timeline.includes('data-cut-ruler'), false, 'Timeline panel no longer owns ruler selector markup inline')
  eq(timeline.includes('data-cut-marker={m.id}'), false, 'Timeline panel no longer owns marker triangle selector markup inline')
  eq(timeline.includes('data-cut-comment-pin'), false, 'Timeline panel no longer owns comment pin selector markup inline')
  eq(timeline.includes('data-cut-marker-ghost'), false, 'Timeline panel no longer owns marker ghost selector markup inline')
  eq(timelineRuler.includes('data-cut-ruler'), true, 'Timeline ruler component owns the ruler selector')
  eq(timelineRuler.includes('data-cut-marker={m.id}'), true, 'Timeline ruler component owns marker triangle selectors')
  eq(timelineRuler.includes('data-cut-comment-pin'), true, 'Timeline ruler component owns comment pin selectors')
  eq(timelineRuler.includes('data-cut-marker-ghost'), true, 'Timeline ruler component owns marker ghost selector')
  eq(timelineRuler.includes('markerClass(m)'), true, 'Timeline ruler component preserves marker class mapping')
  eq(timelineRuler.includes('resolveCommentTime(project, c)'), true, 'Timeline ruler resolves anchored comment timecodes')
  eq(timelineRuler.includes('timecode(time.atMs)'), true, 'Timeline ruler labels comments with resolved timecodes')
  eq(existsSync(timelineOverlaysPath), true, 'Timeline overlays have their own component module')
  eq(timeline.includes("from './TimelineOverlays'"), true, 'Timeline imports the overlays component module')
  eq(timeline.includes('data-cut-asset-drop'), false, 'Timeline panel no longer owns asset drop selector markup inline')
  eq(timeline.includes('data-cut-range'), false, 'Timeline panel no longer owns export range selector markup inline')
  eq(timeline.includes('tl-range__flag'), false, 'Timeline panel no longer owns export range flag markup inline')
  eq(timelineOverlays.includes('data-cut-asset-drop'), true, 'Timeline overlays component owns the asset drop selector')
  eq(timelineOverlays.includes('data-cut-range'), true, 'Timeline overlays component owns the export range selector')
  eq(timelineOverlays.includes('tl-range__flag'), true, 'Timeline overlays component owns range flag markup')
  eq(timelineOverlays.includes('shortDur'), true, 'Timeline overlays component keeps compact duration labels')
  eq(existsSync(timelineEmptyStatePath), true, 'Timeline empty state has its own component module')
  eq(timeline.includes("from './TimelineEmptyState'"), true, 'Timeline imports the empty-state component module')
  eq(timeline.includes('data-cut-import-cta'), false, 'Timeline panel no longer owns import CTA selector inline')
  eq(timeline.includes('Import media to begin'), false, 'Timeline panel no longer owns import CTA copy inline')
  eq(timeline.includes('Create a project in Projects to begin'), false, 'Timeline panel no longer owns no-project copy inline')
  eq(timelineEmptyState.includes('data-cut-import-cta'), true, 'Timeline empty-state component owns the import CTA selector')
  eq(timelineEmptyState.includes('Import media to begin'), true, 'Timeline empty-state component owns import CTA copy')
  eq(timelineEmptyState.includes('Create a project in Projects to begin'), true, 'Timeline empty-state component owns no-project copy')
  eq(existsSync(timelineGestureHudPath), true, 'Timeline gesture HUD has its own component module')
  eq(timeline.includes("from './TimelineGestureHud'"), true, 'Timeline imports the gesture HUD component module')
  eq(timeline.includes('data-cut-hud'), false, 'Timeline panel no longer owns gesture HUD selector markup inline')
  eq(timeline.includes('tl-hud__label'), false, 'Timeline panel no longer owns gesture HUD label markup inline')
  eq(timeline.includes('tl-hud__sub'), false, 'Timeline panel no longer owns gesture HUD sublabel markup inline')
  eq(timelineGestureHud.includes('data-cut-hud'), true, 'Timeline gesture HUD component owns the HUD selector')
  eq(timelineGestureHud.includes('data-cut-hud-tone'), true, 'Timeline gesture HUD component owns the HUD tone selector')
  eq(timelineGestureHud.includes('tl-hud__label'), true, 'Timeline gesture HUD component owns the label markup')
  eq(timelineGestureHud.includes('tl-hud__sub'), true, 'Timeline gesture HUD component owns the sublabel markup')
  eq(existsSync(timelineGuidesPath), true, 'Timeline guide chrome has its own component module')
  eq(timeline.includes("from './TimelineGuides'"), true, 'Timeline imports the guide chrome component module')
  eq(timeline.includes('data-cut-playhead'), false, 'Timeline panel no longer owns playhead selector markup inline')
  eq(timeline.includes('tl-playhead-handle'), false, 'Timeline panel no longer owns playhead handle markup inline')
  eq(timeline.includes('tl-snapline'), false, 'Timeline panel no longer owns snap guide markup inline')
  eq(timeline.includes('tl-marker-line'), false, 'Timeline panel no longer owns marker guide markup inline')
  eq(timelineGuides.includes('data-cut-playhead'), true, 'Timeline guide component owns the playhead selector')
  eq(timelineGuides.includes('tl-playhead-handle'), true, 'Timeline guide component owns the playhead handle markup')
  eq(timelineGuides.includes('tl-snapline'), true, 'Timeline guide component owns the snap guide markup')
  eq(timelineGuides.includes('tl-marker-line'), true, 'Timeline guide component owns marker guide markup')
  eq(timelineGuides.includes('msToPx'), true, 'Timeline guide component keeps the time-to-pixel mapping')
  eq(existsSync(timelineSeamHandlesPath), true, 'Timeline seam handles have their own component module')
  eq(
    timeline.includes("from './TimelineSeamHandles'") || timelineTrackRow.includes("from './TimelineSeamHandles'"),
    true,
    'Timeline render tree imports the seam handles component module',
  )
  eq(timeline.includes('data-cut-seam={`${seam.leftId}:${seam.rightId}`}'), false, 'Timeline panel no longer owns seam selector markup inline')
  eq(timeline.includes('data-cut-seam-xfade'), false, 'Timeline panel no longer owns seam crossfade metadata inline')
  eq(timeline.includes('tl-seam--xfade'), false, 'Timeline panel no longer owns seam active/crossfade class markup inline')
  eq(timelineSeamHandles.includes('data-cut-seam={`${seam.leftId}:${seam.rightId}`}'), true, 'Timeline seam handles component owns seam selectors')
  eq(timelineSeamHandles.includes('data-cut-seam-xfade'), true, 'Timeline seam handles component owns crossfade metadata')
  eq(timelineSeamHandles.includes('tl-seam--xfade'), true, 'Timeline seam handles component owns crossfade class markup')
  eq(timelineSeamHandles.includes('shortDur(seam.xfadeMs)'), true, 'Timeline seam handles component keeps compact duration labels')
  // Handles draw at the LAID boundary; seam.atMs is the EDITORIAL dispatch
  // coordinate and diverges from the drawn position after an upstream
  // crossfade — positioning by it was the harness-caught P1's sibling trap.
  eq(timelineSeamHandles.includes('msToPx(seam.laidMs, zoom)'), true, 'Timeline seam handles draw at the laid boundary')
  eq(timelineSeamHandles.includes('msToPx(seam.atMs, zoom)'), false, 'Timeline seam handles never position by the editorial dispatch coordinate')
  eq(existsSync(timelineTrackRowPath), true, 'Timeline track rows have their own component module')
  eq(timeline.includes("from './TimelineTrackRow'"), true, 'Timeline imports the track row component module')
  eq(timeline.includes('data-cut-track={track.id}'), false, 'Timeline panel no longer owns track row selector markup inline')
  eq(timeline.includes('className="tl-track-head"'), false, 'Timeline panel no longer owns track header markup inline')
  eq(timeline.includes('className="tl-lane"'), false, 'Timeline panel no longer owns lane markup inline')
  eq(timeline.includes('data-cut-ghost'), false, 'Timeline panel no longer owns move ghost selector markup inline')
  eq(timelineTrackRow.includes('data-cut-track={track.id}'), true, 'Timeline track row component owns the row selector')
  eq(timelineTrackRow.includes('className="tl-track-head"'), true, 'Timeline track row component owns the track header markup')
  eq(timelineTrackRow.includes('data-cut-track-kind={track.kind}'), true, 'Timeline track header exposes its kind for compact per-kind layout')
  eq(timelineTrackRow.includes('className="tl-track-meta"'), true, 'Timeline track header separates identity from persistent actions')
  eq(timelineTrackRow.includes('className="tl-lane"'), true, 'Timeline track row component owns the lane markup')
  eq(timelineTrackRow.includes('data-cut-ghost'), true, 'Timeline track row component owns the move ghost selector')
  eq(timelineTrackRow.includes('<ClipView'), true, 'Timeline track row component composes clip rendering')
  eq(timelineTrackRow.includes('<DuckStrip'), true, 'Timeline track row component composes duck envelope strips')
  eq(timelineTrackRow.includes('<TimelineSeamHandles'), true, 'Timeline track row component composes seam handles')
  eq(timelineTrackRow.includes('<GainControl'), true, 'Timeline track row component composes audio track controls')
  eq(existsSync(timelineClipContextModelPath), true, 'Timeline clip context-menu model helpers have their own module')
  eq(
    timeline.includes("from './ClipContextMenuModel'") || timelineClipActions.includes("from './ClipContextMenuModel'"),
    true,
    'Timeline action owner imports the clip context-menu model module',
  )
  eq(timeline.includes('function assetMediaKind'), false, 'Timeline panel no longer defines context-menu asset media kind inline')
  eq(timeline.includes('function assetBasename'), false, 'Timeline panel no longer defines context-menu asset labels inline')
  eq(timeline.includes('function adjacentGapSlot'), false, 'Timeline panel no longer defines context-menu fit-slot lookup inline')
  eq(timeline.includes('function isContiguousRun'), false, 'Timeline panel no longer defines context-menu nest precondition inline')
  eq(timelineClipContextModel.includes('export function assetMediaKind'), true, 'Clip context model owns asset media kind classification')
  eq(timelineClipContextModel.includes('export function assetBasename'), true, 'Clip context model owns picker asset labels')
  eq(timelineClipContextModel.includes('export function adjacentGapSlot'), true, 'Clip context model owns fit-to-fill adjacent gap lookup')
  eq(timelineClipContextModel.includes('export function isContiguousRun'), true, 'Clip context model owns nest contiguous-run detection')
  eq(timelineClipContextModel.includes("k === 'video' || k === 'audio' || k === 'image'"), true, 'Clip context model preserves known media family filtering')
  eq(timelineClipContextModel.includes(String.raw`split(/[\\/]/)`), true, 'Clip context model preserves cross-platform basename splitting')
  eq(timelineClipContextModel.includes("find((g) => g.kind === 'gap'"), true, 'Clip context model preserves gap-item based fit-slot lookup')
  eq(timelineClipContextModel.includes('sel.length < 2'), true, 'Clip context model preserves nest minimum-selection guard')
  eq(existsSync(timelineClipContextMenuPath), true, 'Timeline clip context menu has its own component module')
  eq(timeline.includes("from './ClipContextMenu'"), true, 'Timeline imports the clip context menu component module')
  eq(timeline.includes('data-cut-clip-menu'), false, 'Timeline panel no longer owns clip context menu selector markup inline')
  eq(timeline.includes('data-cut-ctx="copy"'), false, 'Timeline panel no longer owns clip context copy action inline')
  eq(timeline.includes('data-cut-ctx="fit-to-fill"'), false, 'Timeline panel no longer owns fit-to-fill menu action inline')
  eq(timeline.includes('data-cut-ctx="clean-voice"'), false, 'Timeline panel no longer owns clean-voice menu action inline')
  eq(timeline.includes('data-cut-ctx="blur-faces"'), false, 'Timeline panel no longer owns blur-faces menu action inline')
  eq(timeline.includes('data-cut-ctx="remove-track"'), false, 'Timeline panel no longer owns remove-track menu action inline')
  eq(timelineClipContextMenu.includes('data-cut-clip-menu'), true, 'Clip context menu component owns the menu selector')
  eq(timelineClipContextMenu.includes('data-cut-clip-kind="caption"'), true, 'Clip context menu component owns the curated caption menu')
  eq(timelineClipContextMenu.includes('data-cut-ctx="copy"'), true, 'Clip context menu component owns clipboard actions')
  eq(timelineClipContextMenu.includes('data-cut-ctx="fit-to-fill"'), true, 'Clip context menu component owns fit-to-fill action')
  eq(timelineClipContextMenu.includes('data-cut-ctx="clean-voice"'), true, 'Clip context menu component owns clean-voice action')
  eq(timelineClipContextMenu.includes('data-cut-ctx="blur-faces"'), true, 'Clip context menu component owns privacy action')
  eq(timelineClipContextMenu.includes('data-cut-ctx="remove-track"'), true, 'Clip context menu component owns overlay-track removal action')
  eq(timelineClipContextMenu.includes("window.prompt('Playback speed factor"), true, 'Clip context menu component owns custom speed prompt')
  eq(timelineClipContextMenu.includes('assetMediaKind('), true, 'Clip context menu component uses shared asset media classification')
  eq(existsSync(timelineClipActionsPath), true, 'Timeline clip actions have their own hook module')
  eq(timeline.includes("from './useTimelineClipActions'"), true, 'Timeline panel imports the clip action hook')
  eq(timeline.includes('const removeItemById = useCallback'), false, 'Timeline panel no longer owns remove-clip action inline')
  eq(timeline.includes('const fitToFillAdjacent = useCallback'), false, 'Timeline panel no longer owns fit-to-fill action inline')
  eq(timeline.includes('const syncByAudio = useCallback'), false, 'Timeline panel no longer owns sync-by-audio action inline')
  eq(timeline.includes('const applyCrossfade = useCallback'), false, 'Timeline panel no longer owns crossfade action inline')
  eq(timeline.includes('const applySpeed = useCallback'), false, 'Timeline panel no longer owns selected-speed action inline')
  eq(timeline.includes('const cleanVoiceItem = useCallback'), false, 'Timeline panel no longer owns clean-voice action inline')
  eq(timeline.includes('const MUTE_DB = -100'), false, 'Timeline panel no longer owns clip mute gain constant inline')
  eq(timelineClipActions.includes('export function useTimelineClipActions'), true, 'Timeline clip action hook exports the action bundle')
  eq(timelineClipActions.includes('const removeItemById = useCallback'), true, 'Timeline clip action hook owns remove-clip action')
  eq(timelineClipActions.includes('const fitToFillAdjacent = useCallback'), true, 'Timeline clip action hook owns fit-to-fill action')
  eq(timelineClipActions.includes('const syncByAudio = useCallback'), true, 'Timeline clip action hook owns sync-by-audio action')
  eq(timelineClipActions.includes('const applyCrossfade = useCallback'), true, 'Timeline clip action hook owns crossfade action')
  eq(timelineClipActions.includes('const applySpeed = useCallback'), true, 'Timeline clip action hook owns selected-speed action')
  eq(timelineClipActions.includes('const cleanVoiceItem = useCallback'), true, 'Timeline clip action hook owns clean-voice action')
  eq(timelineClipActions.includes('const MUTE_DB = -100'), true, 'Timeline clip action hook owns clip mute gain constant')
  eq(timelineClipActions.includes("callVerb('edit.fit_to_fill'"), true, 'Timeline clip action hook dispatches fit-to-fill edits')
  eq(timelineClipActions.includes("callVerb('edit.multicam_sync'"), true, 'Timeline clip action hook dispatches audio-sync measurement')
  eq(timelineClipActions.includes("runUserVerb('audio.cleanup_voice'"), true, 'Timeline clip action hook dispatches clean-voice edits with visible failure feedback')
  eq(timelineClipActions.includes("runUserVerb('edit.redact'"), true, 'Timeline clip action hook dispatches privacy redaction edits with visible failure feedback')
  eq(`${preview}\n${previewExactReview}`.includes('Save to Assets'), true, 'Preview rendered-section save action uses the shared Assets wording')
  eq(`${preview}\n${previewExactReview}`.includes('Save to library'), false, 'Preview rendered-section save action avoids alternate library wording')
  eq(preview.includes('save to your library'), false, 'Preview rendered-section empty-state hint avoids alternate library wording')
  eq(layout.includes("'generate'"), true, 'Layout supports a persistent Generate left tab')
  eq(layout.includes("leftTab: 'projects'"), true, 'Fresh layout opens the Projects workflow by default')
  eq(layout.includes("['transcript', 'assets', 'generate', 'projects', 'find']"), true, 'Layout persists only permanent editor-side tabs')
  eq(layout.includes("'edit' | 'record' | 'library'"), true, 'Layout model supports the dedicated Library workspace')
  eq(leftPanel.includes('data-cut-left-tab="library"'), false, 'Library is no longer trapped in the narrow left sidebar')
  eq(existsSync(libraryWorkspacePath) && libraryWorkspace.includes('data-cut-library-workspace'), true, 'Library has a dedicated workspace shell')
  eq(appWorkspace.includes("layout.workspaceMode === 'library'") && appWorkspace.includes('<LibraryWorkspace'), true, 'App workspace swaps the editor for Library without losing layout state')
  eq(uiSurface('library')?.action, { kind: 'workspace', workspace: 'library' }, 'ui.open Library uses the same dedicated workspace as the human launcher')
  eq(libraryWorkspace.includes('data-cut-library-close'), true, 'Library workspace has an explicit return to Edit')
  eq(app.includes("hidden={layout.workspaceMode === 'library'}"), true, 'Library hides the right rail without unmounting its event bridge')
  eq(appRightRail.includes("workspaceMode: 'edit'") && appRightRail.includes('if (hidden) return null'), true, 'Review requests leave Library and reveal their requested surface')
  eq(appSurfaceEvents.includes("workspaceMode: 'edit'"), true, 'Document-level editor surface requests leave Library before opening')
  eq(appKeyboardController.includes("workspaceMode: 'edit'"), true, 'Global rail and comments shortcuts do not mutate an invisible editor behind Library')
  eq(existsSync(libraryDetailsPath) && libraryDetails.includes('data-cut-library-details'), true, 'Library workspace owns a bounded details pane')
  eq(libraryPanel.includes('keyboardNavigation.activeId'), true, 'Library details follow the focused item without requiring bulk selection')
  eq(libraryDetails.includes('Use checkboxes for bulk actions.'), true, 'Library distinguishes item inspection from bulk selection')
  eq(libraryDetails.includes('items selected'), true, 'Library details name the multi-selection state')
  eq(libraryPanel.includes('data-cut-library-insert') || libraryActions.includes('data-cut-library-insert'), true, 'Library exposes an explicit Insert at playhead action')
  eq(existsSync(libraryPlacementPath) && libraryPanel.includes("from './libraryPlacement'"), true, 'Library timeline placement is split out of the main controller')
  eq(libraryPlacement.includes('timelineEmpty') && libraryPlacement.includes('placeLinkedAV'), true, 'Library placement avoids duplicate first clips and preserves linked audio')
  eq(leftPanel.includes('data-cut-left-tab="generate"'), true, 'LeftPanel keeps Generate beside project-local Assets')
  eq(leftPanel.includes('data-cut-left-tab="find"') && leftPanel.includes("onClick={() => onTab('find')}"), true, 'LeftPanel exposes Find as a permanent tab')
  eq(leftPanel.includes("tab === 'find' &&") || leftPanel.includes('{tab === \'find\' &&'), false, 'Find tab is no longer transient or topbar-only')
  eq(leftPanel.includes('lp__pane--generate'), true, 'LeftPanel marks Generate for sidebar-specific layout')
  eq(leftPanel.includes('<GenerateTemplatesWorkspace'), true, 'LeftPanel embeds the native Generate workspace as the Generate tab')
  eq(leftPanel.includes('data-cut-find-tab="find-media"'), true, 'Find pane keeps media search as a search-only subtab')
  eq(leftPanel.includes('data-cut-find-tab="find-moment"'), true, 'Find pane keeps moment search as a search-only subtab')
  eq(leftPanel.includes('data-cut-find-tab="sequence-index"'), true, 'Find pane exposes cross-sequence clip and marker search')
  eq(leftPanel.includes('data-cut-find-tab="generate"'), false, 'Find pane does not reintroduce Generate as a search subtab')
  eq(
    leftPanel.includes('StockDrawer project={project}')
      && leftPanel.includes('SearchDrawer project={project} playheadMs={playheadMs}')
      && leftPanel.includes('<SequenceIndex project={project} onProjectChanged={onProjectChanged} />'),
    true,
    'Find pane contains media, moment, and cross-sequence search surfaces',
  )
  eq(stock.includes('data-cut-stock-close'), false, 'Stock has no unreachable legacy modal Close action')
  eq(stock.includes('data-cut-stock-scrim'), false, 'Stock no longer carries unreachable legacy modal chrome')
  eq(cssBlock(drawerCss, '.cd-stock-list').includes('flex: none'), true, 'Stock results cannot collapse below their usable content height')
  eq(fullCoverage.includes("renderGroup(page, S, 'stock-search-results'"), true, 'Stock coverage preserves the populated search result state')
  eq(fullCoverage.includes("renderGroup(page, S, 'stock-import-complete'"), true, 'Stock coverage preserves the completed import state')
  eq(leftPanel.includes('data-cut-panel="generate-templates"') || existsSync(resolve(srcRoot, 'panels/GenerateTemplates/index.tsx')), true, 'Generate tab owns the templates/prompt/storyboard workspace')
  eq(uiSurface('generate-prompt')?.action, { kind: 'generate', tab: 'prompt' }, 'ui.open generate-prompt opens the Native prompt subtab')
  eq(uiSurface('generate-storyboard')?.action, { kind: 'generate', tab: 'storyboard' }, 'ui.open generate-storyboard opens the Storyboard subtab')
  eq(uiSurface('generate-media')?.action, { kind: 'generate', tab: 'media' }, 'ui.open generate-media opens the AI media subtab')
  eq(generateCss.includes('.lp__pane--generate .gt-grid'), true, 'Generate templates use a sidebar layout instead of viewport-only media queries')
  eq(
    cssBlock(generateCss, '.lp__pane--generate .gt-grid').includes('display: flex') &&
      cssBlock(generateCss, '.lp__pane--generate .gt-grid').includes('flex-direction: column'),
    true,
    'Generate templates use one readable vertical flow in the left rail',
  )
  eq(cssBlock(generateCss, '.lp__pane--generate .gt-grid').includes('overflow: auto'), true, 'Generate templates scroll as one rail surface')
  eq(generateTemplatePanel.includes('gt-actions--template-primary') || generateTemplates.includes('gt-actions--template-primary'), true, 'Generate template Preview/Insert actions stay with the selected template details')
  eq(generateTemplatePanel.includes('data-cut-generate-template-preview-inline') || generateTemplates.includes('data-cut-generate-template-preview-inline'), true, 'Generate template preview has a compact visible receipt above the timeline')
  eq(generatePrompt.includes('Native prompt') || generateTemplates.includes('Native prompt'), true, 'Generate prompt tab uses user-facing native prompt wording')
  eq(generatePrompt.includes('data-cut-generate-prompt-run') || generateTemplates.includes('data-cut-generate-prompt-run'), true, 'Generate prompt tab exposes the run action')
  eq(generateStoryboard.includes('data-cut-generate-storyboard-insert') || generateTemplates.includes('data-cut-generate-storyboard-insert'), true, 'Generate storyboard tab exposes the insert action')
  eq(generateTemplates.includes('AI media'), true, 'Generate media tab uses user-facing AI media wording')
  eq(generateTemplates.includes('data-cut-generate-media-intro'), true, 'Generate media tab explains provider-backed asset insertion compactly')
  eq(
    cssBlock(generateCss, '.lp__pane--generate .gt-catalog').includes('max-height: 204px') &&
      cssBlock(generateCss, '.lp__pane--generate .gt-catalog').includes('overflow: hidden'),
    true,
    'Generate catalog stays compact so selected template details are visible above the timeline',
  )
  eq(
    cssBlock(generateCss, '.lp__pane--generate .gt-list').includes('min-height: 76px') &&
      cssBlock(generateCss, '.lp__pane--generate .gt-list').includes('max-height: none'),
    true,
    'Generate template cards scroll inside the compact catalog slice',
  )
  eq(cssBlock(generateCss, '.lp__pane--generate .gt-card__badges').includes('display: none'), true, 'Generate left-rail template cards hide redundant badges instead of clipping them')
  eq(cssBlock(generateCss, '.lp__pane--generate .gt-inspector > .gt-tags').includes('display: none'), true, 'Generate left-rail selected-template tags do not clip above the timeline')
  eq(cssBlock(generateCss, '.lp__pane--generate .gt-fields').includes('margin-top: var(--space-5)'), true, 'Generate left-rail parameter fields start below the first-viewport action receipt')
  eq(cssBlock(generateCss, '.lp__pane--generate .gt-prompt').includes('display: block'), true, 'Generate prompt scrolls as one left-rail surface')
  eq(cssBlock(generateCss, '.lp__pane--generate .gt-storyboard').includes('display: block'), true, 'Generate storyboard scrolls as one left-rail surface')
  eq(libraryCss.includes('min-width: max-content'), false, 'Library list mode does not force a horizontally clipped table in the left rail')
  eq(cssBlock(libraryCss, '.lb-row-actions .lb-actions').includes('flex-wrap: wrap'), true, 'Library list actions wrap instead of disappearing off-screen')
  eq(assemble.includes('data-cut-assemble-dir'), true, 'B-roll drawer exposes the local media folder input')
  eq(assemble.includes("provider: 'local_folder'") && assemble.includes('dir: brollDir.trim()'), true, 'B-roll drawer passes the selected folder to assemble.broll')
  eq(fullCoverage.includes("captureVerbResp(page, 'assemble.broll'"), true, 'B-roll coverage drains the native response before judging placement')
  eq(fullCoverage.includes('const brollSource = SECOND'), true, 'B-roll coverage retrieves a deterministic source not already placed by project bootstrap')
  eq(fullCoverage.includes('placed.length > 0 && landed'), true, 'B-roll coverage requires response-owned clip ids to land before proving Jump')
}

// --- MusicBed mute original: use the non-destructive track mute flag ---------
// This drawer is a convenience copy of the track mute operation. It must not
// silence the original track by overwriting gain, because that loses the user's
// dialed level and diverges from the mixer/timeline mute semantics.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const musicBed = readFileSync(resolve(srcRoot, 'panels/MusicBed/index.tsx'), 'utf8')
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')

  eq(topbar.includes("Mix each track's level, pan, mute, and solo"), true, 'Topbar mixer tooltip names the human mixing outcomes')
  eq(topbar.includes('per-track level/mute/solo via edit.gain'), false, 'Topbar mixer comments do not describe mute/solo as edit.gain')
  eq(musicBed.includes("callVerb('edit.mute'"), true, 'MusicBed mute original dispatches edit.mute')
  eq(musicBed.includes("callVerb('edit.gain'"), false, 'MusicBed mute original does not dispatch edit.gain')
  eq(musicBed.includes('baseAudio?.muted === true'), true, 'MusicBed mute original reflects the track muted flag')
  eq(musicBed.includes('const [muteBusy, setMuteBusy] = useState(false)'), true, 'MusicBed mute original has pending state for rapid double-clicks')
  eq(musicBed.includes('if (muteBusy || !baseAudio) return'), true, 'MusicBed mute original ignores duplicate toggles while pending')
  eq(musicBed.includes('disabled={muteBusy}'), true, 'MusicBed mute original disables the checkbox while the edit.mute call is pending')
  eq(musicBed.includes('edit.gain -100'), false, 'MusicBed visible/help copy does not mention destructive gain mute')
  eq(fullCoverage.includes('music-drawer-mute-original(edit.mute)'), true, 'Full coverage gate clicks MusicBed mute original')
  eq(fullCoverage.includes("'edit.mute', (a) => a.track === track && a.on === target"), true, 'Full coverage gate asserts MusicBed edit.mute result')
}

// --- Audio-layer verifier: mute/solo are flags, not destructive gain writes ---
// The mute/solo regression moved both controls to persistent Track.muted / Track.solo flags. The
// runtime verifier must assert those flags and that gain_db is preserved; otherwise
// it reports false failures against the fixed non-destructive mixer semantics.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const verifyAudioLayer = readFileSync(resolve(here, 'verify-audio-layer.mjs'), 'utf8')

  eq(verifyAudioLayer.includes('async function trackFlags'), true, 'Audio-layer verifier reads track muted/solo flags from project.state')
  eq(verifyAudioLayer.includes('mixer-mute-sets-flag'), true, 'Audio-layer verifier names mute as a flag check')
  eq(verifyAudioLayer.includes('mixer-solo-sets-flag'), true, 'Audio-layer verifier names solo as a flag check')
  eq(verifyAudioLayer.includes('track.muted === true'), true, 'Audio-layer verifier asserts mute turns on Track.muted')
  eq(
    verifyAudioLayer.includes('track.muted === false') || verifyAudioLayer.includes('clear.muted === false'),
    true,
    'Audio-layer verifier asserts unmute clears Track.muted',
  )
  eq(verifyAudioLayer.includes('track.solo === true'), true, 'Audio-layer verifier asserts solo turns on Track.solo')
  eq(
    verifyAudioLayer.includes('track.solo === false') || verifyAudioLayer.includes('clear.solo === false'),
    true,
    'Audio-layer verifier asserts unsolo clears Track.solo',
  )
  eq(verifyAudioLayer.includes('gainPreserved'), true, 'Audio-layer verifier asserts mute/solo preserve gain_db')
  eq(verifyAudioLayer.includes('muted <= -90'), false, 'Audio-layer verifier no longer expects mute to overwrite gain')
  eq(verifyAudioLayer.includes('otherAfter <= -90'), false, 'Audio-layer verifier no longer expects solo to overwrite other-track gain')
  eq(verifyAudioLayer.includes('mixer-mute-drops-gain'), false, 'Audio-layer verifier no longer labels mute as destructive gain drop')
  eq(verifyAudioLayer.includes('mixer-solo-mutes-others'), false, 'Audio-layer verifier no longer labels solo as destructive gain drop')
}

// --- Kinetic captions: readiness is caption-kind based, not cap1-id based ----
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const kinetic = readFileSync(resolve(srcRoot, 'panels/Kinetic/index.tsx'), 'utf8')
  const fullCoverage = readFileSync(resolve(here, 'full-coverage-verify.mjs'), 'utf8')

  eq(kinetic.includes('function captionCueCount'), true, 'Kinetic drawer has a caption cue counter')
  eq(kinetic.includes("t.kind === 'caption'"), true, 'Kinetic drawer detects caption-kind tracks')
  eq(kinetic.includes("t.id === 'cap1'"), false, 'Kinetic drawer readiness does not hard-code cap1')
  eq(fullCoverage.includes('const hasCaptionCues ='), true, 'Full coverage has a caption-kind cue readiness helper')
  eq(fullCoverage.includes('waitForState(hasCaptionCues'), true, 'Full coverage opens Kinetic after caption-kind cues exist')
}

// --- Local Python sidecars: Windows pipes must stay UTF-8 --------------------
// Rust sends JSON over stdin. On Windows, Python's default stdio encoding follows
// the process locale unless forced, which corrupts translated non-ASCII dub text
// before it reaches OmniVoice.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const dubRunner = readFileSync(resolve(root, 'app/perception/py/dub_runner.py'), 'utf8')
  const translateRunner = readFileSync(resolve(root, 'app/perception/py/translate_runner.py'), 'utf8')
  const dubRs = readFileSync(resolve(root, 'app/server/src/dub.rs'), 'utf8')
  const translateRs = readFileSync(resolve(root, 'app/server/src/translate.rs'), 'utf8')
  const sidecarProbePython = python310ForProbe(root)
  const dubWaveProbe = spawnSync(
    sidecarProbePython,
    [
      '-c',
      `
import contextlib
import importlib.util
import io
import json
import sys
import tempfile
from pathlib import Path

root = Path("app/perception/py").resolve()
sys.path.insert(0, str(root))
spec = importlib.util.spec_from_file_location("dub_runner", root / "dub_runner.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
mod.synth_segment = lambda *args, **kwargs: b"\\x00\\x00" * 2400

with tempfile.TemporaryDirectory() as td:
    out = Path(td) / "dub.wav"
    job = {
        "endpoint": "http://127.0.0.1:9001",
        "voice": "test",
        "sample_rate": 24000,
        "out_wav": str(out),
        "segments": [{"i": 0, "start_ms": 0, "slot_ms": 100, "text": "hola"}],
    }
    sys.stdin = io.StringIO(json.dumps(job))
    got = io.StringIO()
    with contextlib.redirect_stdout(got):
        code = mod.main()
    assert code == 0, code
    payload = json.loads(got.getvalue())
    assert out.exists() and out.stat().st_size > 44, (out, out.exists(), out.stat().st_size if out.exists() else 0)
    assert payload["out_wav"] == str(out), payload
`,
    ],
    {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, PYTHONIOENCODING: 'utf-8', PYTHONUTF8: '1' },
    },
  )

  for (const [name, src] of [['dub_runner.py', dubRunner], ['translate_runner.py', translateRunner]] as const) {
    eq(src.includes('sys.stdin.reconfigure(encoding="utf-8")'), true, `${name} reads runner stdin as UTF-8`)
    eq(src.includes('sys.stdout.reconfigure(encoding="utf-8")'), true, `${name} writes runner stdout as UTF-8`)
  }
  eq(translateRunner.includes('_STREAM_RECONFIGURE_ERROR'), false, 'translate_runner does not hide stream reconfigure errors in a dead variable')
  eq(translateRunner.includes('stream encoding reconfigure unavailable'), true, 'translate_runner reports stream reconfigure fallback visibly')
  eq(
    dubWaveProbe.status,
    0,
    `dub_runner writes a WAV when out_wav is resolved as a pathlib path${dubWaveProbe.stderr ? ` (${dubWaveProbe.stderr.trim().slice(0, 180)})` : ''}`,
  )
  for (const [name, src] of [['dub.rs', dubRs], ['translate.rs', translateRs]] as const) {
    eq(src.includes('.env("PYTHONIOENCODING", "utf-8")'), true, `${name} forces Python pipe encoding to UTF-8`)
    eq(src.includes('.env("PYTHONUTF8", "1")'), true, `${name} starts Python in UTF-8 mode`)
  }
}

// --- Generate module consolidation: one surfaced workspace, real verbs -------
// The separate Generate branch must land as one user-facing Generate tab beside
// Assets/Library, backed by native generate.* verbs. The old provider-backed
// assets.generate surface may remain as a sub-surface, but it must not be the
// whole Generate tab anymore.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const client = readFileSync(resolve(srcRoot, 'lib/client.ts'), 'utf8')
  const leftPanel = readFileSync(resolve(srcRoot, 'panels/LeftPanel/index.tsx'), 'utf8')
  const generateTemplates = readFileSync(resolve(srcRoot, 'panels/GenerateTemplates/index.tsx'), 'utf8')
  const app = readFileSync(resolve(srcRoot, 'App.tsx'), 'utf8')
  const appSurfaceEvents = readFileSync(resolve(srcRoot, 'app/useAppSurfaceEvents.ts'), 'utf8')
  const assets = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const generateModelPath = resolve(srcRoot, 'panels/GenerateTemplates/model.ts')
  const generateModel = existsSync(generateModelPath) ? readFileSync(generateModelPath, 'utf8') : ''
  const generateCatalogPath = resolve(srcRoot, 'panels/GenerateTemplates/TemplateCatalog.tsx')
  const generateCatalog = existsSync(generateCatalogPath) ? readFileSync(generateCatalogPath, 'utf8') : ''
  const generateTemplatePanelPath = resolve(srcRoot, 'panels/GenerateTemplates/TemplatePanel.tsx')
  const generateTemplatePanel = existsSync(generateTemplatePanelPath) ? readFileSync(generateTemplatePanelPath, 'utf8') : ''
  const generateTabsPath = resolve(srcRoot, 'panels/GenerateTemplates/WorkspaceTabs.tsx')
  const generateTabs = existsSync(generateTabsPath) ? readFileSync(generateTabsPath, 'utf8') : ''
  const generatePromptPath = resolve(srcRoot, 'panels/GenerateTemplates/PromptPanel.tsx')
  const generatePrompt = existsSync(generatePromptPath) ? readFileSync(generatePromptPath, 'utf8') : ''
  const generateStoryboardPath = resolve(srcRoot, 'panels/GenerateTemplates/StoryboardPanel.tsx')
  const generateStoryboard = existsSync(generateStoryboardPath) ? readFileSync(generateStoryboardPath, 'utf8') : ''
  const mainRs = readFileSync(resolve(root, 'app/server/src/main.rs'), 'utf8')
  const dispatchRs = readFileSync(resolve(root, 'app/server/src/dispatch.rs'), 'utf8')
  const generateHandlersRs = readFileSync(resolve(root, 'app/server/src/generate_handlers.rs'), 'utf8')
  const motionBridgeRsPath = resolve(root, 'app/server/src/motion_bridge.rs')
  const motionBridgeRs = existsSync(motionBridgeRsPath) ? readFileSync(motionBridgeRsPath, 'utf8') : ''
  const motionRuntimeRsPath = resolve(root, 'app/server/src/motion_runtime.rs')
  const motionRuntimeRs = existsSync(motionRuntimeRsPath) ? readFileSync(motionRuntimeRsPath, 'utf8') : ''
  const registryRs = readFileSync(resolve(root, 'app/server/src/registry.rs'), 'utf8')
  const fullCoverage = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const generateCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageGenerateTemplateActions.mjs'), 'utf8')
  const skill = readFileSync(resolve(root, 'skill/shellx-cut/SKILL.md'), 'utf8')
  const reference = readFileSync(resolve(root, 'skill/shellx-cut/reference.md'), 'utf8')
  const featureInventory = readFileSync(resolve(root, 'docs/public/FEATURES.md'), 'utf8')
  const motionBoundary = readFileSync(resolve(root, 'docs/public/SHELLX_MOTION_BOUNDARY.md'), 'utf8')
  const verbs = JSON.parse(readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')) as {
    verbs: Array<{ name: string; args?: { properties?: { panel?: { enum?: string[] } } } }>
  }
  const verbNames = verbs.verbs.map((v) => v.name)
  const generateVerbs = ['generate.list', 'generate.describe', 'generate.preview', 'generate.insert', 'generate.from_prompt', 'generate.storyboard']
  const motionBridgeVerb = 'motion.template_to_cut'
  const motionScriptBridgeVerb = 'motion.script_to_cut'
  const motionJobGetVerb = 'motion.job.get'
  const motionJobListVerb = 'motion.job.list'
  const motionMapImportVerb = 'motion.map_import'
  const motionApplyImportVerb = 'motion.apply_import'
  const motionLinkRefreshVerb = 'motion.link.refresh'
  const motionLinkRelinkVerb = 'motion.link.relink'
  const motionLinkEditVerb = 'motion.link.edit'

  eq(existsSync(resolve(srcRoot, 'panels/GenerateTemplates/index.tsx')), true, 'GenerateTemplates workspace exists in the active repo')
  eq(existsSync(resolve(root, 'schema/generate_templates.json')), true, 'Generate templates catalog is part of schema assets')
  eq(existsSync(resolve(root, 'app/server/src/generate.rs')), true, 'Generate server module exists')
  eq(existsSync(resolve(root, 'app/server/src/generate_handlers.rs')), true, 'Generate server handler module exists')
  eq(leftPanel.includes('<GenerateTemplatesWorkspace'), true, 'Generate tab renders the native Generate workspace')
  eq(leftPanel.includes('<GenerateDrawer'), false, 'Generate tab is not only the legacy assets.generate drawer')
  eq(uiSurface('generate-prompt')?.action, { kind: 'generate', tab: 'prompt' }, 'App routes ui.open generate-prompt to the Native prompt subtab')
  eq(uiSurface('generate-storyboard')?.action, { kind: 'generate', tab: 'storyboard' }, 'App routes ui.open generate-storyboard to the Storyboard subtab')
  eq(uiSurface('generate-media')?.action, { kind: 'generate', tab: 'media' }, 'App routes ui.open generate-media to the AI media subtab')
  eq(assets.includes("detail: { tab: 'media' }"), true, 'Assets Generate shortcut opens the media subtab with the provider-backed assets.generate surface')
  eq(appSurfaceEvents.includes('normalizeGenerateTab') && appSurfaceEvents.includes('CustomEvent<{ tab?: GenerateWorkspaceTab } | GenerateWorkspaceTab | undefined>'), true, 'App honors cut:open-generate detail so shortcuts can target Generate subtabs')
  eq(generateModel.includes('templateListResultFrom') && generateModel.includes('templateSummaryFrom'), true, 'Generate hidden workspace guards malformed generate.list results')
  eq(generateModel.includes('templateManifestFrom') && generateModel.includes('generateParamRecord'), true, 'Generate hidden workspace guards malformed generate.describe results')
  eq(existsSync(generateModelPath), true, 'GenerateTemplates pure model helper module exists')
  eq(existsSync(generateCatalogPath), true, 'GenerateTemplates template catalog component exists')
  eq(existsSync(generateTemplatePanelPath), true, 'GenerateTemplates template tab component exists')
  eq(existsSync(generateTabsPath), true, 'GenerateTemplates workspace tabs component exists')
  eq(existsSync(generatePromptPath), true, 'GenerateTemplates prompt tab component exists')
  eq(existsSync(generateStoryboardPath), true, 'GenerateTemplates storyboard tab component exists')
  eq(generateTemplates.includes("from './model'"), true, 'GenerateTemplates workspace imports pure model helpers')
  eq(generateTemplatePanel.includes("from './TemplateCatalog'"), true, 'Generate template tab imports the template catalog component')
  eq(generateTemplates.includes("from './TemplatePanel'"), true, 'GenerateTemplates workspace imports the template tab component')
  eq(generateTemplates.includes("from './WorkspaceTabs'"), true, 'GenerateTemplates workspace imports the workspace tab component')
  eq(generateTemplates.includes("from './PromptPanel'"), true, 'GenerateTemplates workspace imports the prompt tab component')
  eq(generateTemplates.includes("from './StoryboardPanel'"), true, 'GenerateTemplates workspace imports the storyboard tab component')
  eq(
    generateModel.includes('export function templateListResultFrom') && generateModel.includes('function templateSummaryFrom'),
    true,
    'GenerateTemplates model owns list parsing helpers',
  )
  eq(
    generateModel.includes('export function templateManifestFrom') && generateModel.includes('function generateParamRecord'),
    true,
    'GenerateTemplates model owns manifest parsing helpers',
  )
  eq(generateTemplates.includes('function templateSummaryFrom'), false, 'GenerateTemplates panel no longer defines template parsing inline')
  eq(generateTemplates.includes('function generateParamRecord'), false, 'GenerateTemplates panel no longer defines param parsing inline')
  eq(generateTemplates.includes('className="gt-catalog"'), false, 'GenerateTemplates panel no longer owns catalog markup inline')
  eq(generateTemplates.includes('className="gt-grid"'), false, 'GenerateTemplates panel no longer owns template tab body markup inline')
  eq(generateTemplates.includes('data-cut-generate-template-insert'), false, 'GenerateTemplates panel no longer owns template insert action markup inline')
  eq(generateTemplates.includes('className="gt-tabs"'), false, 'GenerateTemplates panel no longer owns workspace tab strip inline')
  eq(generateTemplates.includes('data-cut-generate-prompt-run'), false, 'GenerateTemplates panel no longer owns prompt tab action markup inline')
  eq(generateTemplates.includes('data-cut-generate-storyboard-insert'), false, 'GenerateTemplates panel no longer owns storyboard action markup inline')
  eq(generateCatalog.includes('data-cut-generate-template-list'), true, 'Generate template catalog preserves the template-list selector')
  eq(generateCatalog.includes('KIND_FILTERS.map'), true, 'Generate template catalog owns kind filter rendering')
  eq(generateTemplatePanel.includes('<TemplateCatalog'), true, 'Generate template tab composes the catalog component')
  eq(generateTemplatePanel.includes('data-cut-generate-template-preview'), true, 'Generate template tab component owns preview action selector')
  eq(generateTemplatePanel.includes('data-cut-generate-template-insert'), true, 'Generate template tab component owns insert action selector')
  eq(generateTemplatePanel.includes('data-cut-generate-template-preview-inline'), true, 'Generate template tab component owns inline preview receipt')
  eq(generateTemplatePanel.includes('data-cut-generate-template-result'), true, 'Generate template tab component owns insert evidence')
  eq(generateTabs.includes('data-cut-generate-tab'), true, 'Generate workspace tabs preserve route selectors')
  eq(generateTabs.includes('GenerateWorkspaceTab'), true, 'Generate workspace tabs use the shared GenerateWorkspaceTab type')
  eq(
    generateTabs.includes('Native prompt') && generateTabs.includes('AI media'),
    true,
    'Generate workspace tabs preserve user-facing labels',
  )
  eq(generatePrompt.includes('data-cut-generate-prompt-panel'), true, 'Generate prompt tab component preserves the panel selector')
  eq(generatePrompt.includes('data-cut-generate-prompt-run'), true, 'Generate prompt tab component owns the run action selector')
  eq(generatePrompt.includes('data-cut-generate-prompt-preview-result'), true, 'Generate prompt tab component owns prompt preview evidence')
  eq(generatePrompt.includes('Native prompt'), true, 'Generate prompt tab component preserves user-facing wording')
  eq(generateStoryboard.includes('data-cut-generate-storyboard'), true, 'Generate storyboard tab component preserves the panel selector')
  eq(generateStoryboard.includes('data-cut-generate-storyboard-plan'), true, 'Generate storyboard tab component owns the plan action selector')
  eq(generateStoryboard.includes('data-cut-generate-storyboard-preview'), true, 'Generate storyboard tab component owns the preview action selector')
  eq(generateStoryboard.includes('data-cut-generate-storyboard-insert'), true, 'Generate storyboard tab component owns the insert action selector')
  eq(generateStoryboard.includes('questions.map'), true, 'Generate storyboard renders every clarifying question returned by the agent')
  eq(generateStoryboard.includes('storyboardResult?.questions?.[0]'), false, 'Generate storyboard does not collapse clarifying questions to the first one only')
  eq(generateStoryboard.includes('data-cut-generate-storyboard-scenes'), true, 'Generate storyboard tab component owns scene evidence')
  eq(generateStoryboard.includes('data-cut-generate-storyboard-insert-result'), true, 'Generate storyboard tab component owns insert evidence')
  eq((fullCoverage.match(/page[.]locator\('\[data-cut-left-tab="generate"\]'\)[.]click\(\)/g) || []).length >= 2, true, 'Native Generate coverage re-enters the owning workspace before prompt and storyboard actions')
  eq(fullCoverage.includes("promptPanel.waitFor({ state: 'visible', timeout: 12_000 })"), true, 'Native Generate prompt coverage requires the panel to be visibly actionable')
  eq(fullCoverage.includes("storyPanel.waitFor({ state: 'visible', timeout: 12_000 })"), true, 'Native Generate storyboard coverage requires the panel to be visibly actionable')
  eq((generateCoverage.match(/control[.]waitFor\(\{ state: 'visible', timeout: 12_000 \}\)/g) || []).length, 2, 'Native Generate control coverage waits for both fill and select controls')
  const uiOpenPanels = verbs.verbs.find((v) => v.name === 'ui.open')?.args?.properties?.panel?.enum ?? []
  eq(uiOpenPanels.includes('generate-prompt'), true, 'ui.open schema advertises the Native prompt Generate route')
  eq(uiOpenPanels.includes('generate-storyboard'), true, 'ui.open schema advertises the Storyboard Generate route')
  eq(uiOpenPanels.includes('generate-media'), true, 'ui.open schema advertises the AI media Generate route')
  for (const name of generateVerbs) {
    eq(verbNames.includes(name), true, `schema exposes ${name}`)
    eq(client.includes(`'${name}'`), true, `client types cover ${name}`)
    eq(dispatchRs.includes(`"${name}"`), true, `dispatch routes ${name}`)
    eq(registryRs.includes(`"${name}"`), true, `registry lists ${name}`)
  }
  eq(verbNames.includes(motionBridgeVerb), true, 'schema exposes motion.template_to_cut')
  eq(client.includes(`'${motionBridgeVerb}'`), true, 'client types cover motion.template_to_cut')
  eq(/"motion\.template_to_cut"\s*=>[\s\S]*motion_bridge::motion_template_to_cut\(state, args, actor\)/.test(dispatchRs), true, 'dispatch routes motion.template_to_cut through the Motion bridge module')
  eq(verbNames.includes(motionScriptBridgeVerb), true, 'schema exposes motion.script_to_cut')
  eq(client.includes(`'${motionScriptBridgeVerb}'`), true, 'client types cover motion.script_to_cut')
  eq(/"motion\.script_to_cut"\s*=>[\s\S]*motion_bridge::motion_script_to_cut\(state, args, actor\)/.test(dispatchRs), true, 'dispatch routes motion.script_to_cut through the Motion bridge module')
  eq(verbNames.includes(motionMapImportVerb), true, 'schema exposes motion.map_import')
  eq(client.includes(`'${motionMapImportVerb}'`), true, 'client types cover motion.map_import')
  eq(/"motion\.map_import"\s*=>[\s\S]*motion_bridge::motion_map_import\(state, args, actor\)/.test(dispatchRs), true, 'dispatch routes motion.map_import through the Motion bridge module')
  eq(verbNames.includes(motionApplyImportVerb), true, 'schema exposes motion.apply_import')
  eq(client.includes(`'${motionApplyImportVerb}'`), true, 'client types cover motion.apply_import')
  eq(/"motion\.apply_import"\s*=>[\s\S]*motion_bridge::motion_apply_import\(state, args, actor\)/.test(dispatchRs), true, 'dispatch routes motion.apply_import through the Motion bridge module')
  eq(verbNames.includes(motionLinkRefreshVerb), true, 'schema exposes motion.link.refresh')
  eq(client.includes(`'${motionLinkRefreshVerb}'`), true, 'client types cover motion.link.refresh')
  eq(/"motion\.link\.refresh"\s*=>[\s\S]*motion_bridge::motion_link_refresh\(state, args, actor\)/.test(dispatchRs), true, 'dispatch routes motion.link.refresh through the Motion bridge module')
  eq(verbNames.includes(motionLinkRelinkVerb), true, 'schema exposes motion.link.relink')
  eq(client.includes(`'${motionLinkRelinkVerb}'`), true, 'client types cover motion.link.relink')
  eq(/"motion\.link\.relink"\s*=>[\s\S]*motion_bridge::motion_link_relink\(state, args, actor\)/.test(dispatchRs), true, 'dispatch routes motion.link.relink through the Motion bridge module')
  eq(verbNames.includes(motionLinkEditVerb), true, 'schema exposes motion.link.edit')
  eq(client.includes(`'${motionLinkEditVerb}'`), true, 'client types cover motion.link.edit')
  eq(/"motion\.link\.edit"\s*=>[\s\S]*motion_bridge::motion_link_edit\(state, args, actor\)/.test(dispatchRs), true, 'dispatch routes motion.link.edit through the Motion bridge module')
  eq(mainRs.includes('mod motion_bridge;'), true, 'server main compiles the Motion bridge module')
  eq(existsSync(motionBridgeRsPath), true, 'Motion bridge server module exists')
  eq(
    motionRuntimeRs.includes('SHELLX_MOTION_CLI') && motionRuntimeRs.includes('SHELLX_MOTION_BIN') && motionRuntimeRs.includes('SHELLX_MOTION_ROOT'),
    true,
    'Extracted Motion runtime honors the canonical SHELLX_MOTION_CLI, the legacy SHELLX_MOTION_BIN alias, and SHELLX_MOTION_ROOT',
  )
  eq(generateHandlersRs.includes('is_motion_generate_lowering(&template.lowering.verb)'), true, 'Generate preview delegates Motion templates through Motion connector verbs')
  eq(generateHandlersRs.includes('"motion.template_to_cut"') && generateHandlersRs.includes('"motion.script_to_cut"') && generateHandlersRs.includes('"checkpoint"'), true, 'Generate insert wraps Motion connector inserts in the Generate checkpoint flow')
  eq(fullCoverage.includes(`'${motionBridgeVerb}'`) && fullCoverage.includes('Motion template bridge'), true, 'full-coverage partition classifies motion.template_to_cut intentionally')
  eq(fullCoverage.includes(`'${motionScriptBridgeVerb}'`) && fullCoverage.includes('Motion scripted-video bridge'), true, 'full-coverage partition classifies motion.script_to_cut intentionally')
  eq(fullCoverage.includes(`'${motionJobGetVerb}'`) && fullCoverage.includes(`'${motionJobListVerb}'`) && fullCoverage.includes('Motion status query'), true, 'full-coverage partition classifies read-only Motion job queries intentionally')
  eq(fullCoverage.includes(`'${motionMapImportVerb}'`) && fullCoverage.includes('Motion import-plan preflight'), true, 'full-coverage partition classifies motion.map_import intentionally')
  eq(fullCoverage.includes(`'${motionApplyImportVerb}'`) && fullCoverage.includes('Motion import-plan apply'), true, 'full-coverage partition classifies motion.apply_import intentionally')
  eq(fullCoverage.includes(`'${motionLinkRefreshVerb}'`) && fullCoverage.includes(`'${motionLinkRelinkVerb}'`), true, 'full-coverage partition covers visible linked Motion actions')
  eq(fullCoverage.includes(`'${motionLinkEditVerb}'`), true, 'full-coverage partition covers Edit in Motion')
  eq(skill.includes('motion.template_to_cut') && skill.includes('motion.script_to_cut') && skill.includes('motion.map_import') && skill.includes('motion.apply_import'), true, 'Cut agent skill mentions the Motion-backed Generate and import-plan bridges')
  eq(reference.includes('motion.template_to_cut') && reference.includes('motion.script_to_cut') && reference.includes('motion.map_import') && reference.includes('motion.apply_import'), true, 'Cut reference documents the Motion-backed bridge verbs')
  eq(featureInventory.includes('motion.template_to_cut') && featureInventory.includes('motion.script_to_cut') && featureInventory.includes('motion.map_import') && featureInventory.includes('motion.apply_import'), true, 'public feature inventory tracks the Motion-backed Generate and import-plan bridges')
  eq(skill.includes('status:"warning"') && skill.includes('successful advisory'), true, 'Cut agent skill treats warned Motion receipts as successful advisories')
  eq(reference.includes('receipt status `passed` or `warning` is successful'), true, 'Cut reference documents both successful Motion receipt statuses')
  eq(featureInventory.includes('Motion receipt status `warning` remains a successful advisory'), true, 'public feature inventory preserves warned-success semantics')
  eq(/accepts\s+`passed` and `warning` as successful receipt attestations/.test(motionBoundary), true, 'bundled Motion boundary defines warned-success receipt handling')
  eq(featureInventory.includes(motionLinkRefreshVerb) && featureInventory.includes(motionLinkRelinkVerb), true, 'public feature inventory tracks linked Motion refresh and relink')
  eq(featureInventory.includes(motionLinkEditVerb), true, 'public feature inventory tracks Edit in Motion')
  eq(
    dispatchRs.includes('"generate.list" => generate_handlers::generate_list(args).await.into()'),
    true,
    'dispatch delegates generate.list to the Generate handler module',
  )
  eq(
    dispatchRs.includes('"generate.describe" => generate_handlers::generate_describe(args).await.into()'),
    true,
    'dispatch delegates generate.describe to the Generate handler module',
  )
  eq(
    /"generate\.preview"\s*=>\s*generate_handlers::generate_preview\(state,\s*args\)\s*\.await\s*\.into\(\)/s.test(dispatchRs),
    true,
    'dispatch delegates generate.preview to the Generate handler module',
  )
  eq(
    /"generate\.insert"\s*=>\s*generate_handlers::generate_insert\(state,\s*args,\s*actor\)\s*\.await\s*\.into\(\)/s.test(dispatchRs),
    true,
    'dispatch delegates generate.insert to the Generate handler module',
  )
  eq(
    /"generate\.from_prompt"\s*=>\s*generate_handlers::generate_from_prompt\(state,\s*args,\s*actor\)\s*\.await\s*\.into\(\)/s.test(dispatchRs),
    true,
    'dispatch delegates generate.from_prompt to the Generate handler module',
  )
  eq(
    /"generate\.storyboard"\s*=>\s*generate_handlers::generate_storyboard\(state,\s*args,\s*actor\)\s*\.await\s*\.into\(\)/s.test(dispatchRs),
    true,
    'dispatch delegates generate.storyboard to the Generate handler module',
  )
  eq(
    generateHandlersRs.includes('pub(crate) fn generate_safe_id_fragment'),
    true,
    'Generate handler module owns Generate ID sanitizing helper',
  )
  eq(
    generateHandlersRs.includes('pub(crate) fn generate_preview_id'),
    true,
    'Generate handler module owns Generate preview ID helper',
  )
  eq(
    dispatchRs.includes('fn generate_safe_id_fragment'),
    false,
    'dispatch no longer defines Generate ID sanitizing helper',
  )
  eq(
    dispatchRs.includes('fn collect_generated_refs'),
    false,
    'dispatch no longer defines Generate result ref collector',
  )
  eq(
    dispatchRs.includes('async fn generate_preview('),
    false,
    'dispatch no longer owns Generate preview implementation',
  )
  eq(
    dispatchRs.includes('async fn generate_insert('),
    false,
    'dispatch no longer owns Generate insert implementation',
  )
  eq(
    dispatchRs.includes('async fn generate_from_prompt('),
    false,
    'dispatch no longer owns Generate prompt implementation',
  )
  eq(
    dispatchRs.includes('async fn generate_storyboard('),
    false,
    'dispatch no longer owns Generate storyboard implementation',
  )
  eq(
    generateHandlersRs.includes('pub(crate) async fn generate_preview'),
    true,
    'Generate handler module owns Generate preview implementation',
  )
  eq(
    generateHandlersRs.includes('pub(crate) async fn generate_insert'),
    true,
    'Generate handler module owns Generate insert implementation',
  )
  eq(
    generateHandlersRs.includes('pub(crate) async fn generate_from_prompt'),
    true,
    'Generate handler module owns Generate prompt implementation',
  )
  eq(
    generateHandlersRs.includes('pub(crate) async fn generate_storyboard'),
    true,
    'Generate handler module owns Generate storyboard implementation',
  )
  eq(mainRs.includes('mod generate;'), true, 'server main compiles the Generate module')
  eq(mainRs.includes('mod generate_handlers;'), true, 'server main compiles the Generate handler module')
}

// --- Canary STT: weak-language tier must have real word timestamps ----------
// Canary has better accuracy for the weak Parakeet languages, but it cannot be
// surfaced unless the engine converts its text output into word spans. Lock the
// full feature workflow: runtime route, packaged dependency, Environment picker,
// verb contract, skill reference, and feature list all move together.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const instruments = readFileSync(resolve(root, 'app/perception/py/instruments.py'), 'utf8')
  const perceptionTypes = readFileSync(resolve(root, 'app/perception/src/types.rs'), 'utf8')
  const perceptionSetup = readFileSync(resolve(root, 'app/server/src/perception_setup.rs'), 'utf8')
  const dispatch = readFileSync(resolve(root, 'app/server/src/dispatch.rs'), 'utf8')
  const renderingHandlers = readFileSync(resolve(root, 'app/server/src/dispatch/rendering.rs'), 'utf8')
  const sttSettingsPath = resolve(root, 'app/server/src/stt_settings.rs')
  const sttSettings = existsSync(sttSettingsPath) ? readFileSync(sttSettingsPath, 'utf8') : ''
  const requirementsFull = readFileSync(resolve(root, 'app/perception/py/requirements-full.txt'), 'utf8')
  const verbs = JSON.parse(readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')) as {
    verbs: Array<{ name: string; description?: string; args?: { properties?: Record<string, { description?: string }> } }>
  }
  const sttVerb = verbs.verbs.find((v) => v.name === 'system.set_stt_model')
  const reference = readFileSync(resolve(root, 'skill/shellx-cut/reference.md'), 'utf8')
  const featureInventory = readFileSync(resolve(root, 'docs/public/FEATURES.md'), 'utf8')
  const readme = readFileSync(resolve(root, 'README.md'), 'utf8')
  const packageJson = JSON.parse(readFileSync(resolve(root, 'ui/package.json'), 'utf8')) as { version: string }
  const interactionVerify = readFileSync(resolve(root, 'ui/public-tests/interaction-verify.mjs'), 'utf8')
  const canaryProbePython = python310ForProbe(root)
  const canarySpanProbe = spawnSync(
    canaryProbePython,
    [
      '-c',
      `
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("cut_instruments", Path("app/perception/py/instruments.py"))
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

class Span:
    def __init__(self, start, end, score):
        self.start = start
        self.end = end
        self.score = score
    def __len__(self):
        return max(1, self.end - self.start)

pairs = mod._mms_word_pairs("Čau, Jānis! Straße")
assert pairs[:3] == [("Čau", "cau"), ("Jānis", "janis"), ("Straße", "strasse")], pairs
words = mod._mms_spans_to_words(
    [("Čau", "cau"), ("pasaule", "pasaule")],
    [[Span(0, 8, 0.9)], [Span(9, 17, 0.7)]],
    ratio=0.02,
    offset_ms=1000,
    audio_end_ms=1600,
)
assert words == [
    {"word": "Čau", "start_ms": 1000, "end_ms": 1160, "confidence": 0.9},
    {"word": "pasaule", "start_ms": 1180, "end_ms": 1340, "confidence": 0.7},
], words
clipped = mod._mms_spans_to_words(
    [("valid", "valid"), ("late", "late")],
    [[Span(0, 10, 0.8)], [Span(50, 60, 0.5)]],
    ratio=0.02,
    offset_ms=1000,
    audio_end_ms=1300,
)
assert clipped == [
    {"word": "valid", "start_ms": 1000, "end_ms": 1200, "confidence": 0.8},
], clipped
assert all(w["start_ms"] < w["end_ms"] <= 1300 for w in clipped), clipped
print(json.dumps(words, ensure_ascii=False))
`,
    ],
    {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, PYTHONIOENCODING: 'utf-8', PYTHONUTF8: '1' },
    },
  )

  eq(STT_MODELS.some((m) => m.id === 'nemo-canary-1b-v2'), true, 'Environment STT picker includes Canary weak-language model')
  eq(existsSync(sttSettingsPath), true, 'server STT settings handler lives in a focused module')
  eq(dispatch.includes('async fn system_set_stt_model'), false, 'dispatch no longer owns the system.set_stt_model handler body')
  eq(
    dispatch.includes('"system.set_stt_model"')
      && dispatch.includes('crate::stt_settings::system_set_stt_model(state, args)')
      && dispatch.includes('.await')
      && dispatch.includes('.into()'),
    true,
    'dispatch delegates system.set_stt_model to the STT settings module',
  )
  eq(sttSettings.includes('pub(crate) async fn system_set_stt_model'), true, 'STT settings module owns the public handler')
  eq(sttSettings.includes('cut_perception::write_stt_setting'), true, 'STT settings module persists the active model/language')
  eq(sttSettings.includes('cut_perception::read_stt_setting'), true, 'STT settings module returns the active model/language')
  eq(sttSettings.includes('"nemo-parakeet-tdt-0.6b-v3"'), true, 'STT settings module preserves the default Parakeet v3 contract')
  eq(instruments.includes('def _words_via_canary'), true, 'Perception sidecar has a Canary transcription route')
  eq(instruments.includes('torchaudio.pipelines.MMS_FA'), true, 'Canary route uses MMS_FA forced alignment for word timestamps')
  eq(instruments.includes('+mms-fa'), true, 'Canary transcript provenance records forced alignment')
  eq(instruments.includes('librosa.beat.beat_track'), false, 'Perception beat grid avoids librosa beat_track segfault path')
  eq(instruments.includes('wave.open') && instruments.includes('np.frombuffer'), true, 'Perception beat grid uses lightweight WAV energy analysis')
  eq(
    instruments.includes('fps_source = "fallback"') && instruments.includes('"fps_source": fps_source') && instruments.includes('"fps_warning"'),
    true,
    'SubjectTrack marks OpenCV FPS fallback explicitly instead of presenting 30fps as measured',
  )
  eq(
    perceptionTypes.includes('pub fps_source: String') && perceptionTypes.includes('pub fps_warning: Option<String>'),
    true,
    'Rust SubjectTrack preserves FPS fallback provenance from the sidecar',
  )
  eq(
    instruments.includes('face_aware = face is not None') && instruments.includes('"face_aware": face_aware'),
    true,
    'SubjectTrack marks when face-aware framing was actually available',
  )
  eq(
    perceptionTypes.includes('pub face_aware: bool'),
    true,
    'Rust SubjectTrack preserves face-aware framing provenance from the sidecar',
  )
  eq(
    renderingHandlers.includes('"subject_fps_source": subject_fps_source') && renderingHandlers.includes('subject_fps_warning') && renderingHandlers.includes('"face_aware": face_aware'),
    true,
    'Reframe receipt surfaces subject FPS fallback and face-awareness provenance',
  )
  eq(instruments.includes('read_audio'), false, 'Silero VAD does not use torchaudio audio I/O that requires torchcodec')
  eq(instruments.includes('soundfile as sf') && instruments.includes('torch.from_numpy'), true, 'Silero VAD reads extracted WAV without torchcodec')
  eq(/^soundfile\b/m.test(requirementsFull), true, 'Full perception requirements include soundfile for silence analysis')
  eq(/"soundfile"/.test(perceptionSetup), true, 'Perception setup installs soundfile for the full silence-analysis path')
  eq(/^torchaudio\b/m.test(requirementsFull), true, 'Full perception requirements include torchaudio for MMS_FA')
  eq(/^torchcodec\b/m.test(requirementsFull), false, 'Full perception requirements do not require unused torchcodec DLLs')
  eq(/^librosa\b/m.test(requirementsFull), false, 'Full perception requirements no longer install librosa for beat analysis')
  eq(/^transformers\b/m.test(requirementsFull), true, 'Full perception requirements include transformers for local translation fallback')
  eq(/^sentencepiece\b/m.test(requirementsFull), true, 'Full perception requirements include sentencepiece for Opus-MT tokenizers')
  eq(/"sentencepiece"/.test(perceptionSetup), true, 'Perception setup installs sentencepiece for local translation tokenizers')
  eq(sttVerb?.description?.includes('nemo-canary-1b-v2'), true, 'system.set_stt_model schema advertises Canary')
  eq(sttVerb?.args?.properties?.language?.description?.includes('Canary'), true, 'system.set_stt_model language hint mentions Canary')
  eq(reference.includes('nemo-canary-1b-v2') && reference.includes('MMS_FA'), true, 'skill reference documents Canary + MMS_FA timestamps')
  eq(reference.includes('torchcodec audio I/O'), false, 'skill reference does not tell agents torchcodec is required')
  eq(featureInventory.includes('Canary') && featureInventory.includes('MMS_FA'), true, 'public feature inventory records the shipped Canary timestamp tier')
  eq(readme.includes('librosa beats'), false, 'README sidecar map does not advertise removed librosa beat analysis')
  eq(featureInventory.includes('needs pyannote+HF'), false, 'public feature inventory no longer describes diarization as a parked pyannote/HF gap')
  eq(featureInventory.includes('Sortformer v2') && featureInventory.includes('mode:"speaker"'), true, 'public feature inventory documents shipped Sortformer speaker switching path')
  eq(reference.startsWith('# ShellX Cut') && reference.includes(`v${packageJson.version}`), true, 'skill reference headline matches current app version')
  eq(reference.includes('mode:"speaker"') || reference.includes('mode:"speaker"'), true, 'skill reference documents multicam speaker mode')
  eq(reference.includes('energy-only (no face / diarization'), false, 'skill reference no longer claims multicam is energy-only')
  eq(
    STT_MODELS.every((m) => /Parakeet|Canary|Whisper/.test(m.label)),
    true,
    'Environment STT picker labels name the actual model/runtime',
  )
  eq(interactionVerify.includes('nemo-parakeet-tdt-0.6b-v2'), false, 'interaction STT gate no longer expects removed Parakeet v2')
  eq(interactionVerify.includes('nemo-canary-1b-v2'), true, 'interaction STT gate verifies the Canary timestamp tier')
  eq(interactionVerify.includes('whisperx-large-v3'), true, 'interaction STT gate verifies the Whisper fallback option')
  eq(
    canarySpanProbe.status,
    0,
    `Canary MMS_FA span mapping emits normalized, timestamped words via ${canaryProbePython}${canarySpanProbe.stderr ? ` (${canarySpanProbe.stderr.trim().slice(0, 160)})` : ''}`,
  )
}

// --- Release gate perception preflight must cover the full sidecar battery ---
// A venv that imports cv2+torch+torchvision can still crash later when the
// sidecar reaches silence/scenes. The release gate should reject that venv up
// front, before it spends a full UI sweep and reports downstream false failures.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const gate = readFileSync(resolve(root, 'scripts/release/full-coverage-gate.mjs'), 'utf8')
  const sweep = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const setupRs = readFileSync(resolve(root, 'app/server/src/perception_setup.rs'), 'utf8')
  const sidecarRs = readFileSync(resolve(root, 'app/perception/src/sidecar.rs'), 'utf8')

  for (const mod of ['torchaudio', 'silero_vad', 'scenedetect', 'supervision', 'rapidocr_onnxruntime']) {
    eq(gate.includes(mod), true, `Release gate wrapper preflight imports ${mod}`)
    eq(sweep.includes(mod), true, `Full coverage sweep preflight imports ${mod}`)
  }
  eq(
    !gate.includes("'torchcodec'") && !sweep.includes("'torchcodec'") && !setupRs.includes('"torchcodec"'),
    true,
    'Release preflight and core perception setup do not require unused torchcodec',
  )
  eq(
    sidecarRs.includes('STALE_SIDECARENV_PACKAGES') && sidecarRs.includes('"torchcodec"'),
    true,
    'Perception sidecar cleans stale torchcodec from older managed venvs on upgrade',
  )
  eq(
    gate.includes('full sidecar') || gate.includes('silence / scenes'),
    true,
    'Release gate failure text explains full sidecar coverage, not only cv2/torch',
  )
}

// --- Desktop packaging ships every runtime runner a visible card advertises ---
// Windows CDP caught Dub/Diarize showing "Runner not found" after install: the
// repo runtime had the scripts, but tauri.conf/build payload guards did not ship
// them. Keep the bundle resource map and both platform build guards in sync.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const tauriConf = readFileSync(resolve(root, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8')
  const buildWindows = readFileSync(resolve(root, 'scripts/build-windows.sh'), 'utf8')
  const buildMacos = readFileSync(resolve(root, 'scripts/build-macos.sh'), 'utf8')
  for (const runner of ['dub_runner.py', 'diarize_runner.py', 'safe_numbers.py']) {
    eq(tauriConf.includes(`../../perception/py/${runner}`), true, `desktop resource map ships ${runner}`)
    eq(buildWindows.includes(`app/perception/py/${runner}`), true, `Windows build payload guard checks ${runner}`)
    eq(buildMacos.includes(`app/perception/py/${runner}`), true, `macOS build payload guard checks ${runner}`)
  }
}

// --- Release gate external-agent fixtures: deterministic, real effects -------
// The exhaustive full-coverage gate must not fail because a paid/interactive
// agent session produced no image, no actionable comment draft, or no judge
// verdict. The gate uses explicit local fixtures for these external-agent seams,
// while still proving the real app effects: agent.chat lands edits through the
// verb API, translations return timestamp-preserving cue text, assets.generate
// imports a generated media file, comment.apply executes a drafted edit, and
// verify.judge updates a receipt through the normal async job path.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const gate = readFileSync(resolve(root, 'scripts/release/full-coverage-gate.mjs'), 'utf8')
  const sweep = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const draftFixture = resolve(root, 'scripts/release/fixtures/comment-draft-adapter.py')
  const judgeFixture = resolve(root, 'scripts/release/fixtures/judge-adapter.py')
  const codexFixture = resolve(root, 'scripts/release/fixtures/codex')
  const claudeFixture = resolve(root, 'scripts/release/fixtures/claude')
  const agentEditFixture = resolve(root, 'scripts/release/fixtures/agent-edit-fixture.mjs')
  const generatedLifecycle = resolve(root, 'ui/public-tests/verify-generated-asset-lifecycle.mjs')
  const publishPackage = resolve(root, 'ui/public-tests/verify-publish-package.mjs')
  const uiPackage = readFileSync(resolve(root, 'ui/package.json'), 'utf8')

  eq(existsSync(draftFixture), true, 'full-coverage gate has a deterministic comment.draft fixture')
  eq(existsSync(judgeFixture), true, 'full-coverage gate has a deterministic verify.judge fixture')
  eq(existsSync(codexFixture), true, 'full-coverage gate has a deterministic assets.generate and agent.chat codex fixture')
  const codexFixtureText = readFileSync(codexFixture, 'utf8')
  eq(codexFixtureText.includes('deterministic cancelled generated slot'), true, 'generation fixture reserves a deterministic UI cancellation window')
  eq(codexFixtureText.includes('Math.max(configuredDelayMs, 5_000)'), true, 'cancellation fixture remains observable on slower native WebViews')
  eq(existsSync(claudeFixture), true, 'full-coverage gate has a deterministic agent.chat/translation claude fixture')
  eq(existsSync(agentEditFixture), true, 'full-coverage gate shares real verb mutations across CLI fixtures')
  eq(existsSync(generatedLifecycle), true, 'generated-asset lifecycle has a focused real-UI verifier')
  eq(uiPackage.includes('verify-generated-lifecycle'), true, 'package scripts expose generated-asset lifecycle verification')
  const lifecycleText = readFileSync(generatedLifecycle, 'utf8')
  for (const proof of ['one_provider_run', 'immutable_source', 'provenance', 'scratch_clean', 'cost_honest', 'reuse_visible', 'cancel_terminal', 'cancel_no_asset', 'cancel_visible']) {
    eq(lifecycleText.includes(proof), true, `generated-asset verifier proves ${proof}`)
  }
  eq(existsSync(publishPackage), true, 'publish-package lifecycle has a focused runtime verifier')
  eq(uiPackage.includes('verify-publish-package'), true, 'package scripts expose publish-package verification')
  const publishText = readFileSync(publishPackage, 'utf8')
  for (const proof of ['terminal_status', 'manifest_exists', 'manifest_contract', 'video_hash', 'thumbnail_hash', 'brand_bound', 'blocked_visible', 'manifest_visible', 'minimum_window', 'no_browser_errors']) {
    eq(publishText.includes(proof), true, `publish-package verifier proves ${proof}`)
  }
  eq(gate.includes('FCV_AGENT_FIXTURES'), true, 'full-coverage gate exposes fixture mode')
  eq(gate.includes('claudeFixture'), true, 'full-coverage gate reports the claude fixture in fixture mode')
  eq(gate.includes('CUTD_DRAFT_ADAPTER'), true, 'full-coverage gate wires the comment.draft fixture into cutd')
  eq(gate.includes('CUTD_JUDGE_ADAPTER'), true, 'full-coverage gate wires the verify.judge fixture into cutd')
  eq(
    gate.includes('FIXTURE_DIR') && gate.includes('prependEnvPath(env, FIXTURE_DIR)'),
    true,
    'full-coverage gate prepends release fixtures through the cross-platform PATH helper',
  )
  eq(gate.includes("key.toLowerCase() === 'path'"), true, 'full-coverage gate resolves Windows environment keys case-insensitively')
  eq(gate.includes('function onPath(cmd, env = process.env)'), true, 'full-coverage gate PATH lookup can use the prepared env')
  eq(gate.includes('function probeClaude(env = process.env)'), true, 'full-coverage gate claude probe can use fixture-adjusted PATH')
  eq(gate.includes('claude: probeClaude(env)'), true, 'full-coverage gate preflight probes claude after fixtures are on PATH')
  eq(gate.includes('assessExternalFixtureContract'), true, 'external full-coverage runs enforce fixture inheritance by the already-running cutd')
  eq(gate.includes('FCV_EXTERNAL_FIXTURES_READY'), true, 'external fixture inheritance requires an explicit rig-launch acknowledgment')
  eq(
    gate.includes('status >= 200') && gate.includes('status < 300'),
    true,
    'full-coverage gate requires a successful service /health response, not just an accepted TCP/HTTP connection',
  )
  eq(
    sweep.includes('aiServiceDetail') && sweep.includes('media.diarize ok=${d.ok}') && sweep.includes('audio.dub ok=${du.ok}'),
    true,
    'full-coverage AI service rows include sidecar error details when diarize/dub fail',
  )
}

// --- Translation backend policy: auto never hides a launched CLI failure -----
// User-requested dubbing/translation policy: the CLI agent is the default path.
// Local MT is only the no-installed-CLI fallback (or an explicit backend:"local" request),
// not a quota/auth/runtime fallback after a CLI agent exists and fails.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const translate = readFileSync(resolve(root, 'app/server/src/translate.rs'), 'utf8')
  const workflows = readFileSync(resolve(root, 'scripts/windows/installed-ai-workflows.mjs'), 'utf8')

  eq(
    translate.includes('auth/quota/runtime') && translate.includes('silently degrading to local MT'),
    true,
    'server translation policy documents that CLI runtime failures stay honest',
  )
  eq(
    translate.includes('CLI translation failed in auto mode; used local translator instead'),
    false,
    'server no longer emits an auto-mode CLI-failure local fallback warning',
  )
  eq(
    workflows.includes('default auto: CLI agent; local only when no CLI is installed'),
    true,
    'Windows AI workflow documents the installed-app translation policy',
  )
  eq(
    workflows.includes('fallbackWarningOk'),
    false,
    'Windows AI workflow no longer accepts CLI-failure local fallback as passing',
  )
  eq(
    workflows.includes("dubResult.translate_backend === 'local' && !cliInstalled"),
    true,
    'Windows AI workflow accepts auto/local only when no CLI agent is installed',
  )
}

// --- Full-coverage harness waits: state polling must be locally bounded ------
// A focused macOS release slice hung after transcript.cut_words because the reel
// proof's waitForState() poll inherited the global 60s verb timeout. The outer
// wait said "10s", but a single stalled project.state fetch could take a minute.
// Keep the polling helper on a short state timeout and keep reel trace points so
// future hangs identify the exact await.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const fcv = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const jobWaiters = resolve(root, 'ui/public-tests/lib/fullCoverageJobs.mjs')
  const projectWaiters = resolve(root, 'ui/public-tests/lib/fullCoverageProject.mjs')
  const visualProof = resolve(root, 'ui/public-tests/lib/fullCoverageVisual.mjs')
  const projectWaitersText = existsSync(projectWaiters) ? readFileSync(projectWaiters, 'utf8') : ''
  const visualProofText = existsSync(visualProof) ? readFileSync(visualProof, 'utf8') : ''
  const topbarLibraryGate = readFileSync(resolve(root, 'ui/public-tests/verify-topbar-library.mjs'), 'utf8')

  eq(fcv.includes('STATE_POLL_TIMEOUT_MS'), true, 'full-coverage waitForState has a local project.state timeout')
  eq(projectWaitersText.includes('statePollTimeoutMs'), true, 'project-state waiter helper accepts a local project.state timeout')
  eq(projectWaitersText.includes('state({ timeoutMs: statePollTimeoutMs })'), true, 'waitForState does not inherit the global verb timeout')
  eq(fcv.includes('UI_ACTION_TIMEOUT_MS'), true, 'full-coverage harness defines a bounded UI action timeout')
  eq(fcv.includes('page.setDefaultTimeout(UI_ACTION_TIMEOUT_MS)'), true, 'full-coverage harness applies the UI action timeout to Playwright locators')
  eq(fcv.includes('cleanupTmp()'), true, 'full-coverage harness cleans its /tmp/fcv working directory before exit')
  eq(
    fcv.includes('maxRetries: 20') && fcv.includes('retryDelay: 250'),
    true,
    'full-coverage cleanup tolerates transient Windows file-handle release without leaving generated temp data',
  )
  eq(
    fcv.includes("FCV_DEFER_TEMP_CLEANUP === '1'") && fcv.includes('if (!DEFER_TEMP_CLEANUP) cleanupTmp()'),
    true,
    'an installed runner can defer its exact temp cleanup until after the app releases output-directory handles',
  )
  eq(fcv.includes('seedMenuToolProject(page)'), true, 'menus gate uses a deterministic transcript/scenes fixture before probing Tools menu effects')
  eq(
    fcv.includes("schema: 'shellx-cut/perception/1'")
      && fcv.includes("const durationMs = Number(sourceProbe.duration_ms)")
      && fcv.includes("scenes: [{ at_ms: sceneAtMs"),
    true,
    'menus gate seeds duration-aware silence and scene facts inside the imported clip',
  )
  eq(
    fcv.includes('await awaitImportJobs(imp, FCV_IMPORT_DRAIN_TIMEOUT_MS)'),
    true,
    'menus gate waits for terminal import/enrichment before writing seeded receipts',
  )
  eq(
    fcv.includes('FCV_IMPORT_DRAIN_TIMEOUT_MS || 600000'),
    true,
    'installed full coverage gives real perception enrichment a bounded ten-minute release window',
  )
  eq(
    fcv.includes('activeJobSummary()'),
    true,
    'project isolation failures preserve the active job identities instead of reporting an opaque timeout',
  )
  eq(fcv.includes('fixture@menu-tools'), true, 'menus gate uses a compact transcript with guaranteed edge trim')
  eq(fcv.includes('const localProjectPath = resolveDriverPath(projectPath)'), true, 'menus gate maps native project paths before writing seeded receipts from WSL')
  eq(fcv.includes('writeSeededMenuReceipts(localProjectPath, assetId)'), true, 'menus gate writes seeded receipts into the driver-visible project path')
  eq(fcv.includes('project.open') && fcv.includes("unlinkSync(join(localProjectPath, 'project.json'))"), true, 'menus gate reopens after seeded receipts so derived transcript/perception links are reconciled')
  eq(topbarLibraryGate.includes('seedScenePerception'), true, 'topbar/library verifier seeds deterministic scene perception for cold dev homes')
  eq(topbarLibraryGate.includes('`${asset}.perception.json`'), true, 'topbar/library verifier writes the scene receipt consumed by scene tools')
  eq(topbarLibraryGate.includes('scene_cut.perception.json'), true, 'topbar/library verifier keeps scene-cut receipt data visible in the fixture contract')
  eq(topbarLibraryGate.includes('seedTrimDeadAirTranscript'), true, 'topbar/library verifier seeds deterministic words for Trim dead air')
  eq(topbarLibraryGate.includes('STT produced 0 transcript words'), false, 'topbar/library verifier no longer skips Trim dead air on empty live STT')
  eq(topbarLibraryGate.includes('library-actions-visible-in-first-viewport'), true, 'topbar/library verifier checks Library row actions stay visible in the first viewport')
  eq(topbarLibraryGate.includes('library-unselected-selects-are-empty-boxes'), true, 'topbar/library verifier checks unselected Library rows do not look selected')
  eq(fcv.includes('[data-cut-panel="topbar"] [data-cut-project]'), true, 'project rename probe scopes the project title to the topbar')
  eq(fcv.includes("page.locator('[data-cut-project]').click().catch"), false, 'project rename probe does not swallow failed project-title clicks')
  eq(fcv.includes('waitForStoryboardSettled'), true, 'storyboard probe waits for the async contact sheet to settle')
  eq(fcv.includes('FCV_DRAIN_IMPORTS'), true, 'full-coverage harness has a low-load import drain flag')
  eq(fcv.includes('const imported = await verb(\'media.import\''), true, 'freshProject captures the media.import response')
  eq(fcv.includes('await awaitImportJobs(imported'), true, 'freshProject drains media.import/enrich jobs before the next section starts')
  eq(existsSync(jobWaiters), true, 'full-coverage job waiters live in a helper module')
  eq(fcv.includes("from './lib/fullCoverageJobs.mjs'"), true, 'full-coverage harness imports job waiters from the helper module')
  eq(fcv.includes('createJobWaiters({ verb, sleep })'), true, 'full-coverage harness binds imported job waiters to verb/sleep')
  eq(fcv.includes('async function awaitJob('), false, 'full-coverage harness does not own the job polling implementation inline')
  eq(fcv.includes('async function awaitImportJobs('), false, 'full-coverage harness does not own import-drain implementation inline')
  eq(existsSync(projectWaiters), true, 'full-coverage project state/op waiters live in a helper module')
  eq(fcv.includes("from './lib/fullCoverageProject.mjs'"), true, 'full-coverage harness imports project state/op waiters from the helper module')
  eq(fcv.includes('createProjectWaiters({ verb, sleep, statePollTimeoutMs: STATE_POLL_TIMEOUT_MS })'), true, 'full-coverage harness binds project waiters to verb/sleep/timeouts')
  eq(fcv.includes('async function state('), false, 'full-coverage harness does not own project.state polling inline')
  eq(fcv.includes('async function ops('), false, 'full-coverage harness does not own project.ops polling inline')
  eq(fcv.includes('async function opsLen('), false, 'full-coverage harness does not own opsLen inline')
  eq(fcv.includes('async function waitForState('), false, 'full-coverage harness does not own waitForState inline')
  eq(fcv.includes('async function opLanded('), false, 'full-coverage harness does not own opLanded inline')
  eq(projectWaitersText.includes('function flatClips('), true, 'project helper owns flattened clip traversal')
  eq(projectWaitersText.includes('function findClip('), true, 'project helper owns clip lookup by id')
  eq(fcv.includes('const flatClips ='), false, 'full-coverage harness does not own flatClips inline')
  eq(fcv.includes('const findClip ='), false, 'full-coverage harness does not own findClip inline')
  eq(existsSync(visualProof), true, 'full-coverage visual proof helpers live in a helper module')
  eq(fcv.includes("from './lib/fullCoverageVisual.mjs'"), true, 'full-coverage harness imports visual proof helpers from the helper module')
  eq(fcv.includes('ffmpegBin: HARNESS_FFMPEG'), true, 'full-coverage harness gives visual proof the cross-host harness ffmpeg')
  eq(fcv.includes('async function frame('), false, 'full-coverage harness does not own render.frame capture inline')
  eq(fcv.includes('function ssim('), false, 'full-coverage harness does not own SSIM calculation inline')
  eq(fcv.includes('function shotPath('), false, 'full-coverage harness does not own screenshot path generation inline')
  eq(fcv.includes('async function renderGroup('), false, 'full-coverage harness does not own render-group screenshot proof inline')
  eq(visualProofText.includes('base64ToBuffer'), true, 'visual proof helper decodes inline render.frame payloads safely')
  eq(visualProofText.includes('function frameExt') && visualProofText.includes('expectPng: ext ==='), true, 'visual proof helper does not assume render.frame inline bytes are PNG')
  eq(visualProofText.includes('copyFileSync(src, dst)'), true, 'path-backed composed frames copy through the cross-platform Node filesystem API')
  eq(visualProofText.includes("spawnSync('cp'"), false, 'Windows full coverage never depends on a Unix cp executable')
  eq(visualProofText.includes('{ timeoutMs: 120_000 }'), true, 'composed-frame evidence has an explicit installed-runtime timeout')
  eq(visualProofText.includes('attempt < 2'), true, 'missing composed-frame evidence gets one bounded read-only retry')
  eq(visualProofText.includes("'_frame-failures'") && visualProofText.includes('resultKeys'), true, 'missing composed frames retain response-shape diagnostics before installed temp cleanup')
  eq(visualProofText.includes('new Map()'), true, 'visual proof helper keeps renderGroup screenshot caching')
  eq(visualProofText.includes('const el = locator\n'), true, 'visual proof preserves an already narrowed native locator instead of resetting its index')
  eq(visualProofText.includes('const el = locator.first()'), false, 'visual proof never replaces caller-owned nth locator selection')
  eq(visualProofText.includes('window.innerWidth') && visualProofText.includes('window.innerHeight'), true, 'visual proof judges native geometry against the live CSS viewport')
  eq(visualProofText.includes("ffmpegBin = 'ffmpeg'") && visualProofText.includes('spawnSync(ffmpegBin'), true, 'visual proof helper uses the configured harness ffmpeg for SSIM')
  eq(visualProofText.includes('attempt < 3') && visualProofText.includes("timeout: 30_000"), true, 'visual proof helper retries only the transient SSIM calculation')
  eq((visualProofText.match(/format=yuv420p/g) || []).length === 2, true, 'SSIM normalizes both composed frames to one pixel format before comparison')
  eq(visualProofText.includes("'_ssim-failures'") && visualProofText.includes("'ffmpeg.json'"), true, 'SSIM failures retain exact inputs and process diagnostics before installed temp cleanup')
  eq(fcv.includes('try { await act() } catch'), false, 'shared verb response capture never hides a failed UI action')
  eq(fcv.indexOf("name: 'auto-zoom'") < fcv.indexOf("name: 'inspector-replace-source'"), true, 'auto-zoom coverage runs before destructive Replace source and speed edits shorten the analyzed clip')
  eq(fcv.includes('const effectFrameMs =') && fcv.includes('frame(effectFrameMs)'), true, 'effect coverage samples strictly inside the retimed clip instead of a half-open endpoint')
  eq(fcv.includes("const FACE = process.env.RELEASE_CLIP_FACE || media('face_hq.mp4', join(TESTDATA, 'moving_face.mp4')"), true, 'full-coverage harness owns a dedicated detector-proven face fixture role')
  eq(fcv.includes("const imp = await verb('media.import', { path: FACE })"), true, 'face-redaction probes import FACE, not the speech/transcript fixture')
  eq(fcv.includes('FACE   '), true, 'full-coverage media plan prints the dedicated face fixture path')
  eq(fcv.includes('FACE_DETECT_MS'), true, 'face-redaction probes use a named known-good detection timestamp')
  eq(fcv.includes('at_ms: FACE_DETECT_MS'), true, 'face-redaction probes do not hard-code a stale 2000ms timestamp')
  eq(fcv.includes('awaitImportJobs(imp'), true, 'face fixture imports wait for media.import/enrich jobs')
  eq(fcv.includes('ensureAssetPerception(asset'), true, 'face fixture imports force perception analysis before face-dependent actions')
  eq(fcv.includes("await expandInspectorSection(page, 'engagement')"), true, 'engagement scoring expands its collapsed inspector section before probing the button')
  eq(fcv.includes("await runMaybeJob('audio.dub', { target_lang: 'lv', asset }, 300000)"), true, 'native dubbing uses a bounded service-aware timeout')
  eq(fcv.includes("await awaitJob(res.job_id, 180000)"), true, 'assets.generate follows its queued job to a terminal result')
  eq(fcv.includes("t.kind === 'audio' && (t.clips || []).some((c) => c.asset)"), true, 'mixer verification selects the audio speech track required by edit.duck and the visible mixer')
  eq(fcv.includes("const t2create = await verb('project.create'"), true, 'Projects setup does not reuse a stale form after the visible create remounts the app')
  eq(fcv.includes('classifyFullCoverageRow,'), true, 'full coverage imports the shared receipt classifier')
  eq(fcv.includes('classification: classifyFullCoverageRow(row)'), true, 'review handoff rows use the shared receipt classifier')
  eq(fcv.includes('await add({'), false, 'review QC does not call an undefined probe helper')
  eq(fcv.includes('function fileBytes(path)'), true, 'render.bundle validates the persisted manifest with a defined file-size helper')
  eq(fcv.includes("verb('ui.state'"), true, 'selectClip verifies the UI-selected clip through ui.state')
  eq(fcv.includes('selected_clip_ids'), true, 'selectClip checks the selected_clip_ids relay field')
  eq(fcv.includes("await freshProject(page, 'video', SPEECH)"), true, 'video inspector section boots on the subject-backed speech fixture')
  eq(fcv.includes('ensureAssetPerception(originalVideoAsset'), true, 'auto-zoom forces perception on the original selected video clip asset')
  eq(fcv.includes("captureVerbResp(page, 'edit.auto_zoom'"), true, 'auto-zoom captures the exact UI verb response for actionable failure evidence')
  eq(fcv.includes('findClip(st, clip)?.keyframes'), true, 'auto-zoom assertion checks the selected subject clip, not a stale clip id')
  eq(fcv.includes('autoZoomKfChanged'), true, 'auto-zoom assertion accepts durable keyframe-state evidence')
  eq(fcv.includes('ensureAssetPerception(baseAsset'), true, 'multicam switch seeds perception for the base angle')
  eq(fcv.includes('ensureAssetPerception(a2'), true, 'multicam switch seeds perception for the shifted angle')
  eq(fcv.includes('colorMatchLanded'), true, 'color-match assertion reports the actual lowered-op landing state')
  eq(fcv.includes('colorMatchGradeChanged'), true, 'color-match assertion accepts durable grade-state evidence when the lowered op is late')
  eq(fcv.includes("await freshProject(page, 'ctx-transition', SPEECH)"), true, 'context-menu add-transition proof resets to a deterministic seam project')
  eq(fcv.includes("await freshProject(page, 'ctx-remove-gap', SPEECH)"), true, 'context-menu remove-gap proof resets before destructive deletion')
  eq(fcv.includes("await freshProject(page, 'ctx-remove', SPEECH)"), true, 'context-menu remove proof resets before destructive deletion')
  const storyboardUiVerify = readFileSync(resolve(root, 'ui/public-tests/generate-storyboard-ui-verify.mjs'), 'utf8')
  eq(storyboardUiVerify.includes('waitForStoryboardPreviewImageLoaded'), true, 'focused storyboard UI verifier waits for decoded preview images')
  eq(fcv.includes('waitForGeneratePreviewImageLoaded'), true, 'full-coverage Generate section waits for decoded preview images')
  eq(fcv.includes("trace(S, 'reel-assemble-direct'"), true, 'transcript reel direct assembly has a trace marker')
  eq(fcv.includes("trace(S, 'reel-wait-timeline'"), true, 'transcript reel timeline wait has a trace marker')
}

// --- Feature additions must update every release-facing surface ---------------
// This is the file-backed replacement for "remember to update the feature list,
// debug surface, skill docs, and test harness". Keep the workflow explicit and
// linked from the agent rules, and keep the exhaustive gate's schema-verb
// classification language in sync with it.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const workflowPath = resolve(root, 'docs/public/FEATURE_CHANGE_WORKFLOW.md')
  const workflow = readFileSync(workflowPath, 'utf8')
  const agents = readFileSync(resolve(root, 'AGENTS.md'), 'utf8')
  const fcv = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const coverageAudit = readFileSync(resolve(root, 'scripts/coverage-audit.sh'), 'utf8')

  eq(existsSync(workflowPath), true, 'feature-change workflow document exists')
  eq(agents.includes('docs/public/FEATURE_CHANGE_WORKFLOW.md'), true, 'AGENTS.md points feature work at the workflow doc')
  for (const phrase of [
    'Contract',
    'Engine',
    'Code Placement and Ownership',
    'Human UI',
    'Environment and Installables',
    'Debug Surface',
    'Agent Skill',
    'Public Docs',
    'Tests and Harnesses',
    'Packaging and release checks',
  ]) {
    eq(workflow.includes(phrase), true, `feature workflow names required surface: ${phrase}`)
  }
  for (const phrase of [
    'schema/verbs.json',
    'ui.open',
    'ui.state',
    'ui.screenshot',
    'skill/shellx-cut/SKILL.md',
    'skill/shellx-cut/reference.md',
    'ui/public-tests/full-coverage-verify.mjs',
    'scripts/release/full-coverage-gate.mjs',
  ]) {
    eq(workflow.includes(phrase), true, `feature workflow names update target: ${phrase}`)
  }
  eq(
    workflow.includes('Do not add substantial feature logic to monolithic shell files') &&
      workflow.includes('dispatch, app shell, or panel index files'),
    true,
    'feature workflow requires a code-placement decision before growing monolithic files',
  )
  eq(
    agents.includes('Code placement') && agents.includes('avoid growing already-large'),
    true,
    'AGENTS.md points feature work at the code-placement rule',
  )
  for (const state of ['Human UI', 'Agent-only intentional', 'Internal helper', 'Rig-only', 'Parked']) {
    eq(workflow.includes(`| ${state} |`), true, `feature workflow classifies ${state}`)
  }
  eq(
    fcv.includes('Every one of the verbs') && fcv.includes('schema/verbs.json') && fcv.includes('KNOWN_NON_UI_VERBS'),
    true,
    'full-coverage gate keeps explicit schema-verb UI/non-UI classifications',
  )
  eq(
    coverageAudit.includes('REST coverage') && coverageAudit.includes('MCP coverage') && coverageAudit.includes('schema/verbs.json'),
    true,
    'coverage audit proves schema verbs are exposed on REST and MCP',
  )
  eq(
    coverageAudit.includes('debug.screenshot') && coverageAudit.includes('structural-audit'),
    true,
    'coverage audit validates debug.screenshot without launching a real OS capture',
  )
}

// --- Render queue output selection must be native-picker first ----------------
// The render queue cannot expect non-specialist users to
// type full output paths as the only path. Keep the manual input as fallback,
// but every row needs a desktop save picker with stable debug selectors.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const modal = readFileSync(resolve(srcRoot, 'topbar/RenderQueueModal.tsx'), 'utf8')
  const css = readFileSync(resolve(srcRoot, 'topbar/renderqueue.css'), 'utf8')
  const tauri = readFileSync(resolve(srcRoot, 'lib/tauri.ts'), 'utf8')
  const desktopShell = readFileSync(resolve(root, 'app/desktop/src-tauri/src/lib.rs'), 'utf8')
  const fcv = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const interaction = readFileSync(resolve(root, 'ui/public-tests/interaction-verify.mjs'), 'utf8')

  eq(tauri.includes('export async function pickRenderOutput'), true, 'Tauri bridge exposes a render-output save picker')
  eq(tauri.includes('const { save } = await import(\'@tauri-apps/plugin-dialog\')'), true, 'Render output picker uses the native save dialog')
  eq(tauri.includes('export async function confirmAction'), true, 'Tauri bridge exposes fail-closed destructive confirmation')
  eq(tauri.includes("const { confirm } = await import('@tauri-apps/plugin-dialog')"), true, 'Destructive confirmation uses the supported dialog module API')
  eq(tauri.includes('export async function showMessage'), true, 'Tauri bridge exposes supported actionable messages')
  eq(desktopShell.includes('.permission("dialog:allow-save")'), true, 'Selected engine origin allows the render-output save dialog')
  eq(desktopShell.includes('.permission("dialog:allow-message")'), true, 'Selected engine origin allows user-initiated confirm and alert dialogs')
  eq(desktopShell.includes('.permission("dialog:allow-ask")'), false, 'Selected engine origin does not grant the removed ask alias')
  eq(desktopShell.includes('.permission("dialog:allow-confirm")'), false, 'Selected engine origin does not grant the removed confirm alias')
  eq(desktopShell.includes('validated_engine_origin'), true, 'Desktop validates one exact loopback engine origin before granting native helpers')
  eq(modal.includes("import { isTauri, pickRenderOutput } from '../lib/tauri'"), true, 'RenderQueueModal imports the output picker helper')
  eq(modal.includes('data-cut-render-queue-output-pick'), true, 'Render queue rows expose a stable output-picker selector')
  eq(modal.includes('Choose output file') && modal.includes('onClick={() => void chooseOutput(i)}'), true, 'Render queue rows provide a picker action beside the manual path field')
  eq(modal.includes('output file (optional'), false, 'Render queue no longer leads with raw-path-only placeholder copy')
  eq(cssBlock(css, '.rq-pick').includes('inline-flex'), true, 'Render queue output picker has a stable icon-button style')
  eq(fcv.includes('data-cut-render-queue-output-pick'), true, 'Full coverage gate accounts for the render-queue output picker')
  eq(interaction.includes('data-cut-render-queue-output-pick'), true, 'Interaction gate checks the render-queue output picker selector')
}

// --- OTIO import is previewed and hash-bound before timeline replacement -------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const modal = readFileSync(resolve(srcRoot, 'topbar/OtioImportModal.tsx'), 'utf8')
  const css = readFileSync(resolve(srcRoot, 'topbar/otio-import.css'), 'utf8')
  const importer = readFileSync(resolve(root, 'app/server/src/dispatch/otio_import.rs'), 'utf8')

  eq(topbar.includes("mode: 'preview'"), true, 'OTIO picker runs read-only preview before replacement')
  eq(topbar.includes("mode: 'replace'") && topbar.includes('expected_hash: otioPreview.source_hash'), true, 'OTIO confirmation binds replace to preview hash')
  eq(modal.includes('data-cut-otio-confirm') && modal.includes('data-cut-otio-missing'), true, 'OTIO modal exposes confirm and missing-media state')
  eq(modal.includes('Current project format stays unchanged'), true, 'OTIO modal states the format preservation result')
  eq(cssBlock(css, '.otio-modal').includes('max-height'), true, 'OTIO modal stays bounded at supported windows')
  eq(importer.includes('MAX_OTIO_BYTES') && importer.includes('replace_timeline_from_otio'), true, 'OTIO server path is bounded and uses atomic core commit')
}

// --- Export destinations are visible, reusable, and one-off overrideable -----
// The export destination is a user setting, not tribal knowledge hidden behind
// one menu. Keep a default export folder in Environment, keep one-off Save As
// controls in export surfaces, and make recording/export verbs pass explicit
// paths when the user chooses one.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const envDestinationPath = resolve(srcRoot, 'panels/Environment/ExportDestination.tsx')
  const exportPrefsPath = resolve(srcRoot, 'lib/exportDestination.ts')
  const envDestination = existsSync(envDestinationPath) ? readFileSync(envDestinationPath, 'utf8') : ''
  const exportPrefs = existsSync(exportPrefsPath) ? readFileSync(exportPrefsPath, 'utf8') : ''
  const envPanel = readFileSync(resolve(srcRoot, 'panels/Environment/index.tsx'), 'utf8')
  const settingsShell = readFileSync(resolve(srcRoot, 'panels/Environment/SettingsShell.tsx'), 'utf8')
  const settingsSections = readFileSync(resolve(srcRoot, 'panels/Environment/SettingsCategoryContent.tsx'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const topbarModel = readFileSync(resolve(srcRoot, 'topbar/model.ts'), 'utf8')
  const timelineGlobalToolsPath = resolve(srcRoot, 'panels/Timeline/TimelineGlobalTools.tsx')
  const timelineGlobalTools = existsSync(timelineGlobalToolsPath) ? readFileSync(timelineGlobalToolsPath, 'utf8') : ''
  const topbarCss = readFileSync(resolve(srcRoot, 'topbar/topbar.css'), 'utf8')
  const statusbar = readFileSync(resolve(srcRoot, 'statusbar/index.tsx'), 'utf8')
  const statusbarCss = readFileSync(resolve(srcRoot, 'statusbar/statusbar.css'), 'utf8')
  const record = readFileSync(resolve(srcRoot, 'panels/Record/index.tsx'), 'utf8')
  const recordActionCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageRecordActions.mjs'), 'utf8')
  const tauri = readFileSync(resolve(srcRoot, 'lib/tauri.ts'), 'utf8')
  const client = readFileSync(resolve(srcRoot, 'lib/client.ts'), 'utf8')
  const dispatch = readFileSync(resolve(root, 'app/server/src/dispatch.rs'), 'utf8')
  const screenRecordHandlers = readFileSync(resolve(root, 'app/server/src/dispatch/screen_record_handlers.rs'), 'utf8')
  const renderingHandlers = readFileSync(resolve(root, 'app/server/src/dispatch/rendering.rs'), 'utf8')
  const reviewHandoff = readFileSync(resolve(root, 'app/server/src/dispatch/review_handoff.rs'), 'utf8')
  const serverState = readFileSync(resolve(root, 'app/server/src/state.rs'), 'utf8')
  const outputPathsPath = resolve(root, 'app/server/src/output_paths.rs')
  const outputPaths = existsSync(outputPathsPath) ? readFileSync(outputPathsPath, 'utf8') : ''
  const schema = readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')
  const windowsUiux = readFileSync(resolve(root, 'scripts/windows/cdp-cut-verify-0650-uiux.mjs'), 'utf8')
  const exportDestinationVerifyPath = resolve(here, 'export-destination-verify.mjs')
  const exportDestinationVerify = existsSync(exportDestinationVerifyPath) ? readFileSync(exportDestinationVerifyPath, 'utf8') : ''
  const exportFileResultsVerifyPath = resolve(here, 'export-file-results-verify.mjs')
  const exportFileResultsVerify = existsSync(exportFileResultsVerifyPath) ? readFileSync(exportFileResultsVerifyPath, 'utf8') : ''
  const timelineSaveDropVerifyPath = resolve(here, 'timeline-save-drop-verify.mjs')
  const timelineSaveDropVerify = existsSync(timelineSaveDropVerifyPath) ? readFileSync(timelineSaveDropVerifyPath, 'utf8') : ''

  eq(existsSync(exportPrefsPath), true, 'shared export destination helper exists')
  eq(exportPrefs.includes('EXPORT_OUTPUT_DIR_STORAGE_KEY'), true, 'export destination helper owns the localStorage key')
  eq(exportPrefs.includes("callVerb('project.set_output_dir'"), true, 'export destination helper applies the default folder to the engine')
  eq(exportPrefs.includes('ensureStoredOutputDirApplied'), true, 'export surfaces can re-assert the stored default before exporting')
  eq(outputDirectoryForPath('/file.mp4'), '/', 'Save As authorization preserves a POSIX root')
  eq(outputDirectoryForPath('/tmp/export/file.mp4'), '/tmp/export', 'Save As authorization resolves a POSIX parent')
  eq(outputDirectoryForPath(String.raw`C:\file.mp4`), 'C:\\', 'Save As authorization preserves a Windows drive root')
  eq(outputDirectoryForPath(String.raw`C:\exports\file.mp4`), String.raw`C:\exports`, 'Save As authorization resolves a Windows parent')
  eq(outputDirectoryForPath(String.raw`\\?\C:\file.mp4`), '\\\\?\\C:\\', 'Save As authorization preserves a verbatim Windows drive root')
  eq(outputDirectoryForPath(String.raw`\\?\C:\exports\file.mp4`), String.raw`\\?\C:\exports`, 'Save As authorization resolves a verbatim Windows parent')
  eq(outputDirectoryForPath(String.raw`\\server\share\file.mp4`), String.raw`\\server\share`, 'Save As authorization resolves a UNC share parent')
  eq(outputDirectoryForPath('file.mp4'), null, 'Save As authorization rejects a path without a parent')
  eq(tauri.includes('export async function pickExportOutput'), true, 'Tauri bridge exposes a generic one-off export save picker')

  eq(existsSync(envDestinationPath), true, 'Environment owns a Default export folder settings row')
  eq(envDestination.includes('data-cut-export-default-folder'), true, 'Environment default export folder row has a stable selector')
  eq(envDestination.includes('data-cut-export-default-pick'), true, 'Environment has a native folder picker for the default export folder')
  eq(envDestination.includes('data-cut-export-default-clear'), true, 'Environment can clear back to the project exports folder')
  eq(envDestination.includes('disabled={!dir}'), true, 'Environment default export clear action remains visible but disabled on project exports')
  eq(envDestination.includes('{dir && ('), false, 'Environment default export clear action is not hidden when no custom folder is set')
  eq(envDestination.includes('data-cut-export-default-heading'), true, 'Environment default export row exposes a stable heading selector')
  eq(envDestination.includes('Default save folder'), true, 'Environment default export row leads with the plain setting name users look for')
  eq(envDestination.includes('Exports and recordings use this by default. Save As can override one file.'), true, 'Environment default export row names what the folder controls and how one-off overrides work')
  eq(envDestination.includes('Choose export folder'), true, 'Environment output row button says what folder is being changed')
  eq(settingsSections.includes('<ExportDestination'), true, 'General Settings renders the default export folder row')
  eq(settingsShell.includes('<h2 className="env-modal-title">Settings</h2>'), true, 'Settings shell title is Settings, not a hidden technical category')
  eq(settingsSections.includes('Default save folder') || settingsSections.includes('<ExportDestination'), true, 'Settings keeps export destination in the beginner-facing General category')

  eq(topbar.includes('data-cut-settings-btn'), true, 'Topbar has an always-visible Settings entry point')
  eq(topbar.includes('export folder, recordings'), true, 'Topbar Settings tooltip names the export folder setting')
  eq(topbar.includes('data-cut-tools-btn'), false, 'Topbar no longer carries the global timeline Tools dropdown')
  eq(topbar.includes('data-cut-tool="trim_edges"'), false, 'Topbar no longer renders trim_edges')
  eq(topbar.includes('data-cut-tool="split_scenes"'), false, 'Topbar no longer renders split_scenes')
  eq(topbar.includes('data-cut-tool="mark_scenes"'), false, 'Topbar no longer renders mark_scenes')
  eq(timelineGlobalTools.includes('function toolResultMessage'), true, 'Timeline tools feedback is centralized instead of always flashing a checkmark')
  eq(timelineGlobalTools.includes('markers_added'), true, 'Timeline Mark scenes feedback reads the engine marker count')
  eq(timelineGlobalTools.includes('splits'), true, 'Timeline Split at scenes feedback reads the engine split count')
  eq(timelineGlobalTools.includes('no scene cuts found'), true, 'Timeline scene tools explain zero-result runs instead of reporting success')
  eq(topbar.includes('Default export folder'), true, 'Export menu labels the folder as the default export folder')
  eq(topbar.includes('data-cut-export-saveas-option'), true, 'Topbar export options expose one-off Save As controls')
  eq(topbar.includes('pickExportOutput'), true, 'Topbar one-off exports use the native save dialog')
  eq(topbar.includes('withAuthorizedOutputPath'), true, 'Topbar exports authorize a native Save As parent and re-assert the stored default')
  eq(topbarCss.includes('@media (max-width: 1700px)'), true, 'Topbar condensed layout includes the 1600px installed-test viewport')
  eq(topbarCss.includes('.tb-brand { flex: 0 0 150px; min-width: 150px; max-width: 150px; }'), true, 'Topbar reserves a clickable project-title slot in the 1600px condensed layout')
  eq(topbarCss.includes('.tb-wordmark { display: none; }'), true, 'Topbar drops the redundant wordmark before it collapses the project title')
  eq(topbarCss.includes('@media (max-width: 1536px)') && topbarCss.includes('.tb-nav-label { display: none; }'), true, 'Laptop topbar switches secondary navigation to explicit icon-only controls')
  eq(topbar.includes('aria-label="Storyboard"') && topbar.includes('aria-busy={sbBusy}'), true, 'Storyboard keeps an accessible button name while busy')
  eq(statusbar.includes('data-cut-output-chip'), true, 'Status bar has an always-visible output-folder chip')
  eq(statusbar.includes('getStoredOutputDir') && statusbar.includes('folderTail'), true, 'Status bar output chip mirrors the shared default export folder setting')
  eq(statusbar.includes("onClick={() => onOpenEnvironment('general')}"), true, 'Status bar output chip opens the General folder setting directly')
  eq(statusbar.includes('export folder:'), true, 'Status bar chip uses explicit export-folder wording')
  eq(statusbar.includes('output:'), false, 'Status bar chip no longer uses ambiguous output wording')
  eq(cssBlock(statusbarCss, '.sb-output').includes('display: flex'), true, 'Status bar output chip has a stable compact chip style')
  eq(windowsUiux.includes('statusbar-output-chip-visible'), true, 'Windows UIUX harness checks the visible output-folder chip')
  eq(windowsUiux.includes('statusbar-output-chip-opens-environment'), true, 'Windows UIUX harness verifies the output chip opens Environment')
  eq(windowsUiux.includes('settings-button-opens-export-folder'), true, 'Windows UIUX harness verifies Settings opens the folder row')
  eq(windowsUiux.includes('record-default-folder-opens-settings'), true, 'Windows UIUX harness verifies Record links to the default folder setting')
  eq(windowsUiux.includes('Save As can override one file'), true, 'Windows UIUX harness requires the default folder row to explain one-off Save As overrides')
  eq(windowsUiux.includes('Exports and recordings go here'), false, 'Windows UIUX harness no longer accepts vague default-folder wording')
  eq(existsSync(exportDestinationVerifyPath), true, 'Lightweight export-destination runtime verifier exists for unsigned batch checks')
  for (const expected of [
    '[data-cut-output-chip]',
    '[data-cut-settings-btn]',
    '[data-cut-export-default-folder]',
    '[data-cut-export-default-clear]',
    '[data-cut-export-btn]',
    '[data-cut-export-choose-folder]',
    '[data-cut-export-saveas-option="video"]',
    'no-obvious-export-settings-overflow',
  ]) {
    eq(exportDestinationVerify.includes(expected), true, `Export-destination runtime verifier covers ${expected}`)
  }

  eq(record.includes('onOpenOutputSettings'), true, 'Record tab accepts a direct opener for default folder settings')
  eq(record.includes('data-cut-rec-output-path'), true, 'Record tab shows the chosen one-off recording output path')
  eq(record.includes('data-cut-action="record-output-pick"'), true, 'Record tab lets users pick a recording output file')
  eq(record.includes('data-cut-action="record-output-default-folder"'), true, 'Record tab has a direct default export folder settings control')
  eq(record.includes('data-cut-rec-output-note'), true, 'Record output picker gives immediate feedback next to the Save file row')
  eq(record.includes('raw_path: rawOutputPath'), true, 'Raw recording stop passes the user-selected output path')
  eq(record.includes('path: outputPath'), true, 'Polished recording export passes a one-off output path')
  eq(record.includes('withAuthorizedOutputPath'), true, 'Record exports authorize a selected Save As parent and restore the stored default')
  // 0.6.106 finding: exportClip ran as `void exportClip()`, so a rejection from
  // withAuthorizedOutputPath (engine refuses the chosen Save As folder) surfaced
  // NOWHERE and the note stayed "Rendering…" with no verb ever issued. Both
  // authorizing callers in this panel must therefore catch and report; the
  // behavioural proof is the record-export-authorization-refused row in
  // lib/fullCoverageRecordActions.mjs (RED before this landed).
  eq(/catch \(error\) \{[\s\S]*?setExportNote\(/.test(record), true, 'Record export reports a failed authorization instead of leaving "Rendering…" on screen')
  eq(record.includes("if (!lastCapture) { setExportNote("), true, 'Record export with no retained capture says so instead of silently returning')
  eq(record.includes('function failureReason('), true, 'Record panel derives one human reason for a rejected verb/authorization promise')
  eq(record.includes('OUTPUT_PATH_HINT'), true, 'Record failure notes tell the user how to get out of a refused output folder')
  eq(record.includes("setErr('server unreachable during finalize')"), false, 'Record finalize no longer reports a refused output folder as an unreachable server')
  eq(recordActionCoverage.includes('record-export-authorization-refused'), true, 'Deterministic Record coverage drives a refused export authorization end to end')

  eq(client.includes("'screen_record.stop': { capture_id: string; autoedit?: boolean; mux_raw?: boolean; raw_path?: string"), true, 'Typed client exposes screen_record.stop raw_path')
  eq(client.includes("'export.frame': { at_ms: number; to_asset?: boolean; path?: string"), true, 'Typed client exposes export.frame path')
  eq(schema.includes('"raw_path"') && schema.includes('Explicit raw recording output path'), true, 'Schema documents screen_record.stop raw_path')
  eq(schema.includes('"name": "export.frame"') && schema.includes('Optional explicit output path for the still frame'), true, 'Schema documents export.frame path')
  eq(screenRecordHandlers.includes('raw_path: Option<String>,'), true, 'screen_record.stop accepts raw_path server-side')
  eq(existsSync(outputPathsPath), true, 'server output path fencing has its own module')
  eq(outputPaths.includes('pub(crate) async fn project_set_output_dir'), true, 'output path module owns project.set_output_dir')
  eq(outputPaths.includes('pub(crate) fn fence_output_path'), true, 'output path module owns shared output fencing')
  eq(outputPaths.includes('pub(crate) fn temp_output_path_for_render'), true, 'output path module owns render-temp output paths')
  eq(outputPaths.includes('pub(crate) fn publish_output_atomic'), true, 'output path module owns atomic temp-output publish')
  eq(outputPaths.includes('fn next_available_output_path') && outputPaths.includes('recording-2.mp4'), true, 'output path module owns default export auto-suffixing')
  eq(
    dispatch.includes('"project.set_output_dir"')
      && dispatch.includes('crate::output_paths::project_set_output_dir(args)')
      && dispatch.includes('.await')
      && dispatch.includes('.into()'),
    true,
    'dispatch delegates project.set_output_dir to the output path module',
  )
  eq(dispatch.includes('static SESSION_OUTPUT_DIR'), false, 'dispatch no longer owns the session output-dir setting')
  eq(dispatch.includes('fn next_available_output_path'), false, 'dispatch no longer owns default export auto-suffixing')
  eq(renderingHandlers.includes('let path = fence_output_path(') && renderingHandlers.includes('&format!("exports/frame_{}.jpg", a.at_ms)'), true, 'export.frame uses the shared output fence/default folder')
  eq(renderingHandlers.includes('let tmp_out = temp_output_path_for_render(&out)'), true, 'export.range renders through a sibling temp output path')
  eq(renderingHandlers.includes('publish_output_atomic(&t, &o)'), true, 'export.range publishes the temp output only after render success')
  // 0.6.106 findings 2 + 3 — both are "the engine delivered the file to the
  // user's chosen export folder, then a second code path could not find it".
  // Pin the two call sites so a regression to the wrong helper is caught by the
  // cheap suite; the behavioural proofs are the output_paths unit tests and the
  // live-verb red/green runs recorded in the commits.
  eq(
    renderingHandlers.includes('let out_path = match fence_project_output_path(&dir, None, &rel)'),
    true,
    'render.bundle keeps every publish-package member in the project tree its manifest lives in',
  )
  eq(
    reviewHandoff.includes('let source = fenced_existing_export_read(')
      && reviewHandoff.includes('"review render"'),
    true,
    'comment.export reads the review render from every authorized export root, not <project>/exports alone',
  )
  eq(
    outputPaths.includes('pub(crate) fn fenced_existing_export_read('),
    true,
    'output path module owns the authorized-export read fence',
  )
  eq(serverState.includes('fn align_sidecar_ffmpeg_env()'), true, 'server owns early ffmpeg-dir alignment for perception sidecars')
  eq(serverState.includes('pub fn new() -> Self {\n        align_sidecar_ffmpeg_env();'), true, 'AppState aligns perception ffmpeg before first import/enrich job')
  eq(serverState.includes('std::env::set_var(cut_media::toolpath::ENV_FFMPEG_DIR, dir);'), true, 'perception sidecar inherits the resolved ffmpeg directory')
  eq(existsSync(exportFileResultsVerifyPath), true, 'Export file-result runtime verifier exists for real output path proof')
  for (const expected of [
    '../../scripts/lib/cross-host-media.mjs',
    'resolveDriverPath',
    'project.set_output_dir',
    'export.frame',
    'to_asset: false',
    'default-folder-export-created',
    'default-output-dedup-suffix',
    'save-as-overwrites-explicit-path',
    'cleared-folder-returns-to-project-exports',
    'stripExtendedPath',
    'driverPath',
    'samePath(explicitPath, saveAsPath)',
  ]) {
    eq(exportFileResultsVerify.includes(expected), true, `Export file-result runtime verifier covers ${expected}`)
  }
  eq(existsSync(timelineSaveDropVerifyPath), true, 'Timeline save/drop runtime verifier exists for focused result proof')
  for (const expected of [
    'cut:asset-dragmove',
    'cut:asset-drop',
    'data-cut-action="save-range"',
    'data-cut-action="save-gif"',
    'export.range',
    'export.gif',
    'timeline-drop-inserted-base-line',
    'timeline-alt-drop-created-overlay-line',
    'timeline-save-range-created-asset',
    'timeline-save-gif-created-asset',
    'existsSync(driverPath',
    'project.state',
  ]) {
    eq(timelineSaveDropVerify.includes(expected), true, `Timeline save/drop runtime verifier covers ${expected}`)
  }
}

// --- Mixer meter stems: cancelled runs must not fetch stale project exports ---
// Rapid project/create-reload cycles leave old async meter renders in flight.
// If an old run fetches /api/export/audio_<track>.wav after the current project
// has changed, the fenced export route correctly 404s against the new project's
// exports directory. The loader must stop before fetch once the effect is
// superseded, and the fetch itself must be abortable.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const mixer = readFileSync(resolve(srcRoot, 'panels/Mixer/index.tsx'), 'utf8')
  const verifyAudioLayer = readFileSync(resolve(here, 'verify-audio-layer.mjs'), 'utf8')

  eq(mixer.includes("const projectKey = project?.name ?? ''"), true, 'Mixer stem loader tracks the mounted project identity')
  eq(mixer.includes('const abort = new AbortController()'), true, 'Mixer stem loader creates an AbortController per effect run')
  eq(mixer.includes('const ids = trackKey ? trackKey.split(\',\') : []'), true, 'Mixer stem loader derives a stable track id list from trackKey')
  eq(mixer.includes('if (cancelled) return'), true, 'Mixer stem loader checks cancellation before fetching exported stems')
  eq(mixer.includes("const currentProject = await callVerb('project.state', {})"), true, 'Mixer stem loader re-checks the current server project before fetching a stem')
  eq(mixer.includes('currentName !== projectKey'), true, 'Mixer stem loader refuses to fetch old project export paths after a project switch')
  eq(mixer.includes('signal: abort.signal'), true, 'Mixer stem fetch is abortable when a newer project/effect supersedes it')
  eq(mixer.includes('abort.abort()'), true, 'Mixer stem loader aborts stale in-flight fetches during cleanup')
  eq(mixer.includes('}, [headOpId, projectKey, trackKey])'), true, 'Mixer stem effect depends on stable project/headOpId/trackKey values, not a fresh array')
  eq(mixer.includes("(t) => t.kind === 'audio' && t.clips.length > 0"), true, 'Mixer excludes video tracks whose audio edits would not enter the render graph')
  eq(mixer.includes('const [trackToggleBusy, setTrackToggleBusy] = useState<Record<string, boolean>>({})'), true, 'Mixer mute/solo controls have per-track pending state')
  eq(mixer.includes("const key = `${verb}:${t.id}`"), true, 'Mixer toggle pending state is keyed by verb and track')
  eq(mixer.includes('disabled={!!trackToggleBusy[`mute:${t.id}`]}'), true, 'Mixer mute button disables while its edit.mute call is pending')
  eq(mixer.includes('disabled={!!trackToggleBusy[`solo:${t.id}`]}'), true, 'Mixer solo button disables while its edit.solo call is pending')
  eq(verifyAudioLayer.includes('staleStemErrors'), true, 'Audio/layer verifier tracks stale Mixer stem fetches as their own failure class')
  eq(verifyAudioLayer.includes('mixer-stale-stem-fetches'), true, 'Audio/layer verifier reports stale Mixer stem fetches as a check result')
  eq(verifyAudioLayer.includes('async function resetRightTab'), true, 'Audio/layer verifier resets the right tab between project-mutating checks')
}

// --- Title drawer: placement and style are separate concepts ------------------
// Title-mode regression: the old drawer labeled the Preset/Animated/Place-anywhere mode
// switch as "Style", mixing where the title goes with how it looks. Keep stable
// selectors so the label cannot quietly drift back.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const title = readFileSync(resolve(srcRoot, 'panels/Title/index.tsx'), 'utf8')

  eq(title.includes('data-cut-title-placement-label') && title.includes('>Placement<'), true, 'Title drawer labels the mode switch as Placement')
  eq(title.includes('<span className="cd-field-label">Style</span>'), false, 'Title drawer does not use Style for the placement mode switch')
  eq(title.includes('data-cut-title-style-label') && title.includes('>Style<'), true, 'Title drawer reserves Style for visual template controls')
}

// --- Exhaustive UI gate must persist per-control result evidence --------------
// Console streams are not enough for release review: every full-coverage run
// must leave a machine-readable result receipt with each PRESENT/RENDER/CLICK/
// RESULT row, its screenshot path, and the classification behind the gate
// verdict. This is what lets a later reviewer prove "clicked and an edit/result
// happened" after the terminal scrollback is gone.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const gate = readFileSync(resolve(root, 'scripts/release/full-coverage-gate.mjs'), 'utf8')
  const sweep = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const helper = resolve(root, 'scripts/lib/full-coverage-receipt.mjs')
  const helperTest = resolve(root, 'scripts/public-tests/full-coverage-receipt.test.mjs')

  eq(existsSync(helper), true, 'full-coverage result receipt helper exists')
  eq(existsSync(helperTest), true, 'full-coverage result receipt helper has a focused test')
  eq(sweep.includes('buildFullCoverageReceipt'), true, 'full-coverage sweep builds a durable result receipt')
  eq(sweep.includes('FCV_RESULT_RECEIPT'), true, 'full-coverage sweep writes the result receipt when FCV_RESULT_RECEIPT is set')
  eq(gate.includes('FCV_RESULT_RECEIPT'), true, 'full-coverage wrapper reserves a result receipt path for rig evidence')
  eq(gate.includes('resultReceipt'), true, 'full-coverage wrapper records the result receipt path in its wrapper receipt')
}

// --- App-wide Import media command must be live ------------------------------
// Several visible affordances dispatch cut:open-import (palette, preview empty
// state, transcript empty state, timeline empty state). App must own one
// listener that opens the native picker and runs media.import, otherwise all
// those buttons are dead while drag-drop still works.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(root, 'ui/src')
  const app = readFileSync(resolve(srcRoot, 'App.tsx'), 'utf8')
  const events = readFileSync(resolve(srcRoot, 'lib/events.ts'), 'utf8')
  const hookPath = resolve(srcRoot, 'app/useAppImportEvents.ts')
  const hook = existsSync(hookPath) ? readFileSync(hookPath, 'utf8') : ''

  eq(existsSync(hookPath), true, 'App import command listener lives in a focused hook')
  eq(app.includes("import { useAppImportEvents } from './app/useAppImportEvents'"), true, 'App imports the global import-command hook')
  eq(app.includes('useAppImportEvents({'), true, 'App mounts the global import-command hook')
  eq(hook.includes("document.addEventListener('cut:open-import'"), true, 'import hook listens for cut:open-import')
  eq(hook.includes("document.removeEventListener('cut:open-import'"), true, 'import hook removes the cut:open-import listener')
  eq(hook.includes('pickMedia()'), true, 'import hook opens the native media picker')
  eq(hook.includes("callVerb('media.import'"), true, 'import hook runs media.import for picked files')
  eq(hook.includes('getGenerateProxies()'), true, 'import hook respects the proxy-generation preference')
  eq(hook.includes('onChangedRef.current?.()'), true, 'import hook refreshes project state after import')
  eq(events.includes("type: 'project_changed'"), true, 'UI event contract includes external project workspace transitions')
  eq(events.includes("case 'project_changed':"), true, 'UI event decoder accepts project workspace transitions')
  eq(app.includes("case 'project_changed':"), true, 'App refreshes when an agent creates, opens, or closes a project')
  eq(app.includes("leftTab: 'projects'"), true, 'App returns to the Projects tab when an external close leaves no project open')
}

// --- Color tab state follows the selected clip -------------------------------
// GradeDrawer is mounted unkeyed in the right rail. Its sliders therefore must
// resync when selected clip/grade changes; initial useState seeds are not enough.
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const grade = readFileSync(resolve(srcRoot, 'panels/Grade/index.tsx'), 'utf8')

  eq(grade.includes('const gradeSeedKey ='), true, 'GradeDrawer derives a stable selected-clip grade seed key')
  eq(grade.includes('setContrast(grade?.contrast ?? NEUTRAL.contrast)'), true, 'GradeDrawer resync effect reseeds contrast')
  eq(grade.includes('setBrightness(grade?.brightness ?? NEUTRAL.brightness)'), true, 'GradeDrawer resync effect reseeds brightness')
  eq(grade.includes('setSaturation(grade?.saturation ?? NEUTRAL.saturation)'), true, 'GradeDrawer resync effect reseeds saturation')
  eq(grade.includes('setGamma(grade?.gamma ?? NEUTRAL.gamma)'), true, 'GradeDrawer resync effect reseeds gamma')
  eq(grade.includes('setTempOn(grade?.temperature_k != null)'), true, 'GradeDrawer resync effect reseeds white-balance enabled state')
  eq(grade.includes('setLut(grade?.lut ?? \'\')'), true, 'GradeDrawer resync effect reseeds LUT')
  eq(grade.includes('}, [gradeSeedKey])'), true, 'GradeDrawer resyncs from clip/grade changes instead of only on first mount')
}

// --- Premium-only matte installation is ready -------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const matte = readFileSync(resolve(srcRoot, 'panels/Matte/index.tsx'), 'utf8')

  eq(matte.includes("const ready = probeState === 'ready' || premiumReady"), true, 'Matte treats base or premium runtime as ready')
  eq(matte.includes(") : !ready && probeState === 'absent' ? ("), true, 'Matte does not show the not-set-up card when premium alone is ready')
  eq(matte.includes('const pollSetupJob = useCallback(async (jobId: string)'), true, 'Matte install keeps progress state until setup_matte job reaches terminal state')
  eq(matte.includes("callVerb('jobs.status', { job_id: jobId })"), true, 'Matte install polls jobs.status for setup_matte progress')
  eq(matte.includes("setErr(`setup failed: ${msg}`)"), true, 'Matte install surfaces setup_matte job failure instead of reverting to idle')
}

// --- UI lifecycle and honest-state regressions -------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const envSetupJob = readFileSync(resolve(srcRoot, 'panels/Environment/useEnvironmentSetupJob.ts'), 'utf8')
  const search = readFileSync(resolve(srcRoot, 'panels/Search/index.tsx'), 'utf8')
  const grade = readFileSync(resolve(srcRoot, 'panels/Grade/index.tsx'), 'utf8')
  const mixer = readFileSync(resolve(srcRoot, 'panels/Mixer/index.tsx'), 'utf8')
  const recipes = readFileSync(resolve(srcRoot, 'panels/Recipes/index.tsx'), 'utf8')
  const review = readFileSync(resolve(srcRoot, 'panels/Review/index.tsx'), 'utf8')
  const stock = readFileSync(resolve(srcRoot, 'panels/Stock/index.tsx'), 'utf8')
  const libraryCss = readFileSync(resolve(srcRoot, 'panels/Library/library.css'), 'utf8')

  eq(envSetupJob.includes('const mountedRef = useRef(true)'), true, 'Environment setup rows track mount state while polling jobs')
  eq(envSetupJob.includes('mountedRef.current = false'), true, 'Environment setup rows mark themselves unmounted on cleanup')
  eq(envSetupJob.includes('if (!mountedRef.current || activeJobRef.current !== jobId) return'), true, 'Environment job polling stops applying stale/unmounted job updates')
  eq(search.includes('const indexedAssetCount = Object.keys(indexed).length'), true, 'Search tracks whether any clip has been indexed this session')
  eq(search.includes('function looksLikeVideoPath'), true, 'Search has an extension fallback for freshly imported videos before probe metadata arrives')
  eq(search.includes('(!probe && looksLikeVideoPath(path))'), true, 'Search includes pre-probe video assets instead of hiding fresh imports')
  eq(search.includes('const INDEX_FPS = 1'), true, 'Search declares the visual-search frame sampling rate in one place')
  eq(search.includes('Index one frame per second for content search'), true, 'Search index control discloses the 1 fps sampling rate')
  eq(search.includes("setErr('Index at least one clip before searching.')"), true, 'Search names the not-indexed state distinctly from a zero-result match')
  eq(search.includes("setNote('No matching indexed moments.')"), true, 'Search zero-result copy names indexed moments')
  eq(grade.includes('if (busy) return'), true, 'Grade reset ignores clicks while an edit.grade call is pending')
  eq(grade.includes('disabled={busy}'), true, 'Grade reset is disabled while edit.grade is pending')
  eq(mixer.includes('Measure track LUFS'), true, 'Mixer loudness action measures the track badge instead of presenting a raw source-only verdict')
  eq(mixer.includes('data-cut-loudness-source-lufs'), true, 'Mixer preserves the raw source LUFS as an inspectable data attribute')
  eq(mixer.includes('data-cut-loudness-mix-state'), true, 'Mixer exposes whether the loudness badge is audible or silent in the mix')
  eq(mixer.includes('reading.integrated_lufs + db'), true, 'Mixer applies the fader gain to the visible LUFS badge')
  eq(mixer.includes('silent in mix'), true, 'Mixer loudness badge names muted/soloed-out tracks as silent')
  eq(recipes.includes('const activeRunJobRef = useRef<string | null>(null)'), true, 'Recipes tracks the active run job separately from stale polls')
  eq(recipes.includes('activeRunJobRef.current = handle.job_id'), true, 'Recipes records the active run job before polling')
  eq(recipes.includes('if (activeRunJobRef.current !== handle.job_id) return'), true, 'Recipes poll ignores stale job results after recipe switches')
  eq(review.includes('const undoTipOp = avail.undo ? tipOp : null'), true, 'Review tip label uses the same undo availability source as the button')
  eq(stock.includes('setHits([]); setFetched({})'), true, 'Stock clears fetched state when a new search starts')
  eq(cssBlock(libraryCss, '.lb-card').includes('content-visibility: auto'), true, 'Library cards use browser render containment for large libraries')
  eq(libraryCss.includes('contain-intrinsic-size: 76px'), true, 'Library rows use browser render containment for large libraries')
}

// --- UI accessibility and status regressions --------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(here, '../src')
  const projects = readFileSync(resolve(srcRoot, 'panels/Projects/index.tsx'), 'utf8')
  const projectsCss = readFileSync(resolve(srcRoot, 'panels/Projects/projects.css'), 'utf8')
  const record = readFileSync(resolve(srcRoot, 'panels/Record/index.tsx'), 'utf8')
  const studioControls = readFileSync(resolve(srcRoot, 'panels/Record/StudioControls.tsx'), 'utf8')
  const settingsShell = readFileSync(resolve(srcRoot, 'panels/Environment/SettingsShell.tsx'), 'utf8')
  const settingsShellCss = readFileSync(resolve(srcRoot, 'panels/Environment/settings-shell.css'), 'utf8')
  const settingsCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageSettings.mjs'), 'utf8')
  const coverageAppUrl = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageAppUrl.mjs'), 'utf8')
  const fullCoverageVerifier = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const installedFixtureCoverages = [
    'fullCoverageLibraryActions.mjs',
    'fullCoverageReviewActions.mjs',
    'fullCoverageChatActions.mjs',
    'fullCoverageDirectorActions.mjs',
    'fullCoverageTranscriptActions.mjs',
  ].map((name) => readFileSync(resolve(root, 'ui/public-tests/lib', name), 'utf8'))
  const webdriverPage = readFileSync(resolve(root, 'ui/public-tests/lib/webdriverIoPage.mjs'), 'utf8')
  const settingsContent = readFileSync(resolve(srcRoot, 'panels/Environment/SettingsCategoryContent.tsx'), 'utf8')
  const agentControl = readFileSync(resolve(srcRoot, 'panels/Environment/AgentControl.tsx'), 'utf8')
  const sequenceIndex = readFileSync(resolve(srcRoot, 'panels/SequenceIndex/index.tsx'), 'utf8')
  const clipboard = readFileSync(resolve(srcRoot, 'lib/clipboard.ts'), 'utf8')
  const envCardRow = readFileSync(resolve(srcRoot, 'panels/Environment/EnvCardRow.tsx'), 'utf8')
  const environmentSetupJob = readFileSync(resolve(srcRoot, 'panels/Environment/useEnvironmentSetupJob.ts'), 'utf8')
  const client = readFileSync(resolve(srcRoot, 'lib/client.ts'), 'utf8')
  const schema = JSON.parse(readFileSync(resolve(root, 'schema/verbs.json'), 'utf8'))
  const reference = readFileSync(resolve(root, 'skill/shellx-cut/reference.md'), 'utf8')
  const renderQueue = readFileSync(resolve(srcRoot, 'topbar/RenderQueueModal.tsx'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const directorCss = readFileSync(resolve(srcRoot, 'director/director.css'), 'utf8')
  const renderQueueCss = readFileSync(resolve(srcRoot, 'topbar/renderqueue.css'), 'utf8')

  eq(projects.includes('type="button"') && projects.includes('className="pj-forget"'), true, 'Projects forget control is a real button')
  eq(projects.includes('type="button"') && projects.includes('className="pj-delete"'), true, 'Projects delete control is a real button')
  eq(projects.includes('role="button"'), false, 'Projects cards do not wrap action buttons inside a role=button container')
  eq(projects.includes('data-cut-project-open'), true, 'Projects cards expose a dedicated keyboard-accessible open control')
  eq(projects.includes('error?.suggested_action'), true, 'Projects surfaces actionable recovery guidance when a project transition is still draining')
  eq(cssBlock(projectsCss, '.pj-card').includes('content-visibility: auto'), true, 'Projects rows use browser render containment for large recent-project lists')
  eq(projectsCss.includes('contain-intrinsic-size: 62px'), true, 'Projects rows keep a stable intrinsic height while offscreen')
  eq(record.includes("rawCapture ? 'Raw recording will save to this file.' : 'Polished export will use this file when you export.'"), true, 'Record output copy distinguishes raw capture from polished export')
  eq(record.includes("addEventListener('devicechange', refreshMic)"), true, 'Record refreshes mic readiness when audio devices change')
  eq(record.includes("removeEventListener('devicechange', refreshMic)"), true, 'Record removes the devicechange listener on unmount')
  eq(record.includes("card.name === 'webcam' && card.status === 'ok'"), false, 'Record does not derive a reachable camera UI from a permanently parked backend')
  eq(record.includes("cards.filter((c) => c.name !== 'webcam')"), true, 'Record shows the parked camera boundary once instead of duplicating its doctor card')
  eq(record.includes("screen_capture: 'Screen capture'"), true, 'Record translates recorder capability ids into user labels')
  eq(record.includes('webcam: studio.camera.enabled'), false, 'Record does not request the unsupported camera stream')
  eq(studioControls.includes('data-cut-studio-camera-unavailable'), true, 'Recording Studio exposes an explicit parked camera status')
  eq(studioControls.includes('Not available in this release.'), true, 'Recording Studio names the camera release boundary')
  eq(studioControls.includes('data-cut-studio-camera-enabled-toggle'), false, 'Recording Studio does not retain unreachable camera controls')
  eq(studioControls.includes('data-cut-studio-camera-position-button'), false, 'Recording Studio does not retain unreachable camera position actions')
  eq(studioControls.includes('F10 Cam'), false, 'Recording Studio does not advertise an unavailable camera hotkey')
  eq(record.includes('cameraAvailable'), false, 'Record does not keep dead camera-availability branches')
  eq(settingsContent.includes('microphone and system-audio support'), true, 'Settings describes the recording capabilities that this release actually verifies')
  eq(settingsContent.includes('microphone and camera support'), false, 'Settings does not advertise parked camera capture')
  eq(settingsShell.includes('data-cut-settings-category-select'), true, 'Compact Settings exposes a bounded category selector')
  eq(settingsShellCss.includes('@media (max-width: 1120px)') && settingsShellCss.includes('.settings-category-select-wrap {\n    display: grid;'), true, 'Compact Settings switches to the category selector at the supported native minimum width')
  eq(settingsShellCss.includes('.settings-nav {\n    display: none;'), true, 'Compact Settings hides the long category rail when the selector is visible')
  eq(settingsCoverage.includes('width: 1100'), true, 'Installed Settings coverage exercises the real native minimum width')
  eq(settingsCoverage.includes('} finally {'), true, 'Installed Settings coverage always restores the prior viewport')
  eq(settingsCoverage.includes('compactCategory.isVisible()'), true, 'Installed Settings routing supports both category navigation forms')
  eq(settingsCoverage.includes('resolveCoverageAppUrl(page, app)')
    && coverageAppUrl.includes('window.location.href'), true, 'Embedded Settings fixtures reload the installed app origin instead of a development URL')
  eq(installedFixtureCoverages.every((source) => source.includes('resolveCoverageAppUrl(page, app)')), true, 'Every conditional installed fixture resolves the active WebView origin')
  eq((fullCoverageVerifier.match(/app: EMBEDDED_WDIO \? '' : APP/g) || []).length, 6, 'Every conditional fixture creator defers to the installed WebView origin')
  eq(webdriverPage.includes('native viewport resize did not reach'), true, 'Native adapter fails honestly when the OS refuses a requested viewport')
  eq(clipboard.includes("document.execCommand('copy')"), true, 'Shared clipboard writing falls back when a webview clipboard API is unavailable')
  eq(agentControl.includes("import { writeClipboardText } from '../../lib/clipboard'"), true, 'Agent control uses the webview-safe clipboard helper')
  eq(sequenceIndex.includes("import { writeClipboardText } from '../../lib/clipboard'"), true, 'Sequence Index reuses the webview-safe clipboard helper')
  eq(envCardRow.includes('useEnvironmentSetupJob(onChanged)'), true, 'Environment cards delegate setup polling to the focused job hook')
  eq(environmentSetupJob.includes('SETUP_JOB_MAX_POLLS'), true, 'Environment setup jobs have a bounded polling lifetime')
  eq(environmentSetupJob.includes('SETUP_JOB_MAX_READ_FAILURES'), true, 'Environment setup jobs stop after repeated status-read failures')
  eq(environmentSetupJob.includes('server unreachable'), true, 'Environment setup surfaces a transport-level start failure')
  eq(client.includes('webcam?: boolean; webcam_device?: string'), false, 'Typed screen-record start args do not advertise unsupported camera capture')
  const screenRecordStart = schema.verbs.find((verb: { name: string }) => verb.name === 'screen_record.start')
  eq(screenRecordStart.args.properties.webcam, undefined, 'Shared schema rejects unsupported live webcam capture')
  eq(screenRecordStart.args.properties.webcam_device, undefined, 'Shared schema rejects unsupported webcam device selection')
  eq(reference.includes('webcam?, webcam_device?'), false, 'Agent reference does not advertise parked camera args')
  eq(reference.includes('Camera arguments are not part of this release contract'), true, 'Agent reference names the camera contract boundary')
  eq(renderQueue.includes('function duplicateOutputPaths(rows: Row[]): string | null'), true, 'Render queue validates duplicate explicit output paths before submit')
  eq(renderQueue.includes("setErr(`Each queued output path must be unique: ${duplicate}`)"), true, 'Render queue surfaces colliding output path before render.queue')
  eq(topbar.includes('setRenderOptsOpen(false); setDirectorOpen(true)'), true, 'Direct opens the Director modal after closing the render-options dropdown')
  eq(directorCss.includes('z-index: var(--z-modal'), true, 'Director modal uses shared modal z-index token')
  eq(renderQueueCss.includes('z-index: var(--z-modal'), true, 'Render queue modal uses shared modal z-index token')
}

// --- Smaller UI robustness regressions ---------------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const dropZone = readFileSync(resolve(srcRoot, 'DropZone.tsx'), 'utf8')
  const tauri = readFileSync(resolve(srcRoot, 'lib/tauri.ts'), 'utf8')
  const assets = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const sourceMonitor = readFileSync(resolve(srcRoot, 'panels/Assets/SourceMonitor.tsx'), 'utf8')
  const placement = readFileSync(resolve(srcRoot, 'lib/placement.ts'), 'utf8')
  const assetCardDrag = readFileSync(resolve(srcRoot, 'lib/useAssetCardDrag.ts'), 'utf8')
  const assemble = readFileSync(resolve(srcRoot, 'panels/Assemble/index.tsx'), 'utf8')
  const clientModel = readFileSync(resolve(srcRoot, 'lib/clientModel.ts'), 'utf8')
  const library = readFileSync(resolve(srcRoot, 'panels/Library/index.tsx'), 'utf8')
  const libraryQuery = readFileSync(resolve(srcRoot, 'panels/Library/useLibraryQuery.ts'), 'utf8')
  const comments = readFileSync(resolve(srcRoot, 'panels/Comments/index.tsx'), 'utf8')
  const clipboard = readFileSync(resolve(srcRoot, 'app/useAppClipboardController.ts'), 'utf8')
  const statusbar = readFileSync(resolve(srcRoot, 'statusbar/index.tsx'), 'utf8')

  const projectBootstrap = readFileSync(resolve(srcRoot, 'lib/projectBootstrap.ts'), 'utf8')
  eq(projectBootstrap.includes('const MEDIA_EXTENSIONS'), true, 'Drop-to-create has an explicit media-extension allowlist')
  eq(dropZone.includes('const unsupported = paths.filter((path) => !isSupportedMediaPath(path))'), true, 'DropZone separates unsupported dropped files before media.import')
  eq(dropZone.includes("await callVerb('project.create', { name })"), true, 'DropZone creates a project when no project is open')
  eq(dropZone.includes('await waitForImport(result.job_id)'), true, 'DropZone waits for the first import before later files can auto-place')
  eq(dropZone.includes('duration_ms: FIRST_STILL_DURATION_MS'), true, 'DropZone places a first still with an explicit duration')
  eq(dropZone.includes('Drop media to start a project'), true, 'DropZone names the no-project action before drop')
  eq(
    tauri.includes("import('@tauri-apps/api/webview')")
      && tauri.includes('getCurrentWebview().onDragDropEvent'),
    true,
    'real OS file drops subscribe to the current Webview target instead of the generic app event bus',
  )
  eq(assetCardDrag.includes("window.removeEventListener('pointermove', onPointerMove)"), true, 'Shared asset drag controller removes pointermove listeners')
  eq(assetCardDrag.includes("window.removeEventListener('pointerup', onPointerUp)"), true, 'Shared asset drag controller removes pointerup listeners')
  eq(assetCardDrag.includes('active.current = null'), true, 'Shared asset drag controller clears active drag on release/unmount')
  eq(assets.includes('planAssetInsertAtPlayhead'), true, 'Assets Insert delegates base-track placement to the shared placement planner')
  eq(assets.includes('data-cut-source-monitor-open'), true, 'Timed Assets expose the Source monitor')
  eq(sourceMonitor.includes('src: asset.proxy ?? sourceUrl(asset.id)'), true, 'Source monitor prefers a ready editing proxy before a potentially unsupported original codec')
  eq(assets.includes('proxy: row.proxy'), true, 'Assets passes the ready proxy into the Source monitor')
  eq(sourceMonitor.includes('data-cut-action="source-monitor-play"'), true, 'Source monitor exposes an accessible Play and Pause action')
  eq(sourceMonitor.includes('await media.play()'), true, 'Source monitor Play drives the real media element')
  eq(sourceMonitor.includes('src_range_ms: [sourceIn, sourceOut]'), true, 'Source monitor inserts the marked range')
  eq(sourceMonitor.includes("asset.kind === 'video' && asset.hasAudio && !result.audioLinked"), true, 'Source monitor reports partial linked-audio placement')
  eq((placement.match(/src_range_ms: opts\.src_range_ms/g) ?? []).length, 3, 'Shared placement applies the source range to audio, video, and linked-audio inserts')
  eq(assets.includes('new line for an inserted clip'), false, 'Assets Insert no longer creates a new overlay track by default')
  eq(assets.includes('Alt-drag or drop on an overlay lane'), true, 'Assets tray explains that overlay placement is explicit')
  eq(assets.includes("Inserted on ${where || 'the base timeline'}"), true, 'Assets Insert success copy names the base timeline fallback')
  eq(assemble.includes('const [brollAtTouched, setBrollAtTouched] = useState(false)'), true, 'Assemble tracks whether the user edited the b-roll placement field')
  eq(assemble.includes('if (!brollAtTouched) setBrollAtS'), true, 'Assemble follows the live playhead until b-roll placement is manually edited')
  eq(assemble.includes('setBrollAtTouched(true)'), true, 'Assemble stops playhead-following once the user edits b-roll placement')
  eq(clientModel.includes('(t.opacity ?? 1) === 1'), true, 'isIdentityTransform treats non-default opacity as non-identity')
  eq(clipboard.includes('warnMultiSelectionClipboard'), true, 'Clipboard shortcuts warn instead of silently using the first multi-selected clip')
  eq(clipboard.includes('if (sel.length > 1)'), true, 'Clipboard shortcut guard checks multi-selection before copy/cut')
  eq(clipboard.includes('window.alert'), false, 'Clipboard multi-selection warning is non-modal')
  eq(statusbar.includes('data-cut-clipboard-notice'), true, 'Status bar surfaces clipboard shortcut notices')
  eq(statusbar.includes("callVerb('jobs.cancel'"), true, 'Status bar exposes the public jobs.cancel action from active job pills')
  eq(statusbar.includes('data-cut-job-cancel'), true, 'Status bar job cancel buttons have stable selectors')
  eq(libraryQuery.includes('const requestSeq = useRef(0)'), true, 'Library reload has a request sequence guard')
  eq(libraryQuery.includes('const seq = ++requestSeq.current'), true, 'Library reload increments the active request sequence')
  eq(libraryQuery.includes('if (seq !== requestSeq.current) return'), true, 'Library ignores stale list responses from older filter/sort requests')
  eq(library.includes("const moveResult = await callVerb('library.move'"), true, 'Library bulk move counts per-item verb results')
  eq(library.includes("const tagResult = await callVerb('library.tag'"), true, 'Library bulk tag counts per-item verb results')
  eq(library.includes("const removeResult = await callVerb('library.remove'"), true, 'Library bulk remove counts per-item verb results')
  eq(library.includes('Selection cleared because filters changed.'), true, 'Library names why selection disappears after a filter/search/sort change')
  eq(library.includes('asset grid (default)'), false, 'Library no longer claims grid is the default view')
  eq(comments.includes('data-cut-action="comment-done"'), true, 'Comments panel exposes a Done/Reopen status action')
  eq(comments.includes("c.status === 'addressed' ? 'open' : 'addressed'"), true, 'Comments Done action uses comment.resolve to toggle addressed state')
}

// --- Source assertions for visible copy and layout quality -------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const repoRoot = resolve(here, '../..')
  const envCards = readFileSync(resolve(repoRoot, 'ui/src/panels/Environment/EnvCards.tsx'), 'utf8')
  const translate = readFileSync(resolve(repoRoot, 'app/server/src/translate.rs'), 'utf8')
  const autopilot = readFileSync(resolve(repoRoot, 'ui/src/panels/Autopilot/index.tsx'), 'utf8')
  const autopilotCss = readFileSync(resolve(repoRoot, 'ui/src/panels/Autopilot/autopilot.css'), 'utf8')
  const clips = readFileSync(resolve(repoRoot, 'ui/src/panels/Clips/index.tsx'), 'utf8')
  const grade = readFileSync(resolve(repoRoot, 'ui/src/panels/Grade/index.tsx'), 'utf8')
  const recipes = readFileSync(resolve(repoRoot, 'ui/src/panels/Recipes/index.tsx'), 'utf8')

  eq(envCards.includes('const [, force]'), false, 'EnvCards does not keep a dead force-render state hook')
  eq(envCards.includes('useEffect'), false, 'EnvCards does not import/use a no-op refresh effect')
  eq(translate.includes('let cli_available = pick_cli_agent(None).is_some();'), false, 'translation backend selection is not duplicated before run_translation_once')
  eq(autopilot.includes("f.failed ? 'ap-tag--failed' : 'ap-tag--auto'"), true, 'Autopilot failed fixes do not reuse manual status styling')
  eq(autopilotCss.includes('.ap-tag--failed'), true, 'Autopilot has distinct failed-fix styling')
  eq(clips.includes('data-cut-clip-thumb-fallback'), true, 'Clips candidate thumbnails have a visible fallback when frame extraction fails')
  eq(clips.includes('onError={(e) =>'), true, 'Clips thumbnail image records frame extraction failure')
  eq(grade.includes('data-cut-grade-scrim'), false, 'Grade drawer no longer carries the dead modal scrim branch')
  eq(grade.includes('legacy modal'), false, 'Grade drawer no longer documents a dead legacy modal branch')
  eq(recipes.includes('recipePlanErrorMessage'), true, 'Recipe preview preserves non-planned dry-run status/reason details')
}

// --- Render busy gate covers all render-family jobs --------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const jobs = readFileSync(resolve(srcRoot, 'topbar/useTopbarJobs.ts'), 'utf8')

  eq(jobs.includes('export function isRenderBlockingJobKind'), true, 'topbar render gate has an explicit job-kind predicate')
  for (const kind of ["kind === 'render'", "kind === 'reframe'", "kind.startsWith('reframe-')", "kind === 'render_queue'"]) {
    eq(jobs.includes(kind), true, `render gate blocks ${kind}`)
  }
  eq(jobs.includes('jobList.some((j) => isRenderBlockingJobKind(j.kind))'), true, 'renderRunning uses the render-family predicate')
}

// --- UI correctness regressions ---------------------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const srcRoot = resolve(here, '../src')
  const repoRoot = resolve(here, '../..')
  const mask = readFileSync(resolve(srcRoot, 'panels/Mask/index.tsx'), 'utf8')
  const recipes = readFileSync(resolve(srcRoot, 'panels/Recipes/index.tsx'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const diff = readFileSync(resolve(srcRoot, 'panels/Review/DiffView.tsx'), 'utf8')
  const opsFeed = readFileSync(resolve(srcRoot, 'panels/Review/OpsFeed.tsx'), 'utf8')
  const statusbar = readFileSync(resolve(srcRoot, 'statusbar/index.tsx'), 'utf8')
  const fullCoverage = readFileSync(resolve(repoRoot, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const pasteAttributes = readFileSync(resolve(srcRoot, 'panels/Timeline/PasteAttributesDialog.tsx'), 'utf8')
  const markerMenu = readFileSync(resolve(srcRoot, 'panels/Timeline/MarkerContextMenu.tsx'), 'utf8')
  const timeline = readFileSync(resolve(srcRoot, 'panels/Timeline/index.tsx'), 'utf8')

  eq(mask.includes('const MASK_EFFECT_COPY'), true, 'Mask result copy uses an explicit effect-to-human label map')
  eq(mask.includes("pixelate: 'pixelated'"), true, 'Mask result copy renders pixelate as pixelated')
  eq(mask.includes("result.effect + 'red'"), false, 'Mask result copy cannot produce pixelatered/blurred via string concatenation')
  eq(mask.includes('privacy redaction or a region cleanup (edit.add_mask)'), false, 'Mask drawer subtitle does not expose edit.add_mask to casual users')
  eq(mask.includes('Apply edit.add_mask'), false, 'Mask apply tooltip does not expose edit.add_mask')
  eq(mask.includes('Apply mask to this clip'), true, 'Mask apply tooltip uses plain action copy')
  eq(mask.includes('const MASK_PRESETS'), true, 'Mask drawer exposes beginner privacy presets')
  eq(mask.includes("id: 'face'"), true, 'Mask drawer includes a face blur preset')
  eq(mask.includes("label: 'Blur face'"), true, 'Mask drawer labels face privacy in plain language')
  eq(mask.includes("label: 'Blur rectangle'"), true, 'Mask drawer includes a general rectangle preset')
  eq(mask.includes("label: 'Hide plate/text'"), true, 'Mask drawer includes a plate/text privacy preset')
  eq(mask.includes('data-cut-mask-duration'), true, 'Mask drawer exposes duration controls')
  eq(mask.includes("callVerb('edit.redact'"), true, 'Mask drawer can dispatch timed redaction edits')
  eq(mask.includes('range_ms: timedRange'), true, 'Timed mask mode sends a clip-local range_ms')
  eq(mask.includes('From playhead'), true, 'Timed mask mode is phrased as an editor action')
  eq(mask.includes('data-cut-mask-duration-seconds'), true, 'Timed mask mode exposes a duration amount control')
  eq(recipes.includes('Named, gated workflows over the editing verbs'), false, 'Recipes drawer subtitle does not expose verb-system copy by default')
  eq(recipes.includes('recipe.list / run'), false, 'Recipes drawer visible copy does not name recipe.list/run')
  eq(recipes.includes('function stageTitle'), true, 'Recipes drawer has a human stage label helper')
  eq(recipes.includes('className="rc-technical"'), true, 'Recipes drawer hides raw stage details in an advanced disclosure')
  eq(topbar.includes('(edit.add_mask)'), false, 'Topbar mask tooltip does not expose edit.add_mask')
  eq(topbar.includes('(recipe.list / run)'), false, 'Topbar recipes tooltip does not expose recipe.list/run')
  eq(fullCoverage.includes('edit.add_mask(verb-level · no UI control)'), false, 'Full coverage no longer claims Mask has no UI control')
  eq(fullCoverage.includes('recipe.list(verb-level · no UI control)'), false, 'Full coverage no longer claims Recipes has no UI control')
  eq(fullCoverage.includes('mask-apply(edit.add_mask)'), true, 'Full coverage drives Mask drawer apply')
  eq(fullCoverage.includes('recipe-drawer-open'), true, 'Full coverage opens the Recipes drawer')
  eq(pasteAttributes.includes('from <code>{fromClip}</code>'), false, 'Paste Attributes dialog does not expose raw source clip ids in visible copy')
  eq(pasteAttributes.includes('data-cut-pa-source={fromClip}'), true, 'Paste Attributes dialog keeps the source clip id as a debug selector attribute')
  eq(pasteAttributes.includes('Choose what to copy onto'), true, 'Paste Attributes dialog explains the action in plain language')
  eq(markerMenu.includes('data-cut-marker-note-input'), true, 'Marker menu exposes a stable marker note input selector')
  eq(markerMenu.includes('data-cut-marker-ctx="note-commit"'), true, 'Marker menu exposes a stable marker note save action')
  eq(timeline.includes('note: m.note'), true, 'Timeline passes marker notes into the marker context menu')
  eq(timeline.includes("runUserVerb('edit.update_marker', { id, note"), true, 'Timeline saves marker notes through edit.update_marker with visible failure feedback')
  eq(diff.includes('const defaultsSeededRef = useRef(false)'), true, 'DiffView seeds default selectors only once per project/checkpoint set')
  eq(diff.includes("setError(from === to ? 'Pick two different comparison points.' : '')"), true, 'DiffView clears stale diff and names equal-selector invalid state')
  eq(diff.includes('setDiff(null)'), true, 'DiffView clears stale diff output before returning from invalid selector state')
  eq(opsFeed.includes("const lastOpId = ops.length > 0 ? ops[ops.length - 1].op_id : ''"), true, 'OpsFeed auto-follow tracks the actual newest op id')
  eq(opsFeed.includes('}, [lastOpId])'), true, 'OpsFeed auto-follow reruns when the newest op changes, not only when length changes')
  eq(opsFeed.includes('key={op.op_id}'), true, 'OpsFeed rows are keyed by stable op id, not index')
  eq(statusbar.includes('doctor!.cards'), false, 'Status bar env chip does not force-unwrap doctor in the ok/default path')
  eq(statusbar.includes('envHealthLevel(doctor)'), true, 'Status bar env chip delegates missing doctor data to the defensive shared health model')
}

// --- UI persistence and status regressions -----------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const srcRoot = resolve(root, 'ui/src')
  const review = readFileSync(resolve(srcRoot, 'panels/Review/index.tsx'), 'utf8')
  const reviewMarkers = readFileSync(resolve(srcRoot, 'panels/Review/reviewMarkers.ts'), 'utf8')
  const qc = readFileSync(resolve(srcRoot, 'panels/Review/QC.tsx'), 'utf8')
  const search = readFileSync(resolve(srcRoot, 'panels/Search/index.tsx'), 'utf8')
  const assets = readFileSync(resolve(srcRoot, 'panels/Assets/index.tsx'), 'utf8')
  const sourceMonitor = readFileSync(resolve(srcRoot, 'panels/Assets/SourceMonitor.tsx'), 'utf8')
  const searchRoutingVerify = readFileSync(resolve(here, 'verify-search-source-routing.mjs'), 'utf8')
  const topbar = readFileSync(resolve(srcRoot, 'topbar/index.tsx'), 'utf8')
  const client = readFileSync(resolve(srcRoot, 'lib/client.ts'), 'utf8')
  const results = readFileSync(resolve(srcRoot, 'lib/clientResults.ts'), 'utf8')
  const registry = readFileSync(resolve(root, 'app/server/src/registry.rs'), 'utf8')
  const schema = readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')

  eq(reviewMarkers.includes('const REVIEWED_STORAGE_PREFIX'), true, 'Review accepted/rejected markers have a persistent localStorage namespace')
  eq(reviewMarkers.includes('reviewedStorageKey(projectName)'), true, 'Review persistence is scoped to the current project')
  eq(reviewMarkers.includes('localStorage.getItem(reviewedStorageKey(projectName))'), true, 'Review marker helper loads persisted accepted markers after reload')
  eq(reviewMarkers.includes('localStorage.setItem(reviewedStorageKey(projectName)'), true, 'Review marker helper saves accepted markers so pending count survives reload')
  eq(review.includes('loadReviewMarkers(project.name)'), true, 'Review loads markers through the shared persistence helper')
  eq(review.includes('saveReviewMarkers(project.name, reviewed)'), true, 'Review saves markers through the shared persistence helper')
  eq(qc.includes('const JUDGE_JOB_STORAGE_KEY'), true, 'QC judge run stores the active job id before polling')
  eq(qc.includes('resumeJudgeJob'), true, 'QC can resume polling an in-flight judge job after tab remount')
  eq(qc.includes("localStorage.setItem(JUDGE_JOB_STORAGE_KEY, jobId)"), true, 'QC persists verify.judge job id immediately after start')
  eq(qc.includes('localStorage.removeItem(JUDGE_JOB_STORAGE_KEY)'), true, 'QC clears persisted judge job id on terminal status')
  eq(schema.includes('"name": "media.index_status"'), true, 'schema exposes media.index_status for visual-search index status')
  eq(client.includes("'media.index_status': { asset?: string }"), true, 'typed client exposes media.index_status args')
  eq(results.includes("'media.index_status': { count: number; assets:"), true, 'typed client exposes media.index_status result')
  eq(registry.includes('"media.index_status"'), true, 'verb registry exposes media.index_status')
  eq(schema.includes('"name": "jobs.cancel"'), true, 'schema exposes jobs.cancel for cancellable background jobs')
  eq(client.includes("'jobs.cancel': { job_id: string }"), true, 'typed client exposes jobs.cancel args')
  eq(results.includes("'jobs.cancel': { job_id: string; cancelled: boolean }"), true, 'typed client exposes jobs.cancel result')
  eq(registry.includes('"jobs.cancel"'), true, 'verb registry exposes jobs.cancel')
  eq(search.includes("callVerb('media.index_status'"), true, 'Search loads persisted index status from the engine')
  eq(search.includes('setIndexed(statusMap)'), true, 'Search replaces session-only indexed state with engine status')
  eq(search.includes('sourceTimelineOccurrences(project, hit.asset, hit.peak_ms)'), true, 'Search maps source-relative hits through real timeline placements')
  eq(search.includes("jump(h.peak_ms)"), false, 'Search no longer treats source time as timeline time')
  eq(search.includes('data-cut-search-source'), true, 'Search results can open the exact source moment')
  eq(search.includes("new CustomEvent('cut:open-source-monitor'"), true, 'Search routes source hits through the shared Source monitor')
  eq(assets.includes("addEventListener('cut:open-source-monitor'"), true, 'Assets receives indexed source-hit open requests')
  eq(sourceMonitor.includes('initialMs = 0'), true, 'Source monitor accepts an exact initial source position')
  eq(searchRoutingVerify.includes("playheadArgs?.at_ms === 13_000"), true, 'Search routing verifier proves source time is mapped to edited timeline time')
  eq(searchRoutingVerify.includes("unusedTimelineAction === 0"), true, 'Search routing verifier proves unused hits do not expose a false timeline jump')
  eq(topbar.includes('const jobsChipTitle ='), true, 'Topbar caps the jobs chip tooltip through a named helper/value')
  eq(topbar.includes('slice(0, 4)'), true, 'Topbar jobs tooltip shows only a short active-job sample')
  eq(topbar.includes("jobList.length > 4"), true, 'Topbar jobs tooltip summarizes overflow jobs instead of dumping every id')
}

// --- Sidecar stdin failures are explicit -------------------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const translate = readFileSync(resolve(root, 'app/server/src/translate.rs'), 'utf8')
  const dub = readFileSync(resolve(root, 'app/server/src/dub.rs'), 'utf8')
  const diarize = readFileSync(resolve(root, 'app/server/src/diarize.rs'), 'utf8')

  for (const [name, text] of [['translate', translate], ['dub', dub], ['diarize', diarize]] as const) {
    eq(text.includes('let _ = stdin.write_all'), false, `${name} runner/CLI stdin write errors are not ignored`)
    eq(text.includes('let _ = stdin.shutdown'), false, `${name} runner/CLI stdin close errors are not ignored`)
  }
  eq(translate.includes('local translate runner stdin write failed'), true, 'Local translate runner reports stdin write failures')
  eq(translate.includes('translation CLI stdin write failed'), true, 'Translation CLI reports stdin write failures')
  eq(dub.includes('dub runner stdin write failed'), true, 'Dub runner reports stdin write failures')
  eq(diarize.includes('diarize runner stdin write failed'), true, 'Diarize runner reports stdin write failures')
}

// --- Matte runner ffmpeg child failures are explicit -------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const matte = readFileSync(resolve(root, 'app/perception/py/matte_runner.py'), 'utf8')
  const matanyone = readFileSync(resolve(root, 'app/perception/py/matanyone_runner.py'), 'utf8')

  for (const [name, text] of [['matte', matte], ['matanyone', matanyone]] as const) {
    eq(text.includes('stderr=subprocess.PIPE'), true, `${name} runner captures ffmpeg stderr`)
    eq(text.includes('if proc.returncode != 0:'), true, `${name} runner checks child return codes`)
    eq(text.includes('failed with exit {proc.returncode}'), true, `${name} runner reports child non-zero exits`)
  }
}

// --- macOS WebDriver automation stays test-only ------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const cargoToml = readFileSync(resolve(root, 'app/desktop/src-tauri/Cargo.toml'), 'utf8')
  const tauriLib = readFileSync(resolve(root, 'app/desktop/src-tauri/src/lib.rs'), 'utf8')
  const buildLinux = readFileSync(resolve(root, 'scripts/build-linux.sh'), 'utf8')
  const buildMac = readFileSync(resolve(root, 'scripts/build-macos.sh'), 'utf8')
  const buildWin = readFileSync(resolve(root, 'scripts/build-windows.sh'), 'utf8')
  const packageJson = JSON.parse(readFileSync(resolve(root, 'ui/package.json'), 'utf8')) as {
    scripts?: Record<string, string>
    dependencies?: Record<string, string>
    devDependencies?: Record<string, string>
    overrides?: Record<string, string>
  }
  const wdioConfigPath = resolve(root, 'ui/wdio.tauri.conf.mjs')
  const wdioSpecPath = resolve(root, 'ui/public-tests/wdio/macos-track-controls.e2e.mjs')
  const mediaDragSpecPath = resolve(root, 'ui/public-tests/wdio/macos-media-drag.e2e.mjs')
  const dropToCreateSpecPath = resolve(root, 'ui/public-tests/wdio/drop-to-create.e2e.mjs')
  const composedPlaybackSpecPath = resolve(root, 'ui/public-tests/wdio/macos-composed-playback.e2e.mjs')
  const wdioRunnerPath = resolve(root, 'scripts/macos-wdio-track-controls.mjs')
  const wdioConfig = existsSync(wdioConfigPath) ? readFileSync(wdioConfigPath, 'utf8') : ''
  const wdioSpec = existsSync(wdioSpecPath) ? readFileSync(wdioSpecPath, 'utf8') : ''
  const mediaDragSpec = existsSync(mediaDragSpecPath) ? readFileSync(mediaDragSpecPath, 'utf8') : ''
  const dropToCreateSpec = existsSync(dropToCreateSpecPath) ? readFileSync(dropToCreateSpecPath, 'utf8') : ''
  const composedPlaybackSpec = existsSync(composedPlaybackSpecPath) ? readFileSync(composedPlaybackSpecPath, 'utf8') : ''
  const wdioRunner = existsSync(wdioRunnerPath) ? readFileSync(wdioRunnerPath, 'utf8') : ''
  const installedRuntimeEvidence = readFileSync(resolve(root, 'scripts/lib/installed-runtime-evidence.mjs'), 'utf8')

  eq(cargoToml.includes('tauri-plugin-wdio-webdriver = { version = "1.2.0", optional = true }'), true, 'WDIO embedded WebDriver dependency is optional')
  eq(cargoToml.includes('tauri-plugin-wdio = { version = "1.2.0", optional = true }'), true, 'WDIO backend helper dependency is optional')
  eq(cargoToml.includes('webdriver-test = ["dep:tauri-plugin-wdio-webdriver", "dep:tauri-plugin-wdio"]'), true, 'WDIO plugins are exposed only through the webdriver-test feature')
  eq(tauriLib.includes('#[cfg(feature = "webdriver-test")]'), true, 'Tauri WDIO registration is cfg-gated')
  eq(tauriLib.includes('tauri_plugin_wdio_webdriver::init()'), true, 'Test builds can register the embedded WDIO WebDriver server')
  eq(tauriLib.includes('tauri_plugin_wdio::init()'), true, 'Test builds can register WDIO backend helpers')
  eq(tauriLib.includes('shellx-cut/webdriver-test-enabled@1'), true, 'Instrumented native builds contain an unambiguous test-feature marker')
  eq(installedRuntimeEvidence.includes("proof: 'binary-marker-absent'"), true, 'External Linux qualification scans the exact shipping binary for test instrumentation')
  eq(tauriLib.includes('#[cfg(not(feature = "webdriver-test"))]'), false, 'WDIO plugin registration is not inverted into shipping builds')
  eq(buildMac.includes('FEATURES_STR="${TAURI_FEATURES:-}"'), true, 'macOS shipping build checks explicit Tauri features before packaging')
  eq(buildMac.includes('FAIL: webdriver-test feature is test-only and must not be enabled for shipping macOS builds'), true, 'macOS shipping build rejects webdriver-test')
  eq(buildWin.includes('FEATURES_STR="${TAURI_FEATURES:-}"'), true, 'Windows shipping build checks explicit Tauri features before packaging')
  eq(buildWin.includes('FAIL: webdriver-test feature is test-only and must not be enabled for shipping Windows builds'), true, 'Windows shipping build rejects webdriver-test')
  eq(buildLinux.includes('FAIL: webdriver-test must not be enabled for shipping Linux builds'), true, 'Linux shipping build rejects webdriver-test')
  eq(
    tauriLib.includes('#[cfg(feature = "webdriver-test")]\n    let capability = capability')
      && tauriLib.includes('.permission("core:event:allow-emit-to")')
      && tauriLib.includes('.permission("wdio:allow-log-frontend");'),
    true,
    'Targeted native event injection and WDIO console forwarding exist only in the test-feature branch',
  )
  eq((tauriLib.match(/\.permission\("core:event:allow-emit-to"\)/g) || []).length, 1, 'Test-only targeted event injection permission has exactly one grant')
  eq(packageJson.scripts?.['wdio:mac-track-controls'], 'wdio run wdio.tauri.conf.mjs --spec public-tests/wdio/macos-track-controls.e2e.mjs', 'package script exposes the macOS WDIO track-control test')
  eq(packageJson.scripts?.['wdio:mac-media-drag'], 'wdio run wdio.tauri.conf.mjs --spec public-tests/wdio/macos-media-drag.e2e.mjs', 'package script exposes the macOS media-drag test')
  eq(packageJson.scripts?.['wdio:native-drop-to-create'], 'wdio run wdio.tauri.conf.mjs --spec public-tests/wdio/drop-to-create.e2e.mjs', 'package script exposes the cross-platform drop-to-create test')
  eq(packageJson.dependencies?.['@tauri-apps/api'], '^2.11.0', 'native file drops use an explicit supported Tauri Webview API dependency')
  eq(packageJson.scripts?.['wdio:mac-composed-playback'], 'wdio run wdio.tauri.conf.mjs --spec public-tests/wdio/macos-composed-playback.e2e.mjs', 'package script exposes the macOS composed-playback test')
  eq(!!packageJson.devDependencies?.['@wdio/tauri-service'], true, 'WDIO Tauri service is declared as test tooling')
  eq(!!packageJson.devDependencies?.['@wdio/cli'], true, 'WDIO CLI is declared as test tooling')
  eq(packageJson.overrides?.['@wdio/native-utils'], '2.5.0', 'WDIO native-utils is pinned to the version that exports installMockSyncOverride')
  eq(
    wdioConfig.includes("process.env.SHELLX_CUT_WDIO_PROVIDER || 'embedded'"),
    true,
    'WDIO config defaults macOS WKWebView candidates to the embedded provider',
  )
  eq(
    wdioConfig.includes('driverProvider,'),
    true,
    'WDIO config forwards the selected embedded or external native provider',
  )
  eq(
    wdioConfig.includes("const captureBackendLogs = process.env.WDIO_CAPTURE_BACKEND_LOGS === '1'"),
    true,
    'WDIO backend log capture is available only through an explicit diagnostic opt-in',
  )
  eq(
    wdioConfig.includes("const captureFrontendLogs = process.env.WDIO_CAPTURE_FRONTEND_LOGS === '1'"),
    true,
    'WDIO frontend log capture is available only through an explicit diagnostic opt-in',
  )
  eq(wdioConfig.includes('captureBackendLogs,'), true, 'WDIO config forwards the backend diagnostic switch')
  eq(wdioConfig.includes('captureFrontendLogs,'), true, 'WDIO config forwards the frontend diagnostic switch')
  eq(wdioConfig.includes('captureBackendLogs: false'), false, 'WDIO backend diagnostic capture is not hard-disabled')
  eq(wdioConfig.includes('captureFrontendLogs: false'), false, 'WDIO frontend diagnostic capture is not hard-disabled')
  eq(wdioConfig.includes('SHELLX_CUT_WDIO_APP'), true, 'WDIO config reads the test-built app path from env')
  eq(wdioConfig.includes('tauri'), true, 'WDIO config registers the Tauri service')
  eq(wdioSpec.includes('data-cut-action="toggle-track-visibility"'), true, 'macOS WDIO spec clicks track visibility control')
  eq(wdioSpec.includes('data-cut-action="toggle-track-lock"'), true, 'macOS WDIO spec clicks track lock control')
  eq(wdioSpec.includes('data-cut-action="set-pan"'), true, 'macOS WDIO spec changes the track pan control')
  eq(wdioSpec.includes('data-cut-action="toggle-mute"'), true, 'macOS WDIO spec clicks timeline header mute')
  eq(wdioSpec.includes('data-cut-action="toggle-solo"'), true, 'macOS WDIO spec clicks timeline header solo')
  eq(wdioSpec.includes('data-cut-action="track-listen"'), true, 'macOS WDIO spec clicks timeline header listen')
  eq(wdioSpec.includes("'export.audio'") || wdioSpec.includes('"export.audio"'), true, 'macOS WDIO spec proves listen through export.audio')
  eq(wdioSpec.includes('project.state'), true, 'macOS WDIO spec asserts effects through project state')
  eq(mediaDragSpec.includes('SHELLX_CUT_WDIO_LIBRARY_CLIP'), true, 'macOS media-placement spec uses real Assets and Library media')
  eq(mediaDragSpec.includes('pointercancel'), true, 'macOS media-drag spec proves cancellation does not place media')
  eq(dropToCreateSpec.includes("emitTauriDropEvent('tauri://drag-drop'"), true, 'Drop-to-create spec crosses the native Tauri event bridge')
  eq(dropToCreateSpec.includes("events.emitTo({ kind: 'Webview', label }"), true, 'Drop-to-create injection targets the same current Webview as real native drops')
  eq(dropToCreateSpec.includes('first video width was not adopted'), true, 'Drop-to-create spec proves first-video timeline format adoption')
  eq(dropToCreateSpec.includes('five-second timeline clip'), true, 'Drop-to-create spec proves still-image placement')
  eq(dropToCreateSpec.includes('final installed three-host gate must still perform a real OS file drag'), true, 'Candidate receipt keeps real OS drag ownership in the final installed gate')
  eq(composedPlaybackSpec.includes("surface === 'live-composite'"), true, 'macOS composed-playback spec requires the responsive live surface')
  eq(composedPlaybackSpec.includes('value.videoTime > startTime + 0.35'), true, 'macOS composed-playback spec proves the media clock advances')
  eq(composedPlaybackSpec.includes('value.posterLuma > 20'), true, 'macOS composed-playback spec proves exact frames contain visible pixels')
  eq(wdioRunner.includes('rsync'), true, 'macOS WDIO wrapper syncs the current source tree to the configured test host')
  eq(/\/Users\/[^/]+\/Developer\//.test(wdioRunner), false, 'macOS WDIO wrapper does not publish a machine-specific home directory')
  eq(wdioRunner.includes('--features webdriver-test'), true, 'macOS WDIO wrapper builds the test-only feature explicitly')
  eq(wdioRunner.includes('pwd -P'), true, 'macOS WDIO wrapper canonicalizes shared Cargo target paths before launch')
  eq(wdioRunner.includes('SHELLX_CUT_WDIO_CLIP'), true, 'macOS WDIO wrapper passes a real media clip to the spec')
}

// --- native minimum window and browser contract stay aligned -----
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const tauri = JSON.parse(readFileSync(resolve(root, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8')) as {
    app?: { windows?: Array<{ label?: string; minWidth?: number; minHeight?: number }> }
  }
  const mainWindow = tauri.app?.windows?.find((window) => window.label === 'main')
  const theme = readFileSync(resolve(root, 'ui/src/theme.css'), 'utf8')
  const previewCss = readFileSync(resolve(root, 'ui/src/panels/Preview/preview.css'), 'utf8')
  const timelineCss = readFileSync(resolve(root, 'ui/src/panels/Timeline/timeline.css'), 'utf8')
  const timelineTrackRow = readFileSync(resolve(root, 'ui/src/panels/Timeline/TimelineTrackRow.tsx'), 'utf8')
  const fullCoverage = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const recordCss = readFileSync(resolve(root, 'ui/src/panels/Record/record.css'), 'utf8')
  const app = readFileSync(resolve(root, 'ui/src/App.tsx'), 'utf8')
  const workspace = readFileSync(resolve(root, 'ui/src/app/AppWorkspace.tsx'), 'utf8')
  const layoutGate = readFileSync(resolve(root, 'ui/public-tests/layout-contract-verify.mjs'), 'utf8')
  const packageJson = JSON.parse(readFileSync(resolve(root, 'ui/package.json'), 'utf8')) as {
    scripts?: Record<string, string>
  }

  eq(mainWindow?.minWidth, 1100, 'Tauri main window keeps the verified 1100px minimum width')
  eq(mainWindow?.minHeight, 680, 'Tauri main window keeps the verified 680px minimum height')
  eq(theme.includes('min-width: 1100px;'), true, 'App CSS minimum width matches the native window contract')
  eq(theme.includes('min-height: 680px;'), true, 'App CSS minimum height matches the native window contract')
  eq(previewCss.includes('@container preview-panel (max-width: 820px)'), true, 'Preview transport compacts from its panel width, not only viewport width')
  eq(timelineCss.includes('justify-content: safe center'), true, 'Overflowing timeline tools fall back to a reachable start edge')
  eq(timelineCss.includes(".tl-track-head[data-cut-track-kind='audio'] .tl-track-actions"), true, 'Audio actions use a bounded second row inside the sticky track rail')
  eq(timelineCss.includes('@keyframes tl-ctx-in'), false, 'Timeline context surfaces do not retain the native-unsafe entrance keyframe')
  eq(timelineCss.includes('animation: none;'), true, 'Timeline context surfaces remain immediately visible in native WebViews')
  eq(timelineCss.includes('transform: translate(-50%, -50%);'), true, 'Paste Attributes opens as a centered modal instead of inheriting raw menu coordinates')
  eq(timelineTrackRow.includes("track.kind === 'audio' && <TrackLockButton"), true, 'Audio lock remains with track identity instead of overflowing into the lane')
  eq(fullCoverage.includes('TRACK-HEADER-OVERFLOW'), true, 'Full coverage fails when a track-header control crosses into the timeline lane')
  eq(fullCoverage.includes('final track-header overflow query'), true, 'Full coverage bounds the final native-WebView layout query')
  eq(fullCoverage.includes('coverage browser disconnect'), true, 'Full coverage bounds the native browser disconnect before writing its receipt')
  eq(app.includes('data-cut-overlay-rail-open='), true, 'App exposes the unpinned overlay rail state to editor layout CSS')
  eq(app.includes("'--cut-overlay-rail-width'"), true, 'App exposes the live overlay rail width to editor layout CSS')
  eq(timelineCss.includes("[data-cut-overlay-rail-open='true'] .tl-toolbar"), true, 'Timeline toolbar reserves the open overlay rail width')
  for (const viewport of ['1100, height: 680', '1280, height: 760', '1440, height: 900', '1920, height: 1080']) {
    eq(layoutGate.includes(viewport), true, `Layout runtime gate covers ${viewport.replace(', height: ', 'x')}`)
  }
  eq(layoutGate.includes('rootHorizontalOverflow'), true, 'Layout runtime gate fails root horizontal overflow')
  eq(layoutGate.includes('topbarOverlaps'), true, 'Layout runtime gate fails topbar rectangle intersections')
  eq(layoutGate.includes('stripFullscreenOverlap'), true, 'Layout runtime gate fails Tools and Full Screen intersections')
  eq(layoutGate.includes('Full Screen enters with an ordinary click'), true, 'Layout runtime gate proves Full Screen without forced clicks')
  eq(layoutGate.includes('Automate stays clickable with overlay tools open'), true, 'Layout runtime gate covers Automate under the open overlay rail')
  eq(layoutGate.includes('Automate opens with an ordinary click'), true, 'Layout runtime gate proves the automation menu without forced clicks')
  eq(layoutGate.includes('Escape closes Automate before overlay tools'), true, 'Layout runtime gate proves layered Escape behavior')
  eq(workspace.includes('recordTimelineDeferred'), true, 'Empty Record workspace defers the edit timeline')
  eq(workspace.includes('track.clips.length > 0'), true, 'Record restores the edit timeline after a take exists')
  eq(workspace.includes('recordTimelineCompact ? 160 : layout.tlH'), true, 'Record keeps an existing take timeline compact')
  eq(recordCss.includes('"settings transport"'), true, 'Record places setup and transport in the first layout row')
  eq(layoutGate.includes('record Source stays in first viewport'), true, 'Layout runtime gate covers the Record source picker')
  eq(layoutGate.includes('record Start stays in first viewport'), true, 'Layout runtime gate covers the Record start action')
  eq(layoutGate.includes('empty Record defers timeline'), true, 'Layout runtime gate covers the empty Record timeline contract')
  eq(fullCoverage.includes('GATE:record-existing-timeline-compact'), true, 'Full coverage gates the existing-take compact timeline')
  eq(packageJson.scripts?.['verify-layout-contract'], 'node public-tests/layout-contract-verify.mjs', 'Package script exposes the layout contract gate')
}

// --- bottom-to-top video-layer contract ---------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const layer = readFileSync(resolve(root, 'ui/src/panels/Layer/index.tsx'), 'utf8')
  const inspector = readFileSync(resolve(root, 'ui/src/panels/Inspector/index.tsx'), 'utf8')
  const inspectorLockGate = readFileSync(resolve(root, 'ui/src/panels/Inspector/TrackLockGate.tsx'), 'utf8')
  const previewModel = readFileSync(resolve(root, 'ui/src/panels/Preview/model.ts'), 'utf8')
  const previewComposite = readFileSync(resolve(root, 'ui/src/panels/Preview/composite.ts'), 'utf8')
  const layerGate = readFileSync(resolve(root, 'ui/public-tests/verify-audio-layer.mjs'), 'utf8')

  eq(layer.includes('trackReorderTargetIndex'), true, 'Layer drawer computes same-kind reorder targets')
  eq(layer.includes('data-cut-layer-edit-fieldset'), true, 'Layer drawer exposes a lockable edit fieldset')
  eq(layer.includes('disabled={busy || place.locked}'), true, 'Locked tracks disable layer edits')
  eq(inspectorLockGate.includes('data-cut-inspector-edit-fieldset'), true, 'Inspector exposes the selected-track edit fieldset')
  eq(inspector.includes('locked={selectedTrackLocked}'), true, 'Locked tracks disable Inspector clip edits')
  eq(previewModel.includes('baseVideoTrackId(project.tracks)'), true, 'Preview resolves one stable base video track')
  eq(previewComposite.includes('if (track.visible === false) continue'), true, 'Preview compositing skips hidden overlay tracks')
  eq(layerGate.includes('layer-stack-order-contract'), true, 'Browser gate includes the video-layer ordering contract')
  eq(layerGate.includes('exactFrameSamples'), true, 'Layer browser gate proves behavior through composed frame pixels')
  eq(layerGate.includes('project.open'), true, 'Layer browser gate proves the saved project reopens')
  eq(layerGate.includes('data-cut-layer-locked-note'), true, 'Layer browser gate verifies locked editing state')
}

// --- generated-media lifecycle contract --------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const schema = JSON.parse(readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')) as {
    verbs: Array<{ name: string; args?: { properties?: Record<string, unknown> } }>
  }
  const generate = schema.verbs.find((verb) => verb.name === 'assets.generate')
  const placement = generate?.args?.properties?.placement
  const historyVerb = schema.verbs.find((verb) => verb.name === 'assets.generated_list')
  const server = readFileSync(resolve(root, 'app/server/src/dispatch/generated_assets.rs'), 'utf8')
  const generateServer = readFileSync(resolve(root, 'app/server/src/dispatch/edit_tools/assets_plugins.rs'), 'utf8')
  const panel = readFileSync(resolve(root, 'ui/src/panels/Generate/index.tsx'), 'utf8')
  const history = readFileSync(resolve(root, 'ui/src/panels/Generate/GenerationHistory.tsx'), 'utf8')
  const fullCoverage = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const generatedMediaCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageGeneratedMediaActions.mjs'), 'utf8')

  eq(!!historyVerb, true, 'Generated-media history is a public read verb')
  eq(!!generate?.args?.properties?.references, true, 'Generation schema exposes registered visual references')
  eq(!!generate?.args?.properties?.variation, true, 'Generation schema exposes immutable variation labels')
  eq(!!generate?.args?.properties?.placement, true, 'Generation schema exposes server-managed timeline placement')
  eq(placement?.oneOf?.[0]?.required?.join(','), 'mode,track,at_ms,duration_ms', 'Insert placement schema requires its complete slot contract')
  eq(placement?.oneOf?.[1]?.required?.join(','), 'mode,target_clip', 'Replace placement schema requires its selected clip contract')
  eq(server.includes('MAX_PROVENANCE_BYTES'), true, 'Generated provenance reads are size-bounded')
  eq(server.includes('read_generation_provenance'), true, 'History and reuse share validated provenance parsing')
  eq(server.includes('validate_generation_references'), true, 'Reference bytes are revalidated around provider work')
  eq(panel.includes("callVerb('assets.generated_list'"), true, 'Generate panel reads durable server history')
  const generatedHistoryCrosscheck = fullCoverage.slice(
    fullCoverage.indexOf("rec(S, 'assets.generated_list(Generate history)'"),
    fullCoverage.indexOf("rec(S, 'assets.generated_list(Generate history)'") + 260,
  )
  eq(generatedHistoryCrosscheck.includes("rowKind: 'support'"), true, 'Generated-history background read cannot impersonate its separately actuated card actions')
  eq(panel.includes("callVerb('assets.generate'"), true, 'Generate panel submits reference and variation inputs')
  eq(panel.includes('data-cut-generate-placement-mode'), true, 'Generate panel exposes asset-only, insert, and replace destinations')
  eq(panel.includes('data-cut-generate-retry'), true, 'Generate panel keeps explicit retry context for pending slots')
  eq(panel.includes('data-cut-generate-close'), false, 'Generate does not retain an unreachable legacy modal action')
  eq(panel.includes('data-cut-generate-embed'), true, 'Generate has one reachable embedded product surface')
  eq(generateServer.includes('PreparedGenerationPlacement'), true, 'Generation placement keeps internal placeholder paths out of public results')
  eq(generateServer.includes('apply_generated_placement'), true, 'Reuse and new generation share one timeline placement finalizer')
  eq(generateServer.includes('cleanup_abandoned_generation_placeholder'), true, 'Deleted pending targets clean abandoned placeholder assets')
  eq(history.includes("item.integrity === 'verified' && item.kind === 'image'"), true, 'Unverified generated images are not loaded as previews')
  eq(history.includes('data-cut-generated-use-reference'), true, 'History cards expose the reference action')
  eq(history.includes('data-cut-generated-variation'), true, 'History cards expose the variation action')
  eq(history.includes('data-cut-generated-compare-dialog'), true, 'History exposes same-family side-by-side comparison')
  eq(history.includes('data-cut-action="generated-compare-backdrop"'), true, 'History exposes the mousedown-driven comparison scrim as one explicit runtime action')
  eq(history.includes('data-cut-generated-insert'), true, 'History exposes direct timeline insertion')
  eq(history.includes('data-cut-generated-replace'), true, 'History exposes in-place selected-clip replacement')
  eq(fullCoverage.includes('createGeneratedMediaActionCoverage'), true, 'Full coverage installs the generated-media action helper')
  eq(fullCoverage.includes("pathname !== `/api/verb/${name}`"), true, 'Full coverage captures only the exact requested verb response')
  eq(generatedMediaCoverage.includes("actionId: 'generated-insert'"), true, 'Full coverage drives generated-history insertion')
  eq(generatedMediaCoverage.includes("actionId: 'generated-replace'"), true, 'Full coverage drives generated-history replacement')
  eq(generatedMediaCoverage.includes('insertedTimelineClip.click()'), true, 'Generated-media Replace setup selects its inserted target through the real timeline')
  eq(generatedMediaCoverage.includes('async function ensureGenerateOpen(page)'), true, 'Generated-media coverage can re-enter the Generate drawer after timeline focus changes')
  eq(generatedMediaCoverage.includes('await ensureGenerateOpen(page)'), true, 'Every generated-media placement click first restores its owning surface')
  eq(generatedMediaCoverage.includes('control.waitFor({ state: \'visible\', timeout: 12_000 })'), true, 'Generated-media placement never actuates a hidden mounted control')
  eq(generatedMediaCoverage.includes('await replace.scrollIntoViewIfNeeded()'), true, 'Generated-media Replace scrolls its restored drawer control into the native viewport')
  eq(generatedMediaCoverage.includes('resetVariation.isVisible().catch(() => false)'), true, 'Generated-media cleanup does not crash on an already hidden variation control')
  eq(generatedMediaCoverage.includes("actionId: 'generate-job-cancel'"), true, 'Full coverage drives generated-job cancellation')
  eq(generatedMediaCoverage.includes("waitFor({ state: 'visible', timeout: 8_000 }).catch(() => {})"), true, 'A missed generated-job cancel control fails its row without crashing the rest of Assets coverage')
}

// --- release-review UI clarity contract --------------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const assets = readFileSync(resolve(root, 'ui/src/panels/Assets/index.tsx'), 'utf8')
  const library = readFileSync(resolve(root, 'ui/src/panels/Library/index.tsx'), 'utf8')
  const libraryActions = readFileSync(resolve(root, 'ui/src/panels/Library/LibraryActions.tsx'), 'utf8')
  const libraryKeyboard = readFileSync(resolve(root, 'ui/src/panels/Library/useLibraryKeyboardNavigation.ts'), 'utf8')
  const libraryRelink = readFileSync(resolve(root, 'ui/src/panels/Library/useLibraryRelink.ts'), 'utf8')
  const libraryQuery = readFileSync(resolve(root, 'ui/src/panels/Library/useLibraryQuery.ts'), 'utf8')
  const libraryPagination = readFileSync(resolve(root, 'ui/src/panels/Library/LibraryPagination.tsx'), 'utf8')
  const libraryWorkspace = readFileSync(resolve(root, 'ui/src/panels/Library/LibraryWorkspace.tsx'), 'utf8')
  const libraryCard = readFileSync(resolve(root, 'ui/src/panels/Library/LibraryCard.tsx'), 'utf8')
  const generate = readFileSync(resolve(root, 'ui/src/panels/Generate/index.tsx'), 'utf8')
  const generateTemplates = readFileSync(resolve(root, 'ui/src/panels/GenerateTemplates/index.tsx'), 'utf8')
  const inspector = readFileSync(resolve(root, 'ui/src/panels/Inspector/index.tsx'), 'utf8')
  const engagement = readFileSync(resolve(root, 'ui/src/panels/Inspector/EngagementSection.tsx'), 'utf8')
  const preview = readFileSync(resolve(root, 'ui/src/panels/Preview/index.tsx'), 'utf8')
  const previewView = readFileSync(resolve(root, 'ui/src/panels/Preview/usePreviewViewOptions.ts'), 'utf8')
  const previewCss = readFileSync(resolve(root, 'ui/src/panels/Preview/preview.css'), 'utf8')
  const matte = readFileSync(resolve(root, 'ui/src/panels/Matte/index.tsx'), 'utf8')
  const clipView = readFileSync(resolve(root, 'ui/src/panels/Timeline/ClipView.tsx'), 'utf8')
  const trackRow = readFileSync(resolve(root, 'ui/src/panels/Timeline/TimelineTrackRow.tsx'), 'utf8')
  const mixer = readFileSync(resolve(root, 'ui/src/panels/Mixer/index.tsx'), 'utf8')
  const topbar = readFileSync(resolve(root, 'ui/src/topbar/index.tsx'), 'utf8')
  const topbarCss = readFileSync(resolve(root, 'ui/src/topbar/topbar.css'), 'utf8')
  const topbarModel = readFileSync(resolve(root, 'ui/src/topbar/model.ts'), 'utf8')
  const rightRail = readFileSync(resolve(root, 'ui/src/app/AppRightRail.tsx'), 'utf8')
  const tauri = readFileSync(resolve(root, 'ui/src/lib/tauri.ts'), 'utf8')
  const schema = readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')
  const dispatch = readFileSync(resolve(root, 'app/server/src/dispatch.rs'), 'utf8')
  const scaleGate = readFileSync(resolve(root, 'ui/public-tests/verify-library-scale.mjs'), 'utf8')
  const fullCoverage = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const assetsCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageAssetsActions.mjs'), 'utf8')
  const assetsSetupCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageAssetsSetupActions.mjs'), 'utf8')
  const assetsPickerCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageAssetsPickerActions.mjs'), 'utf8')
  const offlineMediaCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageOfflineMediaActions.mjs'), 'utf8')
  const previewCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoveragePreviewActions.mjs'), 'utf8')
  const matteCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageMatteActions.mjs'), 'utf8')
  const recordCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageRecordActions.mjs'), 'utf8')
  const environmentCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageEnvironmentActions.mjs'), 'utf8')
  const userActionFeedbackCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageUserActionFeedback.mjs'), 'utf8')
  const gradeCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageGradeActions.mjs'), 'utf8')
  const appChromeCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageAppChromeActions.mjs'), 'utf8')
  const statusbarCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageStatusbarActions.mjs'), 'utf8')
  const searchCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageSearchActions.mjs'), 'utf8')
  const clipsCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageClipsActions.mjs'), 'utf8')
  const clips = readFileSync(resolve(root, 'ui/src/panels/Clips/index.tsx'), 'utf8')
  const autopilotCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageAutopilotActions.mjs'), 'utf8')
  const autopilotPanel = readFileSync(resolve(root, 'ui/src/panels/Autopilot/index.tsx'), 'utf8')
  const inspectorConditionalCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageInspectorConditionalActions.mjs'), 'utf8')
  const motionTrackingSection = readFileSync(resolve(root, 'ui/src/panels/Inspector/MotionTrackingSection.tsx'), 'utf8')
  const sequenceSwitcherCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageSequenceSwitcherActions.mjs'), 'utf8')
  const topbarCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageTopbarActions.mjs'), 'utf8')
  const topbarDialogCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageTopbarDialogActions.mjs'), 'utf8')
  const reviewCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageReviewActions.mjs'), 'utf8')
  const recipeCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageRecipeActions.mjs'), 'utf8')
  const renderQueueCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageRenderQueueActions.mjs'), 'utf8')
  const renderQueueCss = readFileSync(resolve(root, 'ui/src/topbar/renderqueue.css'), 'utf8')
  const scopesCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageScopesActions.mjs'), 'utf8')
  const scopesPanel = readFileSync(resolve(root, 'ui/src/panels/Review/Scopes.tsx'), 'utf8')
  const reviewCss = readFileSync(resolve(root, 'ui/src/panels/Review/review.css'), 'utf8')
  const accessibilityCoverage = readFileSync(resolve(root, 'ui/public-tests/accessibility-surface-verify.mjs'), 'utf8')
  const generateTemplatePanel = readFileSync(resolve(root, 'ui/src/panels/GenerateTemplates/TemplatePanel.tsx'), 'utf8')
  const diffView = readFileSync(resolve(root, 'ui/src/panels/Review/DiffView.tsx'), 'utf8')
  const qcPanel = readFileSync(resolve(root, 'ui/src/panels/Review/QC.tsx'), 'utf8')
  const shapePanel = readFileSync(resolve(root, 'ui/src/panels/Shape/index.tsx'), 'utf8')
  const chatCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageChatActions.mjs'), 'utf8')
  const chatAttachmentPicker = readFileSync(resolve(root, 'ui/src/panels/AgentChat/AttachmentPicker.tsx'), 'utf8')
  const directorCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageDirectorActions.mjs'), 'utf8')
  const directorModal = readFileSync(resolve(root, 'ui/src/director/DirectorModal.tsx'), 'utf8')
  const directorCss = readFileSync(resolve(root, 'ui/src/director/director.css'), 'utf8')
  const transcriptCoverage = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageTranscriptActions.mjs'), 'utf8')
  const transcriptCss = readFileSync(resolve(root, 'ui/src/panels/Transcript/transcript.css'), 'utf8')
  const reviewQc = readFileSync(resolve(root, 'ui/src/panels/Review/QC.tsx'), 'utf8')
  const musicBed = readFileSync(resolve(root, 'ui/src/panels/MusicBed/index.tsx'), 'utf8')
  const drawerCss = readFileSync(resolve(root, 'ui/src/panels/drawer.css'), 'utf8')
  const grade = readFileSync(resolve(root, 'ui/src/panels/Grade/index.tsx'), 'utf8')
  const statusbar = readFileSync(resolve(root, 'ui/src/statusbar/index.tsx'), 'utf8')
  const search = readFileSync(resolve(root, 'ui/src/panels/Search/index.tsx'), 'utf8')
  const appWorkspace = readFileSync(resolve(root, 'ui/src/app/AppWorkspace.tsx'), 'utf8')
  const desktopShell = readFileSync(resolve(root, 'app/desktop/src-tauri/src/lib.rs'), 'utf8')
  const events = readFileSync(resolve(root, 'ui/src/lib/events.ts'), 'utf8')

  eq(assets.includes('Assets <small className="assets__scope">this project</small>'), true, 'Assets identifies its project-local scope')
  eq(libraryWorkspace.includes('Across every project'), true, 'Library identifies its cross-project scope')
  eq(library.includes('it lands here automatically'), false, 'Library empty state does not overclaim every import path')
  eq(library.includes('direct agent imports stay in this project'), true, 'Library empty state states the agent-import boundary')
  eq(assets.includes('data-cut-asset-in-library'), true, 'Project assets identify matching reusable Library media')
  eq(assets.includes('libraryMembershipBatches(projectLibraryIds)'), true, 'Assets checks exact Library membership in bounded batches')
  eq(assets.includes("callVerb('library.list', {\n        ids,"), true, 'Assets membership never relies on the first default Library page')
  eq(assets.includes("void callVerb('library.add'"), false, 'Assets waits for its Library mirror result so the cross-surface badge cannot race')
  eq(assets.includes("addEventListener('cut:library-changed'") && libraryQuery.includes("new CustomEvent('cut:library-changed')"), true, 'Assets refreshes cross-project badges after Library changes')
  eq(libraryCard.includes('data-cut-library-in-project'), true, 'Library media identifies when it is already in the open project')
  eq(library.includes('Store a managed copy in the Library folder'), true, 'Library managed-copy control explains portability')
  eq(libraryQuery.includes('const requestSeq = useRef(0)'), true, 'Library paging ignores stale asynchronous responses')
  eq(libraryQuery.includes("collection === 'favorites' || collection === 'missing'"), true, 'Favorites and Missing filter on the server before paging')
  eq(libraryPagination.includes('data-cut-library-page-prev'), true, 'Library exposes a keyboard-native Previous page action')
  eq(libraryPagination.includes('data-cut-library-page-next'), true, 'Library exposes a keyboard-native Next page action')
  eq(schema.includes('"name": "library.relink"'), true, 'Schema exposes content-honest Library relink')
  eq(dispatch.includes('"library.relink" => library_relink(args)'), true, 'REST and MCP dispatch route Library relink')
  eq(libraryRelink.includes("callVerb('library.relink'"), true, 'Library missing-source action calls the public relink verb')
  eq(tauri.includes('pickLibraryRelinkMedia'), true, 'Library relink uses a single-file native media picker')
  eq(libraryActions.includes('item.media_ok === false && item.src_path && !item.blob'), true, 'Relink is visible only for a missing linked source')
  eq(libraryActions.includes('item.media_ok !== false && item.src_path && !item.blob'), true, 'Missing linked sources do not expose the broken Keep a copy action')
  eq(libraryKeyboard.includes('event.target !== event.currentTarget'), true, 'Library row navigation does not hijack child controls')
  eq(libraryKeyboard.includes("['ArrowUp', 'ArrowDown', 'Home', 'End']"), true, 'Library row frames support bounded standard navigation keys')
  eq(scaleGate.includes('childControlStayedFocused'), true, 'Scale gate proves row navigation leaves child controls alone')
  eq(library.includes('favoriteCount='), false, 'Library does not label page-local collection counts as global totals')
  eq(assetsCoverage.includes('data-cut-action="open-source-monitor"'), true, 'Assets native coverage owns the Source monitor action identity')
  eq(assetsCoverage.includes("actionId: 'source-monitor-play'"), true, 'Assets native coverage proves the Source monitor transport action')
  eq(assetsCoverage.includes('data-cut-action="remove-asset"'), true, 'Assets native coverage owns the remove action identity')
  eq(assetsCoverage.includes('Source monitor dialog mounted'), true, 'Assets native coverage proves the Source monitor opened')
  eq(
    assetsCoverage.includes('Source Insert deliberately keeps the Source Monitor open')
      && assetsCoverage.includes("await sourceDialog.waitFor({ state: 'detached', timeout: 8_000 })"),
    true,
    'Assets coverage closes its proven Source Monitor before returning to modal-blocked asset actions',
  )
  eq(offlineMediaCoverage.includes('document.elementFromPoint(x, y)') && offlineMediaCoverage.includes('timeline Relink is not pointer-actionable'), true, 'Offline timeline Relink must own its live pointer point and reports exact obstruction geometry')
  eq(assetsCoverage.includes('unused asset removed='), true, 'Assets native coverage proves an unused asset left project state')
  eq(assetsPickerCoverage.includes("selectVerb = 'media.import'"), true, 'Assets native picker defaults to the real media import verb')
  eq(assetsPickerCoverage.includes("selectedResponse = await captureVerbResp(\n            page,\n            selectVerb"), true, 'Assets native coverage captures the selected picker verb response')
  eq(assetsSetupCoverage.includes('selectPath: primaryMedia'), true, 'Empty-project Import selects a real media source')
  eq(assetsCoverage.includes('selectPath: secondMedia'), true, 'Populated Assets Import selects a real media source')
  eq(assets.includes('data-cut-import-otio'), true, 'Assets exposes timeline import beside project media')
  eq(topbar.includes("document.addEventListener('cut:import-otio'"), true, 'Topbar-owned OTIO preview listens to the Assets action')
  eq(topbar.includes('data-cut-import-section'), false, 'Export menu no longer owns timeline import')
  eq(topbarModel.includes("{ id: 'deliver', label: 'Deliver' }"), true, 'Export options define a Deliver group')
  eq(topbar.includes('data-cut-export-group={group.id}'), true, 'Export menu renders grouped choices')
  eq(topbar.includes('data-cut-project-format-toggle'), true, 'Render options hide project-wide timeline format behind an explicit advanced disclosure')
  eq(topbar.includes('Advanced · timeline format'), true, 'Timeline format is named as an advanced project-wide control')
  eq(topbar.includes('Affects the whole editing canvas and frame timing.'), true, 'Timeline format explains why it differs from delivery quality')
  eq(topbar.includes('<details className="tb-render-timeline" data-cut-project-format-settings>'), true, 'Timeline format disclosure starts collapsed by default')
  eq(topbarCss.includes('width: min(360px, calc(100vw - 24px));'), true, 'Render options stay inside the viewport at compact desktop widths')
  eq(topbarCss.includes('white-space: normal;'), true, 'Render guidance wraps instead of widening the popover off-screen')
  eq(fullCoverage.includes('timeline-format-advanced-toggle'), true, 'Native full coverage expands the advanced timeline-format disclosure')
  eq(generate.includes('Slot seconds'), true, 'Generate uses seconds in its human-facing slot field')
  eq(generate.includes('value={insertDurationMs / 1000}'), true, 'Generate converts seconds back to the millisecond API boundary')
  eq(generateTemplates.includes("querySelectorAll<HTMLElement>('[data-cut-generate-param]')"), true, 'Generate validation reveals and focuses the first missing field')
  eq(inspector.indexOf('data-cut-inspector-group="project"') < inspector.indexOf('<ProjectCaptionsSection'), true, 'Project and caption controls are separate Inspector groups')
  eq(engagement.includes('title="Short-form score"'), true, 'Inspector gives engagement scoring a plain-language name')
  eq(engagement.includes('titleHint="Rates speech density'), true, 'Short-form scoring explains its contributing signals on hover')
  eq(preview.includes('`${video.clipId}:${video.src}:${video.srcInMs}:${video.srcOutMs}:${video.speed}`'), true, 'Preview transport re-arms at adjacent cuts that reuse one source URL')
  eq(fullCoverage.includes('createPreviewActionCoverage'), true, 'Native full coverage installs the Preview action module')
  for (const actionId of [
    'transport-btn',
    'snapshot-frame',
    'render-section',
    'audio-toggle',
    'quality-toggle',
    'cycle-guides',
    'fullscreen-toggle',
    'save-section',
    'exit-exact',
    'preview-install-ffmpeg',
    'preview-ffmpeg-guide',
    'preview-ffmpeg-recheck',
  ]) {
    eq(previewCoverage.includes(`actionId: '${actionId}'`), true, `Preview native coverage owns ${actionId}`)
  }
  eq(previewCoverage.includes('installMissingDoctorFixture'), true, 'Preview coverage reaches the conditional missing-FFmpeg surface deterministically')
  eq(tauri.includes('setAppWindowFullscreen'), true, 'Desktop bridge exposes narrowly-scoped native fullscreen control')
  eq(tauri.includes("plugin:window|is_fullscreen"), true, 'Desktop fullscreen state can be reconciled after an OS-level exit')
  eq(previewView.includes("fullscreenMode === 'native'"), true, 'Preview uses the native-window fullscreen path in installed WebViews')
  eq(previewView.includes("setFullscreenMode('overlay')"), true, 'Preview retains a reversible viewport fallback when browser fullscreen is rejected')
  eq(previewCss.includes(".pv-root[data-cut-fullscreen='true']"), true, 'Native and fallback fullscreen states cover the whole viewport')
  eq(desktopShell.includes('.permission("core:window:allow-set-fullscreen")'), true, 'Selected engine origin grants only the required fullscreen mutation')
  eq(desktopShell.includes('.permission("core:window:allow-is-fullscreen")'), true, 'Selected engine origin grants fullscreen read-back for honest state')
  eq(fullCoverage.includes('createMatteActionCoverage'), true, 'Native full coverage installs the Matte action module')
  for (const actionId of [
    'matte-apply',
    'matte-bg',
    'matte-close',
    'matte-install-premium',
    'matte-install-rvm',
    'matte-mode-remove',
    'matte-mode-replace',
    'matte-model-premium',
    'matte-model-rvm',
    'matte-pick',
    'matte-pick-x',
    'matte-pick-y',
    'matte-premium-recheck',
    'matte-quality-fast',
    'matte-quality-good',
    'matte-recheck',
    'matte-remove',
  ]) {
    eq(matteCoverage.includes(`actionId: '${actionId}'`), true, `Matte native coverage owns ${actionId}`)
  }
  eq(matteCoverage.includes('matte-install-premium-from-controls'), true, 'Matte coverage exercises the controls-view premium installer')
  eq(matteCoverage.includes('matte-install-premium-from-requirements'), true, 'Matte coverage exercises the requirements-view premium installer')
  eq(matteCoverage.includes('fixture.doctorCalls === doctorCalls'), true, 'Matte coverage proves failed setup does not fall through to doctor')
  eq(matteCoverage.includes('delete target.__fcvMatteOriginalFetch'), true, 'Matte fixture restores the original installed-WebView fetch')
  eq(matte.includes("setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'setup failed'}`)\n        return"), true, 'Matte setup stops immediately on a verb-level failure')
  eq(fullCoverage.includes('createRecordActionCoverage'), true, 'Native full coverage installs the Record action module')
  for (const actionId of [
    'rec-audio-toggle-input',
    'rec-autopolish-toggle-input',
    'rec-dur',
    'rec-export-fmt',
    'rec-fps',
    'rec-keys-toggle-input',
    'rec-mode',
    'rec-source',
    'rec-system-audio-toggle-input',
    'record-add-raw',
    'record-export',
    'record-output-clear',
    'record-output-default-folder',
    'record-output-pick',
    'record-start',
    'record-stop',
    'studio-background-select',
  ]) {
    eq(recordCoverage.includes(actionId), true, `Record native coverage owns ${actionId}`)
  }
  eq(recordCoverage.includes('fixture-capture-2'), true, 'Record coverage mounts and verifies the raw-capture result path')
  eq(recordCoverage.includes("command === 'plugin:dialog|save'"), true, 'Record coverage narrowly intercepts its native Save action')
  eq(recordCoverage.includes('target.__fcvRecordOriginalInternalInvoke'), true, 'Record coverage retains and restores the installed Tauri invoke')
  eq(fullCoverage.includes('createEnvironmentActionCoverage'), true, 'Native full coverage installs the Environment action module')
  for (const actionId of [
    'env-download',
    'env-service-chat',
    'env-service-connect',
    'env-service-primary',
    'env-service-rescan',
    'env-setup-matte',
    'env-setup-perception',
    'setup-manual',
  ]) {
    eq(environmentCoverage.includes(`actionId: '${actionId}'`), true, `Environment native coverage owns ${actionId}`)
  }
  eq(environmentCoverage.includes('server unreachable'), true, 'Environment coverage proves transport-failure recovery')
  eq(environmentCoverage.includes('target.__fcvEnvironmentOriginalFetch'), true, 'Environment fixture restores the installed WebView fetch')
  eq(fullCoverage.includes('createUserActionFeedbackCoverage'), true, 'Native full coverage installs the shared failure-feedback action module')
  eq(userActionFeedbackCoverage.includes('[data-cut-user-action-open-setup]'), true, 'Failure-feedback coverage owns the setup action')
  eq(userActionFeedbackCoverage.includes("category === 'video-performance'"), true, 'Failure-feedback coverage proves setup reaches the exact Settings category')
  eq(userActionFeedbackCoverage.includes('[data-cut-user-action-dismiss]'), true, 'Failure-feedback coverage owns and verifies dismissal')
  eq(mixer.includes('data-cut-mixer-close'), false, 'Mixer no longer retains an unreachable legacy modal close action')
  eq(mixer.includes('data-cut-mixer-scrim'), false, 'Mixer renders only in its canonical Audio rail surface')
  eq(mixer.includes('if (embedded)'), false, 'Mixer has no dead modal-versus-embedded render branch')
  for (const actionId of [
    'mixer-add-audio',
    'mixer-fader',
    'mixer-loud-target-select',
    'mixer-mute',
    'mixer-pan',
    'mixer-solo',
    'verify-loudness',
  ]) {
    eq(fullCoverage.includes(`actionId: '${actionId}'`), true, `Mixer native coverage owns ${actionId}`)
  }
  eq(fullCoverage.includes("r?.result?.target_lufs === -16"), true, 'Mixer coverage proves its target selector reaches verify.loudness')
  eq(fullCoverage.includes('createGradeActionCoverage'), true, 'Native full coverage installs the Grade action module')
  for (const actionId of [
    'grade-apply',
    'grade-input',
    'grade-lut',
    'grade-lut-advanced-toggle',
    'grade-lut-pick',
    'grade-reset',
    'grade-temp-on',
  ]) {
    eq(gradeCoverage.includes(`actionId: '${actionId}'`), true, `Grade native coverage owns ${actionId}`)
  }
  for (const control of ['contrast', 'brightness', 'saturation', 'gamma', 'temperature_k']) {
    eq(gradeCoverage.includes(`'${control}'`), true, `Grade native coverage drives ${control}`)
  }
  eq(gradeCoverage.includes('sameHostPath(clip.grade.lut, lutPath)'), true, 'Grade coverage proves the chosen LUT reaches project state across host path forms')
  eq(gradeCoverage.includes('return clip && clip.grade == null'), true, 'Grade coverage proves Reset clears the stored grade')
  eq(grade.includes('onClose?:'), false, 'Grade removes its obsolete modal-close prop')
  eq(grade.includes('embedded?:'), false, 'Grade removes its obsolete modal-versus-rail prop')
  eq(fullCoverage.includes('createAppChromeActionCoverage'), true, 'Native full coverage installs the app-chrome action module')
  for (const actionId of [
    'collapse-left',
    'expand-left',
    'command-search',
    'command',
    'rail-close',
    'theme-toggle',
  ]) {
    eq(appChromeCoverage.includes(`actionId: '${actionId}'`), true, `App-chrome native coverage owns ${actionId}`)
  }
  eq(appChromeCoverage.includes('[data-cut-command="mixer"]'), true, 'Command-palette coverage runs a concrete result row')
  eq(appChromeCoverage.includes("localStorage.getItem('cut.theme')"), true, 'Theme coverage proves persistence as well as appearance')
  eq(appWorkspace.includes('Show sidebar (Projects, Assets, Transcript, Generate, Find)'), true, 'Collapsed sidebar names every destination it restores')
  eq(appWorkspace.includes('app__side-expand-label">Sidebar'), true, 'Collapsed sidebar uses a stable concise label')
  eq(fullCoverage.includes('createStatusbarActionCoverage'), true, 'Native full coverage installs the Statusbar action module')
  for (const actionId of [
    'env-chip',
    'output-chip',
    'last-receipt',
    'job-cancel',
  ]) {
    eq(statusbarCoverage.includes(`actionId: '${actionId}'`), true, `Statusbar native coverage owns ${actionId}`)
  }
  eq(statusbarCoverage.includes("terminal?.error?.code === 'job_cancelled'"), true, 'Statusbar cancellation coverage proves a terminal cancelled job')
  eq(statusbarCoverage.includes('[data-cut-review-tab="receipts"][aria-selected="true"]'), true, 'Statusbar receipt coverage proves Inspect navigation')
  eq(statusbar.includes('Server unreachable. Click to retry cancellation.'), true, 'Statusbar keeps cancellation retryable after a transport failure')
  eq(statusbar.includes('data-cut-job-cancel-error'), true, 'Statusbar exposes cancellation failure state to native verification')
  eq(search.includes('data-cut-search-close'), false, 'Find moment removes its unreachable legacy drawer Close action')
  eq(search.includes('onClose?:'), false, 'Find moment has one canonical left-rail surface')
  eq(search.includes('embedded?:'), false, 'Find moment no longer carries a dead modal-versus-embedded branch')
  eq(fullCoverage.includes('createSearchActionCoverage'), true, 'Native full coverage installs the Find moment action module')
  for (const actionId of [
    'search-index',
    'search-query',
    'search-go',
    'search-jump',
    'search-source',
  ]) {
    eq(searchCoverage.includes(`actionId: '${actionId}'`), true, `Find moment native coverage owns ${actionId}`)
  }
  eq(searchCoverage.includes("response.result?.playhead_ms === 1000"), true, 'Find moment timeline coverage proves the connected UI playhead moved')
  eq(searchCoverage.includes('[data-cut-source-current]'), true, 'Find moment Source coverage proves the exact source time')
  eq(searchCoverage.includes("await verb('edit.insert'"), true, 'Find moment coverage explicitly places imported media before testing timeline routing')
  eq(searchCoverage.includes('if (!inserted?.ok)'), true, 'Find moment setup fails honestly when timeline placement cannot be created')
  eq(searchCoverage.includes('delete target.__fcvSearchOriginalFetch'), true, 'Find moment fixture restores the installed WebView fetch')
  eq(fullCoverage.includes('createClipsActionCoverage'), true, 'Native full coverage installs the Clips action module')
  for (const actionId of [
    'clips-btn',
    'clips-close',
    'clips-platform',
    'clip-make',
  ]) {
    eq(clipsCoverage.includes(`actionId: '${actionId}'`), true, `Clips native coverage owns ${actionId}`)
  }
  for (const aspect of ['9:16', '1:1', '16:9']) {
    eq(clipsCoverage.includes(`data-cut-clips-platform="${aspect}"`), true, `Clips native coverage drives ${aspect}`)
  }
  eq(clipsCoverage.includes("JSON.stringify(call?.platforms) === JSON.stringify(['9:16'])"), true, 'Clips coverage proves selected formats reach render.bundle')
  eq(clipsCoverage.includes('delete target.__fcvClipsOriginalFetch'), true, 'Clips fixture restores the installed WebView fetch')
  eq(clips.includes('aria-pressed={platforms.has(p)}'), true, 'Clips format chips expose their selected state')
  eq(clips.includes('{p} {PLATFORM_LABELS[p]}'), true, 'Clips format chips explain aspect ratios in plain language')
  eq(clips.includes('role="status" aria-live="polite"'), true, 'Clips announces asynchronous discovery and package completion')
  eq(clips.includes('Suggestions use opening strength and pacing as guides.'), true, 'Clips explains candidate ranking without API jargon')
  eq(clipsCoverage.includes("groupName: 'clips-package-ready'"), true, 'Clips coverage captures the completed package before closing it')
  eq(fullCoverage.includes('createAutopilotActionCoverage'), true, 'Native full coverage installs the conditional Autopilot action module')
  for (const actionId of [
    'autopilot-apply',
    'autopilot-restore',
    'autopilot-inspect',
  ]) {
    eq(autopilotCoverage.includes(`actionId: '${actionId}'`), true, `Autopilot native coverage owns ${actionId}`)
  }
  eq(autopilotCoverage.includes("call?.policy === 'auto_low_risk'"), true, 'Autopilot Apply coverage proves the approved policy reaches the engine')
  eq(autopilotCoverage.includes("call?.to === checkpoint"), true, 'Autopilot Restore coverage proves the report checkpoint reaches project.revert')
  eq(autopilotCoverage.includes('[data-cut-review-tab="receipts"][aria-selected="true"]'), true, 'Autopilot Inspect coverage proves receipt navigation')
  eq(autopilotCoverage.includes('closeBox.x + closeBox.width <= drawerBox.x + drawerBox.width'), true, 'Autopilot coverage proves Close remains inside the compact drawer')
  eq(autopilotCoverage.includes('delete target.__fcvAutopilotOriginalFetch'), true, 'Autopilot fixture restores the installed-WebView fetch')
  eq(autopilotPanel.includes('Preview a quality pass, then approve low-risk fixes.'), true, 'Autopilot introduction uses plain workflow language')
  eq(autopilotPanel.includes('actionLabel(p.fix_verb)'), true, 'Autopilot preview translates engine verb names for users')
  eq(autopilotPanel.includes('actionLabel(f.via)'), true, 'Autopilot applied report translates engine verb names for users')
  eq(fullCoverage.includes('createInspectorConditionalActionCoverage'), true, 'Native full coverage installs the conditional Inspector action module')
  for (const actionId of [
    'inspector-open-video-setup',
    'score-clip-again',
    'motion-edit',
    'motion-refresh',
    'motion-relink',
    'motion-tracking-analysis',
    'motion-tracking-analyze',
    'motion-tracking-apply',
    'motion-tracking-asset',
    'motion-tracking-detach',
    'motion-tracking-inspect',
    'motion-tracking-layer',
    'motion-tracking-mode',
    'motion-tracking-region-field',
    'motion-tracking-sample',
    'motion-tracking-verify',
  ]) {
    eq(inspectorConditionalCoverage.includes(`actionId: '${actionId}'`), true, `Conditional Inspector native coverage owns ${actionId}`)
  }
  eq(inspectorConditionalCoverage.includes("call?.preset === 'mp4-h264'"), true, 'Linked Motion refresh coverage proves the fixed render preset')
  eq(inspectorConditionalCoverage.includes('nativeAction: {'), true, 'Linked Motion relink coverage actuates the installed directory picker')
  eq(inspectorConditionalCoverage.includes('nativeOsActionsEnabled ? projectPath'), true, 'Linked Motion relink proves the selected native directory reaches the API')
  eq(inspectorConditionalCoverage.includes("call?.model === 'homography'"), true, 'Motion tracking coverage proves Planar selects the homography model')
  eq(inspectorConditionalCoverage.includes("region?.width === 0.4"), true, 'Motion tracking coverage proves frame-percent inputs reach normalized request geometry')
  eq(inspectorConditionalCoverage.includes('delete target.__fcvInspectorMotionOriginalFetch'), true, 'Conditional Inspector fixture restores the installed-WebView fetch')
  eq((motionTrackingSection.match(/await loadInventory\(true\)/g) ?? []).length, 3, 'Motion tracking preserves Analyze, Apply, and Detach completion feedback across inventory refresh')
  eq(fullCoverage.includes('createSequenceSwitcherActionCoverage'), true, 'Native full coverage installs the Sequence switcher action module')
  for (const actionId of [
    'sequence-create-cancel',
    'sequence-create-submit',
    'sequence-delete',
    'sequence-from',
    'sequence-name',
    'sequence-new',
    'sequence-rename',
    'sequence-rename-input',
    'sequence-rename-save',
    'sequence-switch',
    'sequence-trigger',
  ]) {
    eq(sequenceSwitcherCoverage.includes(`actionId: '${actionId}'`), true, `Sequence switcher native coverage owns ${actionId}`)
  }
  eq(sequenceSwitcherCoverage.includes("args?.from === 'active'"), true, 'Sequence Create coverage proves Duplicate reaches the engine')
  eq(sequenceSwitcherCoverage.includes("args?.rationale === 'user: rename sequence'"), true, 'Sequence Rename coverage proves the exact user action reaches the engine')
  eq(sequenceSwitcherCoverage.includes("args?.rationale === 'user: delete sequence'"), true, 'Sequence Delete coverage proves the confirmed exact action reaches the engine')
  eq(fullCoverage.includes('createTopbarActionCoverage'), true, 'Native full coverage installs the remaining topbar action module')
  for (const actionId of [
    'export-ffmpeg-guide',
    'export-ffmpeg-recheck',
    'export-install-ffmpeg',
    'gpu-toggle',
    'manual-link',
    'projects-btn',
    'render-btn',
    'render-gpu',
    'render-profile',
  ]) {
    eq(topbarCoverage.includes(`actionId: '${actionId}'`), true, `Topbar native coverage owns ${actionId}`)
  }
  eq(topbarCoverage.includes("label?.includes('Faster OFF')"), true, 'Topbar GPU coverage follows the current user-facing Faster label')
  eq(topbarCoverage.includes("call?.hardware === 'auto'"), true, 'Topbar Render coverage proves the synchronized GPU choice reaches render.final')
  eq(topbarCoverage.includes("call?.profile === 'talking_head'"), true, 'Topbar Render coverage proves the selected footage profile reaches render.final')
  eq(topbarCoverage.includes('fixture.doctorCalls.every((args) => args.refresh === true)'), true, 'Topbar Re-check coverage proves a fresh environment scan')
  eq(topbarCoverage.includes('delete target.__fcvTopbarOriginalFetch'), true, 'Topbar fixture restores the installed-WebView fetch')
  eq(fullCoverage.includes('createTopbarDialogActionCoverage'), true, 'Native full coverage installs conditional topbar dialog actions')
  for (const actionId of [
    'otio-cancel',
    'otio-close',
    'otio-confirm',
    'pregate-cancel',
    'pregate-close',
    'pregate-details-toggle',
    'pregate-guide',
  ]) {
    eq(topbarDialogCoverage.includes(`actionId: '${actionId}'`), true, `Topbar dialog native coverage owns ${actionId}`)
  }
  eq(topbarDialogCoverage.includes("expected_hash === 'sha256:fixture-otio'"), true, 'OTIO confirmation coverage proves the preview hash reaches replacement')
  eq(topbarDialogCoverage.includes('fixture.renderCalls.length !== 1'), true, 'Preflight coverage proves continuation survives cancellation')
  eq(topbarDialogCoverage.includes('target.__fcvTopbarDialogsOriginalInternalInvoke'), true, 'Topbar dialog fixture restores the native picker bridge')
  for (const actionId of [
    'qc-brand-clear',
    'qc-brand-colors',
    'qc-brand-editor-toggle',
    'qc-brand-fonts',
    'qc-brand-max-size',
    'qc-brand-min-size',
    'qc-brand-position',
    'qc-reflow',
  ]) {
    eq(fullCoverage.includes(`actionId: '${actionId}'`), true, `Review QC native coverage owns ${actionId}`)
  }
  eq(fullCoverage.includes("r?.result?.brand?.aspect === '16:9'"), true, 'Review QC coverage proves the stored brand kit is checked')
  eq(fullCoverage.includes('[data-cut-action="qc-brand"]:not([disabled])'), true, 'Review QC waits for the post-save automatic check to release the explicit Check action')
  eq(fullCoverage.includes('sameJsonValue(probe._brandSaveArgs, expected)'), true, 'Review QC compares nested request fields independent of key insertion order')
  eq(fullCoverage.includes("r?.result?.cleared === true"), true, 'Review QC coverage proves Clear reaches project.brand and returns a cleared result')
  eq(reviewQc.includes("rationale: 'save project brand kit from Review QC'"), true, 'Review QC save gives brand changes a durable rationale')
  eq(reviewQc.includes("rationale: 'clear project brand kit from Review QC'"), true, 'Review QC clear gives brand removal a durable rationale')
  eq(fullCoverage.includes('createReviewActionCoverage'), true, 'Native full coverage installs the conditional Review action module')
  for (const actionId of [
    'accept-op',
    'collapse-rail',
    'diff-op',
    'dismiss-guidance',
    'guidance-revert',
    'rebase-cancel',
    'rebase-confirm',
    'rebase-reject-op',
    'receipt-check-toggle',
    'receipt-judge-toggle',
    'reject-op',
    'seek',
  ]) {
    eq(reviewCoverage.includes(`actionId: '${actionId}'`), true, `Review native coverage owns ${actionId}`)
  }
  eq(reviewCoverage.includes("call?.args?.at_ms === 41200"), true, 'Receipt evidence seek proves its exact measured time')
  eq(reviewCoverage.includes("call?.args?.at_ms === 25300"), true, 'Judge issue seek proves its exact issue time')
  eq(reviewCoverage.includes("call?.args?.at_ms === 900"), true, 'Diff operation seek proves its exact affected timeline time')
  eq(reviewCoverage.includes("mode: 'rebase'"), true, 'Review rebase coverage proves the explicit selective-undo mode')
  eq(reviewCoverage.includes("delete target.__fcvReviewActionFixture"), false, 'Review fixture is isolated by full document navigation rather than mutating the next section')
  eq(events.includes('function judgeEnvelopeFrom'), true, 'Receipt events preserve structured judge reviews during live delivery')
  eq(events.includes("...(judge !== undefined ? { judge } : {})"), true, 'Live receipt decoding retains completed and explicit not-run judge state')
  eq(events.includes('function fixActionFrom'), true, 'Receipt events preserve validated repair actions for the status summary')
  eq(fullCoverage.includes('createRecipeActionCoverage'), true, 'Native full coverage installs every conditional Recipe action')
  for (const actionId of [
    'recipe-inspect',
    'recipe-param-input',
    'recipe-plan-technical-toggle',
    'recipe-restore',
    'recipe-run',
    'recipe-sample',
    'recipe-technical-toggle',
    'recipes-back',
  ]) {
    eq(recipeCoverage.includes(`actionId: '${actionId}'`), true, `Recipe native coverage owns ${actionId}`)
  }
  eq(recipeCoverage.includes("call?.args?.intensity === 'jumpy'"), true, 'Recipe coverage proves a changed parameter reaches the run')
  eq(recipeCoverage.includes("call?.to === 'checkpoint_fcv_recipe'"), true, 'Recipe coverage proves Restore targets the run checkpoint')
  eq(recipeCoverage.includes('await recipeReport.scrollIntoViewIfNeeded()'), true, 'Recipe coverage brings the completed Restore action on screen')
  eq(recipeCoverage.includes("starter === 'first-edit'"), true, 'Recipe coverage proves the no-project sample creates the bundled starter')
  eq(recipeCoverage.includes('create?.settings?.width === 640'), true, 'Recipe sample compares settings semantically across hosts')
  eq(fullCoverage.includes('createRenderQueueActionCoverage'), true, 'Native full coverage installs every Render Queue terminal action')
  for (const actionId of [
    'render-queue-add',
    'render-queue-done-close',
    'render-queue-error-back',
    'render-queue-error-close',
    'render-queue-remove',
  ]) {
    eq(renderQueueCoverage.includes(`actionId: '${actionId}'`), true, `Render Queue native coverage owns ${actionId}`)
  }
  eq(renderQueueCoverage.includes("{ preset: 'high', output: '/fixture/master.mp4' }"), true, 'Render Queue coverage proves edited row settings reach the batch request')
  eq(renderQueueCoverage.includes("pathname === '/api/verb/project.set_output_dir'"), true, 'Render Queue conditional coverage isolates and proves picker-folder authorization')
  eq(renderQueueCoverage.includes('fixture.queueCalls.length === 3'), true, 'Render Queue coverage proves both error recovery paths')
  eq(
    renderQueueCss.includes('overflow-wrap: anywhere;') && renderQueueCss.includes('white-space: normal;'),
    true,
    'Render Queue engine errors wrap inside the dialog',
  )
  eq(fullCoverage.includes('createScopesActionCoverage'), true, 'Native full coverage installs every Scopes option action')
  for (const actionId of ['scopes-images', 'scopes-kind']) {
    eq(scopesCoverage.includes(`actionId: '${actionId}'`), true, `Scopes native coverage owns ${actionId}`)
  }
  eq(scopesCoverage.includes("call.kinds[0] === 'waveform'") && scopesCoverage.includes("call.kinds[1] === 'histogram'"), true, 'Scopes coverage proves the changed kind selection reaches verify.scopes')
  eq(scopesCoverage.includes('call?.scope_images === true'), true, 'Scopes coverage compares request fields without object insertion-order drift')
  eq(scopesCoverage.includes("'retain-last-selected-scope', ['vectorscope']"), true, 'Scopes coverage proves the last selected kind cannot be removed')
  eq(scopesCoverage.includes("'/api/frame?at_ms=0&compose=1'"), true, 'Scopes coverage uses a stable native-WebView image route')
  eq(scopesCoverage.includes('page.route('), false, 'Scopes coverage does not require unsupported WebDriver request routing')
  eq(scopesCoverage.includes('page.unroute('), false, 'Scopes fixture cleanup does not require unsupported WebDriver request routing')
  eq(scopesCoverage.includes('node.complete && node.naturalWidth > 0'), true, 'Scopes coverage proves requested images load')
  eq(scopesCoverage.includes("'scopes-result-completed'"), true, 'Scopes coverage preserves visual proof of the completed report')
  eq(scopesCoverage.includes('scroll.scrollTop > 0') && scopesCoverage.includes('scroll.lastVisible'), true, 'Scopes coverage proves generated images are reachable through the visible scroller')
  eq(scopesPanel.includes('data-cut-scopes-bar') && scopesPanel.includes('data-cut-scopes-kinds'), true, 'Scopes configuration groups have stable verification identities')
  eq(
    /\.scopes\s*\{[^}]*flex:\s*1;[^}]*min-height:\s*0;[^}]*overflow-y:\s*auto;/s.test(reviewCss),
    true,
    'Scopes owns a bounded scrollbar instead of clipping its long report in the Review body',
  )
  eq(accessibilityCoverage.includes('Accessibility.getFullAXTree'), true, 'Accessibility coverage reads the browser accessibility tree')
  eq(accessibilityCoverage.includes("verb.name === 'ui.open'"), true, 'Accessibility coverage follows the complete shared surface registry')
  eq(
    accessibilityCoverage.includes("'unnamed-control'")
      && accessibilityCoverage.includes("'unnamed-dialog'")
      && accessibilityCoverage.includes("'enabled-control-not-focusable'"),
    true,
    'Accessibility coverage rejects unnamed and keyboard-unreachable controls plus unnamed dialogs',
  )
  eq(generateTemplatePanel.includes('aria-label={`${fieldLabel(name)} color value`}'), true, 'Generate color text values have a programmatic name')
  eq(diffView.includes('aria-label="Compare from"') && diffView.includes('aria-label="Compare to"'), true, 'Diff endpoints have distinct programmatic names')
  eq(qcPanel.includes('aria-label="Caption shift in milliseconds"'), true, 'Caption shift input names its unit and purpose')
  eq(shapePanel.includes('aria-label="Fill color"'), true, 'Shape fill value has a programmatic name')
  eq(fullCoverage.includes('createChatActionCoverage'), true, 'Native full coverage installs the conditional Agent Chat action module')
  for (const actionId of [
    'chat-accept',
    'chat-attach',
    'chat-attachment',
    'chat-attachment-remove',
    'chat-diff',
    'chat-preview',
    'chat-prompt',
    'chat-prompt-library',
    'chat-retry',
    'chat-revert',
  ]) {
    eq(chatCoverage.includes(`actionId: '${actionId}'`), true, `Agent Chat native coverage owns ${actionId}`)
  }
  eq(chatCoverage.includes("first.call?.args?.attachments) !== JSON.stringify(['a1'])"), true, 'Agent Chat coverage proves registered attachments reach the turn request')
  eq(chatCoverage.includes("localStorage.setItem('cut.chatAgent', 'claude')"), true, 'Agent Chat fixture pins the asserted provider instead of inheriting user state')
  eq(chatCoverage.includes("if_tip: 'op_000011'"), true, 'Agent Chat retry coverage proves optimistic whole-turn revert safety')
  eq(chatCoverage.includes("chat.locator('[data-cut-chat-log]')") && chatCoverage.includes('log.scrollTop = log.scrollHeight'), true, 'Agent Chat retry proof scrolls the actual chat log container')
  eq(chatCoverage.includes('reviewVisibleInLog') && chatCoverage.includes('cardRect.bottom <= logRect.bottom'), true, 'Agent Chat retry proof requires the completed review card to be inside the visible log')
  eq(
    chatCoverage.indexOf('await sleep(200)') < chatCoverage.indexOf('await revealReviewCard(chat, third.review)'),
    true,
    'Agent Chat retry proof settles response effects before its final native geometry check',
  )
  eq(
    chatCoverage.includes('log.scrollTop + cardRect.top - logRect.top - centeredTop'),
    true,
    'Agent Chat retry proof centers the completed turn with scroll-container-relative geometry before native capture',
  )
  eq(chatCoverage.includes('stored.op_000008'), true, 'Agent Chat Accept coverage proves shared Review markers')
  eq(chatAttachmentPicker.includes('event.stopPropagation()'), true, 'Attachment Escape does not collapse the parent Tools rail')
  eq(chatAttachmentPicker.includes("addEventListener('keydown', onKey, true)"), true, 'Attachment Escape is captured before the parent rail handler')
  eq(chatAttachmentPicker.includes("querySelector<HTMLButtonElement>('[data-cut-chat-attach]')?.focus()"), true, 'Attachment Escape restores focus to its trigger')
  eq(fullCoverage.includes('createDirectorActionCoverage'), true, 'Native full coverage installs the conditional Director action module')
  for (const actionId of ['pick', 'director-close', 'director-repick', 'director-done-close', 'director-error-close']) {
    eq(directorCoverage.includes(`actionId: '${actionId}'`), true, `Director native coverage owns ${actionId}`)
  }
  eq(directorCoverage.includes('{ 0: { cx: 0.35 } }'), true, 'Director coverage proves a subject choice reaches render.reframe')
  eq(directorCoverage.includes("{ 0: { mode: 'widen' } }"), true, 'Director coverage proves Widen reaches render.reframe')
  eq(directorModal.includes('aria-pressed={cur.kind'), true, 'Director choices expose their selected state')
  eq(
    directorCss.includes('overflow-wrap: anywhere;') && directorCss.includes('white-space: normal;'),
    true,
    'Director error messages wrap instead of overflowing the modal',
  )
  eq(fullCoverage.includes('createTranscriptActionCoverage'), true, 'Native full coverage installs the conditional Transcript action module')
  eq(transcriptCoverage.includes('captureVerbResp'), true, 'Transcript Restore coverage owns its exact response before asserting visible restoration')
  for (const actionId of [
    'aggressiveness',
    'add-to-reel',
    'assemble-reel',
    'cut-words',
    'filler-pass',
    'generate-captions',
    'generate-chapters',
    'ignore-words',
    'kinetic-apply',
    'kinetic-close',
    'kinetic-position',
    'kinetic-replace',
    'mute-words',
    'open-kinetic',
    'reel-clear',
    'reel-mode',
    'reel-remove',
    'restore',
    'retakes-pass',
    'setup-perception',
    'silence-pass',
    'timeline-clear-sel',
    'timeline-cut-words',
    'transcript-search',
    'unignore-words',
    'unmute-words',
    'view-clip',
    'view-program',
    'view-source',
  ]) {
    eq(transcriptCoverage.includes(`actionId: '${actionId}'`), true, `Transcript native coverage owns ${actionId}`)
  }
  eq(transcriptCoverage.includes("word_ranges: [[5, 6]]"), true, 'Transcript coverage proves ordered reel assembly arguments')
  eq(transcriptCoverage.includes("warm_model: true"), true, 'Transcript setup coverage proves the first transcription model is warmed')
  eq(transcriptCoverage.includes("clip: 'c1'"), true, 'Timeline transcript coverage proves cuts stay scoped to the selected clip')
  eq(transcriptCoverage.includes("() => restore.click()") && transcriptCoverage.includes("removed.waitFor({ state: 'detached', timeout: 12_000 })"), true, 'Transcript Restore proof uses the native pointer path and waits for visible state convergence')
  eq(transcriptCoverage.includes('[data-cut-action="view-source"].tx__viewbtn--on'), true, 'Transcript source actions wait for Source mode before selecting its words')
  eq(transcriptCoverage.includes("page.locator('[data-cut-toolbar]')"), false, 'Transcript selection never mistakes the always-visible top toolbar for its floating range toolbar')
  eq(transcriptCoverage.includes("if (!await toolbar.isVisible().catch(() => false))") && transcriptCoverage.includes("await toolbar.waitFor({ state: 'visible', timeout: 12_000 })"), true, 'Transcript source actions retry the native-safe range gesture only when the selection toolbar did not mount')
  eq(fullCoverage.includes('const selectTranscriptSourceRange = async (first, last, expectedAction) =>'), true, 'Real-STT Transcript coverage shares the native-safe range-selection path')
  eq(fullCoverage.includes("getAttribute('data-cut-transcript-view')"), true, 'Real-STT Transcript range selection does not re-click an already-active Source toggle')
  eq(fullCoverage.includes('Transcript Source view did not settle for ${expectedAction}'), true, 'Real-STT Transcript range selection waits through delayed project-refresh remounts')
  eq(fullCoverage.includes('if (!sourceReady) continue'), true, 'Real-STT Transcript source settling retries when a refresh wins the first view-toggle race')
  eq(fullCoverage.includes('await action.waitFor({ state: \'visible\', timeout: 12_000 })'), true, 'Real-STT Transcript selection requires the exact floating-toolbar action before continuing')
  eq(fullCoverage.includes("const ensureTools = async (expectedAction = '') =>"), true, 'Real-STT Transcript Tools actions reopen their remounted surface after project refresh')
  eq(fullCoverage.includes("throw new Error(`Transcript Tools did not expose ${expectedAction || 'its menu'}`)"), true, 'Real-STT Transcript Tools helper fails at the missing precondition instead of cascading absent actions')
  eq(fullCoverage.includes("first.waitFor({ state: 'attached', timeout: 5000 })"), true, 'Real-STT Transcript coverage refetches attached source words across delayed renders')
  eq(fullCoverage.includes("first.dispatchEvent('mousedown', {"), true, 'Real-STT Transcript coverage drives the product word-selection event without a native viewport race')
  eq(fullCoverage.includes("last.dispatchEvent('mousedown', {") && fullCoverage.includes('shiftKey: true'), true, 'Real-STT Transcript coverage delivers Shift on the product word-selection event')
  eq(transcriptCss.includes('.tx-restore {\n  display: inline-block;'), true, 'Transcript Restore remains keyboard-reachable without requiring pointer hover')
  eq(transcriptCss.includes('  position: static;'), true, 'Transcript Restore stays in inline flow instead of covering adjacent selectable words')
  eq(transcriptCss.includes('opacity: 1;') && transcriptCss.includes('pointer-events: auto;'), true, 'Transcript Restore stays visibly actionable without hover discovery')
  eq(drawerCss.includes('.cd-head > :first-child { min-width: 0; }'), true, 'Drawer headers let long descriptions shrink before displacing Close')
  eq(drawerCss.includes('.cd-head > .cd-btn { flex: none; }'), true, 'Drawer Close actions never shrink out of reach')
  for (const actionId of [
    'musicbed-bedgain-input',
    'musicbed-duckdb-input',
    'musicbed-beats',
  ]) {
    eq(fullCoverage.includes(`actionId: '${actionId}'`), true, `Music Bed native coverage owns ${actionId}`)
  }
  eq(fullCoverage.includes("args?.bed_gain_db === -22"), true, 'Music Bed coverage proves the chosen level reaches audio.add_music')
  eq(fullCoverage.includes("args?.duck === false"), true, 'Music Bed coverage proves the disabled duck state reaches audio.add_music')
  eq(fullCoverage.includes("args?.beat_markers === true"), true, 'Music Bed coverage proves the beat-marker choice reaches audio.add_music')
  eq(fullCoverage.includes("new URL(request.url(), APP).pathname === '/api/verb/audio.add_music'"), true, 'Music Bed coverage captures request arguments from both relative and absolute native URLs')
  eq(fullCoverage.includes('page.waitForRequest('), false, 'Music Bed coverage does not depend on a Playwright-only request waiter')
  eq(fullCoverage.includes("captureVerbResp(page, 'render.queue'"), true, 'Render Queue uses the shared response capture path that drains native bridge events')
  eq(fullCoverage.includes("[data-cut-render-queue-start]:not([disabled])"), true, 'Render Queue waits for a genuinely actionable Start control')
  eq(fullCoverage.includes("[data-cut-kinetic-apply]:not([disabled])"), true, 'Kinetic coverage waits for captions to make Apply actionable')
  eq(fullCoverage.includes("}, 240_000)"), true, 'Kinetic native coverage gives its frame-by-frame render a bounded debug-build window')
  eq(musicBed.includes('Open Assets and import an audio file'), true, 'Music Bed empty state gives a human workflow instead of an API verb')
  eq(musicBed.includes('Speech reductions and beat markers are saved with the project'), true, 'Music Bed persistence guidance uses plain language')
  eq(musicBed.includes('{c.label} ({c.id})'), false, 'Music Bed choices do not expose internal asset ids')
  eq(musicBed.includes('<label className="cd-duck-depth">'), true, 'Music Bed duck-depth range has an accessible label')
  eq(musicBed.includes('<dt>bed clip</dt>'), false, 'Music Bed result does not expose an internal clip id')
  eq(fullCoverage.includes("groupName: 'music-drawer-completed'"), true, 'Music Bed coverage captures the completed result before closing it')
  for (const actionId of [
    'comments-collapse',
    'comment-filter',
    'comment-disclosure',
    'comment-seek',
    'comment-done',
  ]) {
    eq(fullCoverage.includes(`actionId: '${actionId}'`), true, `Comments native coverage owns ${actionId}`)
  }
  eq(
    fullCoverage.includes("for (const filter of ['open', 'addressed', 'dismissed', 'all'])")
      && fullCoverage.includes('name: `comment-filter-${filter}`'),
    true,
    'Comments native coverage drives every status filter',
  )
  eq(fullCoverage.includes("response.result?.playhead_ms"), true, 'Comments seek coverage proves the connected UI playhead moved')
  eq(fullCoverage.includes("groupName: 'comments-drafted-actions'"), true, 'Comments coverage captures and exercises the proposed-edit Apply surface')
  eq(fullCoverage.includes('body.getBoundingClientRect().height > 0'), true, 'Comments disclosure waits for its action body to become visibly usable')
  eq(
    fullCoverage.includes("comment.status === 'addressed')") && fullCoverage.includes('), 30_000)'),
    true,
    'Comments Apply allows native project-state convergence without weakening the durable re-read',
  )
  eq(fullCoverage.includes("doneExpected = current === 'addressed' ? 'open' : 'addressed'"), true, 'Comments Done coverage proves both valid status directions')
  eq(
    (fullCoverage.match(/rowKind: 'support'[\s\S]{0,300}duplicate reel engine cross-check/g) || []).length,
    2,
    'Duplicate reel lowering rows cannot masquerade as unactuated UI actions',
  )
  eq(clipView.includes('displayName?: string') && clipView.includes('{label}</span>'), true, 'Timeline clips render a supplied source display name')
  eq(trackRow.includes('assetLabels.get(it.asset)'), true, 'Timeline rows resolve media ids to readable filenames')
  eq(rightRail.includes('app__side-expand--has-selection'), true, 'Collapsed Tools rail visibly marks a selected clip')
}

// --- Mask overlay ownership + empty-geometry contract ------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const maskPanel = readFileSync(resolve(root, 'ui/src/panels/Mask/index.tsx'), 'utf8')
  const maskOverlay = readFileSync(resolve(root, 'ui/src/panels/Preview/MaskOverlay.tsx'), 'utf8')

  eq(
    maskOverlay.includes('data-cut-mask-capture-shape={shape}'),
    true,
    'Mask capture overlay has a dedicated identity separate from shape option controls',
  )
  eq(
    maskOverlay.includes('data-cut-mask-shape-kind={shape}'),
    false,
    'Mask capture overlay does not collide with the shape option selector',
  )
  eq(
    maskPanel.includes('data-cut-mask-clear-shape\n            disabled={!geometry?.points.length}'),
    true,
    'Clear shape stays disabled when a remounted overlay reports empty geometry',
  )
}

// --- multi-identity runtime action recording ---------------------
{
  const events = new EventEmitter()
  const recorder = await createRuntimeActionRecorder(events, ['primary-action', 'alias-action'])
  events.emit('action', ['primary-action', 'alias-action', 'test-only-metadata'])
  eq(recorder.observed(), ['alias-action', 'primary-action'], 'Runtime recorder observes every expected identity exposed by one event')
  eq(recorder.unexpected(), [], 'Runtime recorder ignores non-contract metadata when an expected identity matched')
  events.emit('action', ['unknown-action', 'second-unknown'])
  eq(recorder.unexpected(), ['unknown-action'], 'Runtime recorder retains one diagnostic identity when no expected action matched')
}

// --- strict timeline action actuation contract -------------------
{
  const here = dirname(fileURLToPath(import.meta.url))
  const root = resolve(here, '../..')
  const timeline = readFileSync(resolve(root, 'ui/src/panels/Timeline/index.tsx'), 'utf8')
  const sweep = readFileSync(resolve(root, 'ui/public-tests/full-coverage-verify.mjs'), 'utf8')
  const dialogs = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageTimelineDialogActions.mjs'), 'utf8')
  const libraryActions = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageLibraryActions.mjs'), 'utf8')
  const settingsTasks = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageSettingsTasks.mjs'), 'utf8')
  const sequenceSwitcherActions = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageSequenceSwitcherActions.mjs'), 'utf8')
  const nativeOtioActions = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageNativeOtioActions.mjs'), 'utf8')
  const runtimeActionRecorder = readFileSync(resolve(root, 'ui/public-tests/lib/fullCoverageRuntimeActionRecorder.mjs'), 'utf8')
  const fullCoverageReceipt = readFileSync(resolve(root, 'scripts/lib/full-coverage-receipt.mjs'), 'utf8')

  eq(timeline.includes('markerGhostRef.current = nextMarkerGhost; setMarkerGhost(nextMarkerGhost)'), true, 'Marker drag commits the latest pointer proposal without a React-render race')
  eq(dialogs.includes("actionId: 'ctx-fit-to-fill'"), true, 'Fit-to-fill parent is clicked beside a real gap')
  eq(dialogs.includes('sourceDurationMs / gapDurationMs') && dialogs.includes('fitSpeed > 4'), true, 'Fit-to-fill coverage derives a supported gap from the selected real media')
  eq(dialogs.includes("Number(next.assets?.[fillAsset]?.probe?.duration_ms || 0) > 0"), true, 'Fit-to-fill coverage waits for the imported source probe before sizing its gap')
  eq(libraryActions.includes('bulkRemoveResponseTimeoutMs') && libraryActions.includes('FCV_NATIVE_ACTION_TIMEOUT_MS'), true, 'Library bulk remove keeps response capture alive through native confirmation recovery')
  eq(dialogs.includes('ctx-split-edit-${kind}'), true, 'J/L cut menu actions are both directly actuated')
  eq(dialogs.includes("captureVerbResp(page, 'edit.split_edit'"), true, 'J/L cut coverage captures the real UI response')
  eq(sweep.includes("captureVerbResp(page, 'edit.move_marker'"), true, 'Marker coverage drags the real marker control')
  eq(sweep.includes("captureVerbResp(page, 'edit.restore'"), true, 'Review Reject coverage clicks the real handler')
  eq(sweep.includes("'edit.split_edit',\n  'comment.export'"), false, 'Split edit is no longer delegated to a sibling gate')
  const delegatedVerbs = sweep.slice(
    sweep.indexOf('const DELEGATED_VERBS'),
    sweep.indexOf('const DELEGATED_NOTE'),
  )
  eq(delegatedVerbs.includes("'comment.export'"), false, 'Review handoff controls are directly actuated in the full action matrix')
  eq(sweep.includes("actionId: 'comment-export-review'"), true, 'Full action matrix clicks the review-package export control')
  eq(sweep.includes("actionId: 'comment-import-feedback'"), true, 'Full action matrix selects a feedback file through the import control')
  eq(delegatedVerbs.includes("'screen_record.start'"), false, 'Live recorder verbs are owned by the installed full action matrix')
  eq(sweep.includes("FCV_REAL_SCREEN_RECORD === '1'"), true, 'Live recording is an explicit installed-run capability')
  eq(sweep.includes("name: 'record-live-stop-autoedit-polish'"), true, 'Installed recording actuates Stop and proves autoedit plus polish')
  eq(sweep.includes("name: 'record-live-export'"), true, 'Installed recording actuates Export clip and proves bytes')
  eq(sweep.includes("!liveResponses['screen_record.stop'] || !liveResponses['screen_record.polish']") && sweep.includes('await page.flushEvents?.()'), true, 'Installed recording drains delayed native Stop and Polish responses before judging bytes')
  eq(sweep.includes("liveResponses['screen_record.export'] = await captureVerbResp"), true, 'Installed recording drains the native Export response before judging its selected file')
  eq(sweep.includes('`record-live-${seq++}.mp4`'), true, 'Installed recording selects an exact output through the host Save dialog')
  eq(sweep.includes('selectedPath=${basenameHostPath'), true, 'Installed recording proves Export used the selected host path')
  eq(delegatedVerbs.includes("'system.mcp_test'"), false, 'Settings owns the MCP self-test inside the installed final matrix')
  eq(delegatedVerbs.includes("'project.sequence_list'"), false, 'Sequence lifecycle verbs are owned by the installed final matrix')
  eq(sweep.includes("captureVerbResp(page, 'system.mcp_test'"), false, 'MCP response capture stays in the bounded Settings module')
  eq(settingsTasks.includes("'system.mcp_test'"), true, 'Settings Test MCP captures the exact system.mcp_test response')
  eq(settingsTasks.includes("result?.schema === 'shellx-cut/mcp-self-test/1'"), true, 'Settings verifies the MCP proxy result contract')
  eq(sequenceSwitcherActions.includes("'project.sequence_list'"), true, 'Sequence trigger captures the exact project.sequence_list response')
  eq(sequenceSwitcherActions.includes("!project.sequences?.some((sequence) => sequence.id === 'seq2')"), true, 'Sequence delete keeps its request capture alive through the native confirmation result')
  eq(nativeOtioActions.includes("actionId: 'import-otio'"), true, 'Installed OTIO coverage actuates the real desktop picker')
  eq(nativeOtioActions.includes("verb('export.otio'"), true, 'Installed OTIO coverage creates a real round-trip source')
  eq(nativeOtioActions.includes("sourceProbe = await verb('import.otio'"), true, 'Installed OTIO coverage proves the generated file is readable before opening the native picker')
  eq(nativeOtioActions.includes('native OTIO source preflight failed before opening the picker'), true, 'Installed OTIO coverage reports a missing generated file before Explorer can obscure the cause')
  eq(nativeOtioActions.includes("'import.otio',"), true, 'Installed OTIO coverage captures the real preview response')
  eq(nativeOtioActions.includes("request?.expected_hash === preview?.result?.source_hash"), true, 'Installed OTIO confirmation binds to the preview hash')
  eq(delegatedVerbs.includes("'import.otio'"), false, 'OTIO import is not delegated from the installed final matrix')
  const defaultExportLoop = sweep.slice(
    sweep.indexOf('for (const opt of EXPORT_OPTIONS)'),
    sweep.indexOf('if (NATIVE_OS_ACTIONS.enabled)', sweep.indexOf('for (const opt of EXPORT_OPTIONS)')),
  )
  eq(defaultExportLoop.includes('nativeAction:'), false, 'Ordinary Export actions dispatch directly without waiting for a nonexistent Save dialog')
  eq(sweep.includes('name: `export-save-as-${opt.id}`'), true, 'Installed final coverage actuates every one-off Save As control')
  eq(sweep.includes("actionId: 'export-saveas-option'"), true, 'Installed Save As rows map to the source action identity')
  eq(sweep.includes('requestExact && saveTerminal?.state === \'done\''), true, 'Async Save As exports prove exact paths and terminal jobs')
  eq(sweep.includes('basenameHostPath(output) === basenameHostPath(chosenPath)'), true, 'File Save As exports prove the selected host output')
  eq(sweep.includes("name: 'caption-import', actionId: 'caption-import'"), true, 'Caption import selects a real subtitle through the installed picker')
  eq(sweep.includes("name: 'render-queue-output-picker', actionId: 'render-queue-output-pick'"), true, 'Render Queue actuates its output picker with the stable source action identity')
  eq(sweep.includes('basenameHostPath(selected) === basenameHostPath(queueOutputPath)'), true, 'Render Queue proves the host-selected output reaches its editable row')
  eq(sweep.includes('secondReplacementEngine: joinHostPath'), true, 'Native picker fixtures provide distinct same-content paths for both Assets relink controls')
  eq(sweep.includes('replacementDriver,'), true, 'Native Assets coverage can make the first relink target offline before exercising the second control')
  eq(sweep.includes('createRuntimeActionRecorder'), true, 'Final matrix records source action identities from real DOM interaction')
  eq(sweep.includes('runtimeSourceActionManifest.matchesExpected'), true, 'Final matrix fails when a source action was not actuated at runtime')
  eq(runtimeActionRecorder.includes("document.addEventListener('click', record, true)"), true, 'Playwright runtime recorder observes real clicks')
  eq(runtimeActionRecorder.includes("document.addEventListener('input', record, true)"), true, 'Playwright runtime recorder observes real field edits')
  eq(runtimeActionRecorder.includes('for (const candidate of matched) observed.add(candidate)'), true, 'Runtime recorder retains every expected identity exposed by one actuated control')
  eq(runtimeActionRecorder.includes('if (normalized[0]) unexpected.add(normalized[0])'), true, 'Runtime recorder reports one stable diagnostic identity only when no expected identity matched')
  eq(fullCoverageReceipt.includes('expectedRuntimeSourceActionIds'), true, 'Strict receipt owns the expected runtime action manifest')
  eq(fullCoverageReceipt.includes('observed,'), true, 'Runtime receipt preserves exact observed action identities for gap diagnosis')
}

// ── ui.screenshot capture-failure diagnostics (2026-08-06 macOS hardening) ──
// html-to-image rejects with the raw resource-load EVENT; String(event) is
// "[object Event]" — the exact opaque error the macOS bug-probe hit. The
// helpers must name the stage + the failing element instead.
{
  const { CaptureError, captureFailureDetail, describeCaptureError } = await import('../src/lib/capture')

  // A DOM-less stand-in for the html-to-image rejection: a real Event whose
  // target is a stubbed <img> (node's Event allows an own-property override).
  const fakeImgError = new Event('error')
  Object.defineProperty(fakeImgError, 'target', {
    value: { tagName: 'IMG', currentSrc: 'http://127.0.0.1:6161/api/frame?at_ms=0' },
  })
  const detail = captureFailureDetail(fakeImgError)
  eq(detail.includes('[object Event]'), false, 'capture failure detail never collapses to [object Event]')
  eq(detail.includes('error event'), true, 'capture failure detail names the event type')
  eq(detail.includes('<img>'), true, 'capture failure detail names the failing element')
  eq(detail.includes('/api/frame'), true, 'capture failure detail names the failing resource URL')

  const staged = describeCaptureError(new CaptureError('dom-rasterize', detail))
  eq(staged.stage, 'dom-rasterize', 'CaptureError carries its pipeline stage')
  eq(staged.message.includes('dom-rasterize'), true, 'staged message names the stage')
  eq(describeCaptureError('plain failure').stage, 'unknown', 'non-CaptureError failures report stage unknown honestly')
  eq(describeCaptureError(new Error('boom')).message, 'boom', 'Error rejections keep their message')

  // Source contract: the WS answerer performs exactly one bounded retry and
  // sends the structured error frame the server parses (ui_system.rs).
  const srcRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../src')
  const eventsSrc = readFileSync(resolve(srcRoot, 'lib/events.ts'), 'utf8')
  eq(eventsSrc.includes('await capture.captureApp()'), true, 'screenshot answerer captures via the real module')
  eq((eventsSrc.match(/await capture\.captureApp\(\)/g) || []).length, 2, 'exactly TWO capture attempts — one bounded retry, never a loop')
  eq(eventsSrc.includes("code: 'capture_failed'"), true, 'final failure sends the structured capture_failed frame')
  eq(eventsSrc.includes('stage: described.stage'), true, 'error frame carries the failing pipeline stage')
  eq(eventsSrc.includes('attempts: 2'), true, 'error frame reports the attempt count')
  const captureSrc = readFileSync(resolve(srcRoot, 'lib/capture.ts'), 'utf8')
  eq(captureSrc.includes("throw new CaptureError('dom-rasterize'"), true, 'html-to-image failures are staged as dom-rasterize')
  eq(captureSrc.includes("throw new CaptureError('png-encode'"), true, 'toDataURL failures are staged as png-encode')
}

// --- Shell update-state model (topbar button + Settings > About) ------------
// Red-proofs the UI half of the update state machine: available → the one
// quiet button shows; none/idle/error/unsupported → hidden with an honest
// status line; malformed bridge payloads are rejected. The shell half lives in
// app/desktop/src-tauri/src/update_state.rs unit tests.
{
  const {
    describeUpdateStatus,
    formatCheckedAgo,
    releaseNotesUrl,
    shouldShowUpdateButton,
    updateButtonLabel,
    validShellUpdateState,
  } = await import('../src/lib/updateState')
  const snap = (over: Record<string, unknown> = {}) => ({
    schema: 'shellx-cut/update-state/1' as const,
    status: 'none' as const,
    version: null,
    current: '0.6.105',
    checked_at: 1_000,
    error: null,
    checking: false,
    installing: false,
    supported: true,
    ...over,
  })

  eq(validShellUpdateState(snap()), true, 'update-state: a well-formed shell snapshot validates')
  eq(validShellUpdateState(null), false, 'update-state: null payload is rejected')
  eq(validShellUpdateState({ ...snap(), schema: 'shellx-cut/update-state/2' }), false, 'update-state: a future schema is rejected, not misread')
  eq(validShellUpdateState({ ...snap(), status: 'sideways' }), false, 'update-state: an unknown status is rejected')
  eq(validShellUpdateState({ ...snap(), checking: 'yes' }), false, 'update-state: non-boolean flags are rejected')

  const available = snap({ status: 'available', version: '0.7.0' }) as never
  eq(shouldShowUpdateButton(available), true, 'update-state: available → the topbar button shows')
  eq(shouldShowUpdateButton(snap() as never), false, 'update-state: up-to-date → no topbar button')
  eq(shouldShowUpdateButton(snap({ status: 'idle' }) as never), false, 'update-state: idle → no topbar button')
  eq(shouldShowUpdateButton(snap({ status: 'error', error: 'offline' }) as never), false, 'update-state: check error alone → no topbar button')
  eq(shouldShowUpdateButton(snap({ status: 'unsupported', supported: false }) as never), false, 'update-state: Linux deb/rpm → no topbar button')
  eq(shouldShowUpdateButton(snap({ status: 'available', version: '' }) as never), false, 'update-state: available without a version string stays hidden')
  eq(shouldShowUpdateButton(null), false, 'update-state: no snapshot (browser build) → no topbar button')

  eq(updateButtonLabel(available), 'Update to v0.7.0', 'update-state: button label names the offered version')
  eq(
    updateButtonLabel(snap({ status: 'available', version: '0.7.0', installing: true }) as never),
    'Installing update…',
    'update-state: install in flight relabels the button',
  )

  eq(describeUpdateStatus(snap() as never).text, "You're on the latest version.", 'update-state: none reads as latest')
  eq(describeUpdateStatus(available).text, 'ShellX Cut 0.7.0 is available.', 'update-state: available names the version')
  eq(
    describeUpdateStatus(snap({ status: 'error', error: 'update check failed: dns' }) as never),
    { tone: 'error', text: 'Update check failed: update check failed: dns' },
    'update-state: a failed check surfaces the exact failure text',
  )
  eq(
    describeUpdateStatus(snap({ status: 'unsupported', supported: false }) as never).text,
    'Linux builds update through deb/rpm package downloads — the in-app updater is not used.',
    'update-state: Linux explains its packaging instead of a dead surface',
  )
  eq(describeUpdateStatus(snap({ checking: true }) as never).text, 'Checking for updates…', 'update-state: in-flight check reads as checking')
  eq(describeUpdateStatus(snap({ status: 'idle', checked_at: null }) as never).tone, 'muted', 'update-state: idle stays muted')

  eq(formatCheckedAgo(null, 100_000), null, 'update-state: no completed check → no fake timestamp')
  eq(formatCheckedAgo(90_000, 100_000), 'Checked just now', 'update-state: <45s reads as just now')
  eq(formatCheckedAgo(100_000, 160_000), 'Checked a minute ago', 'update-state: ~1min band')
  eq(formatCheckedAgo(0.5 * 3_600_000, 1.0 * 3_600_000 + 0), 'Checked 30 minutes ago', 'update-state: minute band is exact')
  eq(formatCheckedAgo(0, 3_600_000), null, 'update-state: epoch zero means never-checked, not 1970')
  eq(formatCheckedAgo(1_000, 90 * 60_000 + 1_000), 'Checked an hour ago', 'update-state: ~1h band')
  eq(formatCheckedAgo(1_000, 5 * 3_600_000 + 1_000), 'Checked 5 hours ago', 'update-state: hour band')
  eq(formatCheckedAgo(1_000, 26 * 3_600_000 + 1_000), 'Checked 1 day ago', 'update-state: day band singular')
  eq(formatCheckedAgo(1_000, 72 * 3_600_000 + 1_000), 'Checked 3 days ago', 'update-state: day band plural')

  eq(
    releaseNotesUrl(available),
    'https://github.com/martinsbrezauckis/shellx-cut/releases/tag/v0.7.0',
    'update-state: release notes link the exact offered release',
  )
  eq(
    releaseNotesUrl(snap() as never),
    'https://github.com/martinsbrezauckis/shellx-cut/releases/latest',
    'update-state: without an offer the notes link the latest release',
  )
  eq(
    releaseNotesUrl(null),
    'https://github.com/martinsbrezauckis/shellx-cut/releases/latest',
    'update-state: browser build links the latest release',
  )
}

if (failures) {
  // eslint-disable-next-line no-console
  console.error(`\n${failures} unit check(s) FAILED`)
  process.exit(1)
}
// eslint-disable-next-line no-console
console.log('\nall lib unit checks passed')
