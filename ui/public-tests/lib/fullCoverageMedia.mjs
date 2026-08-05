// fullCoverageMedia.mjs - real-media role resolver for the exhaustive verifier.
//
// Installed runs may execute the Node harness from WSL/SSH while the engine
// imports from a native Windows/macOS path. This helper keeps local existence
// checks, engine import paths, and fallback-role evidence in one place.

import { resolveMediaRole } from '../../../scripts/lib/cross-host-media.mjs'

export function createFullCoverageMedia({ mediaDir, engineMediaDir = mediaDir }) {
  if (!mediaDir) throw new TypeError('createFullCoverageMedia requires mediaDir')
  const fallbacks = []

  function media(realName, fallback, role) {
    const resolved = resolveMediaRole({
      localDir: mediaDir,
      engineDir: engineMediaDir,
      realName,
      fallback,
      role,
    })
    if (!resolved.fallbackUsed) return resolved.path
    fallbacks.push({
      role,
      realName,
      fallback,
      dir: mediaDir,
      existsPath: resolved.existsPath,
      engineDir: engineMediaDir,
    })
    return resolved.path
  }

  return { media, fallbackRoles: fallbacks }
}
