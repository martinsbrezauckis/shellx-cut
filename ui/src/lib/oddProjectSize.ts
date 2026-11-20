// lib/oddProjectSize.ts — an older project with an odd frame size (641x361).
//
// Video needs an even width and height, so the engine refuses every video
// export at an odd project size up front (code `odd_project_size`) instead of
// rounding during the export. The person is told when the project opens and
// again when an export is refused, each time with ONE action: change the
// project to the even size just below (`project.format`). Pure helpers here;
// the notice is components/OddProjectSizeNotice.tsx.

import { callVerb } from './client'

/** The engine's code for a video export refused at an odd project size. */
export const ODD_PROJECT_SIZE = 'odd_project_size'
/** Fired by `callVerb` when any verb is refused with that code. */
export const ODD_PROJECT_SIZE_EVENT = 'cut:odd-project-size'

export interface OddSize {
  width: number
  height: number
  evenWidth: number
  evenHeight: number
}

/** The odd size and its even replacement, or null for an even (or unknown) size. */
export function oddProjectSize(settings: { width?: number; height?: number } | null | undefined): OddSize | null {
  const width = settings?.width
  const height = settings?.height
  if (!Number.isInteger(width) || !Number.isInteger(height) || !width || !height) return null
  if (width % 2 === 0 && height % 2 === 0) return null
  return { width, height, evenWidth: width - (width % 2), evenHeight: height - (height % 2) }
}

/** The even size from a refusal's suggested_action (`project.format {"width":640,"height":360}`). */
export function evenSizeFromAction(action: string | null | undefined): { evenWidth: number; evenHeight: number } | null {
  const match = /^project\.format\s+(\{.*\})\s*$/.exec(action?.trim() ?? '')
  if (!match) return null
  try {
    const args = JSON.parse(match[1]) as { width?: unknown; height?: unknown }
    const { width, height } = args
    if (typeof width === 'number' && typeof height === 'number' && width % 2 === 0 && height % 2 === 0) {
      return { evenWidth: width, evenHeight: height }
    }
  } catch { /* not the engine's call */ }
  return null
}

export function oddSizeMessage(size: OddSize): string {
  return `This project is ${size.width}x${size.height}, but video exports need an even width and height. Change it to ${size.evenWidth}x${size.evenHeight} before exporting.`
}

export function changeSizeLabel(size: { evenWidth: number; evenHeight: number }): string {
  return `Change project to ${size.evenWidth}x${size.evenHeight}`
}

/** Apply the one fix: the project's size becomes the even size (frame rate unchanged). */
export function changeProjectToEven(size: { evenWidth: number; evenHeight: number }) {
  return callVerb('project.format', {
    width: size.evenWidth,
    height: size.evenHeight,
    rationale: 'user: video exports need an even frame size',
  })
}
