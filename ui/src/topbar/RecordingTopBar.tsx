import ThemeToggle from '../components/ThemeToggle'
import { envHealthLevel, type DoctorReport } from '../lib/doctor'
import type { Project } from '../lib/client'
import { BrandMark, Icon } from '../icons'
import type { WorkspaceMode } from '../layout/useLayout'
import { WorkspaceModeTabs } from './WorkspaceModeTabs'
import SequenceSwitcher from './SequenceSwitcher'

interface RecordingTopBarProps {
  project: Project | null
  onProjectChanged?: () => void
  onSequenceChanged?: () => void
  doctor: DoctorReport | null
  manualOpen: boolean
  onMode: (mode: WorkspaceMode) => void
  backDisabled?: boolean
  backReason?: string | null
  onOpenSetup: () => void
  onOpenManual: () => void
}

/** Focused chrome for Recording Studio. Editing and export tools deliberately
 * stay out of this mode because they cannot affect an active capture. */
export default function RecordingTopBar({
  project,
  onProjectChanged,
  onSequenceChanged,
  doctor,
  manualOpen,
  onMode,
  backDisabled = false,
  backReason = null,
  onOpenSetup,
  onOpenManual,
}: RecordingTopBarProps) {
  const health = envHealthLevel(doctor)

  return (
    <header
      className="tb tb--recording"
      data-panel="topbar"
      data-cut-panel="topbar"
      data-cut-recording-chrome
    >
      <div className="tb-brand">
        <BrandMark />
        <span className="tb-title">
          <span className="tb-wordmark">ShellX CUT</span>
          <span className="tb-proj" data-cut-project>
            · {project ? `${project.name}.cutproj` : 'new recording'}
          </span>
        </span>
      </div>

      {project && (
        <SequenceSwitcher
          project={project}
          onProjectChanged={onProjectChanged}
          onSequenceChanged={onSequenceChanged}
          disabled={backDisabled}
        />
      )}

      <WorkspaceModeTabs
        mode="record"
        onMode={onMode}
        recordingExitAdmission={{ blocked: backDisabled, reason: backReason }}
      />

      <span className="tb-spacer" />

      {backDisabled && backReason && (
        <span id="cut-record-back-reason" className="tb-recording-back-reason" data-cut-record-back-reason role="status" title={backReason}>
          {backReason}
        </span>
      )}
      <button
        type="button"
        className="tb-btn tb-btn--secondary tb-nav"
        data-cut-setup-btn
        data-cut-settings-btn
        data-cut-setup-health={health}
        aria-label="Recording settings"
        title="Recording destinations, permissions, and tools"
        onClick={onOpenSetup}
      >
        <Icon name="settings" size={16} tone="brand" />
        <span className="tb-nav-label">Settings</span>
      </button>
      <button
        type="button"
        className="tb-btn tb-btn--secondary tb-nav"
        data-cut-manual-link
        aria-label="Recording manual"
        aria-pressed={manualOpen}
        title={manualOpen ? 'Close the bundled manual' : 'Open the Recording Studio manual'}
        onClick={onOpenManual}
      >
        <Icon name="manual" size={16} tone="brand" />
        <span className="tb-nav-label">Manual</span>
      </button>
      <ThemeToggle variant="icon" />
    </header>
  )
}
