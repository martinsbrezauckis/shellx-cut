import type { SequenceIndexClipRow, SequenceIndexMarkerRow } from '../../lib/client'

function csvCell(value: string | number | boolean | undefined): string {
  const raw = value == null ? '' : String(value)
  const text = typeof value === 'string' && /^[\t\r\n ]*[=+@-]/.test(raw) ? `'${raw}` : raw
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text
}

export function sequenceIndexCsv(rows: Array<SequenceIndexClipRow | SequenceIndexMarkerRow>): string {
  const columns = [
    'sequence', 'kind', 'label', 'at_ms', 'end_ms', 'track', 'track_kind',
    'clip_kind', 'asset', 'offline', 'effects', 'issues', 'track_visible',
    'track_locked', 'track_muted', 'marker_note', 'marker_color',
  ]
  const lines = rows.map((row) => {
    const clip = row.kind === 'clip' ? row : null
    const marker = row.kind === 'marker' ? row : null
    return [
      row.sequence_name,
      row.kind,
      row.label,
      row.at_ms,
      row.end_ms,
      clip?.track_id,
      clip?.track_kind,
      clip?.clip_kind,
      clip?.asset,
      clip?.offline,
      clip?.effects?.join('|'),
      clip?.issues?.join('|'),
      clip?.track_visible,
      clip?.track_locked,
      clip?.track_muted,
      marker?.note,
      marker?.color,
    ].map(csvCell).join(',')
  })
  return `${columns.join(',')}\r\n${lines.join('\r\n')}\r\n`
}
