// PropertyRow numeric draft regression — a signed/decimal value remains text
// until the existing single blur/Enter commit. This is source/UI unit coverage;
// native receipt qualification remains with the installed matrix.

import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

// This project compiles JSX with the classic runtime. The product shell supplies
// React globally; mirror that environment before dynamically importing the TSX
// component for markup coverage.
;(globalThis as typeof globalThis & { React?: { createElement: typeof createElement } }).React = { createElement }
const { default: PropertyRow, propertyRowNumericDraft } = await import('../src/components/inspector/PropertyRow')

const props = {
  label: 'Gain',
  value: 0,
  min: -60,
  max: 12,
  step: 0.5,
  default: 0,
  propKey: 'gain',
  onCommit: () => {},
}

// Keep unfinished values as text. They must neither clamp to zero nor issue a
// preview/commit value before the user finishes the number.
assert.equal(propertyRowNumericDraft('-', props.min, props.max), null)
assert.equal(propertyRowNumericDraft('-.', props.min, props.max), null)
assert.equal(propertyRowNumericDraft('', props.min, props.max), null)
assert.equal(propertyRowNumericDraft('-6', props.min, props.max), -6)
assert.equal(propertyRowNumericDraft('-.5', props.min, props.max), -0.5)
assert.equal(propertyRowNumericDraft('-99', props.min, props.max), -60, 'completed values retain existing clamp bounds')
assert.equal(propertyRowNumericDraft('0x10', props.min, props.max), null, 'hex is not decimal text input')
assert.equal(propertyRowNumericDraft(' 1', props.min, props.max), null, 'whitespace is not silently normalized')

// React renders the stable selector as a text-backed decimal entry, allowing a
// real keyboard to retain its intermediate sign before Enter blurs once.
const initial = renderToStaticMarkup(createElement(PropertyRow, props))
assert.match(initial, /data-cut-prop-input="gain"/)
assert.match(initial, /type="text"/)
assert.match(initial, /inputMode="decimal"/)
assert.match(initial, /role="spinbutton"/)
assert.match(initial, /aria-valuemin="-60"/)
assert.match(initial, /aria-valuemax="12"/)
assert.match(initial, /aria-valuenow="0"/)
assert.match(initial, /value="0\.0"/)

// A fresh committed prop value re-seeds the same React component on the next
// render, while the source keeps a raw edit string only during focus.
const external = renderToStaticMarkup(createElement(PropertyRow, { ...props, value: -6 }))
assert.match(external, /value="-6\.0"/, 'external committed values retain signed formatting')

const source = await (await import('node:fs/promises')).readFile(
  new URL('../src/components/inspector/PropertyRow.tsx', import.meta.url),
  'utf8',
)
assert.match(source, /onBlur=\{\(e\) => \{\s*editingInput\.current = false\s*commitInput\(inputDraft \?\? e\.target\.value\)/,
  'blur leaves editing mode before the only numeric commit path')
assert.match(source, /if \(e\.key === 'Enter'\)[\s\S]*?\.blur\(\)/,
  'Enter delegates to blur instead of issuing a second commit')
assert.match(source, /e\.key === 'ArrowUp' \|\| e\.key === 'ArrowDown'/,
  'spinbutton arrow keys retain the existing step preview route')
assert.match(source, /\['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'\]\.includes\(e\.key\)/,
  'slider ignores focus-transfer keys such as Tab instead of committing a duplicate value')

console.log('PASS PropertyRow signed/decimal numeric draft and single-commit contract')
