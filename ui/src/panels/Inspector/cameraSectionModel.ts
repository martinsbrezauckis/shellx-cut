import type { ClipTransform } from '../../lib/client'

// Clip/asset ids also identify baked compatibility media; only a recorded look
// makes that provenance an editable camera.
export interface CameraTake { camera_clip?: string | null; camera_look?: unknown }
export type CameraPosition = 'top_left' | 'top_right' | 'bottom_left' | 'bottom_right' | 'centre'
export const CAMERA_RING = { width: 0.02, colour: '#ffffffe6' }

export function isTakeCamera(
  clip: string | { id: string; recording?: { camera?: boolean } },
  takes?: readonly CameraTake[],
): boolean {
  const id = typeof clip === 'string' ? clip : clip.id
  return (typeof clip !== 'string' && clip.recording?.camera === true)
    || (takes?.some((take) => take.camera_look != null && take.camera_clip === id) ?? false)
}

export function cameraPreset(position: CameraPosition, scale: number, width: number, height: number) {
  // Record/studioTypes placementForPosition uses a 4% margin. Unlike Record's
  // height-based size, an Edit transform's scale is a fraction of frame width.
  const bubbleHeight = scale * width / height
  if (position === 'centre') return { x: Math.max(0, (1 - scale) / 2), y: Math.max(0, (1 - bubbleHeight) / 2) }
  return {
    x: position.endsWith('left') ? .04 : Math.max(.04, 1 - .04 - scale),
    y: position.startsWith('top') ? .04 : Math.max(.04, 1 - .04 - bubbleHeight),
  }
}

export function cameraTransform(stored: ClipTransform | null | undefined, patch: Partial<ClipTransform>): ClipTransform {
  const next = { x: 0, y: 0, scale: 1, opacity: 1, ...stored, ...patch }
  // edit.transform replaces the whole look: rectangle cannot carry a ring or
  // rounded-corner radius. Clear both in this same SET, never a second write.
  return next.shape === 'rect' ? { ...next, ring: undefined, radius: undefined } : next
}
