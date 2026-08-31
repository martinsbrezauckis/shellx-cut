import type { Project } from '../../lib/client'
import type { DoctorReport } from '../../lib/doctor'

export interface InspectorProps {
  project: Project | null
  /** Ephemeral project.state revision passed separately from durable Project. */
  projectRevision?: string | null
  /** The selected clip id (Timeline selection). */
  selectedClipId: string | null
  /** Live playhead (timeline ms) — the start anchor for a placed caption card. */
  playheadMs?: number
  /** Seek a selected caption to its timeline-owned range start. */
  onSeek: (atMs: number) => void
  /** Installed capability truth used to explain or enable environment-dependent actions. */
  doctor: DoctorReport | null
}
