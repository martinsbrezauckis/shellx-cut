// Preview Compare is deliberately a monitor-only, revision-bound review tool.
// Source and CSS assertions pin its novice-facing trigger, no-Undo,
// stale-response, and narrow-layout guarantees without a desktop runtime.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const component = readFileSync(resolve(root, 'src/panels/Preview/PreviewComparison.tsx'), 'utf8')
const css = readFileSync(resolve(root, 'src/panels/Preview/previewComparison.css'), 'utf8')

assert.match(component, /data-cut-preview-compare-trigger/, 'Preview exposes one stable Compare control for the manual and desktop checks')
assert.match(component, /data-cut-action="close-preview-comparison"/, 'the temporary comparison has one stable close action')
assert.match(component, /<span>\{busy \? 'Comparing…' : 'Compare'\}<\/span>/, 'the trigger uses novice-facing Compare copy')
assert.match(component, /aria-haspopup="dialog"/, 'Compare communicates its temporary review surface')
assert.match(component, /data-cut-preview-compare-state=\{busy \? 'loading' : comparison \? 'open' : notice \? 'refused' : 'idle'\}/, 'the initial state is inspectable without pretending a pair exists')
assert.match(component, /\{comparison && \(/, 'no frame pair is rendered before the server proves one')

assert.match(component, /onPause\(\)[\s\S]*callVerb\('render\.compare'/, 'entering Compare pauses playback before requesting either frame')
assert.match(component, /response\.result\.current_revision !== revision \|\| response\.result\.at_ms !== atMs/, 'a response for another revision or playhead is discarded')
assert.match(component, /comparison\.current_revision !== revision \|\| comparison\.at_ms !== Math\.max/, 'an open pair closes when the revision or playhead changes')
assert.match(component, /useBlockingOverlay<HTMLElement>\(close, Boolean\(comparison\)\)/, 'Escape, focus containment, and focus return use the shared blocking-overlay owner')
assert.match(component, /onKeyDown=\{overlay\.onDialogKeyDown\}/, 'the temporary review surface installs the shared keyboard contract')
assert.doesNotMatch(component, /project\.undo|project\.redo|edit\.restore/, 'the UI never manufactures Before through project mutation')
assert.match(component, /data-cut-preview-compare-before/, 'Before has a stable exact-frame selector')
assert.match(component, /data-cut-preview-compare-current/, 'Current has a stable exact-frame selector')

assert.match(css, /grid-template-columns: repeat\(2, minmax\(0, 1fr\)\)/, 'desktop Compare is a side-by-side pair by default')
assert.match(css, /@media \(max-width: 640px\)[\s\S]*grid-template-columns: 1fr/, 'narrow screens stack the pair rather than shrinking either frame beyond use')
assert.match(css, /\.pv-compare__close \{ min-height: 44px; \}/, 'narrow Compare keeps its close target touch-sized')

console.log('PASS Preview Compare revision-bound review contract')
