import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')
const controller = readFileSync(new URL('../src/app/useUiCommandController.ts', import.meta.url), 'utf8')
// Explicit seeks and playback clock publications go through one owner: an
// explicit seek bumps the seek revision, a clock publication carries the
// revision it observed (see playhead-seeks.test.ts for the ordering rule).
assert.match(app, /const playheadSeeks = useMemo\(\(\) => createPlayheadSeeks\(setPlayheadMs, setSeekRevision\), \[\]\)/)
assert.match(app, /const onSeek = playheadSeeks\.seek\n/)
assert.match(app, /useUiCommandController\(\{\s*stateRef: uiStateRef,\s*project,\s*setPlayheadMs: onSeek,/)
assert.match(controller, /case 'ui\.playhead':[\s\S]*?setPlayheadMs\(atMs\)/)
assert.match(controller, /handleVoiceoverPlaybackCommand\(\{[\s\S]*?setPlayheadMs,/)
assert.match(app, /const onPlaybackSeek = playheadSeeks\.playback\n/)
console.log('PASS UI command explicit seek routing')
