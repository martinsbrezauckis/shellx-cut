import type { ScreenRecordLiveFrameResult } from '../../lib/clientResults'

const MAX_FRAME_BYTES = 4 * 1024 * 1024

export interface RecordingLiveFramePresentation {
  captureId: string
  state: ScreenRecordLiveFrameResult['state'] | 'connecting' | 'read_error'
  detail: string
  frameUrl: string | null
  generation: number | null
  capturedAtMs: number | null
}

export function awaitingRecordingLiveFrame(captureId: string): RecordingLiveFramePresentation {
  return { captureId, state: 'connecting', detail: 'Waiting for frames from this recording.', frameUrl: null, generation: null, capturedAtMs: null }
}

/** Admit pixels only when the native owner identifies this capture and current generation. */
export function recordingLiveFramePresentation(
  captureId: string,
  result: ScreenRecordLiveFrameResult,
): RecordingLiveFramePresentation {
  if (result.capture_id !== captureId || !Number.isSafeInteger(result.generation) || result.generation < 0) {
    return { captureId, state: 'unavailable', detail: 'The recording preview returned a different capture.', frameUrl: null, generation: null, capturedAtMs: null }
  }
  const frame = result.frame
  const validFrame = result.state === 'ready'
    && frame !== null
    && frame.generation === result.generation
    && frame.mime === 'image/bmp'
    && Number.isSafeInteger(frame.bytes)
    && frame.bytes > 0 && frame.bytes <= MAX_FRAME_BYTES
    && typeof frame.base64 === 'string' && frame.base64.length > 0
    && frame.base64.length <= Math.ceil(MAX_FRAME_BYTES / 3) * 4
    && Number.isSafeInteger(frame.captured_at_ms) && frame.captured_at_ms >= 0
    && result.frame_age_ms !== null && Number.isFinite(result.frame_age_ms)
    && result.frame_age_ms >= 0 && result.frame_age_ms <= 5_000
  if (validFrame) {
    return { captureId, state: 'ready', detail: 'Live pixels from this recording.',
      frameUrl: `data:image/bmp;base64,${frame.base64}`, generation: result.generation, capturedAtMs: frame.captured_at_ms }
  }
  const detail = result.reason?.trim() || {
    awaiting_source: 'Waiting for the selected source.',
    awaiting_frame: 'Waiting for the first recorded frame.',
    ready: 'A current recording frame is not available yet.',
    stale: 'The last recorded frame is stale. Waiting for a current frame.',
    unavailable: 'Live recording preview is unavailable.',
    terminal: 'This recording has ended.',
  }[result.state]
  return { captureId, state: result.state, detail, frameUrl: null, generation: result.generation, capturedAtMs: null }
}

/** An older reply can never restore pixels after a newer physical generation or frame. */
export function acceptRecordingLiveFrame(
  previous: RecordingLiveFramePresentation | null,
  next: RecordingLiveFramePresentation,
): RecordingLiveFramePresentation {
  if (!previous || previous.captureId !== next.captureId) return next
  if (next.generation !== null && previous.generation !== null) {
    if (next.generation < previous.generation) return previous
    if (next.generation === previous.generation && next.capturedAtMs !== null
      && previous.capturedAtMs !== null && next.capturedAtMs < previous.capturedAtMs) return previous
  }
  return next
}
