import { normalizePlayheadMs } from '../panels/Preview/playbackClock'

/** The two ways the playhead moves: an explicit seek (user, agent command,
 * voiceover start) and a playback clock publication. Each explicit seek bumps
 * a revision. A clock publication carries the revision its clock last saw and
 * is dropped once a newer explicit seek exists, so a clock tick that lands in
 * the same render batch as a seek can never overwrite that seek. */
export function createPlayheadSeeks(
  setPlayheadMs: (positionMs: number) => void,
  setSeekRevision: (revision: number) => void,
) {
  let revision = 0
  return {
    seek(atMs: number) {
      revision += 1
      setSeekRevision(revision)
      setPlayheadMs(normalizePlayheadMs(atMs))
    },
    /** Returns false when a newer explicit seek superseded this publication. */
    playback(atMs: number, observedRevision: number): boolean {
      if (observedRevision !== revision) return false
      setPlayheadMs(normalizePlayheadMs(atMs))
      return true
    },
  }
}
