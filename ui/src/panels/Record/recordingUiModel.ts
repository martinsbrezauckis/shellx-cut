export interface RecordCard {
  name: string
  status: string
  detail: string
}

const RECORD_CARD_LABELS: Record<string, string> = {
  ffmpeg: 'Media tools',
  screen_capture: 'Screen capture',
  video_encoder: 'Video recording',
  system_audio: 'System audio',
  input_hook: 'Pointer and keys',
  gstreamer: 'Screen recording',
  wayland_input: 'Pointer and keyboard access',
}

/** The check's name in sentence case ("Pointer and keys"); an unlisted id gets its words and a capital first letter. */
export function recordCardLabel(name: string): string {
  if (Object.hasOwn(RECORD_CARD_LABELS, name)) return RECORD_CARD_LABELS[name]
  const words = name.replaceAll('_', ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

export const DUR_PRESETS: { label: string; ms: number | null }[] = [
  { label: 'No limit', ms: null },
  { label: '10s', ms: 10_000 },
  { label: '30s', ms: 30_000 },
  { label: '1 min', ms: 60_000 },
  { label: '2 min', ms: 120_000 },
]

export function fmtElapsed(totalSec: number): string {
  const m = Math.floor(totalSec / 60)
  const s = totalSec % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}

export function outputFileLabel(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) || 'Chosen file'
}

export function failureReason(error: unknown): string {
  if (error instanceof TypeError) return 'server unreachable'
  return error instanceof Error && error.message ? error.message : 'server unreachable'
}

/** These variants have no distinct error code: Doctor exposes only a card id,
 * status and free-text detail. Translate the producer's instruction, not a loose
 * keyword such as "permission" (which could give advice for the wrong device).
 * Keep replacements idempotent: a session notice is presented again by Record.
 */
const RECORDING_INSTRUCTIONS: readonly (readonly [RegExp, string])[] = [
  [/ffmpeg not found — install it or set SHELLX_RECORD_FFMPEG to its path/g,
    'Video tools are missing. Install FFmpeg, or set SHELLX_RECORD_FFMPEG to its location.'],
  [/no graphical session \(WAYLAND_DISPLAY\/DISPLAY unset\) — at a login greeter or headless SSH the ScreenCast portal has no backend; log into a desktop first/g,
    'Sign in to a desktop before recording.'],
  [/Wayland session but \/dev\/input not readable — add user to the 'input' group: sudo usermod -aG input \$USER, then re-login/g,
    'Pointer and keyboard access is not allowed for your user. To allow it, run sudo usermod -aG input $USER in a terminal, then log out and back in.'],
  [/gst present but pipewiresrc missing — install gstreamer1\.0-pipewire/g,
    'Screen recording needs an additional system package. Install gstreamer1.0-pipewire.'],
  [/gst-launch-1\.0 not found — install gstreamer1\.0-tools \+ gstreamer1\.0-pipewire/g,
    'Screen recording needs additional system packages. Install gstreamer1.0-tools and gstreamer1.0-pipewire.'],
  [/GStreamer element x264enc is missing — install gstreamer1\.0-plugins-ugly \(Debian\/Ubuntu\) or gstreamer1-plugins-ugly from RPM Fusion \(Fedora\)/g,
    'Screen recording needs an additional system package. Install gstreamer1.0-plugins-ugly (Debian/Ubuntu) or gstreamer1-plugins-ugly from RPM Fusion (Fedora).'],
  [/GStreamer element mp4mux is missing — install gstreamer1\.0-plugins-good \(Debian\/Ubuntu\) or gstreamer1-plugins-good \(Fedora\)/g,
    'Screen recording needs an additional system package. Install gstreamer1.0-plugins-good (Debian/Ubuntu) or gstreamer1-plugins-good (Fedora).'],
  [/Core Audio system capture is compiled; use Test system audio to check Audio Capture permission and packet delivery because Doctor will not start an aggregate tap or trigger the system prompt/g,
    'Use Test system audio to check sound recording. Allow Audio Capture for ShellX Cut if asked.'],
  [/WASAPI process loopback is compiled; use Test system audio to check first-packet delivery because Doctor will not start a loopback stream/g,
    'Use Test system audio to check that sound can be recorded.'],
  [/PipeWire default-sink monitor capture is compiled; use Test system audio to check first-packet delivery because Doctor will not open the monitor stream/g,
    'Use Test system audio to check that sound can be recorded.'],
  [/A previous native preview could not be released\. Restart ShellX Cut \(or the Cut server\) before previewing or recording\./g,
    'A previous preview could not be closed. Restart ShellX Cut before previewing or recording.'],
  [/restart ShellX Cut \(or the Cut server\) before starting another preview or recording/gi,
    'Restart ShellX Cut before starting another preview or recording.'],
  [/On macOS, enable Pause & resume for one exact display at an integer frame rate\. Screen plus optional microphone and system audio are sealed together before an acknowledgement\./g,
    'On macOS, choose one display and a whole-number frame rate, then enable Pause & resume. Screen, microphone and system sound are saved together before a change is confirmed.'],
  [/(?:Windows Graphics Capture \(WGC\)|WGC|ScreenCaptureKit \(cursor hidden, system audio supported\)|XDG ScreenCast portal \+ GStreamer(?: \(cursor hidden, CFR normalize\))?); not checked yet: screen capture is checked when you press Check or start a recording/g,
    'Screen capture is not checked yet. Press Check or start a recording to check it.'],
  [/ScreenCaptureKit \(cursor hidden, system audio supported\); (screen recording permission is needed for ShellX Cut: turn it on in System Settings > Privacy & Security > Screen & System Audio Recording, then quit and reopen ShellX Cut)/g,
    '$1'],
  [/choose one source from the current screen_record\.doctor result/g, 'Choose one current source from the source picker.'],
  [/choose a display from screen_record\.doctor instead of a window/g, 'Choose a display in the source picker instead of a window.'],
]

/** Engine diagnostics are not editing instructions. Keep plain instructions; never show code or implementation details. */
export function recordingNotice(value: string, fallback = 'This recording action could not be completed. Check your recording setup and try again.'): string {
  const translated = RECORDING_INSTRUCTIONS.reduce((text, [diagnostic, instruction]) => text.replace(diagnostic, instruction), value)
  const text = translated.replace(/^[a-zA-Z][a-zA-Z0-9_]*:\s*/, '')
    .replace(/\bnative\s+/gi, '')
    .replace(/\bseal the pause/gi, 'pause the recording')
    .replace(/\bseal the resume/gi, 'resume recording')
    .replace(/\bdurable acknowledgement/gi, 'confirmation that the change was saved')
    .replace('Preview is paused and its session was released.', 'Preview is paused. Resume to see your source again.')
    .replace('Preview is stopped and its source was released.', 'Preview is stopped. Start it again to see your source.')
  const isDiagnostic = (part: string) => /\b(engine|server|fetch|TypeError|ReferenceError|schema|compiled|rdevin|evdev|X11|Wayland|GStreamer|ffmpeg|WGC|WASAPI|ScreenCaptureKit|AVFoundation|AVCaptureDevice|CoreAudio|PipeWire|DXGI|D3D11|WinRT|XDG|portal|advertise|backend|endpoint|opaque|payload|DTO|journal|packets?|samples?|logical_media_time_ms|capture_id|capture ID|first.frame|generation|lease|token|hook|sealed|seals)\b|\b[a-z]+_[a-z_]+\b/i.test(part.replaceAll('Install FFmpeg, or set SHELLX_RECORD_FFMPEG to its location.', '')
    // Literal package names and commands are instructions, not diagnostic prose.
    .replace(/\bgstreamer1\.0-pipewire\b/g, ''))
  if (!isDiagnostic(text)) return text
  // Refusals and Stop recovery join a diagnostic to a separate suggested action.
  // Hide only the diagnostic, never an already plain instruction beside it.
  const actions = text.split(/(?<=[.!?;])\s+(?=[A-Za-z])/).filter(part =>
    /^(?:choose|turn (?:off|on)|run|log (?:out|in)|sign in|restart|reopen|retry|check|press|allow|grant|connect|plug|stop|start|wait|keep|review|inspect|try|use|install|enable|refresh)\b/i.test(part)
      && !isDiagnostic(part))
  return [fallback, ...actions].join(' ')
}

/** Simplifying a diagnostic must not hide a running take or an already saved original. */
export function recordingSessionNotice(value: string, phase: string): string {
  const fallback = value.startsWith('Raw MP4 saved')
    ? 'Your original recording was saved. The remaining work could not be confirmed; review the recording before continuing.'
    : phase === 'recording' ? 'Recording. Some recording details could not be shown. Stay here until you stop.'
      : phase === 'starting' ? 'Starting the recording. Stay on the Record page.'
        : phase === 'finalizing' ? 'Finishing the recording. Stay on the Record page.'
          : phase === 'done' ? 'The recording has finished. Review it before continuing.'
            : 'The recording needs attention. Check its status before starting another take.'
  return recordingNotice(value, fallback)
}

/** The one step that grants pointer and keyboard access on Linux. It is the instruction itself, so it is shown as it is. */
export const INPUT_ACCESS_COMMAND = 'sudo usermod -aG input $USER'

/**
 * What to tell a person when pointer and keyboard access is missing. The engine's own sentence names system parts
 * and is not shown, but the step that fixes it must never be lost with it: when the engine names that step, the
 * advice carries it in plain words.
 */
export function recordingInputAdvice(detail: string): string {
  return detail.includes('usermod')
    ? `Pointer and keyboard access is not allowed for your user. To allow it, run ${INPUT_ACCESS_COMMAND} in a terminal, then log out and back in.`
    : recordingNotice(detail, 'Pointer and keyboard access is unavailable. Check your system permissions.')
}

/** The sentence for a Start that asks for keystrokes while pointer and keyboard access is missing. */
export function keystrokesUnavailableNotice(detail: string): string {
  return `Show keystrokes is unavailable. ${recordingInputAdvice(detail)} Turn it off to record screen and sound.`
}

/** Prefer the stable card id/status when they identify the missing requirement. */
export function recordingCardNotice(card: RecordCard, fallback: string): string {
  // System sound's OK card still asks for a real sound test. Pointer support
  // likewise is not evidence that capture startup has been checked.
  if (card.status === 'ok' && card.name !== 'system_audio') {
    if (card.name === 'input_hook' && card.detail.includes('capture startup unchecked')) {
      return 'Pointer and keys are available. They will be checked when recording starts.'
    }
    return 'Ready'
  }
  if (card.name === 'ffmpeg' && card.status === 'missing') {
    return 'Video tools are missing. Install FFmpeg, or set SHELLX_RECORD_FFMPEG to its location.'
  }
  if (card.name === 'wayland_input' && card.status !== 'ok') return recordingInputAdvice(card.detail)
  return recordingNotice(card.detail, fallback)
}

/** Optional Wayland event data is separate from screen and sound capture. */
export function recordingInputUnavailable(cards: readonly RecordCard[]): string | null {
  const input = cards.find(card => card.name === 'wayland_input' && card.status !== 'ok')
  return input ? input.detail : null
}
