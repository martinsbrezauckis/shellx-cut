import { folderTail, getStoredOutputDir } from '../../lib/exportDestination'
import { cardLabel, type DoctorReport } from '../../lib/doctor'
import type { SettingsCategoryId } from './settingsModel'

interface SettingsOverviewProps {
  report: DoctorReport | null
  onNavigate: (category: SettingsCategoryId) => void
  onOpenRecording: () => void
}

function statusFor(report: DoctorReport | null, ids: string[]) {
  const cards = report?.cards.filter((card) => ids.includes(card.id)) ?? []
  if (!report || cards.length === 0) return { tone: 'unknown', label: 'Checking…' }
  const missing = cards.find((card) => card.status === 'missing')
  if (missing) return { tone: 'missing', label: `${cardLabel(missing).title} needs setup` }
  if (cards.some((card) => card.status === 'degraded' || card.status === 'unknown')) {
    return { tone: 'degraded', label: 'Needs attention' }
  }
  return { tone: 'ok', label: 'Ready' }
}

export default function SettingsOverview({ report, onNavigate, onOpenRecording }: SettingsOverviewProps) {
  const outputDir = getStoredOutputDir()
  // Keep the overview focused on required capabilities. Optional accelerators
  // and matte tooling remain visible in their detailed categories without
  // making a healthy first-run setup look broken.
  const video = statusFor(report, ['ffmpeg', 'ffprobe'])
  const ai = statusFor(report, ['perception'])
  const apiReady = Boolean(report?.addr)

  const rows = [
    {
      id: 'video',
      label: 'Video editing',
      detail: video.label,
      tone: video.tone,
      action: 'Review video setup',
      run: () => onNavigate('video-performance'),
    },
    {
      id: 'destination',
      label: 'Default save folder',
      detail: outputDir ? folderTail(outputDir) : 'Each project /exports folder',
      tone: 'ok',
      action: 'Change folder',
      run: () => onNavigate('general'),
    },
    {
      id: 'ai',
      label: 'AI & transcription',
      detail: ai.label,
      tone: ai.tone,
      action: 'Review AI setup',
      run: () => onNavigate('ai-transcription'),
    },
    {
      id: 'recording',
      label: 'Recording',
      detail: 'Check sources and permissions in the Record workspace',
      tone: 'unknown',
      action: 'Open Record workspace',
      run: onOpenRecording,
    },
    {
      id: 'agent',
      label: 'Local agent control',
      detail: apiReady ? 'Debug API running · MCP available' : 'Waiting for the local engine',
      tone: apiReady ? 'ok' : 'unknown',
      action: 'View agent control',
      run: () => onNavigate('agent-control'),
    },
  ]

  return (
    <section className="settings-section" aria-labelledby="settings-overview-title" data-cut-settings-overview>
      <div className="settings-section-head">
        <p className="settings-eyebrow">At a glance</p>
        <h3 id="settings-overview-title">Your editing setup</h3>
        <p>Start with the item that needs attention. Advanced details stay inside each destination.</p>
      </div>
      <div className="settings-overview-list">
        {rows.map((row) => (
          <div className="settings-overview-row" key={row.id} data-cut-settings-overview-row={row.id}>
            <span className={`settings-status settings-status--${row.tone}`} aria-hidden="true" />
            <span className="settings-overview-copy">
              <strong>{row.label}</strong>
              <span>{row.detail}</span>
            </span>
            <button type="button" className="env-btn env-btn--ghost" data-cut-settings-overview-action={row.id} onClick={row.run}>
              {row.action}
            </button>
          </div>
        ))}
      </div>
    </section>
  )
}
