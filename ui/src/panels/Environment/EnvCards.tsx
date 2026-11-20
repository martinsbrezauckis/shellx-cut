// EnvCards.tsx — the shared capability-card grid for the start wizard and
// Settings > Environment. This file owns grouping/orchestration; each row owns
// its own actions, progress, service detail, STT picker, and advanced facts.

import { groupCards, type DoctorCard, type DoctorReport } from '../../lib/doctor'
import EnvCardRow from './EnvCardRow'
import { toolsForCard } from './NeededFor'
import './environment.css'
import './ffmpeg-setup.css'
import './needed-for.css'

export type EnvCardGroup = 'tools' | 'perception' | 'matte' | 'services' | 'judges' | 'disk'
const ALL_GROUPS: readonly EnvCardGroup[] = ['tools', 'perception', 'matte', 'services', 'judges', 'disk']

/** A labeled group of cards. */
function CardGroup({
  label,
  cards,
  os,
  arch,
  onChanged,
  report,
}: {
  label: string
  cards: DoctorCard[]
  os: string
  arch: string
  onChanged: () => void
  /** For each card's "Needed for" line (the doctor's requirements section). */
  report?: Pick<DoctorReport, 'requirements'>
}) {
  if (cards.length === 0) return null
  return (
    <section className="env-group" data-cut-env-group={label.toLowerCase()}>
      <h3 className="env-group-title">{label}</h3>
      {cards.map((c) => (
        <EnvCardRow key={c.id} card={c} os={os} arch={arch} onChanged={onChanged} requirements={report ? toolsForCard(report, c.id) : []} />
      ))}
    </section>
  )
}

/** The shared card grid. `report` is the doctor result; `onChanged` re-fetches
 *  it after a fetch action (the parent owns the doctor state). */
export default function EnvCards({
  report,
  onChanged,
  groups = ALL_GROUPS,
  showMeta = false,
}: {
  report: DoctorReport
  onChanged: () => void
  groups?: readonly EnvCardGroup[]
  showMeta?: boolean
}) {
  const g = groupCards(report.cards)
  const visible = new Set(groups)

  return (
    <div className="env-cards" data-cut-env-cards data-cut-env-essential-ok={report.essential_ok} data-cut-env-scanned-at={report.scanned_at}>
      {visible.has('tools') && <CardGroup label="Video processing" cards={g.tools} os={report.os} arch={report.arch} onChanged={onChanged} report={report} />}
      {visible.has('perception') && <CardGroup label="Captions and transcription" cards={g.perception} os={report.os} arch={report.arch} onChanged={onChanged} report={report} />}
      {visible.has('matte') && <CardGroup label="Background removal" cards={g.matte} os={report.os} arch={report.arch} onChanged={onChanged} report={report} />}
      {visible.has('services') && <CardGroup label="Voice & speaker services" cards={g.services} os={report.os} arch={report.arch} onChanged={onChanged} report={report} />}
      {visible.has('judges') && <CardGroup label="Delivery review" cards={g.judges} os={report.os} arch={report.arch} onChanged={onChanged} report={report} />}
      {visible.has('disk') && <CardGroup label="Storage" cards={g.disk} os={report.os} arch={report.arch} onChanged={onChanged} report={report} />}
      {showMeta && (
        <div className="env-meta" data-cut-env-meta>
          {report.os}/{report.arch} · cut {report.app_version} · {report.addr ?? 'local'} · scanned{' '}
          {new Date(report.scanned_at).toLocaleTimeString()}
        </div>
      )}
    </div>
  )
}
