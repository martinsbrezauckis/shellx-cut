// "Cannot decode" detection for the preview's proxy video.
//
// The preview plays the engine's H.264 proxies in the webview. When the
// operating system has no H.264 decoder for the webview (Linux: no GStreamer
// avdec_h264 / openh264dec, e.g. Fedora or Debian/Ubuntu without recommended
// packages), every proxy element fails with MediaError 3 (DECODE) or 4
// (SRC_NOT_SUPPORTED) and the monitor stays black. Some webviews (WebKitGTK)
// also report these codes TRANSIENTLY after a seek on a freshly loaded
// element; the playback clock reloads such an element and it plays. So a
// failure only counts as "cannot decode" when it repeats after a reload AND no
// proxy has decoded a single frame in this session: one decoded proxy proves
// the decoder exists, because every proxy has the same format. Pure module.

/** A loaded proxy element with no video picture (videoWidth 0): WebKitGTK
 * without an H.264 decoder may play the AAC track only instead of failing. */
export const NO_VIDEO_PICTURE = 'no_video_picture'

/** MediaError codes that mean the element could not decode or open the
 * media, or a loaded element that has no picture. */
export function isDecodeErrorCode(code: number | typeof NO_VIDEO_PICTURE | null | undefined): boolean {
  return code === 3 || code === 4 || code === NO_VIDEO_PICTURE
}

export type DecodeVerdict = 'ignore' | 'retry' | 'cannot_decode'

/** Failures before the verdict: the first one is retried with a reload. */
export const DECODE_FAILURES_BEFORE_VERDICT = 2

export class MediaDecodeHealth {
  private anyDecoded = false
  private readonly failures = new Map<string, number>()

  /** A proxy element showed a frame (readyState reached HAVE_CURRENT_DATA).
   * Returns true when this changes the verdict state. */
  decoded(): boolean {
    const changed = !this.anyDecoded || this.failures.size > 0
    this.anyDecoded = true
    this.failures.clear()
    return changed
  }

  /** An element of `src` failed with MediaError `code`. */
  failed(src: string, code: number | typeof NO_VIDEO_PICTURE | null | undefined): DecodeVerdict {
    if (!isDecodeErrorCode(code) || this.anyDecoded) return 'ignore'
    const count = (this.failures.get(src) ?? 0) + 1
    this.failures.set(src, count)
    return count >= DECODE_FAILURES_BEFORE_VERDICT ? 'cannot_decode' : 'retry'
  }
}

/** What the monitor and `ui.state` say when the preview cannot decode its video. */
export interface PreviewMediaProblem {
  kind: 'cannot_decode'
  assetIds: string[]
  title: string
  detail: string
}

/** Plain words, with the package to install on Linux (names as the Linux
 * packages and the recording Doctor card use them). */
export function cannotDecodeProblem(os: string | null | undefined, assetIds: string[]): PreviewMediaProblem {
  const title = 'This video cannot play in the preview'
  const linux = os === 'linux'
  const detail = linux
    ? 'The system has no decoder for the preview\'s H.264 video. Install the GStreamer decoder: '
      + 'gstreamer1.0-libav (Debian/Ubuntu) or gstreamer1-plugin-openh264 (Fedora), then restart ShellX Cut. '
      + 'Rendering and export are not affected.'
    : 'The system cannot decode the preview\'s H.264 video. Rendering and export are not affected.'
  return { kind: 'cannot_decode', assetIds: [...assetIds].sort(), title, detail }
}
