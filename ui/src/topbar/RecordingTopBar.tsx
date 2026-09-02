import ThemeToggle from '../components/ThemeToggle'
import { envHealthLevel, type DoctorReport } from '../lib/doctor'
import { BrandMark, Icon } from '../icons'

interface RecordingTopBarProps {
  projectName: string | null
  doctor: DoctorReport | null
  manualOpen: boolean
  onBackToEdit: () => void
  onOpenSetup: () => void
  onOpenManual: () => void
}

/** Focused chrome for Recording Studio. Editing and export tools deliberately
 * stay out of this mode because they cannot affect an active capture. */
export default function RecordingTopBar({
  projectName,
  doctor,
  manualOpen,
  onBackToEdit,
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
            · {projectName ? `${projectName}.cutproj` : 'new recording'}
          </span>
        </span>
      </div>

      <span className="tb-recording-mode" data-cut-recording-mode-active>
        <span aria-hidden="true" />
        Recording Studio
      </span>

      <span className="tb-spacer" />

      <button
        type="button"
        className="tb-btn tb-btn--secondary tb-recording-back"
        data-cut-action="record-back-edit"
        data-cut-record-back-edit
        onClick={onBackToEdit}
      >
        <Icon name="collapseLeft" size={14} />
        Back to Edit
      </button>
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
