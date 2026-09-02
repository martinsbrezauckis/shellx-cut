import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { expandHome } from './windows-qualification-runner-utils.mjs'

export function resolveWindowsInstalledMediaInputs({ root, signedFinal, requestedFace, arg, argv = process.argv }) {
  if (!requestedFace) {
    throw new Error('Windows installed qualification requires --face with a detector-proven local video; no generated default face fixture exists')
  }
  const roleArgs = {
    scene: arg('--scene', join(root, 'testdata/talking_head.mp4')),
    speech: arg('--speech', join(root, 'testdata/talking_head.mp4')),
    face: requestedFace,
    // This role needs speech/audio to prove media.diarize and audio.dub.
    speakers: arg('--speakers', join(root, 'testdata/talking_head.mp4')),
    second: arg('--second', join(root, 'testdata/silent_screen.mp4')),
  }
  if (signedFinal) {
    for (const role of Object.keys(roleArgs)) {
      if (!argv.includes(`--${role}`)) throw new Error(`--signed-final requires --${role} with real release media`)
    }
  }
  for (const [role, path] of Object.entries(roleArgs)) {
    roleArgs[role] = resolve(expandHome(path))
    if (!existsSync(roleArgs[role])) throw new Error(`${role} media not found: ${roleArgs[role]}`)
  }
  return roleArgs
}
