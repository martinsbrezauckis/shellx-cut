type VerbArgs = Record<string, unknown>

interface MockCapture {
  captureId: string
  args: VerbArgs
  paused: boolean
  startedAt: number
}

interface MockPreview {
  state: 'idle' | 'starting' | 'ready' | 'paused' | 'hidden' | 'stopped'
  generation: number | null
  leaseNonce: string | null
}

export interface MockRecordingLifecycle {
  handle(name: string, args: VerbArgs): { handled: boolean; value?: unknown }
  snapshot(): { active_capture_ids: string[]; preview: ReturnType<MockRecordingLifecycle['previewStatus']>; rehearsal_handle: string | null }
  previewStatus(): { state: MockPreview['state']; recursion: 'none'; has_frame: boolean; generation: number | null; lease_nonce?: string }
  mediaUrl(url: string): string | null
}

interface MockRecordingLifecycleOptions { startMode: string; jobs: Map<string, unknown> }

const PREVIEW_FRAME_BASE64 = 'Qk06AAAAAAAAADYAAAAoAAAAAQAAAAEAAAABABgAAAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEyW6AA='

// A half-second, solid-colour H.264 fragment generated from an original local
// fixture. It lets the disposable browser rehearsal exercise the real video
// element without requesting cutd or standing in for captured desktop media.
const REHEARSAL_VIDEO_URL = 'data:video/mp4;base64,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'

function previewGeneration(args: VerbArgs): number | null {
  const value = args.expected_generation
  return typeof value === 'number' && Number.isInteger(value) && value >= 1 ? value : null
}

function previewLeaseNonce(args: VerbArgs): string | null {
  const value = args.expected_lease_nonce
  return typeof value === 'string' && /^[0-9a-f]{32}$/.test(value) ? value : null
}

function meter(enabled: boolean) {
  return enabled
    ? {
        state: 'live' as const,
        peak_dbfs: -16,
        rms_dbfs: -28,
        decayed_peak_dbfs: -18,
        sample_age_ms: 42,
        stale: false,
        clipping: false,
        detail: 'Offline fixture level from the admitted mock capture.',
      }
    : {
        state: 'not_requested' as const,
        peak_dbfs: null,
        rms_dbfs: null,
        decayed_peak_dbfs: null,
        sample_age_ms: null,
        stale: false,
        clipping: false,
        detail: 'This input was not requested for the mock capture.',
      }
}

function isMockVideoUrl(url: string): boolean {
  const base = typeof location === 'undefined' ? 'http://localhost' : location.origin
  const path = new URL(url, base).pathname
  return path.startsWith('/api/recording-rehearsal/') || path.startsWith('/api/source/')
}

export function createMockRecordingLifecycle({
  startMode,
  jobs,
}: MockRecordingLifecycleOptions): MockRecordingLifecycle {
  const captures = new Map<string, MockCapture>()
  let sequence = 0
  let rehearsalHandle: string | null = null
  let previewGenerationSequence = 0
  let previewLeaseSequence = 0
  let microphone: { mode: 'system_default' | 'selected'; label?: string; status: 'ready' } = {
    mode: 'system_default',
    status: 'ready',
  }
  let preview: MockPreview = { state: 'idle', generation: null, leaseNonce: null }

  const previewStatus = () => ({
    state: preview.state,
    recursion: 'none' as const,
    has_frame: preview.state === 'ready',
    generation: preview.generation,
    ...(preview.generation !== null && preview.leaseNonce !== null
      ? { lease_nonce: preview.leaseNonce }
      : {}),
  })

  const snapshot = () => ({
    active_capture_ids: [...captures.keys()],
    preview: previewStatus(),
    rehearsal_handle: rehearsalHandle,
  })

  const handle = (name: string, args: VerbArgs): { handled: boolean; value?: unknown } => {
    switch (name) {
      case 'screen_record.doctor':
        return {
          handled: true,
          value: {
            ok: true,
            result: {
              ready: true,
              start_allowed: true,
              cards: [
                { name: 'screen_capture', status: 'ok', detail: 'Offline mock capture lifecycle is ready.' },
                { name: 'ffmpeg', status: 'ok', detail: 'Mock media transport is ready.' },
              ],
              monitors: [
                { id: 'mock-monitor-v1:primary', index: 1, name: 'Mock primary display', width: 1920, height: 1080, primary: true },
                { id: 'mock-monitor-v1:secondary', index: 2, name: 'Mock secondary display', width: 1280, height: 720, primary: false },
              ],
              windows: [{ id: 'mock-window-v1:editor', title: 'Mock source window', app: 'ShellX Cut fixture' }],
              microphones: [{ token: 'mock-microphone-token', label: 'Mock microphone' }],
              microphone_selection: microphone,
              mic_warm: args.warm_mic === true ? { live: true, peak_dbfs: -18, supported: true } : undefined,
              camera: {
                supported: false,
                devices: [],
                detail: 'Camera recording is unavailable in this offline fixture.',
              },
              pause: {
                supported: true,
                incompatible: [],
                detail: 'Pause and resume is available for the selected mock display.',
              },
              quality: { supported: true, output_sizes: ['source', '1080p', '720p'], profiles: ['standard', 'high'] },
            },
          },
        }
      case 'screen_record.microphone_selection':
        microphone = args.mode === 'selected'
          ? { mode: 'selected', label: 'Mock microphone', status: 'ready' }
          : { mode: 'system_default', status: 'ready' }
        return { handled: true, value: { ok: true, result: microphone } }
      case 'screen_record.preview_capability':
        return { handled: true, value: { ok: true, result: { state: 'available', source_selection: 'exact' } } }
      case 'screen_record.preview_status':
        return { handled: true, value: { ok: true, result: previewStatus() } }
      case 'screen_record.preview_start':
        preview = {
          state: 'starting',
          generation: ++previewGenerationSequence,
          leaseNonce: (++previewLeaseSequence).toString(16).padStart(32, '0'),
        }
        return { handled: true, value: { ok: true, result: { action: 'start', status: previewStatus() } } }
      case 'screen_record.preview_frame': {
        if (preview.state === 'starting') preview = { ...preview, state: 'ready' }
        const status = previewStatus()
        return {
          handled: true,
          value: {
            ok: true,
            result: {
              status,
              frame: status.state === 'ready' && status.generation !== null
                ? { mime: 'image/bmp', bytes: 58, generation: status.generation, captured_at_ms: 48, base64: PREVIEW_FRAME_BASE64 }
                : null,
            },
          },
        }
      }
      case 'screen_record.preview_pause':
      case 'screen_record.preview_resume':
      case 'screen_record.preview_hide':
      case 'screen_record.preview_stop': {
        if (
          preview.generation === null
          || preview.leaseNonce === null
          || previewGeneration(args) !== preview.generation
          || previewLeaseNonce(args) !== preview.leaseNonce
        ) {
          return {
            handled: true,
            value: {
              ok: false,
              error: {
                code: 'stale_preview_lease',
                message: 'The mock preview lease is no longer current. Refresh the preview before changing it.',
              },
            },
          }
        }
        const action = name.slice(name.lastIndexOf('_') + 1) as 'pause' | 'resume' | 'hide' | 'stop'
        const releasesLease = action === 'hide' || action === 'stop'
        preview = {
          state: action === 'pause' ? 'paused' : action === 'resume' ? 'starting' : action === 'hide' ? 'hidden' : 'stopped',
          generation: action === 'resume' ? ++previewGenerationSequence : releasesLease ? null : preview.generation,
          leaseNonce: releasesLease ? null : preview.leaseNonce,
        }
        return { handled: true, value: { ok: true, result: { action, status: previewStatus() } } }
      }
      case 'screen_record.rehearsal_start':
        rehearsalHandle = `mock-rehearsal-${Date.now()}`
        return {
          handled: true,
          value: {
            ok: true,
            result: {
              playback_handle: rehearsalHandle,
              playback_url: `/api/recording-rehearsal/${rehearsalHandle}`,
              duration_ms: typeof args.duration_ms === 'number' ? args.duration_ms : 3_000,
              discard_on_start_recording: true,
              note: 'Offline fixture rehearsal is disposable and does not create project media.',
            },
          },
        }
      case 'screen_record.rehearsal_discard': {
        const handle = typeof args.handle === 'string' ? args.handle : null
        const discarded = handle !== null && handle === rehearsalHandle
        if (discarded) rehearsalHandle = null
        return { handled: true, value: { ok: true, result: { handle, discarded, state: discarded ? 'discarded' : 'stopping' } } }
      }
      case 'screen_record.start': {
        if (startMode === 'malformed') return { handled: true, value: { ok: true, result: {} } }
        const captureId = `mock-capture-${String(++sequence).padStart(4, '0')}`
        captures.set(captureId, { captureId, args, paused: false, startedAt: Date.now() })
        return {
          handled: true,
          value: {
            ok: true,
            result: {
              capture_id: captureId,
              out_dir: `/mock/recordings/${captureId}`,
              cadence: {
                schema: 'shellx-record/capture-cadence/1',
                requested: { num: typeof args.fps === 'number' ? Math.round(args.fps) : 30, den: 1 },
                backend_requested: { num: typeof args.fps === 'number' ? Math.round(args.fps) : 30, den: 1 },
              },
              ...(args.pause && typeof args.pause === 'object' ? { pause: { enabled: true } } : {}),
            },
          },
        }
      }
      case 'screen_record.status': {
        const captureId = typeof args.capture_id === 'string' ? args.capture_id : ''
        const capture = captures.get(captureId)
        if (!capture) return { handled: true, value: { ok: false, error: { code: 'not_found', message: 'Mock recording capture is no longer active.' } } }
        return {
          handled: true,
          value: {
            ok: true,
            result: {
              capture_id: capture.captureId,
              ready: !capture.paused,
              terminal: false,
              state: capture.paused ? 'awaiting_first_screen_frame' : 'ready',
              audio_meters: {
                microphone: meter(capture.args.audio !== false),
                system_audio: meter(capture.args.system_audio === true),
              },
              source_lifecycle: {
                state: 'active',
                reason: 'Offline fixture reports an active selected source; it is not native capture proof.',
              },
              controller_placement: {
                state: 'excluded',
                reason: 'Offline fixture controller is not part of a captured desktop.',
              },
            },
          },
        }
      }
      case 'screen_record.pause':
      case 'screen_record.resume': {
        const captureId = typeof args.capture_id === 'string' ? args.capture_id : ''
        const capture = captures.get(captureId)
        if (!capture) return { handled: true, value: { ok: false, error: { code: 'not_found', message: 'Mock recording capture is no longer active.' } } }
        const paused = name === 'screen_record.pause'
        capture.paused = paused
        return {
          handled: true,
          value: {
            ok: true,
            result: {
              action: paused ? 'pause' : 'resume',
              saved: true,
              state: paused ? 'paused' : 'recording',
              logical_media_time_ms: Math.max(0, Date.now() - capture.startedAt),
            },
          },
        }
      }
      case 'screen_record.studio_event': {
        const captureId = typeof args.capture_id === 'string' ? args.capture_id : ''
        const capture = captures.get(captureId)
        if (!capture) return { handled: true, value: { ok: false, error: { code: 'not_found', message: 'Mock recording capture is no longer active.' } } }
        const event = args.event && typeof args.event === 'object' ? args.event as VerbArgs : {}
        return {
          handled: true,
          value: { ok: true, result: { last_event: { ...event, logical_ts: Math.max(0, Date.now() - capture.startedAt) } } },
        }
      }
      case 'screen_record.stop': {
        const captureId = typeof args.capture_id === 'string' ? args.capture_id : ''
        const capture = captures.get(captureId)
        if (!capture) return { handled: true, value: { ok: false, error: { code: 'not_found', message: 'Mock recording capture is no longer active.' } } }
        captures.delete(captureId)
        const base = `/mock/recordings/${captureId}`
        const rawStreams = {
          screen: true,
          camera: false,
          mic: capture.args.audio !== false,
          system: capture.args.system_audio === true,
          studio_events: false,
        }
        if (args.mux_raw === true) {
          return {
            handled: true,
            value: {
              ok: true,
              result: {
                raw_path: `${base}/raw.mp4`,
                raw_has_mic: capture.args.audio !== false,
                raw_has_system: capture.args.system_audio === true,
                raw_streams: rawStreams,
              },
            },
          }
        }
        return { handled: true, value: { ok: true, result: { source: `${base}/source.mp4`, plan: `${base}/plan.json`, raw_streams: rawStreams } } }
      }
      case 'screen_record.polish':
        return { handled: true, value: { ok: true, result: { clip_id: `mock-recording-clip-${sequence}` } } }
      case 'screen_record.export': {
        const jobId = `mock-recording-export-${jobs.size + 1}`
        const path = typeof args.path === 'string' ? args.path : '/mock/exports/recording.mp4'
        jobs.set(jobId, { path, elapsed_ms: 80 })
        return { handled: true, value: { ok: true, result: { job_id: jobId, path } } }
      }
      default:
        return { handled: false }
    }
  }

  return {
    handle,
    snapshot,
    previewStatus,
    mediaUrl: (url: string) => isMockVideoUrl(url) ? REHEARSAL_VIDEO_URL : null,
  }
}
