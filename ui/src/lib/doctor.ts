// doctor.ts — typed client mirror of the environment doctor (system.*).
// Role: the UI's view of `system.doctor` (capability cards) + `system.fetch_tool`
// (consented download job). 1:1 with app/server/src/doctor.rs DoctorReport /
// Card and schema/verbs.json — if those change, change this in the same commit.
// The wizard, the Settings>Environment panel, and the status-bar chip ALL read
// this ONE shape (the shellX provider-model-card pattern). Callers: panels/
// Environment/*, statusbar/, App.tsx. Deps: lib/client (callVerb).

import { callVerb } from './client'

/** Card status (mirror doctor.rs CardStatus, lowercase serde). 'unknown' is the
 *  HONEST middle state a timed-out/errored probe degrades to (readiness tri-state) — neither
 *  presence nor absence confirmed; rendered neutrally as "Couldn't verify —
 *  Re-scan", never as a confident OK/MISSING/Ready badge. */
export type CardStatus = 'ok' | 'missing' | 'degraded' | 'unknown'

/** Resolution rung for tool cards (mirror doctor.rs CardSource, kebab serde). */
export type CardSource = 'env' | 'bundled-or-appdata' | 'path' | 'missing'

/** One capability card. `details` is free-form per kind (the UI renders the
 *  fields it knows; agents read whatever they need). */
export interface DoctorCard {
  id: string
  kind: 'tool' | 'perception' | 'judge' | 'disk' | 'matte' | 'service' | string
  status: CardStatus
  source?: CardSource
  version?: string
  hint?: string
  details: Record<string, unknown>
}

/** The full env report — cached server-side, refreshable, WS-pushed on change. */
export interface DoctorReport {
  schema: string
  scanned_at: string
  os: string
  arch: string
  app_version: string
  addr?: string
  cards: DoctorCard[]
  /** true when the ESSENTIAL deps (ffmpeg) are ok — false surfaces the wizard. */
  essential_ok: boolean
}

/** The job result shape from a completed system.fetch_tool. */
export interface FetchOutcome {
  tool: string
  installed_dir: string
  version?: string | null
  sha256: string
  source_url: string
  bytes: number
  ffmpeg_ok_after: boolean
}

/** Fetch the doctor report. `refresh` forces a re-scan (else cached read). */
export async function fetchDoctor(refresh = false): Promise<DoctorReport | null> {
  const r = await callVerb('system.doctor', refresh ? { refresh: true } : {})
  return r.ok ? (r.result as DoctorReport) : null
}

/** Kick the consented tool download. Returns the job id to poll, or null on a
 *  verb-level error (e.g. unknown tool — never happens from the wizard UI). */
export async function fetchTool(tool: 'ffmpeg'): Promise<{ job_id: string } | null> {
  const r = await callVerb('system.fetch_tool', { tool })
  return r.ok ? (r.result as { job_id: string }) : null
}

/** True only when the doctor has CONFIRMED ffmpeg is absent. Unknown means the
 *  probe timed out or could not verify; do not present that as missing. */
export function isFfmpegMissing(report: DoctorReport | null | undefined): boolean {
  return report?.cards.find((card) => card.id === 'ffmpeg')?.status === 'missing'
}

/** Provision the Python perception sidecar (system.setup_perception). Mirrors
 *  fetchTool's start-and-return-job-id shape so EnvCards polls it identically.
 *  warm_model:true pulls the model in the SAME job (first transcription warm).
 *  The job is LONG (downloads uv, a Python runtime, torch, the model — several
 *  minutes); the caller must poll without a timeout assumption. Returns the job
 *  id to poll, or null on a verb-level error. */
export async function setupPerception(): Promise<{ job_id: string } | null> {
  const r = await callVerb('system.setup_perception', { warm_model: true })
  return r.ok ? (r.result as { job_id: string }) : null
}

/** Provision an AI background-removal (matte) runtime (system.setup_matte).
 *  Two tiers, lifted 1:1 from panels/Matte (the clip-contextual drawer):
 *    • 'rvm'       — the default ~14 MB RVM model (free, on-device).
 *    • 'matanyone' — the MatAnyone2 PREMIUM tier (cleaner edges + pick the
 *      subject). It is NON-COMMERCIAL (NTU S-Lab License 1.0), so the verb
 *      REQUIRES `accept_noncommercial:true` — passing it here IS the consent,
 *      mirroring the drawer's "installing accepts it" affordance.
 *  Both return a {job_id} (no path) per schema, so EnvCards polls them with the
 *  same runJob helper as ffmpeg/perception. Returns the job id, or null on a
 *  verb-level error. */
export async function setupMatte(tier: 'rvm' | 'matanyone'): Promise<{ job_id: string } | null> {
  const args =
    tier === 'matanyone'
      ? { model: 'matanyone' as const, accept_noncommercial: true }
      : { model: 'rvm' as const }
  const r = await callVerb('system.setup_matte', args)
  return r.ok ? (r.result as { job_id: string }) : null
}

/** The recommended STT (transcription) models the engine ships first-class
 *  (system.set_stt_model). The stack is intentionally THREE-MODEL: (1) word-level
 *  transcript editing needs tight timings, so Parakeet v3 remains the fast default;
 *  (2) smaller/weak European languages use Canary-1B-v2 plus MMS_FA forced alignment
 *  for word timestamps; (3) Whisper large-v3 remains the compatibility fallback.
 *  Any other onnx-asr/whisperx id is accepted (advanced) and downloads on first use. */
export const STT_MODELS: { id: string; label: string }[] = [
  { id: 'nemo-parakeet-tdt-0.6b-v3', label: 'Parakeet v3' },
  { id: 'nemo-canary-1b-v2', label: 'Canary-1B-v2 + MMS_FA' },
  { id: 'whisperx-large-v3', label: 'Whisper large-v3' },
]

/** The default STT model id the engine resets to (clear:true / no override). */
export const STT_DEFAULT_MODEL = 'nemo-parakeet-tdt-0.6b-v3'

/** Languages Parakeet v3 covers (NVIDIA model card — 25 European). Shown in the
 *  picker so a user knows when their language needs the Whisper tier instead. */
export const STT_V3_LANGUAGES =
  'Bulgarian, Croatian, Czech, Danish, Dutch, English, Estonian, Finnish, French, ' +
  'German, Greek, Hungarian, Italian, Latvian, Lithuanian, Maltese, Polish, ' +
  'Portuguese, Romanian, Russian, Slovak, Slovenian, Spanish, Swedish, Ukrainian'

/** Persist the chosen transcription model (system.set_stt_model). Pass null/''
 *  to RESET to the built-in default (clear:true). The choice applies to the NEXT
 *  uncached perception run. Returns ok. */
export async function setSttModel(model: string | null): Promise<boolean> {
  const r = await callVerb('system.set_stt_model', model ? { model } : { clear: true })
  return r.ok
}

// ---------------------------------------------------------------------------
// Card grouping + presentation helpers (shared by wizard + settings)
// ---------------------------------------------------------------------------

/** Human label + one-line role for a card id (UI copy lives here, not inline). */
export function cardLabel(card: DoctorCard): { title: string; role: string } {
  switch (card.id) {
    case 'ffmpeg':
      return { title: 'Video processing', role: 'Import, preview, export, and extract frames' }
    case 'ffprobe':
      return { title: 'Media inspection', role: 'Read clip duration, size, codecs, and audio tracks' }
    case 'perception':
      return { title: 'Captions and transcription', role: 'Word edits, captions, silence cleanup, and search' }
    case 'matte':
      return { title: 'Background removal', role: 'Cut out a subject without a green screen' }
    case 'matte_premium':
      return { title: 'Cleaner background removal', role: 'Sharper edges and click-to-pick subject selection' }
    case 'dub':
      return { title: 'AI dubbing', role: 'Re-voice translated speech as a new audio track' }
    case 'diarize':
      return { title: 'Speaker labels', role: 'Mark who speaks when in a transcript' }
    case 'disk':
      return { title: 'Download space', role: 'Room for optional tools, models, and cached media' }
    case 'gpu-encode':
      return { title: 'Faster exports', role: 'Use GPU encoding when this machine supports it' }
    default:
      if (card.id.startsWith('judge.')) {
        const prov = card.id.slice('judge.'.length)
        const legacy = card.details?.legacy === true
        return {
          title: `Judge: ${prov}${legacy ? ' (legacy)' : ''}`,
          role: 'render review via your own CLI subscription (no API key)',
        }
      }
      return { title: card.id, role: '' }
  }
}

/** Is this card an ESSENTIAL dependency (ffmpeg)? Drives the wizard gate. */
export function isEssential(card: DoctorCard): boolean {
  return card.id === 'ffmpeg'
}

/** readiness tri-state: should the first-run wizard AUTO-POP? ONLY when an essential tool is
 *  CONFIRMED missing (the ffmpeg card status is exactly 'missing') — NEVER on an
 *  UNVERIFIED ('unknown') probe-timeout, which must not nag the user with a "one
 *  essential tool is missing" modal that sticks until a manual re-scan. Kept
 *  DECOUPLED from `essential_ok` (the broader "can we edit at all" gate) so a
 *  future change there can't silently re-introduce auto-popping on a slow probe.
 *  The server already keeps `essential_ok` TRUE on an unverified ffmpeg, so this
 *  is belt-and-suspenders that also documents the precise auto-pop condition. */
export function shouldAutoPopWizard(report: DoctorReport | null): boolean {
  if (!report) return false
  const ffmpeg = report.cards.find((c) => c.id === 'ffmpeg')
  return ffmpeg?.status === 'missing'
}

/** Does this card have a one-click fix action in the UI? Only the ffmpeg tool
 *  card maps to system.fetch_tool, and ONLY on platforms with a BtbN build
 *  (Windows + Linux). macOS has no in-app fetch — there the card's hint guides
 *  the user to Homebrew, so we DON'T render a dead Install button. readiness tri-state: only on
 *  a CONFIRMED missing/degraded card — NOT on 'unknown' (an unverified probe must
 *  not push a confident "Install" for a binary that's probably present; the
 *  neutral Re-scan affordance handles it instead). */
export function hasFetchAction(card: DoctorCard, os: string): boolean {
  return (
    card.id === 'ffmpeg' &&
    (card.status === 'missing' || card.status === 'degraded') &&
    os !== 'macos'
  )
}

/** Does this card have the "Install captions" action? The perception card maps
 *  to system.setup_perception when it is missing OR degraded (e.g. a partial
 *  venv). NOT on 'unknown' (readiness tri-state: don't push setup off an unverified probe).
 *  Everything else is informational. */
export function hasSetupAction(card: DoctorCard): boolean {
  return (
    card.kind === 'perception' &&
    card.id === 'perception' &&
    (card.status === 'missing' || card.status === 'degraded')
  )
}

/** Does this card have a one-click background-removal install? Both matte cards
 *  (kind:'matte' → id 'matte' = RVM free tier, 'matte_premium' = MatAnyone2)
 *  map to system.setup_matte when missing OR degraded. The tier is keyed off the
 *  card id by the caller (matte_premium → 'matanyone', else 'rvm'). readiness tri-state: NOT on
 *  'unknown' — an UNVERIFIED matte_premium (its CUDA probe timed out) is already
 *  installed, so "Install premium" would be wrong; it gets the Re-scan affordance. */
export function hasMatteSetupAction(card: DoctorCard): boolean {
  return card.kind === 'matte' && (card.status === 'missing' || card.status === 'degraded')
}

// ---------------------------------------------------------------------------
// Chat-agent state — the agent.chat multi-agent selection dropdown
// ---------------------------------------------------------------------------

/** The coding-agent CLIs cutd can drive for `agent.chat`, in preference order
 *  (claude — the editor-sandboxed agent — is the default). Mirrors chat.rs
 *  CHAT_AGENTS. Each is detected as a `judge.<name>` card by the doctor. */
export const CHAT_AGENTS = ['claude', 'codex', 'grok'] as const
export type ChatAgentName = (typeof CHAT_AGENTS)[number]

/** Per-agent chat readiness, folded onto the `judge.<provider>` card by the
 *  backend as `details.chat` (doctor.rs `chat_agent_block`). Drives the
 *  AgentChat selector's 3-LEVEL badge: ABSENT (`!installed` → "install") →
 *  PRESENT-but-UNAUTH (`authenticated !== 'yes'` → "needs login") → READY.
 *  `authenticated` is BEST-EFFORT: 'yes' = a confirmed session, 'no' = no
 *  credentials, 'unknown' = a creds FILE exists but its session can't be
 *  verified headlessly (grok's expiring-token case) — deliberately NOT counted
 *  as ready, so the dropdown never shows a false green. `posture` is the
 *  INFORMATIONAL security tag (editor-sandboxed / cutd-tools / full system
 *  access) — transparency only, NEVER a gate. */
export interface ChatAgentState {
  installed: boolean
  resolved: string | null
  wired: boolean
  authenticated: 'yes' | 'no' | 'unknown'
  auth_detail: string
  ready: boolean
  posture: string | null
}

/** One row for the agent-selection dropdown: the agent name + its chat state
 *  (null when the report has no `judge.<name>` card or no `details.chat` block
 *  yet — rendered as "install", the absent state). */
export interface ChatAgentOption {
  name: ChatAgentName
  state: ChatAgentState | null
}

/** Extract the three chat agents (claude/codex/grok) from a doctor report, in
 *  preference order, reading each `judge.<provider>` card's `details.chat`.
 *  Populates the AgentChat selector. A report missing a card or its chat block
 *  yields `state:null` (the agent shows as not installed). */
export function chatAgentsFrom(report: DoctorReport | null): ChatAgentOption[] {
  return CHAT_AGENTS.map((name) => {
    const card = report?.cards.find((c) => c.id === `judge.${name}`)
    const chat = card?.details?.chat
    const state =
      chat && typeof chat === 'object' ? (chat as unknown as ChatAgentState) : null
    return { name, state }
  })
}

/** The 3-state badge for one agent (drives both the dropdown rows and the
 *  selector trigger). `kind` keys the colour class; `label` is the chip text;
 *  `hint` is the remediation copy (a login command / install note, '' for
 *  ready). Mirrors the three states returned by system.doctor. */
export interface ChatAgentBadge {
  kind: 'ready' | 'login' | 'install'
  label: string
  hint: string
}

export function chatAgentBadge(name: ChatAgentName, state: ChatAgentState | null): ChatAgentBadge {
  if (!state || !state.installed) {
    return { kind: 'install', label: 'Install', hint: `${name} is not installed on this machine` }
  }
  // PRESENT. Only surface "Needs login" when the session is CONFIRMED absent
  // ('no'). When it can't be verified ('unknown' — e.g. grok has NO safe
  // non-interactive status check, and its creds file persists across token
  // expiry), show the CLI as AVAILABLE: blocking a working agent behind a false
  // "needs login" is worse than letting the CLI's own auth prompt fire on first
  // call, which is exactly what happens during a first-run scan.
  if (state.authenticated === 'no') {
    return { kind: 'login', label: 'Needs login', hint: `run \`${name} login\`` }
  }
  // 'yes' = confirmed session → Ready; 'unknown' = present but unverifiable →
  // Available (the CLI signs in on first call if the session has lapsed).
  return state.authenticated === 'yes'
    ? { kind: 'ready', label: 'Ready', hint: '' }
    : { kind: 'ready', label: 'Available', hint: 'signs in on first call if needed' }
}

/** Display string for an agent's INFORMATIONAL security-posture tag. The backend
 *  reports the raw floor (editor-sandboxed / cutd-tools / full system access);
 *  codex's full-access floor gets a ⚠ prefix so the higher-trust posture reads at
 *  a glance (per the decided spec). Transparency only — never gates selection. */
export function chatAgentPosture(state: ChatAgentState | null): { text: string; warn: boolean } | null {
  if (!state?.posture) return null
  const warn = state.posture === 'full system access'
  return { text: warn ? `⚠ ${state.posture}` : state.posture, warn }
}

/** Group order for the settings/wizard render (tools, perception, background
 *  removal, then the optional AI services, judges, disk) — stable, scannable.
 *  The `matte` bucket (kind:'matte' → the RVM + MatAnyone2 cards) was previously
 *  DROPPED: the hub never surfaced background-removal install, so it lived only
 *  in the clip-contextual Matte drawer. Now it has an always-on home here too.
 *  The `services` bucket (kind:'service' → dub/diarize) is OPTIONAL + often
 *  REMOTE — rendered with the neutral Unknown styling ("not reachable" ≠ a red
 *  error), no install action (configured via the endpoint env var, not a verb). */
export function groupCards(cards: DoctorCard[]): {
  tools: DoctorCard[]
  perception: DoctorCard[]
  matte: DoctorCard[]
  services: DoctorCard[]
  judges: DoctorCard[]
  disk: DoctorCard[]
} {
  return {
    tools: cards.filter((c) => c.kind === 'tool'),
    perception: cards.filter((c) => c.kind === 'perception'),
    matte: cards.filter((c) => c.kind === 'matte'),
    services: cards.filter((c) => c.kind === 'service'),
    judges: cards.filter((c) => c.kind === 'judge'),
    disk: cards.filter((c) => c.kind === 'disk'),
  }
}

// ---------------------------------------------------------------------------
// Env health severity — shared by the status-bar chip + the topbar Setup dot
// ---------------------------------------------------------------------------

/** Env health SEVERITY rung (no UI copy — just the verdict). 'unknown' while the
 *  first doctor scan is still in flight. */
export type EnvHealthLevel = 'unknown' | 'ok' | 'degraded' | 'missing'

/** Collapse the doctor report to one health rung, missing-essential first:
 *  missing ESSENTIAL dep (ffmpeg → red) > any degraded card (amber) > ok (green).
 *  The single source of truth for BOTH the status-bar env chip and the topbar
 *  Setup button's nudge dot, so the two can never disagree. */
export function envHealthLevel(doctor: DoctorReport | null): EnvHealthLevel {
  if (!doctor) return 'unknown'
  if (!doctor.essential_ok) return 'missing'
  // A DEGRADED card OR an UNVERIFIED one (readiness tri-state: a probe that timed out) both
  // warrant the amber "needs a look" nudge — neither is a confident failure, but
  // both ask the user to re-scan / act. essential_ok already stays TRUE on an
  // unverified ffmpeg, so a slow probe never shows red (missing) here.
  // EXCEPTION: OPTIONAL remote services (kind:'service' → dub/diarize) are
  // normally ABSENT on a plain editing box, so their Unknown ("not reachable") is
  // the EXPECTED state — it must NOT paint the global env chip / Setup dot amber
  // for everyone who never runs them. They surface their own neutral OPTIONAL
  // chip inside the Environment panel instead; the global nudge stays green.
  if (
    doctor.cards.some(
      (c) => c.kind !== 'service' && (c.status === 'degraded' || c.status === 'unknown'),
    )
  )
    return 'degraded'
  return 'ok'
}
