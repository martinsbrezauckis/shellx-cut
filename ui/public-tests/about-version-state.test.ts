import assert from 'node:assert/strict'

import {
  createUiObservableState,
  readUiDomState,
} from '../src/app/uiControlState'
import { UI_DOM_MUTATION_OBSERVER_OPTIONS } from '../src/app/useUiStatePublisher'
import type { UiStateSource } from '../src/app/uiControlState'

interface FakeElement {
  dataset?: Record<string, string | undefined>
  textContent?: string | null
}

function domRoot(elements: Record<string, FakeElement | null>) {
  return {
    querySelector(selector: string) {
      return elements[selector] ?? null
    },
  } as unknown as Pick<Document, 'querySelector'>
}

function observable(dom: ReturnType<typeof readUiDomState>) {
  return createUiObservableState({
    revision: 4,
    layout: {
      workspaceMode: 'edit', leftCollapsed: false, leftTab: 'assets', findSurface: 'find-media',
      railCollapsed: false, railPinned: false, rightTab: 'properties',
    },
    generateTab: 'templates',
    wizardOpen: false,
    envOpen: true,
    envCategory: 'about',
    commentsOpen: false,
    activeDrawer: null,
    highlight: null,
    playheadMs: 0,
    selectedClipIds: [],
    exportRange: null,
    project: null,
    dom,
  } as unknown as UiStateSource)
}

const elements: Record<string, FakeElement | null> = {
  '[data-cut-about]': { dataset: { cutAppVersion: '0.6.114' } },
  '[data-cut-about-version]': { textContent: 'v—' },
}
const root = domRoot(elements)

assert.equal(readUiDomState(root).aboutVersion, null,
  'the fallback mark is never published as an installed About version')
elements['[data-cut-about-version]'] = { textContent: 'v0.6.114' }
const committed = readUiDomState(root)
assert.equal(committed.aboutVersion, '0.6.114',
  'the doctor version is published only after its rendered text commits')
assert.equal(observable(committed).about.displayed_version, '0.6.114',
  'ui.state carries the committed rendered version')

elements['[data-cut-about-version]'] = { textContent: 'v0.6.113' }
assert.equal(readUiDomState(root).aboutVersion, null,
  'a text and attribute mismatch cannot certify an About version')
elements['[data-cut-about-version]'] = { textContent: ' v0.6.114 ' }
assert.equal(readUiDomState(root).aboutVersion, '0.6.114',
  'the observable value is parsed from the committed badge text')
elements['[data-cut-about]'] = null
elements['[data-cut-about-version]'] = null
const closed = readUiDomState(root)
assert.equal(closed.aboutVersion, null, 'closing About clears the DOM state')
assert.equal(observable(closed).about.displayed_version, null,
  'ui.state clears the rendered version after About closes')

assert.equal(UI_DOM_MUTATION_OBSERVER_OPTIONS.subtree, true)
assert.equal(UI_DOM_MUTATION_OBSERVER_OPTIONS.childList, true)
assert.equal(UI_DOM_MUTATION_OBSERVER_OPTIONS.characterData, true,
  'doctor text updates schedule a fresh DOM-backed ui.state')
assert.deepEqual(UI_DOM_MUTATION_OBSERVER_OPTIONS.attributeFilter,
  ['aria-selected', 'data-cut-app-version'])

console.log('PASS About rendered-version ui.state')
