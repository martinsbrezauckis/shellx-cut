import type { JobView } from '../topbar/useTopbarJobs'

const LABELS: Record<string, string> = {
  render: 'Rendering video',
  render_queue: 'Render queue',
  proxy: 'Building proxy',
  proxy_check: 'Checking proxies',
  title_remake: 'Drawing titles again',
  probe: 'Checking media',
  transcribe: 'Transcribing',
  perception: 'Analyzing media',
  judge: 'Reviewing output',
  import_chain: 'Preparing media',
  screen_record_export: 'Exporting recording',
  enrich: 'Analyzing media',
  diarize: 'Labelling speakers',
  media_intelligence: 'Preparing search',
  recipe: 'Running recipe',
  autopilot: 'Running autopilot',
  bundle: 'Making social versions',
  asset_generate: 'Generating media',
  screen_record_copy_raw: 'Copying recording',
  screen_record_polish: 'Polishing recording',
  recording_recovery: 'Preparing interrupted recording',
  motion_import_apply: 'Importing motion',
  fetch_tool: 'Downloading tool',
  setup_perception: 'Setting up captions',
  setup_matte: 'Setting up background removal',
  cache_purge: 'Cleaning cache',
  cache_rebuild: 'Rebuilding cache',
  portable_package: 'Packaging project copy',
  'verify-rerun': 'Checking output',
}

/** What a failed job was doing, as a short noun phrase a person recognises.
 * Every job kind the engine creates has an entry; an unknown kind falls back to
 * its words so the row never shows an internal id as the only text. */
const TASK_NAMES: Record<string, string> = {
  render: 'Render',
  render_queue: 'Render queue',
  'reframe-qc': 'Reframe check',
  bundle: 'Social versions',
  autopilot: 'Autopilot',
  proxy: 'Proxy build',
  proxy_check: 'Proxy check',
  title_remake: 'Title drawing',
  probe: 'Media check',
  transcribe: 'Transcription',
  perception: 'Media analysis',
  enrich: 'Media analysis',
  judge: 'Output review',
  import_chain: 'Media import',
  media_intelligence: 'Search preparation',
  diarize: 'Speaker labelling',
  recipe: 'Recipe run',
  asset_generate: 'Generation',
  screen_record_export: 'Recording export',
  screen_record_copy_raw: 'Recording copy',
  screen_record_polish: 'Recording polish',
  recording_recovery: 'Recording preparation',
  motion_import_apply: 'Motion import',
  fetch_tool: 'Tool download',
  setup_perception: 'Captions setup',
  setup_matte: 'Background removal setup',
  cache_purge: 'Cache cleanup',
  cache_rebuild: 'Cache rebuild',
  portable_package: 'Project copy',
  'verify-rerun': 'Output check',
}

export function jobTaskName(kind: string): string {
  if (kind === 'reframe' || kind === 'reframe-direct' || (kind.startsWith('reframe-') && kind !== 'reframe-qc')) return 'Reframe'
  const known = TASK_NAMES[kind]
  if (known) return known
  const words = kind.replaceAll(/[-_]+/g, ' ').trim()
  return words ? `${words[0].toUpperCase()}${words.slice(1)}` : 'Background task'
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
