// panels/Review/mock.ts — verification harness (ACTIVE ONLY on `?mock=1`).
// Role: installs a fake fetch (/api/verb/*) + fake WebSocket (/api/events) at
// module-eval time so the FULL real wiring (App resync → panels → verbs →
// op_applied events) runs against deterministic fixtures with no cutd. Used by
// the Playwright screenshot/interaction loop (UI contract: UI verification is a
// primitive) and handy for any agent eyeballing the UI offline.
// Mutating verbs APPEND a real op and emit op_applied through the fake socket —
// the confirmed-truth loop (rule 8) is exercised, not bypassed.
// Side effects: overrides window.fetch + window.WebSocket when the flag is set.
// Callers: imported for side effects by panels/Review/index.tsx (zero effect
// without the flag). Deps: lib/client types only.

import type { OpRecord, Project, RenderReceipt, Transcript } from '../../lib/client'

const MOCK_PARAMS = typeof location !== 'undefined'
  ? new URLSearchParams(location.search)
  : new URLSearchParams()
const MOCK_ON = MOCK_PARAMS.has('mock')
const MOCK_TRANSCRIPT_MISSING = MOCK_PARAMS.has('mockTranscriptMissing')
const MOCK_DIRECTOR_ERROR = MOCK_PARAMS.has('mockDirectorError')
const MOCK_ENVIRONMENT = MOCK_PARAMS.has('mockEnvironment')
const MOCK_KINETIC = MOCK_PARAMS.has('mockKinetic')
const MOCK_LIBRARY_TOTAL = Math.max(0, Number.parseInt(MOCK_PARAMS.get('mockLibraryTotal') ?? '0', 10) || 0)
// NOTE: install() is invoked at the BOTTOM of this module — class declarations
// (FakeWS) are not hoisted, so installation must follow them.
const isObject = (v: unknown): v is object => v !== null && typeof v === 'object'
const isWordRange = (v: unknown): v is [number, number] =>
  Array.isArray(v) && v.length === 2 && typeof v[0] === 'number' && typeof v[1] === 'number'
function parseArgs(body: BodyInit | null | undefined): Record<string, unknown> {
  if (!body) return {}
  try {
    const parsed = JSON.parse(String(body))
    return isObject(parsed) ? Object.fromEntries(Object.entries(parsed)) : {}
  } catch {
    return {}
  }
}

// ---------------------------------------------------------------------------
// Fixtures — a ~60s talking-head edit session mid-review.
// ---------------------------------------------------------------------------

/** Script with deliberate fillers + a long silence gap (matches test-asset shape). */
const SCRIPT =
  `okay so um today I want to show you how the agent cuts a screen demo ` +
  `without um you know touching the timeline by hand . every change goes ` +
  `through a verb and every verb leaves an operation record with a rationale ` +
  `attached . uh the silence pass just removed two and a half seconds of dead ` +
  `air right here . and like the best part is the render receipt at the end ` +
  `which measures loudness caption presence and uh whether any cut landed ` +
  `inside a word . if a check fails the agent sees the exact timecode and ` +
  `fixes it before claiming anything is done .`

function buildTranscript(): Transcript {
  const words = []
  let t = 0
  let idx = 0
  for (const w of SCRIPT.split(/\s+/)) {
    if (w === '.') {
      t += 2400 // sentence gap = the silence the pass targets
      continue
    }
    const dur = 180 + (w.length % 5) * 60 // deterministic word lengths
    words.push({ idx, word: w, start_ms: t, end_ms: t + dur, confidence: 0.97 })
    t += dur + 90
    idx++
  }
  return { asset: 'a1', model: 'whisperx-small', language: 'en', words }
}

const TRANSCRIPT = buildTranscript()
const wms = (i: number) => TRANSCRIPT.words[i]?.start_ms ?? 0
const wend = (i: number) => TRANSCRIPT.words[i]?.end_ms ?? 0

const PROJECT: Project = {
  schema: 'shellx-cut/1',
  name: 'demo-cut',
  settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48000 },
  assets: {
    a1: { path: 'testdata/talking_head.mp4', hash: 'sha256:9f3aa01c44', transcript: 'receipts/a1.words.json' },
    // still image (probe kind=image — drives the photo tint) + a music bed
    a2: { path: 'testdata/real/intro-canvas.png', hash: 'sha256:0a11c0ffee', probe: { kind: 'image', width: 1920, height: 1080 } },
    m1: { path: 'assets/music-bed.mp3', hash: 'sha256:5150bedbed', probe: { kind: 'audio', duration_ms: 60000 } },
  },
  tracks: [
    { id: 'v1', kind: 'video', clips: [{ id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 58000 }] },
    // overlay video track (v2 — composites above v1): an intro image card,
    // then a PiP'd clip carrying edit.transform geometry
    {
      id: 'v2',
      kind: 'video',
      clips: [
        { id: 'c8', asset: 'a2', src_in_ms: 0, src_out_ms: 3000 },
        { kind: 'gap', duration_ms: 5000 },
        { id: 'c9', asset: 'a1', src_in_ms: 30000, src_out_ms: 38000, transform: { x: 0.65, y: 0.05, scale: 0.3 } },
      ],
    },
    { id: 'a1t', kind: 'audio', clips: [{ id: 'c2', asset: 'a1', src_in_ms: 0, src_out_ms: 58000 }] },
    // music bed with edit.duck windows (dimmed envelope strips) and
    // edit.fade in/out ramps (corner triangles)
    {
      id: 'a2t',
      kind: 'audio',
      clips: [{ id: 'c10', asset: 'm1', src_in_ms: 0, src_out_ms: 58000, fade: { in_ms: 2000, out_ms: 3000, kind: 'audio' } }],
      gain_windows: [
        { range_ms: [3200, 11600], db: -15, attack_ms: 250 },
        { range_ms: [16400, 30800], db: -15, attack_ms: 250 },
        { range_ms: [35000, 54200], db: -15, attack_ms: 250 },
      ],
    },
  ],
  markers: [],
  caption_styles: {},
  checkpoints: [
    { id: 'cp1', name: 'imported', at_op: 'op_000001', ts: iso(-3600) },
    { id: 'cp2', name: 'before-silence-pass', at_op: 'op_000003', ts: iso(-1500) },
  ],
}

function iso(secAgo: number): string {
  return new Date(Date.now() + secAgo * 1000).toISOString()
}

let opSeq = 0
let chatTurnSeq = 0
let mockTranscriptIgnores: Array<{ asset: string; word_range: [number, number] }> = []
let mockMuteRanges: Array<[number, number]> = []
const mockJobs = new Map<string, unknown>()
const nextOpId = () => `op_${String(++opSeq).padStart(6, '0')}`

function op(partial: Omit<OpRecord, 'op_id' | 'ts' | 'status'> & { ts?: string }): OpRecord {
  return { op_id: nextOpId(), ts: partial.ts ?? iso(0), status: 'applied', ...partial }
}

const agent = { kind: 'agent' as const, name: 'claude', via: 'mcp' }
const human = { kind: 'human' as const, name: 'editor', via: 'ui' }

/** Seeded op log: import → agent cuts → human trim → a rejected cut. */
const OPS: OpRecord[] = [
  op({ ts: iso(-3620), actor: agent, verb: 'media.import', args: { path: 'testdata/talking_head.mp4' }, rationale: 'source footage', effects: [] }),
  op({
    ts: iso(-1700), actor: agent, verb: 'transcript.cut_words',
    args: { asset: 'a1', word_range: [2, 2] }, rationale: "filler 'um' (word 2)",
    effects: [{ track: 'v1', removed_ms: [wms(2), wend(2)] }, { track: 'a1t', removed_ms: [wms(2), wend(2)] }],
  }),
  op({
    ts: iso(-1640), actor: agent, verb: 'transcript.cut_words',
    args: { asset: 'a1', word_range: [17, 19] }, rationale: "filler run 'um you know' (words 17–19)",
    effects: [{ track: 'v1', removed_ms: [wms(17), wend(19)] }, { track: 'a1t', removed_ms: [wms(17), wend(19)] }],
  }),
  op({
    ts: iso(-1400), actor: agent, verb: 'transcript.remove_silences',
    args: { aggressiveness: 'natural' }, rationale: 'silence pass (natural): 2.4s sentence gap',
    effects: [{ track: 'v1', removed_ms: [wend(11) + 200, wend(11) + 2300] }],
  }),
  op({ ts: iso(-900), actor: human, verb: 'edit.trim', args: { clip: 'c1', src_out_ms: 57200 }, effects: [{ track: 'v1', at_ms: 57200 }] }),
  op({
    ts: iso(-700), actor: agent, verb: 'transcript.cut_words',
    args: { asset: 'a1', word_range: [55, 60] }, rationale: 'tangent — considers it off-topic (words 55–60)',
    effects: [{ track: 'v1', removed_ms: [wms(55), wend(60)] }],
  }),
]
// Human rejected the tangent cut → restore op references it (op row dims).
OPS.push(
  op({
    ts: iso(-600), actor: human, verb: 'edit.restore',
    args: { op_id: OPS[5].op_id }, rationale: 'tangent stays — it lands the joke',
    effects: [{ track: 'v1', range_ms: [wms(55), wend(60)] }],
  }),
)

/** Three receipts: an older 2-FAILED run, a clean 6/6 PASS (judge stubbed),
 * then a profile-waived run with a completed judge review (needs_review). */
const RECEIPT_FAIL: RenderReceipt = {
  render_id: 'r_0001', ts: iso(-1200), output_path: 'out/demo-cut.mp4', output_hash: 'sha256:b71d22e09a44',
  duration_ms: 55400, preset: 'h264-1080p', at_op: OPS[3].op_id, pass: false,
  checks: [
    { name: 'cut_on_word', pass: true, details: { boundaries_checked: 6 }, evidence: { checked: 6, worst_margin_ms: 52 } },
    { name: 'lufs', pass: false, details: { integrated_lufs: -9.2, true_peak_db: 0.4 }, evidence: { target_lufs: -14, integrated_lufs: -9.2, loudest_window_ms: 41200 } },
    { name: 'caption_presence', pass: true, details: { captions: 0 }, evidence: { caption_track: 'none requested' } },
    { name: 'black_or_frozen_frames', pass: false, details: { count: 1 }, evidence: { frozen_from_ms: 41200, frozen_to_ms: 43000 } },
    { name: 'silence_at_edges', pass: true, details: { head_ms: 120, tail_ms: 240 }, evidence: { head_ms: 120, tail_ms: 240 } },
    { name: 'duration_matches_edl', pass: true, details: { duration_ms: 55400 }, evidence: { edl_ms: 55400, rendered_ms: 55400 } },
  ],
  judge: null,
}

const RECEIPT_PASS: RenderReceipt = {
  render_id: 'r_0002', ts: iso(-300), output_path: 'out/demo-cut.mp4', output_hash: 'sha256:e4c10a9d7f21',
  duration_ms: 53100, preset: 'h264-1080p', at_op: OPS[6].op_id, pass: true,
  checks: [
    { name: 'cut_on_word', pass: true, details: { boundaries_checked: 8 }, evidence: { checked: 8, worst_margin_ms: 47 } },
    { name: 'lufs', pass: true, details: { integrated_lufs: -14.1, true_peak_db: -1.3 }, evidence: { target_lufs: -14, integrated_lufs: -14.1 } },
    { name: 'caption_presence', pass: true, details: { captions: 14 }, evidence: { covered_pct: 96.2 } },
    { name: 'black_or_frozen_frames', pass: true, details: { count: 0 }, evidence: { scanned_frames: 1593 } },
    { name: 'silence_at_edges', pass: true, details: { head_ms: 110, tail_ms: 180 }, evidence: { head_ms: 110, tail_ms: 180 } },
    { name: 'duration_matches_edl', pass: true, details: { duration_ms: 53100 }, evidence: { edl_ms: 53100, rendered_ms: 53100 } },
  ],
  judge: { status: 'not_run' },
}

/** Profile-waived + judged receipt — fixture mirrors the real profile-waived receipt shape:
 * silent_screen_demo waivers (measured outcome preserved), footage_profile
 * metadata entry, and a completed judge envelope with one seekable issue. */
const RECEIPT_JUDGED: RenderReceipt = {
  render_id: 'r_0003', ts: iso(-60), output_path: 'out/demo-cut-v2.mp4', output_hash: 'sha256:c3c99fc41bd6',
  duration_ms: 45000, preset: 'high', at_op: OPS[6].op_id, pass: true,
  checks: [
    { name: 'cut_on_word', pass: true, details: { boundaries_checked: 9 }, evidence: { checked: 9, worst_margin_ms: 61 } },
    {
      name: 'lufs', pass: true,
      details: { integrated_lufs: -70.0, waived_by_profile: 'silent_screen_demo', waiver_reason: 'loudness target does not apply to silent footage', measured_pass: false },
      evidence: { target_lufs: -14, integrated_lufs: -70.0 },
    },
    {
      name: 'caption_presence', pass: true,
      details: { captions: 1, waived_by_profile: 'silent_screen_demo', waiver_reason: 'no speech to caption', measured_pass: true },
      evidence: { caption_track: 'txt1' },
    },
    { name: 'black_or_frozen_frames', pass: true, details: { variant: 'silent_screen_demo (UI-tuned)', stuck_span_count: 0, waived_frozen_span_count: 7 }, evidence: { waived_frozen_spans_ms: [[4100, 9800]] } },
    {
      name: 'silence_at_edges', pass: true,
      details: { head_ms: 45000, waived_by_profile: 'silent_screen_demo', waiver_reason: 'whole render is silent by design', measured_pass: false },
      evidence: { head_ms: 45000, tail_ms: 45000 },
    },
    { name: 'duration_matches_edl', pass: true, details: { duration_ms: 45000 }, evidence: { edl_ms: 45000, rendered_ms: 45000 } },
    {
      name: 'footage_profile', pass: true,
      details: { active_profile: 'silent_screen_demo', selection: 'explicit', proposed_profile: 'silent_screen_demo', note: 'auto-detect only PROPOSES a profile' },
      evidence: { proposal_reasons: ['no speech: 0 transcript words in output and sources', 'silent: silence covers 88% of 45000 ms, integrated -70 LUFS', 'low motion: frozen spans cover 67% of the duration'] },
    },
  ],
  judge: {
    schema: 'shellx-cut/judge-review/1',
    status: 'completed',
    backend: { name: 'cli', provider: 'claude', model: 'claude/sonnet', frames_sent: 20, watched: true, listened: false },
    cli: { model: 'sonnet', accounting_cost_usd: 0.3279, duration_ms: 243163 },
    review: {
      verdict: 'needs_review',
      confidence: 0.65,
      summary:
        'The 45 s render delivers the intended narrative arc — landing page → zoom/select beat → headline rewrite → agent-edit finale. One visual concern at the 25 s scene cut.',
      issues: [
        {
          at_ms: 25300, end_ms: 26100, kind: 'visual_artifact', severity: 'major',
          evidence: 'Frame at 25300 ms darkens noticeably ~800 ms before the 26.1 s scene cut — reads as a flash/dip rather than an intentional transition.',
          suggested_fix: 'Trim the dark tail or move the cut to 25.3 s so the dip never renders.',
        },
      ],
      cannot_assess: ['audio quality — no audio stream received (vision-only judge)'],
    },
  },
}

// ---------------------------------------------------------------------------
// Fake WebSocket — minimal surface events.ts touches (onopen/onmessage/
// onclose/send/close/readyState + static OPEN). Emits the two receipts after
// connect; mutating verbs push op_applied frames through the live instance.
// ---------------------------------------------------------------------------

let liveSocket: FakeWS | null = null
const verbCalls: Array<{ name: string; args: Record<string, unknown> }> = []

class FakeWS {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readyState = FakeWS.CONNECTING
  onopen: (() => void) | null = null
  onmessage: ((m: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null

  constructor(public url: string) {
    liveSocket = this
    setTimeout(() => {
      this.readyState = FakeWS.OPEN
      this.onopen?.()
      // receipts arrive like live receipt_ready events (oldest first)
      this.push({ type: 'receipt_ready', receipt: RECEIPT_FAIL })
      this.push({ type: 'receipt_ready', receipt: RECEIPT_PASS })
      this.push({ type: 'receipt_ready', receipt: RECEIPT_JUDGED })
    }, 30)
  }

  push(ev: unknown): void {
    setTimeout(() => this.onmessage?.({ data: JSON.stringify(ev) }), 10)
  }

  /** Outgoing frames (ui_state pushes, ui_command_acks…) — recorded for the
   * verification harness; cutd would just consume them. */
  static sent: string[] = []

  send(data: string): void {
    FakeWS.sent.push(data)
  }

  close(): void {
    this.readyState = FakeWS.CLOSED
    this.onclose?.()
  }
}

// ---------------------------------------------------------------------------
// Fake verb endpoint — mutating verbs append a REAL op + emit op_applied so
// the panels' confirmed-truth rendering is what gets verified.
// ---------------------------------------------------------------------------

function applyOp(newOp: OpRecord): void {
  OPS.push(newOp)
  liveSocket?.push({ type: 'op_applied', op: newOp })
}

function handleVerb(name: string, args: Record<string, unknown>): unknown {
  switch (name) {
    case 'project.state':
      {
        const tracks = PROJECT.tracks.map((track) => track.id === 'a1t'
          ? {
              ...track,
              clips: (track.clips || []).map((clip) => (
                'id' in clip && clip.id === 'c2'
                  ? { ...clip, mute_ranges: [...mockMuteRanges] }
                  : clip
              )),
            }
          : track)
        if (MOCK_KINETIC) {
          tracks.push({
            id: 'cap1',
            kind: 'caption',
            clips: [
              { id: 'cap_mock_1', text: 'Mock kinetic caption one', range_ms: [0, 1200] },
              { id: 'cap_mock_2', text: 'Mock kinetic caption two', range_ms: [1300, 2500] },
            ],
          })
        }
      return {
        ok: true,
        result: MOCK_TRANSCRIPT_MISSING
          ? {
              ...PROJECT,
              tracks,
              transcript_ignores: [...mockTranscriptIgnores],
              assets: {
                ...PROJECT.assets,
                a1: { ...PROJECT.assets.a1, transcript: undefined },
              },
            }
          : {
              ...PROJECT,
              tracks,
              transcript_ignores: [...mockTranscriptIgnores],
            },
      }
      }
    case 'project.ops':
      return { ok: true, result: { ops: OPS } }
    // Current App bootstrap reads these surfaces before Review mounts. Keep the
    // offline harness structurally complete so UI verification cannot crash
    // merely because a newer shell consumer was added after the original mock.
    case 'project.list':
      return { ok: true, result: { projects: [] } }
    case 'library.list': {
      if (MOCK_LIBRARY_TOTAL <= 0) {
        return {
          ok: true,
          result: { items: [], folders: [], tags: [], total: 0, offset: 0, limit: 100, next_offset: null },
        }
      }
      const offset = typeof args.offset === 'number' ? Math.max(0, Math.floor(args.offset)) : 0
      const limit = typeof args.limit === 'number' ? Math.max(1, Math.floor(args.limit)) : 100
      const all = Array.from({ length: MOCK_LIBRARY_TOTAL }, (_, index) => ({
        id: `mock-library-${String(index).padStart(4, '0')}`,
        type: index % 3 === 0 ? 'video' : index % 3 === 1 ? 'audio' : 'image',
        name: `Mock library item ${String(index + 1).padStart(3, '0')}`,
        src_path: `/mock/library/item-${index}.mp4`,
        tags: [],
        favorite: false,
        added_ms: MOCK_LIBRARY_TOTAL - index,
        source: 'user',
        // Pagination fixtures have no backing files. Render the normal
        // type-glyph fallback instead of issuing 101 knowingly invalid poster
        // requests against the real cutd origin.
        media_ok: false,
      }))
      const items = all.slice(offset, offset + limit)
      return {
        ok: true,
        result: {
          items,
          folders: [],
          tags: [],
          total: all.length,
          offset,
          limit,
          next_offset: offset + items.length < all.length ? offset + items.length : null,
        },
      }
    }
    case 'jobs.list':
      return { ok: true, result: { jobs: [] } }
    case 'jobs.status':
      return {
        ok: true,
        result: {
          job_id: String(args.job_id ?? ''),
          state: 'done',
          progress: 1,
          result: mockJobs.get(String(args.job_id ?? '')) ?? {},
        },
      }
    case 'system.doctor':
      return {
        ok: true,
        result: {
          schema: 'shellx-cut/doctor/1',
          scanned_at: iso(0),
          os: 'mock',
          arch: 'mock',
          app_version: 'mock',
          cards: MOCK_ENVIRONMENT
            ? [{
                id: 'gpu-encode',
                kind: 'tool',
                title: 'Hardware encoding',
                status: 'unknown',
                hint: 'The first probe timed out; re-scan to verify this machine.',
                details: {
                  hardware_available: false,
                  resolved: '/mock/bin/ffmpeg',
                  enable_help: 'Install a supported GPU driver, then re-scan hardware encoding.',
                },
              }]
            : MOCK_TRANSCRIPT_MISSING
              ? [{
                id: 'perception',
                title: 'Captions and transcription',
                status: 'missing',
                hint: 'Install captions to create searchable, word-level transcripts.',
                details: { stt_ready: false },
              }]
              : [],
          essential_ok: true,
        },
      }
    case 'media.check':
      return {
        ok: true,
        result: {
          assets: Object.keys(PROJECT.assets).map((asset) => ({
            asset,
            exists: true,
            modified_ms: Date.now(),
          })),
        },
      }
    case 'media.bin_list':
      return { ok: true, result: { bins: [] } }
    case 'media.waveform':
      return {
        ok: true,
        result: {
          asset: String(args.asset ?? ''),
          bucket_count: 0,
          peaks: [],
          source_ms: 58_000,
          sample_rate: 48_000,
        },
      }
    case 'verify.checks':
      return {
        ok: false,
        error: {
          code: 'not_found',
          message: 'no persisted receipt in the offline harness',
          cause: 'receipt fixtures arrive through receipt_ready events',
        },
      }
    case 'transcript.get':
      return { ok: true, result: TRANSCRIPT }
    case 'transcript.search': {
      const query = String(args.query ?? '').trim().toLocaleLowerCase()
      const matches = TRANSCRIPT.words.filter((word) => word.word.toLocaleLowerCase().includes(query))
      return {
        ok: true,
        result: {
          query,
          match_count: matches.length,
          matches: matches.map((word) => ({ asset: 'a1', word_index: word.idx, at_ms: word.start_ms })),
        },
      }
    }
    case 'transcript.timeline': {
      const clip = typeof args.clip === 'string' ? args.clip : 'c1'
      const entries = TRANSCRIPT.words.slice(0, 12).map((word) => ({
        clip_id: clip,
        track: 'v1',
        track_kind: 'video',
        asset: 'a1',
        word_index: word.idx,
        word: word.word,
        src_start_ms: word.start_ms,
        src_end_ms: word.end_ms,
        timeline_start_ms: word.start_ms,
        timeline_end_ms: word.end_ms,
      }))
      return {
        ok: true,
        result: {
          clip: typeof args.clip === 'string' ? args.clip : null,
          track: typeof args.track === 'string' ? args.track : null,
          word_count: entries.length,
          entries,
        },
      }
    }
    case 'ui.playhead':
      return { ok: true, result: { playhead_ms: args.at_ms } }
    case 'ui.select':
      return { ok: true, result: {} }
    case 'transcript.cut_words': {
      const range = isWordRange(args.word_range) ? args.word_range : [0, 0]
      const o = op({
        actor: human, verb: name, args, rationale: typeof args.rationale === 'string' ? args.rationale : undefined,
        effects: [{ track: 'v1', removed_ms: [wms(range[0]), wend(range[1])] }],
      })
      applyOp(o)
      return { ok: true, op_ids: [o.op_id], result: { word_range: range } }
    }
    case 'transcript.ignore_words': {
      const range: [number, number] = isWordRange(args.word_range) ? args.word_range : [0, 0]
      if (args.remove === true) {
        mockTranscriptIgnores = mockTranscriptIgnores.filter((entry) => (
          entry.asset !== String(args.asset ?? 'a1')
          || entry.word_range[0] !== range[0]
          || entry.word_range[1] !== range[1]
        ))
      } else {
        mockTranscriptIgnores = [{
          asset: String(args.asset ?? 'a1'),
          word_range: range,
        }]
      }
      const o = op({ actor: human, verb: name, args, effects: [] })
      applyOp(o)
      return {
        ok: true,
        op_ids: [o.op_id],
        result: { word_range: range, transcript_ignores: [...mockTranscriptIgnores] },
      }
    }
    case 'transcript.mute_words': {
      const range: [number, number] = isWordRange(args.word_range) ? args.word_range : [0, 0]
      const span: [number, number] = [Math.max(0, wms(range[0]) - 40), wend(range[1]) + 40]
      mockMuteRanges = [span]
      const o = op({ actor: human, verb: name, args, effects: [{ track: 'a1t', clip: 'c2', mute_ms: span }] })
      applyOp(o)
      return { ok: true, op_ids: [o.op_id], result: { muted: ['c2'], range_ms: span } }
    }
    case 'edit.mute_range': {
      const remove = isWordRange(args.remove_ms) ? args.remove_ms : null
      if (remove) mockMuteRanges = []
      const o = op({ actor: human, verb: name, args, effects: [{ track: 'a1t', clip: 'c2', remove_ms: remove }] })
      applyOp(o)
      return { ok: true, op_ids: [o.op_id], result: { clip: 'c2', mute_ranges: [...mockMuteRanges] } }
    }
    case 'transcript.remove_silences': {
      if (!args.aggressiveness) {
        return { ok: false, error: { code: 'missing_arg', message: 'aggressiveness is required (the required-argument contract)', cause: 'no default exists by design' } }
      }
      const o = op({
        actor: human, verb: name, args, rationale: `silence pass (${String(args.aggressiveness)})`,
        effects: [{ track: 'v1', removed_ms: [wend(30) + 200, wend(30) + 1900] }],
      })
      applyOp(o)
      return { ok: true, op_ids: [o.op_id] }
    }
    case 'transcript.remove_fillers': {
      // one op per filler run — emit two, like the real verb (skim-reviewable)
      const runs: Array<[number, number]> = [[9, 9], [47, 47]]
      const ids: string[] = []
      for (const r of runs) {
        const o = op({
          actor: human, verb: name, args: { ...args, word_range: r }, rationale: `filler '${TRANSCRIPT.words[r[0]]?.word}'`,
          effects: [{ track: 'v1', removed_ms: [wms(r[0]), wend(r[1])], asset: 'a1', word_range: r }],
        })
        applyOp(o)
        ids.push(o.op_id)
      }
      return { ok: true, op_ids: ids, result: { fillers_removed: runs.length } }
    }
    case 'transcript.remove_retakes': {
      const range: [number, number] = [30, 31]
      const o = op({
        actor: human,
        verb: name,
        args: { ...args, asset: 'a1', word_range: range },
        rationale: 'retake removed; latest complete take kept',
        effects: [{
          track: 'v1',
          removed_ms: [wms(range[0]), wend(range[1])],
          asset: 'a1',
          word_range: range,
        }],
      })
      applyOp(o)
      return {
        ok: true,
        op_ids: [o.op_id],
        result: {
          removed_takes: 1,
          kept: 1,
          keep_policy: 'last',
          words_removed: 2,
          ms_removed: wend(range[1]) - wms(range[0]),
          clusters: [],
        },
      }
    }
    case 'transcript.chapters':
      return {
        ok: true,
        result: {
          chapters: [
            { title: 'Opening', start_ms: 0 },
            { title: 'Review loop', start_ms: 12_000 },
          ],
        },
      }
    case 'captions.generate': {
      const o = op({
        actor: human,
        verb: name,
        args,
        rationale: 'generate captions from transcript',
        effects: [{ track: 'txt1', cue_count: 12 }],
      })
      applyOp(o)
      return {
        ok: true,
        op_ids: [o.op_id],
        result: { track_id: 'txt1', cue_count: 12 },
      }
    }
    case 'captions.kinetic': {
      const o = op({
        actor: human,
        verb: name,
        args,
        rationale: typeof args.rationale === 'string' ? args.rationale : undefined,
        effects: [{ track: 'kinetic_mock', cue_count: 2 }],
      })
      applyOp(o)
      return {
        ok: true,
        op_ids: [o.op_id],
        result: {
          title_track: 'kinetic_mock',
          asset_id: 'kinetic_asset_mock',
          clip_id: 'kinetic_clip_mock',
          cue_count: 2,
          cleared_static: args.replace_static === true ? 2 : 0,
          range_ms: [0, 2500],
        },
      }
    }
    case 'transcript.assemble': {
      const ranges = Array.isArray(args.word_ranges)
        ? args.word_ranges.filter(isWordRange)
        : []
      const o = op({
        actor: human,
        verb: name,
        args,
        rationale: typeof args.rationale === 'string' ? args.rationale : undefined,
        effects: ranges.map((wordRange, index) => ({
          track: 'v1',
          clip: `reel_${index + 1}`,
          asset: typeof args.asset === 'string' ? args.asset : 'a1',
          word_range: wordRange,
        })),
      })
      applyOp(o)
      return {
        ok: true,
        op_ids: [o.op_id],
        result: {
          spans_placed: ranges.length,
          total_ms: ranges.reduce((total, range) => total + Math.max(0, wend(range[1]) - wms(range[0])), 0),
        },
      }
    }
    case 'system.setup_perception':
      return { ok: true, result: { job_id: 'job_mock_perception' } }
    case 'render.direct': {
      if (MOCK_DIRECTOR_ERROR) {
        return {
          ok: false,
          error: {
            code: 'mock_director_unavailable',
            message: 'The scene-analysis runtime is unavailable in this deterministic failure fixture.',
          },
        }
      }
      const jobId = 'job_mock_direct'
      mockJobs.set(jobId, {
        direct_id: 'direct_mock_1',
        contact_sheet: 'mock-director-contact-sheet.jpg',
        scene_count: 1,
        preset: typeof args.preset === 'string' ? args.preset : 'talking_head',
        scenes: [{
          scene: 0,
          t_ms: 1_000,
          keyframe_frame: 30,
          candidates: [
            {
              label: 'A',
              cls: 'person',
              conf: 0.98,
              cx: 0.35,
              cy: 0.5,
              box: [0.2, 0.1, 0.5, 0.9],
              has_face: true,
            },
          ],
        }],
      })
      return { ok: true, result: { job_id: jobId } }
    }
    case 'render.reframe': {
      const jobId = `job_mock_reframe_${verbCalls.length}`
      const reframeId = `reframe_mock_${verbCalls.length}`
      mockJobs.set(jobId, { reframe_id: reframeId })
      return { ok: true, result: { job_id: jobId, reframe_id: reframeId } }
    }
    case 'render.qc': {
      const jobId = `job_mock_qc_${verbCalls.length}`
      mockJobs.set(jobId, {
        reframe_id: typeof args.reframe_id === 'string' ? args.reframe_id : 'reframe_mock',
        qc_sheet: 'mock-director-qc-sheet.jpg',
        scene_count: 1,
        review_count: 1,
        scenes: [{
          scene: 0,
          t_ms: 1_000,
          subject_present: true,
          face_present: true,
          face_cx: 0.76,
          off_center: 0.26,
          headroom: 0.1,
          needs_review: true,
          issues: ['subject is too close to the crop edge'],
        }],
      })
      return { ok: true, result: { job_id: jobId } }
    }
    case 'agent.chat': {
      const baseline = OPS[OPS.length - 1]?.op_id ?? 'op_000001'
      const turnId = `turn_mock_${++chatTurnSeq}`
      const attachments = Array.isArray(args.attachments)
        ? args.attachments.filter((item): item is string => typeof item === 'string')
        : []
      const message = typeof args.message === 'string' ? args.message : ''
      const o = op({
        actor: agent,
        verb: 'edit.add_marker',
        args: { at_ms: 3000, label: `Chat ${chatTurnSeq}` },
        rationale: message,
        effects: [{ track: 'v1', at_ms: 3000 }],
      })
      applyOp(o)
      return {
        ok: true,
        result: {
          ok: true,
          agent: typeof args.agent === 'string' ? args.agent : 'claude',
          reply: `Applied one reviewable edit for: ${message}`,
          actions: [{ op_id: o.op_id, verb: o.verb }],
          attachments,
          plan: {
            request: message,
            reference_ids: attachments,
            policy: ['show the exact turn result for review'],
          },
          review: {
            turn_id: turnId,
            baseline,
            checkpoint: null,
            tip: o.op_id,
            diff: {
              clips_added: 0,
              clips_removed: 0,
              clips_moved: 0,
              duration_delta_ms: 0,
              tracks_touched: ['v1'],
              ops: [o],
            },
            diff_error: null,
            revert_safe: true,
            concurrent_actions: [],
          },
          cost_usd: 0.01,
        },
      }
    }
    case 'project.revert': {
      const o = op({
        actor: human,
        verb: name,
        args,
        rationale: typeof args.rationale === 'string' ? args.rationale : undefined,
        effects: [{ track: 'v1', reverted_to: args.to }],
      })
      applyOp(o)
      return {
        ok: true,
        op_ids: [o.op_id],
        result: { reverted_to: args.to, op_ids: [o.op_id] },
      }
    }
    case 'edit.restore': {
      const o = op({ actor: human, verb: name, args, effects: [] })
      applyOp(o)
      return { ok: true, op_ids: [o.op_id], result: { restored_op_id: args.op_id } }
    }
    case 'project.diff':
      return {
        ok: true,
        result: {
          from_op: 'op_000001', to_op: OPS[OPS.length - 1].op_id,
          ops: OPS.slice(1), clips_added: 0, clips_removed: 3, clips_moved: 0,
          duration_delta_ms: -4900, tracks_touched: ['v1', 'a1t'],
        },
      }
    default:
      return { ok: true, result: {} }
  }
}

function install(): void {
  const realFetch = window.fetch.bind(window)
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url
    const m = /\/api\/verb\/([a-z_.]+)/.exec(url)
    if (!m) return realFetch(input, init)
    const args = parseArgs(init?.body)
    verbCalls.push({ name: m[1], args })
    const body = handleVerb(m[1], args)
    return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
  }
  Object.defineProperty(window, 'WebSocket', { configurable: true, value: FakeWS })
  // Test hook for the Playwright loop: emulate a server push (e.g. a
  // ui_command relay) and inspect what the client sent back (acks).
  Object.defineProperty(window, '__cutMock', { configurable: true, value: {
    push: (ev: unknown) => liveSocket?.push(ev),
    sent: () => FakeWS.sent,
    calls: () => verbCalls,
  } })
  console.info('[shellx-cut] review mock harness active (?mock=1) — no cutd needed')
}

if (MOCK_ON) install()

export {}
