import type { ReactNode } from 'react'
import ThemeToggle from '../../components/ThemeToggle'
import { FIXED_KEY_ACTIONS, displayBinding } from '../../lib/keymap'
import type { DoctorReport } from '../../lib/doctor'
import About from './About'
import AgentControl from './AgentControl'
import EnvCards, { type EnvCardGroup } from './EnvCards'
import ExportDestination from './ExportDestination'
import KeymapEditor from './KeymapEditor'
import SettingsOverview from './SettingsOverview'
import UpdateNetworkSettings from './UpdateNetworkSettings'
import type { SettingsCategoryId } from './settingsModel'
import './settings-sections.css'

interface SettingsCategoryContentProps {
  active: SettingsCategoryId
  report: DoctorReport | null
  onRefresh: () => void
  onNavigate: (category: SettingsCategoryId) => void
  onOpenRecording: () => void
}

function Section({
  id,
  eyebrow,
  title,
  description,
  children,
}: {
  id: SettingsCategoryId
  eyebrow: string
  title: string
  description: string
  children: ReactNode
}) {
  return (
    <section className="settings-section" aria-labelledby={`settings-${id}-title`} data-cut-settings-section={id}>
      <div className="settings-section-head">
        <p className="settings-eyebrow">{eyebrow}</p>
        <h3 id={`settings-${id}-title`}>{title}</h3>
        <p>{description}</p>
      </div>
      {children}
    </section>
  )
}

function CardGroups({
  report,
  onRefresh,
  groups,
}: {
  report: DoctorReport | null
  onRefresh: () => void
  groups: readonly EnvCardGroup[]
}) {
  return report ? (
    <EnvCards report={report} onChanged={onRefresh} groups={groups} showMeta={false} />
  ) : (
    <div className="env-empty">Checking this machine…</div>
  )
}

export default function SettingsCategoryContent({
  active,
  report,
  onRefresh,
  onNavigate,
  onOpenRecording,
}: SettingsCategoryContentProps) {
  switch (active) {
    case 'overview':
      return <SettingsOverview report={report} onNavigate={onNavigate} onOpenRecording={onOpenRecording} />
    case 'general':
      return (
        <Section id={active} eyebrow="Basics" title="General" description="Choose where finished work goes and how the interface looks.">
          <ExportDestination />
          <section className="env-appearance"><ThemeToggle variant="row" /></section>
        </Section>
      )
    case 'editing':
      return (
        <Section id={active} eyebrow="Editor" title="Editing" description="Customise editor keys without changing fixed app or recording controls.">
          <KeymapEditor />
        </Section>
      )
    case 'video-performance':
      return (
        <Section id={active} eyebrow="Local tools" title="Video & performance" description="Check the tools that power import, preview and export on this machine.">
          <CardGroups report={report} onRefresh={onRefresh} groups={['tools']} />
        </Section>
      )
    case 'ai-transcription':
      return (
        <Section id={active} eyebrow="Optional local AI" title="AI & transcription" description="Set up speech analysis, captions and background removal only when you need them.">
          <CardGroups report={report} onRefresh={onRefresh} groups={['perception', 'matte']} />
        </Section>
      )
    case 'recording': {
      const recordingKeys = FIXED_KEY_ACTIONS.filter((action) => action.group === 'recording')
      return (
        <Section id={active} eyebrow="Capture" title="Recording" description="Permissions and sources are checked live inside the Record workspace.">
          <div className="settings-callout">
            <div>
              <strong>Run the live readiness check</strong>
              <p>Open Record to verify screens, windows, microphone and system-audio support on this machine.</p>
            </div>
            <button type="button" className="env-btn env-btn--primary" data-cut-settings-open-recording onClick={onOpenRecording}>
              Open Record workspace
            </button>
          </div>
          <details className="settings-advanced" data-cut-settings-recording-keys>
            <summary data-cut-settings-recording-keys-toggle>Fixed recording shortcuts ({recordingKeys.length})</summary>
            <dl>
              {recordingKeys.map((action) => (
                <div key={action.id}><dt>{action.label}</dt><dd><kbd>{displayBinding(action.binding)}</kbd></dd></div>
              ))}
            </dl>
          </details>
        </Section>
      )
    }
    case 'services-integrations':
      return (
        <Section id={active} eyebrow="Optional connections" title="Services & integrations" description="Connect only the external speech and review tools your workflow uses.">
          <CardGroups report={report} onRefresh={onRefresh} groups={['services', 'judges']} />
        </Section>
      )
    case 'agent-control':
      return (
        <Section id={active} eyebrow="Local control" title="Agent control" description="CALI or another agent can control Cut through the Debug API or MCP without a second authority layer.">
          <AgentControl report={report} />
        </Section>
      )
    case 'storage-privacy':
      return (
        <Section id={active} eyebrow="Local-first" title="Storage & privacy" description="Understand what stays local before enabling optional external workflows.">
          <div className="settings-privacy-note">
            <strong>Projects and edit history stay on this machine.</strong>
            <p>Media leaves the machine only when you deliberately use an external generation, dubbing, review, or agent workflow.</p>
          </div>
          <UpdateNetworkSettings />
          <CardGroups report={report} onRefresh={onRefresh} groups={['disk']} />
        </Section>
      )
    case 'about':
      return (
        <Section id={active} eyebrow="Application" title="About ShellX Cut" description="Version, update policy and project links.">
          <About report={report} />
          {report && (
            <div className="env-meta" data-cut-env-meta>
              {report.os}/{report.arch} · scanned {new Date(report.scanned_at).toLocaleTimeString()}
            </div>
          )}
        </Section>
      )
  }
}
