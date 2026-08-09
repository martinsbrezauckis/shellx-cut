// Environment/index.tsx — the start wizard + Settings>Environment surface.
// Role: two presentations of the SAME EnvCards grid:
//   • StartWizard — a first-run MODAL that surfaces when the doctor reports a
//     missing/degraded ESSENTIAL (ffmpeg). Dismissible ("continue without"),
//     never blocks the editor for non-essentials. Family modal chrome.
//   • EnvironmentPanel — a settings DRAWER reachable any time from the status-
//     bar environment chip, rendering the same cards.
// Both are RELAY-DRIVABLE per public contract invariant 1 + the 100%-surface rule: an
// agent opens them with ui.open{panel:"wizard"|"environment"} (App.tsx routes
// the ui_command), and their open/closed state is reported in ui.state. The
// download action goes through system.fetch_tool (handled inside EnvCards).
// Callers: App.tsx. Deps: EnvCards, lib/doctor, environment.css.

import { useEffect, useState } from 'react'
import type { DoctorReport } from '../../lib/doctor'
import { openCutManual } from '../../lib/manual'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import EnvCards from './EnvCards'
import SettingsCategoryContent from './SettingsCategoryContent'
import SettingsShell from './SettingsShell'
import type { SettingsCategoryId } from './settingsModel'
import './environment.css'

interface CommonProps {
  report: DoctorReport | null
  /** Re-fetch the doctor (after a fetch action / manual refresh). */
  onRefresh: () => Promise<DoctorReport | null>
  /** Close this surface. */
  onClose: () => void
}

interface EnvironmentPanelProps extends CommonProps {
  onOpenRecording?: () => void
  onOpenAssets?: () => void
  hasProject?: boolean
  projectSession?: number
  initialCategory?: SettingsCategoryId
}

function SetupPath({ essentialMissing }: { essentialMissing: boolean }) {
  return (
    <section className="env-setup-path" data-cut-setup-path>
      <div className="env-setup-step" data-cut-setup-step="ffmpeg">
        <span className={`env-setup-num${essentialMissing ? ' env-setup-num--missing' : ''}`}>1</span>
        <div>
          <strong>Video processing</strong>
          <span>Install the required video tools so preview, import, and export work.</span>
        </div>
      </div>
      <div className="env-setup-step" data-cut-setup-step="media">
        <span className="env-setup-num">2</span>
        <div>
          <strong>Add media</strong>
          <span>Import a clip, then drag it to the base timeline or add it at the playhead.</span>
        </div>
      </div>
      <div className="env-setup-step" data-cut-setup-step="agent">
        <span className="env-setup-num">3</span>
        <div>
          <strong>Optional agent</strong>
          <span>Connect Codex, Claude, or Grok when you want Generate or chat workflows.</span>
        </div>
      </div>
      <button
        type="button"
        className="env-btn env-btn--ghost env-setup-manual"
        data-cut-setup-manual
        onClick={() => openCutManual('cut.preview.ffmpeg_setup')}
        title="Open the setup guide in the online manual"
      >
        Setup guide
      </button>
    </section>
  )
}

// ---------------------------------------------------------------------------
// Start wizard (first-run modal)
// ---------------------------------------------------------------------------

/** First-run wizard. Shown when an essential dep is missing. Dismissible. */
export function StartWizard({ report, onRefresh, onClose }: CommonProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose, Boolean(report))

  if (!report) return null
  const essentialMissing = !report.essential_ok

  return (
    <div className="env-scrim" data-cut-wizard-scrim onMouseDown={overlay.onScrimMouseDown}>
      <div
        ref={overlay.dialogRef}
        className="env-modal"
        data-cut-wizard
        data-cut-wizard-open="true"
        data-cut-blocking-overlay
        data-cut-wizard-essential-ok={report.essential_ok}
        role="dialog"
        aria-modal="true"
        aria-label="ShellX Cut setup"
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
      >
        <header className="env-modal-head">
          <div>
            <h2 className="env-modal-title">Welcome to ShellX Cut</h2>
            <p className="env-modal-sub">
              {essentialMissing
                ? 'One essential tool is missing. Install it to start editing — the rest is optional.'
                : 'Your environment is ready. Here is what ShellX Cut found.'}
            </p>
          </div>
          <button className="env-btn env-btn--ghost" data-cut-wizard-refresh onClick={onRefresh} title="Check this machine again">
            Re-scan
          </button>
        </header>

        <div className="env-modal-body">
          <SetupPath essentialMissing={essentialMissing} />
          <EnvCards report={report} onChanged={onRefresh} groups={['tools']} showMeta={false} />
        </div>

        <footer className="env-modal-foot">
          {/* never blocks the editor — non-essentials are skippable */}
          <button className="env-btn env-btn--secondary" data-cut-wizard-dismiss onClick={onClose}>
            {essentialMissing ? 'Continue without' : 'Start editing'}
          </button>
        </footer>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Environment panel (settings drawer)
// ---------------------------------------------------------------------------

/** Settings>Environment drawer — the same cards, reachable any time. */
export function EnvironmentPanel({
  report,
  onRefresh,
  onClose,
  onOpenRecording = onClose,
  onOpenAssets = onClose,
  hasProject = false,
  projectSession = 0,
  initialCategory = 'overview',
}: EnvironmentPanelProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const [active, setActive] = useState<SettingsCategoryId>(initialCategory)
  const [query, setQuery] = useState('')
  // ui.open can navigate between Settings destinations while this drawer is
  // already mounted. Keep the internal category synchronized with that route.
  useEffect(() => setActive(initialCategory), [initialCategory])

  return (
    <div className="env-scrim" data-cut-environment-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="env-drawer env-settings-shell"
        data-cut-environment
        data-cut-environment-open="true"
        data-cut-blocking-overlay
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
      >
        <SettingsShell
          active={active}
          onActive={setActive}
          query={query}
          onQuery={setQuery}
          onRefresh={onRefresh}
          onClose={onClose}
        >
          <SettingsCategoryContent
            active={active}
            report={report}
            onRefresh={onRefresh}
            onNavigate={setActive}
            onOpenRecording={onOpenRecording}
            onOpenAssets={onOpenAssets}
            hasProject={hasProject}
            projectSession={projectSession}
          />
        </SettingsShell>
      </aside>
    </div>
  )
}
