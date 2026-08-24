import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '..')
const repoRoot = resolve(uiRoot, '..')
const read = (relative: string) => readFileSync(resolve(repoRoot, relative), 'utf8')

const userFacingSources = [
  'ui/src/panels/Inspector/MotionLinkSection.tsx',
  'app/server/src/motion_edit_return.rs',
  'docs/public/DEBUG_API.md',
  'docs/public/SHELLX_MOTION_BOUNDARY.md',
  'docs/public/site/manual/manual.js',
  'skill/shellx-cut/SKILL.md',
  'skill/shellx-cut/reference.md',
  'schema/verbs.json',
]

for (const source of userFacingSources) {
  const contents = read(source)
  assert.match(contents, /ShellX Motion/, `${source} must name ShellX Motion`)
  assert.doesNotMatch(
    contents,
    /\b(?:ShellX Canvas|Canvas Motion)\b/i,
    `${source} must use the ShellX Motion product name`,
  )
}

const motionBridge = read('app/server/src/motion_bridge.rs')
assert.match(motionBridge, /"ShellX Motion editor is not available"/)
assert.match(motionBridge, /"ShellX Motion could not be launched"/)
assert.doesNotMatch(motionBridge, /"(?:ShellX Canvas executable|ShellX Canvas could not|Canvas Motion)/)
const bridgeWithoutLegacyExecutablePaths = motionBridge
  .replaceAll('ShellX Canvas.exe', '')
  .replaceAll('ShellX Canvas.app', '')
assert.doesNotMatch(
  bridgeWithoutLegacyExecutablePaths,
  /ShellX Canvas/,
  'Canvas wording is limited to legacy executable paths',
)
assert.match(
  motionBridge,
  /LEGACY_CANVAS_BIN_ENV: &str = "SHELLX_CANVAS_BIN"/,
  'the historical environment variable remains a named compatibility override',
)
assert.match(
  motionBridge,
  /LEGACY_CANVAS_PROGRAM_NAMES_WINDOWS[\s\S]*shellx-canvas\.exe[\s\S]*ShellX Canvas\.exe/,
  'legacy Windows executable discovery remains available',
)
assert.match(
  motionBridge,
  /LEGACY_CANVAS_PROGRAM_NAMES_UNIX[\s\S]*shellx-canvas[\s\S]*shellx-canvas-app/,
  'legacy Unix executable discovery remains available',
)

const boundary = read('docs/public/SHELLX_MOTION_BOUNDARY.md')
assert.match(
  boundary,
  /SHELLX_CANVAS_BIN[\s\S]*historical `shellx-canvas` executable names are backward-compatibility\n  discovery fallbacks only/,
  'the public Motion boundary classifies Canvas names as discovery fallback only',
)

console.log('PASS motion-terminology-contract')
