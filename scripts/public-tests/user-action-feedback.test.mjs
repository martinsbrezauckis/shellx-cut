import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const root = new URL('../../', import.meta.url)

async function source(path) {
  return readFile(new URL(path, root), 'utf8')
}

test('audited human edit surfaces cannot silently fire and forget verbs', async () => {
  const mustUseSharedFeedback = [
    'ui/src/App.tsx',
    'ui/src/panels/Inspector/AudioInspectorTools.tsx',
    'ui/src/panels/Inspector/VideoInspectorTools.tsx',
    'ui/src/panels/Inspector/VolumeSection.tsx',
    'ui/src/panels/Mixer/index.tsx',
    'ui/src/panels/Timeline/TrackControls.tsx',
  ]
  for (const path of mustUseSharedFeedback) {
    const text = await source(path)
    assert.match(text, /runUserVerb/, `${path} must route human mutations through shared feedback`)
    assert.doesNotMatch(text, /\bvoid\s+callVerb\s*\(/, `${path} has a silent fire-and-forget verb`)
  }

  const transcript = await source('ui/src/panels/Transcript/index.tsx')
  assert.doesNotMatch(
    transcript,
    /\bvoid\s+callVerb\s*\(\s*['"](?:edit|captions|transcript\.(?:cut|mute|ignore))\b/,
    'Transcript mutations must visibly report failures',
  )

  const preview = await source('ui/src/panels/Preview/index.tsx')
  assert.doesNotMatch(preview, /\bvoid\s+callVerb\s*\(\s*['"]edit\./, 'Preview edits must visibly report failures')

  const record = await source('ui/src/panels/Record/index.tsx')
  assert.doesNotMatch(
    record,
    /\bvoid\s+callVerb\s*\(\s*['"]screen_record\.studio_event['"]/,
    'Live recording controls must not claim success before the event is stored',
  )
})

test('the few direct timeline calls retain explicit local failure recovery', async () => {
  for (const [path, expected] of [
    ['ui/src/panels/Timeline/useTimelineClipActions.ts', 1],
    ['ui/src/panels/Timeline/index.tsx', 3],
  ]) {
    const text = await source(path)
    const calls = [...text.matchAll(/\bvoid\s+callVerb\s*\(/g)]
    assert.equal(calls.length, expected, `${path} direct call count changed; review every new action`)
    for (const call of calls) {
      const guarded = text.slice(call.index, call.index + 1_100)
      assert.match(guarded, /\.then\([\s\S]*showVerbFailure/, `${path} direct call lacks result feedback`)
      assert.match(guarded, /\.catch\([\s\S]*showVerbFailure/, `${path} direct call lacks transport feedback`)
    }
  }
})

test('shared feedback is mounted, actionable, and accessible', async () => {
  const [app, component] = await Promise.all([
    source('ui/src/App.tsx'),
    source('ui/src/components/UserActionFeedback.tsx'),
  ])
  assert.match(app, /<UserActionFeedback\s*\/>/)
  assert.match(component, /data-cut-user-action-feedback/)
  assert.match(component, /role="alert"/)
  assert.match(component, /data-cut-user-action-open-setup/)
  assert.match(component, /cut:open-ui-surface/)
  assert.match(component, /data-cut-user-action-dismiss/)
})
