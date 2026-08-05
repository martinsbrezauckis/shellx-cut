// Transient linked-Motion projection returned by project.state.

export interface MotionEffectKeyingSummary {
  keyColor?: string | null
  spillSuppression?: number | null
  matteCleanup: boolean
}

export interface MotionEffectRotoSummary {
  frameCount: number
  tracked: boolean
  model?: 'translation' | 'similarity' | 'homography' | null
}

export interface MotionEffectLayerSummary {
  id?: string | null
  name?: string | null
  type?: 'image' | 'video' | string | null
  keying?: MotionEffectKeyingSummary
  roto?: MotionEffectRotoSummary
}

export interface MotionEffectsSummary {
  schema: 'shellx-cut/motion-effects-summary@1'
  available: boolean
  ownership: 'motion'
  editableInCut: false
  reason?: 'unreadable-motion-document'
  keyedLayerCount?: number
  rotoLayerCount?: number
  trackedRotoLayerCount?: number
  truncated?: boolean
  layers?: MotionEffectLayerSummary[]
}

export interface MotionPackageRenderLineage {
  schema: 'shellx-motion/package-render-lineage@1'
  manifestSha256: string
  motionSha256: string
  adapterId?: 'adapter.gltf'
  sourceSha256?: string
  normalizedSourceSha256?: string
  loweringReceiptSha256?: string
}

export interface MotionReceiptAttestationProof {
  id: string
  operation: string
  status: 'passed' | 'warning'
  sha256?: string
}

export interface MotionCurrentPackageLineage {
  schema: 'shellx-cut/current-motion-package-lineage@1'
  status: 'exact' | 'changed' | 'unavailable'
  lineage: MotionPackageRenderLineage | null
  changedFields: ('manifestSha256' | 'motionSha256' | 'adapterId' | 'sourceSha256' | 'normalizedSourceSha256' | 'loweringReceiptSha256')[]
  reason: 'package-dir-not-provided' | 'artifact-lineage-unavailable' | 'package-unreadable' | null
}

/** Immutable, path-free proof captured when a rendered Motion artifact enters Cut. */
export interface MotionImportAttestation {
  schema: 'shellx-cut/motion-import-attestation@1'
  status: 'verified' | 'legacy-unverified'
  artifactHandleId: string
  artifactOperationHash: string
  artifactDescriptorSha256: string
  packageLineage: MotionPackageRenderLineage | null
  currentPackage?: MotionCurrentPackageLineage
  renderReceipt: MotionReceiptAttestationProof
  connectorReceipt: MotionReceiptAttestationProof | null
  cutPlanReceipt: MotionReceiptAttestationProof | null
}

export interface MotionClipLink {
  schema: 'shellx-cut/motion-link@1'
  clipId: string
  assetId: string
  motionSourceId: string
  packageId: string
  motionId: string
  sourceRevision: string
  sourceRevisionKind?: 'cut-import-plan' | 'motion-package'
  sourcePath?: string | null
  planPath: string
  mode: 'rendered_media' | 'editable_lowering'
  state: 'linked-current' | 'source-dirty' | 'rendering' | 'missing-source' | 'render-error' | 'incompatible' | 'relinking'
  render: { path: string; sha256: string; byteLength: number; artifactHandleId: string | null }
  fallbackPath: string
  lastReceiptId?: string | null
  lastReceiptPath?: string | null
  originAttestation?: MotionImportAttestation | null
  editableInCut?: string[]
  opaqueInCut?: string[]
  availability?: {
    source: boolean
    plan: boolean
    render: boolean
    fallback: boolean
    canRefresh: boolean
    canRelink: boolean
    canEditInMotion: boolean
  }
  effects?: MotionEffectsSummary
  tracking?: {
    analysisId?: string | null
    assetId?: string | null
    mode?: 'point' | 'planar'
    model?: 'translation' | 'similarity' | 'homography'
    lifecycleState?: string | null
    attachedLayerId?: string | null
    fidelity?: string | null
  }
}
