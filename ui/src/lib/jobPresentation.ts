import type { JobView } from '../topbar/useTopbarJobs'

const LABELS: Record<string, string> = {
  render: 'Rendering video',
  render_queue: 'Render queue',
  proxy: 'Building proxy',
  probe: 'Checking media',
  transcribe: 'Transcribing',
  perception: 'Analyzing media',
  judge: 'Reviewing output',
  import_chain: 'Preparing media',
  screen_record_export: 'Exporting recording',
}

export function activeJobLabel(kind: string): string {
  if (kind === 'reframe' || kind.startsWith('reframe-')) return 'Reframing video'
  const known = LABELS[kind]
  if (known) return known
  const words = kind.replaceAll(/[-_]+/g, ' ').trim()
  return words ? `${words[0].toUpperCase()}${words.slice(1)}` : 'Background task'
}

export function activeJobProgress(job: JobView): string {
  if (job.state === 'queued') {
    const queue = job.queue
    if (!queue) return 'waiting to start'
    const resource = queue.resource === 'render' || queue.resource === 'render_queue'
      ? 'render capacity'
      : queue.resource === 'analysis' || queue.resource === 'enrich'
        ? 'analysis capacity'
        : queue.resource === 'proxy'
          ? 'proxy capacity'
          : queue.resource === 'asset_generate'
            ? 'generation capacity'
            : `${queue.resource.replaceAll(/[._-]+/g, ' ')} capacity`
    const slots = Math.max(1, queue.max_running)
    const position = queue.position && queue.waiting
      ? `${queue.position} of ${queue.waiting} waiting for`
      : 'waiting for'
    return `${position} ${resource} · ${slots} slot${slots === 1 ? '' : 's'}`
  }
  const pct = Math.round(Math.min(1, Math.max(0, job.progress)) * 100)
  const message = job.message?.trim()
  const waiting = job.waiting_on
    ? ` · waiting on ${activeJobLabel(job.waiting_on.kind).toLowerCase()} ${job.waiting_on.job_id}`
    : ''
  return message ? `${pct}% · ${message}${waiting}` : `${pct}% complete${waiting}`
}
