// clientResults.ts — typed result payloads for verb client responses.
//
// Role: result-side contracts for non-trivial verb payloads. Keep these out of
// client.ts so the verb client stays focused on args, transport, and public
// compatibility re-exports.

import type {
  BrandKit, CutError,
  JobRecord,
  OpRecord,
  Project,
  RenderReceipt, SequenceSummary,
  TimelineTranscript,
  Transcript,
  Waveform,
} from './clientModel'
import type { MotionImportAttestation } from './motionLinkModel'

// Director-model job-result shapes (land in
// jobs.status → result.result for render.direct / render.qc).
/** One candidate subject on a director contact-sheet scene. */
export interface DirectorCandidate {
  label: string // A/B/C… ordered left→right
  cls: string
  conf: number
  cx: number // normalized 0..1 — feed this into render.reframe direction
  cy: number
  box: [number, number, number, number]
  has_face: boolean
}
export interface DirectorScene {
  scene: number
  t_ms: number
  keyframe_frame: number
  candidates: DirectorCandidate[]
}
export interface DirectorResult {
  direct_id: string
  contact_sheet: string
  contact_sheet_url?: string
  scene_count: number
  scenes: DirectorScene[]
  preset: string
}
/** One scene's composition QC on a reframed output. */
export interface QcScene {
  scene: number
  t_ms: number
  subject_present: boolean
  face_present: boolean
  face_cx: number | null
  off_center: number | null
  headroom: number | null
  needs_review: boolean
  issues: string[]
}
export interface QcResult {
  reframe_id: string
  qc_sheet: string
  qc_sheet_url?: string
  scene_count: number
  review_count: number
  scenes: QcScene[]
}

/** edit.restore result. A TIP restore returns {restored_op_id, op_ids}; a
 * REBASE restore additionally carries {mode:"rebase", rebased_over} — the ids
 * of the later ops the rebase re-based OVER (kept intact). The rebase fields
 * ride ONLY on a rebase op (absent for tip). Mirrors dispatch.rs shape_core_result
 * + verbs.json result string. `op_ids` is the appended restore op's id(s). */
export interface RestoreResult {
  restored_op_id: string
  op_ids: string[]
  mode?: 'rebase'
  rebased_over?: string[]
}

/** render.storyboard result — a contact-sheet JPEG of the composed timeline:
 * `count` evenly-spaced frames tiled into a `grid` of [cols, rows]. `path` is
 * the JPEG on disk; `base64` is the raw JPEG bytes (present only when called
 * with inline:true — the UI renders it directly as a data: URL, no /api fetch).
 * Display-only (zero-local-mutation contract): the engine creates no op for this. */
export interface StoryboardResult {
  path: string
  mime: string
  count: number
  grid: [number, number]
  frame_height: number
  duration_ms: number
  base64?: string
}

/** One ranked short-form candidate (clip.candidates — social repurposing).
 *  Honest heuristic scoring; `reason` says why it ranked where it did. at_ms /
 *  dur_ms are SOURCE-time. */
export interface ClipCandidate {
  asset: string
  word_range: [number, number]
  at_ms: number
  dur_ms: number
  hook_score: number
  retention_score: number
  score: number
  reason: string
  transcript_excerpt: string
}

export interface ClipCandidatesResult {
  candidates: ClipCandidate[]
  count: number
  scoring: 'heuristic'
  scoring_note?: string
  note?: string
}

/** One platform's artifacts in a render.bundle job result. */
export interface BundlePlatform extends BundlePlatformArtifacts {
  aspect: string
  width: number
  height: number
  path: string
  caption_path: string | null
  vtt_path: string | null
  caption_count: number
  thumb: string | null
  receipt_id: string | null
  pass: boolean | null
  duration_ms: number
}

/** autopilot.run job result (lands in jobs.status.result when the job is done).
 *  The CLEAN report — receipts/ops stay in the Inspect rail, not here. */
export interface AutopilotReport {
  summary_line: string
  policy: 'preview' | 'auto_low_risk'
  goal: string
  checks_pass: boolean
  stalled: boolean
  iterations: number
  fixes_applied: { check: string; via: string; clip?: string; failed?: string }[]
  plan: { check: string; fix_verb: string; auto_fixable: boolean; rationale: string }[]
  changed?: { ops?: number; clips_added?: number; clips_removed?: number; duration_delta_ms?: number; tracks_touched?: string[] }
  checkpoint: string
  receipt_ids: string[]
  restore_hint: string
}

// ---------------------------------------------------------------------------
// Generate module — built-in template catalog and normalized Generate IR.
// ---------------------------------------------------------------------------

export type GenerateKind = 'title' | 'caption' | 'shape' | 'motion' | 'social' | 'batch'
export type GenerateSource = 'builtin' | 'project' | 'user'

export interface GenerateParam {
  type: string
  required: boolean
  default?: unknown | null
  description?: string | null
  enum?: unknown[] | null
  minimum?: number | null
  maximum?: number | null
  step?: number | null
}

export interface GenerateLowering {
  verb: string
  args: Record<string, unknown>
}

export interface GenerateTemplateSummary {
  id: string
  source: GenerateSource
  kind: GenerateKind
  title: string
  summary: string
  tags: string[]
  params: Record<string, GenerateParam>
  capabilities: string[]
}

export interface GenerateTemplateManifest extends GenerateTemplateSummary {
  defaults: Record<string, unknown>
  lowering: GenerateLowering
  verification: Record<string, unknown>
}

export interface GeneratePreviewResult {
  id: string
  preview_id: string
  path: string
  url?: string
  mime: 'image/png'
  width: number
  height: number
  frame_ms: number
  params: Record<string, unknown>
  lowering: GenerateLowering
  supported: boolean
  warnings: string[]
  motion?: MotionTemplateToCutResult | MotionScriptToCutResult | null
}

export interface GenerateInsertResult {
  id: string
  instance_id: string
  checkpoint: Record<string, unknown>
  op_ids: string[]
  clips: string[]
  assets: string[]
  params: Record<string, unknown>
  lowering: GenerateLowering
  result: Record<string, unknown>
  restore_hint: string
}

export interface MotionTemplateToCutResult {
  policy: 'preview' | 'insert'
  template: string
  params: Record<string, unknown>
  motion_job_id?: string | null
  connector: Record<string, unknown>
  preview?: Record<string, unknown> | null
  render?: Record<string, unknown> | null
  checkpoint?: Record<string, unknown> | null
  import?: Record<string, unknown> | null
  insert?: Record<string, unknown> | null
  op_ids?: string[]
  clips?: string[]
  assets?: string[]
  artifacts: unknown[]
  receiptPath?: string | null
  warnings: string[]
  restore_hint?: string
}

export interface MotionScriptToCutResult {
  policy: 'preview' | 'insert'
  script?: Record<string, unknown> | null
  scriptPath: string
  motion_job_id?: string | null
  connector: Record<string, unknown>
  preview?: Record<string, unknown> | null
  render?: Record<string, unknown> | null
  checkpoint?: Record<string, unknown> | null
  import?: Record<string, unknown> | null
  insert?: Record<string, unknown> | null
  op_ids?: string[]
  clips?: string[]
  assets?: string[]
  artifacts: unknown[]
  receiptPath?: string | null
  warnings: string[]
  restore_hint?: string
}

export type MotionJobLifecycle = 'pending' | 'running' | 'ended'
export type MotionJobOutcome = 'succeeded' | 'failed' | 'cancelled' | 'skipped'
export type MotionJobState = 'pending' | 'running' | MotionJobOutcome

export interface MotionJobStatus {
  schema: 'shellx-motion/job-status@1'
  jobId: string
  lifecycle: MotionJobLifecycle
  outcome?: MotionJobOutcome | null
  state: MotionJobState
  lane?: string
  operation?: string
  createdAtMs?: number
  startedAtMs?: number
  endedAtMs?: number
  durationMs?: number
  queueWaitMs?: number
  error?: Record<string, unknown>
  cancellation?: Record<string, unknown>
  skip?: Record<string, unknown>
  warnings?: string[]
  pollAfterMs?: number
  receiptAvailable: boolean
}

export interface MotionJobQueryResult {
  schema: 'shellx-cut/motion-job-query@1'
  caller_scope: 'active-project'
  job: MotionJobStatus
}

export interface MotionJobListResult {
  schema: 'shellx-cut/motion-job-list@1'
  caller_scope: 'active-project'
  job_count: number
  in_flight_count: number
  state_counts: Record<MotionJobState, number>
  jobs: MotionJobStatus[]
}

export interface MotionImportPlanStep {
  verb: 'media.import' | 'edit.insert' | string
  args: Record<string, unknown>
}

export interface MotionImportMapResult {
  ok: true
  schema: 'shellx-cut/motion-import-map@1'
  planPath: string
  packageDir?: string | null
  packageId: string
  motionId: string
  targetId: string
  mode: 'rendered_media' | 'editable_lowering' | string
  integration: Record<string, unknown>
  operationCount: number
  operations: Record<string, unknown>[]
  renderedMedia?: Record<string, unknown> | null
  artifactHandles: Record<string, unknown>[]
  lineageProofs: MotionImportAttestation[]
  planned: MotionImportPlanStep[]
  warnings: string[]
}

export interface MotionImportApplyResult {
  ok: true
  schema: 'shellx-cut/motion-import-apply@1'
  dryRun: boolean
  wouldMutate: boolean
  planPath: string
  packageDir?: string | null
  packageId: string
  motionId: string
  targetId: string
  mode: 'rendered_media' | 'editable_lowering' | string
  integration: Record<string, unknown>
  lineageProofs: MotionImportAttestation[]
  planned: MotionImportPlanStep[]
  import?: Record<string, unknown> | null
  insert?: Record<string, unknown> | null
  imports?: Record<string, unknown>[]
  inserts?: Record<string, unknown>[]
  bindings?: { sourceLayerId: string; sourceVerb: string; cutVerb: string; clipId: string; trackId?: string | null; assetId?: string | null; dynamicParams?: string[] }[]
  mappingOpId?: string
  alreadyApplied?: boolean
  reimported?: boolean
  op_ids?: string[]
  clips?: string[]
  assets?: string[]
  warnings: string[]
  restore_hint?: string
}

export interface AgentChatPlan {
  request: string
  reference_ids: string[]
  policy: string[]
}

export interface AgentChatReview {
  turn_id: string
  baseline: string
  checkpoint: string | null
  tip: string | null
  diff: {
    clips_added?: string[] | number
    clips_removed?: string[] | number
    clips_moved?: string[] | number
    duration_delta_ms?: number
    tracks_touched?: Array<string | { track?: string; ranges_ms?: [number, number][] }>
    ops?: OpRecord[]
  } | null
  diff_error: string | null
  revert_safe: boolean
  concurrent_actions: Array<{
    op_id: string
    verb: string
    actor: OpRecord['actor']
  }>
}

export interface MotionImportApplyJob {
  ok: true
  schema: 'shellx-cut/motion-import-apply-job@1'
  job_id: string
  cancellable: true
  status: 'queued'
}

export interface GeneratePromptPlan {
  template_id: string
  params: Record<string, unknown>
  at_ms?: number
  rationale?: string
  confidence?: number
  alternatives?: unknown[]
}

export interface GenerateFromPromptResult {
  status: 'completed' | 'not_run' | 'error'
  request: Record<string, unknown>
  backend?: Record<string, unknown> | null
  plan?: GeneratePromptPlan | null
  validation?: Record<string, unknown> | null
  preview?: GeneratePreviewResult | null
  insert?: GenerateInsertResult | null
  reason?: string | null
  warnings: string[]
  next_actions: string[]
}

export type GenerateStoryboardMode = 'quick_prompt' | 'director_brief' | 'script' | 'existing_media'
export type GenerateStoryboardStatus = 'draft' | 'needs_input' | 'valid' | 'previewed' | 'inserted'
export type GenerateStoryboardSceneSource = 'generate_template' | 'existing_media' | 'assemble_slot' | 'generated_asset' | 'caption' | 'audio'

export interface GeneratedAssetMetadata {
  schema: 'shellx-cut/generated-asset/1' | 'shellx-cut/generated-asset/2'
  generation_id: string
  family_id?: string
  provider: 'codex' | 'grok'
  kind: 'image' | 'video'
  model: string | null
  prompt: string
  variation?: string | null
  references?: Array<{ asset_id: string; content_hash: string; kind: 'image' | 'video' }>
  created_at_ms?: number | null
  reused: boolean
  cost_usd: null
  cost_note: string
  provenance_path: string
}

export interface AssetsGenerateResult {
  job_id: string
  generation_id: string
  family_id: string
  provider: 'codex' | 'grok'
  kind: 'image' | 'video'
  variation: string | null
  placement: GeneratedAssetPlacement | null
  state: 'queued'
}

export interface GeneratedAssetPlacement {
  mode: 'insert' | 'replace'
  target_clip: string
  track: string
  duration_ms: number
  at_ms?: number
  state: 'pending' | 'applied' | 'failed'
  edit?: Record<string, unknown> | null
  cleanup?: Record<string, unknown> | null
  error?: { code?: string; message?: string; cause?: string; suggested_action?: string } | null
}

export type GeneratedAssetIntegrity =
  | 'verified'
  | 'changed'
  | 'offline'
  | 'missing_provenance'
  | 'invalid_provenance'
  | 'provenance_mismatch'
  | 'unsafe_source'

export interface GeneratedAssetRecord {
  asset_id: string
  generation_id: string
  family_id: string
  provider: 'codex' | 'grok' | null
  kind: 'image' | 'video' | null
  model: string | null
  prompt: string
  variation: string | null
  reference_asset_ids: string[]
  created_at_ms: number | null
  cost_usd: null
  cost_note: string
  content_hash: string
  provenance_schema: string | null
  integrity: GeneratedAssetIntegrity
}

export interface GeneratedAssetsListResult {
  items: GeneratedAssetRecord[]
  total: number
  verified: number
}

export interface AssetsGenerateJobResult {
  ok: true
  asset_id: string
  /** The normal media-import enrichment job, null when an imported asset was reused. */
  job_id: string | null
  generated: GeneratedAssetMetadata
  placement?: GeneratedAssetPlacement | null
  op?: OpRecord
}

export interface GenerateStoryboardScene {
  scene_id: string
  index: number
  role: string
  range_ms: [number, number]
  source: GenerateStoryboardSceneSource
  template_id?: string
  params?: Record<string, unknown>
  query?: string
  screen_text?: string
  narration?: string
  asset_refs?: string[]
  missing_assets?: string[]
  motion?: string
  transition_in?: string
  transition_out?: string
  evidence?: Record<string, unknown>
}

export interface GenerateStoryboard {
  schema: 'shellx-cut/generate-storyboard/1'
  storyboard_id: string
  mode: GenerateStoryboardMode
  status: GenerateStoryboardStatus
  brief: Record<string, unknown>
  brief_meta?: { stated?: string[]; inferred?: string[]; missing?: string[] }
  scenes: GenerateStoryboardScene[]
  validation?: Record<string, unknown>
  next?: Record<string, unknown>
}

export interface GenerateStoryboardQuestion {
  id: string
  prompt: string
  field?: string
  choices?: string[]
}

export interface GenerateStoryboardValidation {
  ok: boolean
  result: 'pass' | 'warn' | 'fail'
  errors: string[]
  warnings: string[]
  missing_inputs: string[]
  scene_count: number
  duration_ms: number
  template_ids: string[]
}

export interface GenerateStoryboardPreviewScene {
  scene_id: string
  index: number
  status: 'previewed'
  source: 'generate_template'
  template_id: string
  range_ms: [number, number]
  preview_id: string
  path: string
  url?: string
  mime: 'image/png'
  width: number
  height: number
  frame_ms: number
  params: Record<string, unknown>
  lowering: GenerateLowering
}

export interface GenerateStoryboardPreview {
  policy: 'preview'
  mutated: false
  scenes: GenerateStoryboardPreviewScene[]
  unsupported: unknown[]
  warnings: string[]
}

export interface GenerateStoryboardInsertScene {
  scene_id: string
  index: number
  status: 'inserted'
  source: 'generate_template'
  template_id: string
  range_ms: [number, number]
  checkpoint: Record<string, unknown>
  op_ids: string[]
  clips: string[]
  assets: string[]
  params: Record<string, unknown>
  lowering: GenerateLowering
  result: Record<string, unknown>
  restore_hint: string
}

export interface GenerateStoryboardInsert {
  policy: 'insert'
  mutated: true
  scenes: GenerateStoryboardInsertScene[]
  checkpoints: string[]
  op_ids: string[]
  clips: string[]
  assets: string[]
  restore_hint: string
  unsupported: unknown[]
  warnings: string[]
}

export interface GenerateStoryboardResult {
  status: 'completed' | 'needs_input' | 'not_run' | 'error'
  request: Record<string, unknown>
  backend?: Record<string, unknown> | null
  storyboard?: GenerateStoryboard | null
  questions: GenerateStoryboardQuestion[]
  validation?: GenerateStoryboardValidation | null
  evidence: {
    policy: 'plan' | 'preview' | 'insert'
    mutated: boolean
    skill_path: string[]
    scene_count: number
    duration_ms: number
    template_ids: string[]
    brief_fields: { stated: string[]; inferred: string[]; missing: string[] }
  }
  preview?: GenerateStoryboardPreview | null
  insert?: GenerateStoryboardInsert | null
  reason?: string | null
  warnings: string[]
  next_actions: string[]
}

// ---------------------------------------------------------------------------
// Recipe layer — named, gated pipeline manifests (recipe.list/describe/run).
// Result shapes mirror dispatch.rs describe_recipe / recipe_run / finish_recipe.
// The Recipes panel (panels/Recipes) renders these; pinned here so the panel is
// not casting `unknown`. See schema/verbs.json recipe.* + schema/recipes.json.
// ---------------------------------------------------------------------------

/** One declared recipe param (recipe.list compact form + recipe.describe full
 *  form). `type` is the JSON type string ("string" | "integer" | …); `enum` +
 *  `description` appear only in the describe (full-manifest) form. */
export interface RecipeParam {
  name: string
  type: string
  required: boolean
  default: unknown
  description?: string
  enum?: string[]
}

/** A stage's success GATE — receipt `checks` (named verify checks the render
 *  must pass) and/or render-free `state` predicates (a project fact vs a bound).
 *  null in the manifest when a stage has no gate. */
export interface RecipeGate {
  checks?: string[]
  state?: { fact: string; op: string; value: unknown }[]
}

/** One ordered stage of a recipe (recipe.describe / the dry-run plan): the verb
 *  it dispatches, the interpolated args, why, and its gate. */
export interface RecipeStage {
  id: string
  verb: string
  args: Record<string, unknown>
  rationale?: string
  await_job?: boolean
  gate: RecipeGate | null
}

/** recipe.list entry — the discovery surface (name/title/summary + stage count). */
export interface RecipeSummary {
  name: string
  title: string
  description: string
  params: RecipeParam[]
  stage_count: number
}

/** recipe.describe — the full resolved manifest (params + ordered stages). */
export interface RecipeManifest {
  name: string
  title: string
  description: string
  params: RecipeParam[]
  stages: RecipeStage[]
}

/** recipe.run{policy:'dry_run'} — the resolved PLAN, returned directly (no job,
 *  no checkpoint, nothing dispatched). This is the receipt-legibility preview:
 *  the exact op list the recipe WOULD apply, with each stage's gate. */
export interface RecipeDryRun {
  recipe: string
  policy: 'dry_run'
  status: 'planned'
  params: Record<string, unknown>
  stages: { id: string; verb: string; args: Record<string, unknown>; rationale?: string; gate: RecipeGate | null }[]
}

/** recipe.run{policy:'run'} — the SYNC handle: the run is a job; the clean
 *  receipt (RecipeReport) lands in jobs.status.result when it finishes. */
export interface RecipeRunHandle {
  job_id: string
  checkpoint: string
  recipe: string
  stages: { id: string; verb: string }[]
}

/** One stage's recorded outcome in the finished run receipt. `gate` is the
 *  evaluated gate (pass + per-check / per-state evidence); `error` is set on the
 *  failing stage that stopped the run. */
export interface RecipeStageResult {
  id: string
  verb: string
  ok: boolean
  op_ids: string[]
  job_id?: string
  job_result?: unknown
  gate?: {
    pass: boolean
    checks?: { name: string; pass: boolean; found?: unknown; evidence?: unknown }[]
    state?: { fact: string; op: string; value: unknown; measured?: unknown; pass: boolean }[]
  }
  error?: CutError
}

/** recipe.run job result (lands in jobs.status.result when the run is done) —
 *  the CLEAN receipt: one summary line + per-stage results + what changed + the
 *  single-step restore checkpoint. */
export interface RecipeReport {
  summary_line: string
  recipe: string
  status: 'completed' | 'completed_with_warnings' | 'failed' | 'gate_failed'
  policy: string
  stages_run: number
  stage_results: RecipeStageResult[]
  changed?: { ops?: number; clips_added?: number; clips_removed?: number; duration_delta_ms?: number; tracks_touched?: string[] }
  checkpoint: string
  receipt_ids: string[]
  restore_hint: string
}

/** render.bundle job result (lands in jobs.status.result when the job is done). */
export interface BundlePlatformArtifacts {
  hash: string
  caption_hash: string | null
  vtt_hash: string | null
  thumb_hash: string | null
}

export interface PublishPackageIssue {
  code: string
  severity: 'warning' | 'error'
  aspect?: string
  detail: string
}

export interface BundleResult {
  bundle_id: string
  range_ms: [number, number]
  platforms: BundlePlatform[]
  receipt_ids: string[]
  status: 'ready' | 'needs_review' | 'blocked'
  pass: boolean
  issues: PublishPackageIssue[]
  warnings: string[]
  manifest_path: string
  manifest_hash: string
  brand?: BundleBrandResult
}

export interface BrandCheckResult {
  source: 'stored' | 'explicit'
  styles_checked: number
  pass: boolean
  note?: string
  violations: Record<string, unknown>
  brand: BrandKit
}

export interface BundleBrandResult {
  source: 'stored' | 'explicit'
  pass: boolean
  brand: BrandKit
  platforms: Array<{ aspect: string; width: number; height: number; check: Omit<BrandCheckResult, 'source'> }>
}

/** One project in the recent-projects index (project.list). `missing` = the
 *  .cutproj vanished on disk (greyed out, not auto-removed). */
export interface ProjectEntry {
  id: string
  name: string
  path: string
  created_ms: number
  last_opened_ms: number
  thumb?: string
  duration_ms?: number
  clip_count?: number
  missing?: boolean
}

/** Slim probe facts on a library item. */
export interface LibProbe {
  duration_ms?: number
  width?: number
  height?: number
  has_audio?: boolean
}

/** One global asset library item. Exactly one of src_path (linked original) /
 *  blob (content-addressed stored-copy filename, served via /api/library-blob). */
export interface LibItem {
  id: string
  type: 'video' | 'audio' | 'image'
  name: string
  src_path?: string
  blob?: string
  tags: string[]
  folder?: string | null
  favorite?: boolean
  added_ms: number
  used_ms?: number
  uses?: number
  source: 'user' | 'agent'
  probe?: LibProbe
  /** Computed at list time: the resolved source/blob exists on disk. false ⇒
   *  a linked original went missing — show the kind glyph, don't request a
   *  poster (it can only 404, and every 404 is webview console noise). */
  media_ok?: boolean
}

export interface LibraryListResult {
  items: LibItem[]
  folders: string[]
  /** Stable global tag facets; unlike items, these do not collapse after a tag filter. */
  tags: string[]
  /** Exact number of matches before paging. */
  total: number
  /** Echoed zero-based page offset and bounded page size. */
  offset: number
  limit: number
  /** Present when another page exists; null/undefined at the end. */
  next_offset?: number | null
}

/** Per-channel colour stats of one sampled frame (edit.color_match), 0..255. */
export interface ColorMatchStats {
  mean_r: number
  mean_g: number
  mean_b: number
  std_r: number
  std_g: number
  std_b: number
  mean_luma: number
  std_luma: number
  mean_chroma: number
  std_chroma: number
}

export interface PregateRisk {
  kind: string
  severity: 'high' | 'med' | 'low'
  detail?: string
  range_ms?: [number, number]
}

export interface PregateReport {
  pass: boolean
  risks: PregateRisk[]
  summary?: string
  thresholds?: Record<string, unknown>
  perception_assets?: number
  uninstrumented_assets?: string[]
}

export interface MotionTrackingState {
  analysisId?: string | null
  assetId?: string | null
  mode?: 'point' | 'planar'
  model?: 'translation' | 'similarity' | 'homography'
  lifecycleState?: string | null
  attachedLayerId?: string | null
  fidelity?: string | null
}

export interface MotionTrackingInventory {
  packageId: string
  motionId: string
  width: number
  height: number
  durationMs: number
  fps: number
  videoAssets: Array<{ id: string; name: string; available: boolean }>
  targetLayers: Array<{ id: string; name: string; kind: string; trackingAttached: boolean }>
  analyses: Array<{ analysisId: string; state: string; assetId?: string | null }>
}

export interface MotionTrackingLifecycleSummary {
  analysisId?: string | null
  state?: string | null
  attempt?: number | null
  updatedAt?: string | null
  source?: Record<string, unknown> | null
  lastGood?: Record<string, unknown> | null
}

export interface MotionTrackingSourceSummary {
  assetId?: string | null
  current?: boolean | null
  sha256?: string | null
  byteLength?: number | null
}

export interface MotionTrackingReceiptSummary {
  id?: string | null
  operation?: string | null
  status?: string | null
}

export interface MotionTrackingMutationResult {
  ok: true
  clip: string
  receipt: MotionTrackingReceiptSummary
  warnings: string[]
  restore_hint?: string
}

export interface SequenceIndexBaseRow {
  kind: 'clip' | 'marker'
  sequence_id: string
  sequence_name: string
  active: boolean
  id: string
  at_ms: number
  end_ms: number
  label: string
}

export interface SequenceIndexClipRow extends SequenceIndexBaseRow {
  kind: 'clip'
  track_id: string
  track_kind: 'video' | 'audio' | 'caption'
  clip_kind: 'media' | 'caption' | 'gap'
  asset?: string
  src_in_ms?: number
  src_out_ms?: number
  effect_count: number
  effects: string[]
  offline: boolean
  track_visible: boolean
  track_locked: boolean
  track_muted: boolean
  issues: Array<'offline' | 'gap'>
}

export interface SequenceIndexMarkerRow extends SequenceIndexBaseRow {
  kind: 'marker'
  note?: string
  color?: string
}

export interface SequenceIndexResult {
  query: string
  kind: 'all' | 'clip' | 'marker'
  sequence?: string
  track_kind?: 'video' | 'audio' | 'caption'
  status: 'all' | 'issues' | 'offline' | 'gaps' | 'effects' | 'hidden' | 'locked' | 'muted'
  total: number
  clip_count: number
  marker_count: number
  issue_count: number
  effect_clip_count: number
  truncated: boolean
  results: Array<SequenceIndexClipRow | SequenceIndexMarkerRow>
}

export interface MediaCheckRow {
  asset: string
  path: string
  exists: boolean
  modified_ms?: number
  referenced: number
}

export interface MediaCheckResult {
  count: number
  offline_count: number
  assets: MediaCheckRow[]
}

export type ProjectHealthJournalStatus = 'verified' | 'recovered' | 'attention' | 'unavailable'
export type ProjectHealthDerivedState = 'available' | 'missing' | 'not_recorded' | 'not_applicable'

export interface ProjectHealthNotice {
  code: 'journal_tail_recovered' | 'project_cache_rebuilt' | 'history_snapshot_rejected' | 'identity_revalidation_failed' | string
  message: string
  discarded_bytes?: number
  discarded_start?: number
  discarded_end?: number
}

export interface ProjectHealthAsset {
  asset: string
  source: 'available' | 'offline'
  proxy: ProjectHealthDerivedState
  filmstrip: ProjectHealthDerivedState
}

export interface ProjectHealthPageCounts {
  offline: number
  proxy_available: number
  proxy_missing: number
  proxy_not_recorded: number
  proxy_not_applicable: number
  filmstrip_available: number
  filmstrip_missing: number
  filmstrip_not_recorded: number
  filmstrip_not_applicable: number
}

export interface ProjectHealthResult {
  schema: 'shellx-cut/project-health/1'
  project_revision?: string
  journal: {
    status: ProjectHealthJournalStatus
    log_records?: number
    cache: 'matched' | 'rebuilt'
    snapshot: { status: 'not_present' | 'verified' | 'rejected'; prefix_ops?: number }
    notices: ProjectHealthNotice[]
  }
  media: {
    status: 'ready' | 'unavailable'
    asset_count: number
    checked_count: number
    page: ProjectHealthPageCounts
    assets: ProjectHealthAsset[]
    limit: number
    cursor?: string | null
    next_cursor?: string
    has_more: boolean
  }
}

export interface JobPersistenceNotice {
  code: 'job_record_quarantined' | string
  record: string
  message: string
  quarantine?: string
}

export interface JobsListResult {
  jobs: JobRecord[]
  persistence_notices?: JobPersistenceNotice[]
}

/** Path-free, project-local screen-record recovery status. The Settings model
 * still validates every fetched page before presenting this server projection. */
export type CaptureRecoveryState =
  | 'complete'
  | 'recovered'
  | 'quarantined'
  | 'interrupted'
  | 'owner_ambiguous'
  | 'torn_journal'
  | 'corrupt'

export interface CaptureRecoveryReceipt {
  state: 'complete' | 'recovered' | 'quarantined' | 'interrupted'
  recovered_segments: number
  lost_tail_ms: number | null
  lost_tail_lower_bound_ms: number
  lost_tail_upper_bound_ms: number | null
  audio_first_packet_offset_ms: number | null
  source: string | null
}

export interface CaptureRecoveryItem {
  capture_id: string
  state: CaptureRecoveryState
  checkpoints: number
  has_open_segment: boolean
  receipt?: CaptureRecoveryReceipt | null
}

export interface ScreenRecordRecoveryStatusResult {
  captures: CaptureRecoveryItem[]
  next_cursor: string | null
}

export interface McpSelfTestResult {
  schema: 'shellx-cut/mcp-self-test/1'
  mode: 'proxy'
  read_only: true
  executable: string
  command: [string, 'mcp']
  protocol_version: string
  ping: true
  tools: number
  expected_tools: number
  tools_list_bytes: number
  tools_list_max_bytes: number
  proxy_addr: string
  same_engine: true
}

/** Typed result payloads where the shape is pinned by the contract. */
export interface VerbResults {
  'project.create': { path: string; project: Project; starter_asset_path?: string }
  // Default project.state is always the materialized Project. Incremental
  // callers decode the sync-only delta in projectSync.ts after passing
  // since_revision; keeping this default type preserves existing readers.
  'project.state': Project
  'project.sequence_list': { active_sequence: string; sequences: SequenceSummary[] }
  'project.sequence_index': SequenceIndexResult
  'project.sequence_create': { sequence: SequenceSummary; active_sequence: string }
  'project.sequence_switch': { active_sequence: string }
  'project.sequence_rename': { id: string; name: string }
  'project.sequence_delete': { deleted: boolean; id: string }
  'project.ops': { ops: OpRecord[]; cursor?: string; next_cursor?: string; has_more: boolean; limit: number; encoded_bytes: number; undo_available: boolean; redo_available: boolean; project_revision?: string | null }
  'project.undo': { to_op: string | null; cursor: number; undo_available: boolean; redo_available: boolean }
  'project.redo': { to_op: string | null; cursor: number; undo_available: boolean; redo_available: boolean }
  'project.brand': { brand: BrandKit | null; cleared: boolean }
  'verify.brand': BrandCheckResult
  // agent.chat — SUCCESS path is {ok:true, agent, reply, actions, attachments, cost_usd}.
  // The ok:false path adds structured fields the UI renders
  // inline so the user always knows WHY a turn did not execute:
  //   reason         — the human explanation (also mirrored into `reply` for
  //                    back-compat with the v1 bubble).
  //   error          — a machine category: not_available | auth | quota |
  //                    blocked | cli_error | no_change | timeout (drives a small tag).
  //   agent_message  — the agent's OWN final words on the ran-but-no-change /
  //                    refusal path (parsed from the CLI output); null otherwise.
  //   detail         — the raw CLI tail (diagnostics). All four omitted on success.
  'agent.chat': {
    ok: boolean
    agent: string | null
    reply: string
    actions: Array<{ op_id: string; verb: string }>
    attachments: string[]
    plan: AgentChatPlan
    review: AgentChatReview | null
    cost_usd: number | null
    reason?: string
    error?: string
    agent_message?: string | null
    detail?: string | null
  }
  'jobs.status': JobRecord
  'jobs.list': JobsListResult
  'jobs.cancel': { job_id: string; cancelled: boolean }
  'system.mcp_test': McpSelfTestResult
  'assets.generate': AssetsGenerateResult
  'assets.generated_list': GeneratedAssetsListResult
  'media.index_status': { count: number; assets: Array<{ asset: string; indexed_frames: number; dim: number; model: string; path?: string }> }
  'generate.list': { templates: GenerateTemplateSummary[] }
  'generate.describe': GenerateTemplateManifest
  'generate.preview': GeneratePreviewResult
  'generate.insert': GenerateInsertResult
  'generate.from_prompt': GenerateFromPromptResult
  'generate.storyboard': GenerateStoryboardResult
  'motion.template_to_cut': MotionTemplateToCutResult
  'motion.script_to_cut': MotionScriptToCutResult
  'motion.job.get': MotionJobQueryResult
  'motion.job.list': MotionJobListResult
  'motion.map_import': MotionImportMapResult
  'motion.apply_import': MotionImportApplyResult | MotionImportApplyJob
  'motion.link.refresh': {
    ok: true
    schema: 'shellx-cut/motion-link-refresh@1'
    clip: string
    asset: string
    job_id: string
    motion_job_id?: string | null
    packageId: string
    motionId: string
    sourceRevision: string
    render: { path: string; sha256: string; byteLength: number; preset: string }
    lastReceiptId?: string | null
    receiptPath: string
    state: 'linked-current'
    restore_hint: string
  }
  'motion.link.relink': {
    ok: true
    schema: 'shellx-cut/motion-link-relink@1'
    clip: string
    packageDir: string
    packageId: string
    motionId: string
    sourceRevision: string
    state: 'source-dirty'
  }
  'motion.link.edit': {
    ok: true
    schema: 'shellx-cut/motion-link-edit@1'
    clip: string
    packageId: string
    motionId: string
    launched: true
    pid?: number | null
    localOnly: true
    remotePublish: false
  }
  'motion.link.tracking.inventory': {
    ok: true
    schema: 'shellx-cut/motion-tracking-inventory@1'
    clip: string
    inventory: MotionTrackingInventory
    tracking?: MotionTrackingState | null
    localOnly: true
  }
  'motion.link.tracking.request': MotionTrackingMutationResult & {
    schema: 'shellx-cut/motion-tracking-request@1'
    analysisId: string
    lifecycle: MotionTrackingLifecycleSummary
    state: 'linked-current'
  }
  'motion.link.tracking.inspect': {
    ok: true
    schema: 'shellx-cut/motion-tracking-inspect@1'
    clip: string
    analysisId: string
    lifecycle: MotionTrackingLifecycleSummary
    source: MotionTrackingSourceSummary
    current: boolean
    receipt: MotionTrackingReceiptSummary
    warnings: string[]
    localOnly: true
  }
  'motion.link.tracking.apply': MotionTrackingMutationResult & {
    schema: 'shellx-cut/motion-tracking-apply@1'
    analysisId: string
    layerId: string
    plan: { status?: string | null; fidelity?: string | null; segmentCount: number; warnings: string[] }
    changedPaths: string[]
    state: 'source-dirty'
    refreshRequired: true
  }
  'motion.link.tracking.verify': {
    ok: true
    schema: 'shellx-cut/motion-tracking-verify@1'
    clip: string
    layerId: string
    analysisId?: string
    verification: { attached?: boolean; current?: boolean; reasons?: string[]; mismatchedTargets?: string[] }
    lifecycle: MotionTrackingLifecycleSummary
    source: MotionTrackingSourceSummary
    receipt: MotionTrackingReceiptSummary
    warnings: string[]
    localOnly: true
  }
  'motion.link.tracking.detach': MotionTrackingMutationResult & {
    schema: 'shellx-cut/motion-tracking-detach@1'
    layerId: string
    analysisId?: string | null
    restoredPreviousKeyframes: boolean
    changedPaths: string[]
    state: 'source-dirty'
    refreshRequired: true
  }
  // Recipe layer. recipe.run is a UNION: dry_run returns the plan DIRECTLY,
  // run returns the {job_id} handle (the receipt lands in jobs.status). The
  // Recipes panel narrows on `policy`/`job_id`.
  'recipe.list': { recipes: RecipeSummary[] }
  'recipe.describe': RecipeManifest
  'recipe.run': RecipeDryRun | RecipeRunHandle
  // edit.cut_to_beat: cuts = resulting cut positions (split) or the new boundary
  // positions (snap); moves present only in snap mode. beats_used = cuts made.
  'edit.cut_to_beat': {
    mode: 'split' | 'snap'
    track: string | null
    cuts: number[]
    beats_used: number
    every_n: number
    beats_available?: number
    moves?: Array<{ from: number; to: number }>
    note?: string
  }
  // Non-destructive mute/solo FLAG toggles — echo the track + old/new flag value.
  'edit.track_visible': { track: string; visible: boolean; old_visible: boolean }
  'edit.track_lock': { track: string; locked: boolean; old_locked: boolean }
  'edit.mute': { track: string; muted: boolean; old_muted: boolean }
  'edit.solo': { track: string; solo: boolean; old_solo: boolean }
  'clip.candidates': ClipCandidatesResult
  'media.waveform': Waveform
  'media.check': MediaCheckResult
  'project.health': ProjectHealthResult
  'screen_record.recovery_status': ScreenRecordRecoveryStatusResult
  'media.relink': {
    asset: string
    path: string
    old_path: string
    hash_changed: boolean
    derived_cleared: boolean
    freed: string[]
    job_id?: string
  }
  'transcript.get': Transcript
  'transcript.timeline': TimelineTranscript
  'transcript.ignore_words': {
    asset: string
    word_range: [number, number]
    text: string
    action: 'add' | 'remove'
    changed: boolean
    transcript_ignores: Array<{ asset: string; word_range: [number, number] }>
  }
  'transcript.chapters': {
    asset: string
    count: number
    chapters: Array<{
      index: number
      start_ms: number
      end_ms: number
      title: string
      word_range: [number, number]
    }>
    next: string
  }
  'transcript.remove_retakes': {
    removed_takes: number
    kept: number
    keep_policy: string
    words_removed: number
    ms_removed: number
    clusters: Array<{
      asset: string
      kept: { word_range: [number, number]; at_ms: [number, number]; text: string }
      removed: Array<{ word_range: [number, number]; at_ms: [number, number]; text: string }>
    }>
    note?: string
  }
  // i18n: transcript translated → a sibling receipts/<asset>.<lang>.words.json.
  // backend_proven=false flags a best-effort CLI agent (codex/grok). word_timing
  // is "interpolated" (segment timing exact; per-word linearly interpolated).
  'transcript.translate': {
    asset: string
    source_lang: string | null
    target_lang: string
    backend: 'cli' | 'local'
    backend_proven: boolean
    model: string
    agent: string | null
    segments_translated: number
    words: number
    transcript: string
    word_timing: 'interpolated'
    note: string
  }
  // i18n: caption cues translated, each at its source cue's EXACT range_ms.
  // mode:track adds target_track; mode:replace reports `replaced` cue count.
  'captions.translate': {
    source_lang: string | null
    target_lang: string
    backend: 'cli' | 'local'
    backend_proven: boolean
    model: string
    agent: string | null
    cues_translated: number
    mode: 'track' | 'replace'
    source_track: string
    target_track: string
    timestamps_preserved: true
    replaced?: number
    reflow?: { cues_before: number; cues_after: number; extended: number; split: number; still_too_fast: number }
    reflow_hint?: string
  }
  // edit.color_match — the derived grade + both stat sets (the committed grade
  // op id rides along in op_ids). Stats are 0..255 per-channel mean+std.
  'edit.color_match': {
    clip: string
    reference: string
    space: string
    strength: number
    identity: boolean
    derived: { contrast: number; brightness: number; saturation: number; gamma: number; temperature_k?: number }
    stats: {
      target: ColorMatchStats
      reference: ColorMatchStats
    }
    op_ids: string[]
  }
  // edit.auto_balance — the derived auto white-balance/exposure grade + the
  // sampled frame stats (the committed grade op id rides along in op_ids).
  // highlight_warmth is set only for white_patch (null ⇒ gray-world or no
  // qualifying highlights → it used the whole-frame average).
  'edit.auto_balance': {
    clip: string
    mode: 'gray_world' | 'white_patch'
    strength: number
    identity: boolean
    derived: { contrast: number; brightness: number; saturation: number; gamma: number; temperature_k?: number }
    stats: {
      mean_rgb: [number, number, number]
      mean_luma: number
      channel: ColorMatchStats
      highlight_warmth: number | null
    }
    op_ids: string[]
  }
  // edit.auto_zoom — the emphasis punches that were committed as scale keyframes.
  // peak is the trigger metric (momentary LUFS for energy, preceding pause-gap ms
  // for transcript); scale = 1.0+intensity. count:0 + note when nothing was applied.
  'edit.auto_zoom': {
    clip: string
    zooms: Array<{ at_ms: number; peak: number; scale: number }>
    intensity: number
    trigger: string
    count: number
    op_ids?: string[]
    note?: string
  }
  // edit.multicam_switch — the active-speaker switch program. Each shot's active angle
  // was committed as a plain edit.insert onto the `program` video track (an
  // orchestrator: no op of its own; revert via the checkpoint). `camera` = the
  // on-screen angle's track id; `energy` = its mean momentary LUFS over the shot.
  'edit.multicam_switch': {
    status: string
    shots: Array<{ start_ms: number; end_ms: number; camera: string; energy: number }>
    switches: number
    tracks: string[]
    min_shot_ms: number
    program_track: string
    span_ms: [number, number]
    /** Which switching mode ran: 'energy' (loudest mic) or 'speaker' (diarized
     *  speaker→camera). In speaker mode also carries the speaker→camera map. */
    mode?: {
      mode: 'speaker' | 'energy'
      diarize_asset?: string
      num_speakers?: number
      speaker_to_cam?: Array<{ speaker: string; camera: string }>
    }
    checkpoint: string
    revert_hint: string
    note?: string
  }
  'verify.checks': RenderReceipt
  'verify.pregate': PregateReport
  'edit.restore': RestoreResult
  'render.storyboard': StoryboardResult
  'project.list': { projects: ProjectEntry[] }
  'library.list': LibraryListResult
  'library.add': { item: LibItem }
  'library.relink': { item: LibItem }
  'library.move': { item: LibItem }
  'library.tag': { item: LibItem }
  'library.favorite': { item: LibItem }
  'assemble.shorts': {
    asset: string
    count: number
    target_ms: number
    aspect: string
    shorts: Array<{
      rank: number
      word_range: [number, number]
      range_ms: [number, number]
      duration_ms: number
      score: number
      reason: string
      title: string
      factors?: Record<string, number>
      reframe: { aspect: string; crop: { x: number; y: number; w: number; h: number } | null }
      has_captions: boolean
    }>
    next: string
    note?: string
  }
}
