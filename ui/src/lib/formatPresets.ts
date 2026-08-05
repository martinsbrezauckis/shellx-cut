// formatPresets — shared timeline OUTPUT FORMAT presets (resolution + frame rate).
//
// Single source of truth for the topbar's expert `project.format` Timeline
// picker. New projects intentionally do not ask for these: their first video
// adopts its source geometry/frame rate, while delivery settings live at Render.
//
// Extracted from topbar/index.tsx (where these were originally defined) so the
// two surfaces stay in lockstep — change a preset here, both pickers follow.
// Lower resolution/fps = much faster renders + proxies on heavy footage.
//
// Callers: topbar/index.tsx, panels/Projects/index.tsx. Deps: none.

/** Timeline output RESOLUTION presets. `w`/`h` are the pixel geometry passed to
 *  project.format. Order = high → low. */
export const RES_PRESETS = [
  { label: '2160p · 4K', w: 3840, h: 2160 },
  { label: '1080p · FHD', w: 1920, h: 1080 },
  { label: '720p · HD', w: 1280, h: 720 },
  { label: '480p · SD', w: 854, h: 480 },
] as const

/** Timeline output FRAME-RATE presets (fps). */
export const FPS_PRESETS = [24, 25, 30, 50, 60] as const

/** Match a project's current geometry to a preset label, else "custom". */
export function resKey(s?: { width: number; height: number }): string {
  const m = RES_PRESETS.find((r) => r.w === s?.width && r.h === s?.height)
  return m ? m.label : 'custom'
}
