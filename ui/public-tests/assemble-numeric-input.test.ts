import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

;(globalThis as typeof globalThis & { React?: { createElement: typeof createElement } }).React = { createElement }
const { AssemblePlanControls, assembleNumericDraft } = await import('../src/panels/Assemble/AssemblePlanControls')

const props = {
  mode: 'shorts' as const,
  busy: false,
  effectiveAsset: 'asset-1',
  count: 5,
  targetS: 30,
  prompt: '',
  script: '',
  minScore: 0.35,
  aspect: '9:16' as const,
  shortsPlan: null,
  repurposePlan: null,
  scriptPlan: null,
  applyState: 'idle' as const,
  onCount: () => {},
  onTargetS: () => {},
  onPrompt: () => {},
  onScript: () => {},
  onMinScore: () => {},
  onAspect: () => {},
  onRunShorts: () => {},
  onRunRepurpose: () => {},
  onRunFromScript: () => {},
  onApplyShorts: () => {},
  onApplyRepurpose: () => {},
  onApplyFromScript: () => {},
}

function input(markup: string, selector: string) {
  const match = markup.match(new RegExp(`<input[^>]*${selector}[^>]*>`))
  assert.ok(match, `expected ${selector} input`)
  return match[0]
}

// The native full-coverage flow sends real keys one at a time. A first key may
// preview a clamped value, but the retained text still produces the exact final
// target instead of appending to that preview.
assert.equal(assembleNumericDraft('1', 3, 600), 3)
assert.equal(assembleNumericDraft('12', 3, 600), 12)
assert.equal(assembleNumericDraft('0', 0, 1), 0)
assert.equal(assembleNumericDraft('0.', 0, 1), 0)
assert.equal(assembleNumericDraft('0.4', 0, 1), 0.4)
assert.equal(assembleNumericDraft('.', 0, 1), null)
assert.equal(assembleNumericDraft('0x10', 0, 1), null)
assert.equal(assembleNumericDraft('2.5', 1, 50, true), null, 'count cannot preview a non-integer request')
assert.equal(assembleNumericDraft('2', 1, 50, true), 2)

const shorts = renderToStaticMarkup(createElement(AssemblePlanControls, props))
const target = input(shorts, 'data-cut-assemble-target')
assert.match(target, /type="text"/)
assert.match(target, /inputMode="numeric"/)
assert.match(target, /role="spinbutton"/)
assert.match(target, /aria-valuemin="3"/)
assert.match(target, /aria-valuemax="600"/)
assert.match(target, /aria-valuenow="30"/)

const fromScript = renderToStaticMarkup(createElement(AssemblePlanControls, { ...props, mode: 'from_script' }))
const minScore = input(fromScript, 'data-cut-assemble-minscore')
assert.match(minScore, /type="text"/)
assert.match(minScore, /inputMode="decimal"/)
assert.match(minScore, /role="spinbutton"/)
assert.match(minScore, /step="0\.05"/)
assert.match(minScore, /aria-valuenow="0\.35"/)

console.log('PASS Assemble numeric controls retain trusted keyboard drafts')
