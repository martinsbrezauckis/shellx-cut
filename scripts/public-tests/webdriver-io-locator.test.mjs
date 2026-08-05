import assert from 'node:assert/strict'
import test from 'node:test'
import { createWebdriverIoLocatorFactory } from '../../ui/public-tests/lib/webdriverIoLocator.mjs'

function fakeNode(textContent, descendants = {}) {
  return {
    textContent,
    tagName: 'DIV',
    querySelectorAll: (selector) => descendants[selector] || [],
  }
}

test('WebdriverIO locator filters by normalized string and regular-expression text', async () => {
  const nodes = [
    fakeNode('Page 1–100 of 101 results'),
    fakeNode('Page 101–101   of 101 results'),
    fakeNode('Indexed 7 frames'),
  ]
  const previousDocument = globalThis.document
  globalThis.document = {
    querySelectorAll: (selector) => selector === '.status' ? nodes : [],
  }

  try {
    const browser = {
      execute: async (fn, ...args) => fn(...args),
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => {},
      defaultTimeout: () => 100,
    })
    const pageLocator = (selector) => new Locator([{ selector }])

    assert.equal(
      await pageLocator('.status').filter({ hasText: '101–101 of 101' }).count(),
      1,
    )
    assert.equal(
      await pageLocator('.status').filter({ hasText: /indexed\s+7 frames/i }).count(),
      1,
    )
    assert.equal(
      await pageLocator('.missing').locator('.status').count(),
      0,
      'an empty parent locator must not restart its child query at document',
    )
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
})

test('WebdriverIO locator focuses the resolved native-WebView control', async () => {
  let focusCalls = 0
  let drainCalls = 0
  const node = {
    ...fakeNode('Restore'),
    focus: () => { focusCalls += 1 },
  }
  const previousDocument = globalThis.document
  globalThis.document = {
    querySelectorAll: (selector) => selector === '[data-cut-action="restore"]' ? [node] : [],
  }

  try {
    const browser = {
      execute: async (fn, ...args) => fn(...args),
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => { drainCalls += 1 },
      defaultTimeout: () => 100,
    })

    await new Locator([{ selector: '[data-cut-action="restore"]' }]).focus()

    assert.equal(focusCalls, 1)
    assert.equal(drainCalls, 1)
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
})

test('WebdriverIO locator press focuses once without a pointer click', async () => {
  let focusCalls = 0
  let pointerActions = 0
  const keys = []
  const node = {
    ...fakeNode('Reel mode'),
    focus: () => { focusCalls += 1 },
  }
  const previousDocument = globalThis.document
  globalThis.document = {
    querySelectorAll: (selector) => selector === '[data-cut-action="reel-mode"]' ? [node] : [],
  }

  try {
    const browser = {
      execute: async (fn, ...args) => fn(...args),
      keys: async (sequence) => { keys.push(sequence) },
      action: () => { pointerActions += 1 },
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => {},
      defaultTimeout: () => 100,
    })

    await new Locator([{ selector: '[data-cut-action="reel-mode"]' }]).press('Enter')

    assert.equal(focusCalls, 1)
    assert.deepEqual(keys, [['Enter']])
    assert.equal(pointerActions, 0)
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
})

test('WebdriverIO locator hovers the resolved native-WebView control', async () => {
  let pointerMove = null
  let performed = 0
  let drainCalls = 0
  const node = {
    ...fakeNode('Accept'),
    getBoundingClientRect: () => ({ x: 10, y: 20, width: 80, height: 30 }),
    scrollIntoView: () => {},
  }
  const previousDocument = globalThis.document
  globalThis.document = {
    querySelectorAll: (selector) => selector === '[data-cut-action="accept-op"]' ? [node] : [],
  }

  try {
    const action = {
      move: (value) => {
        pointerMove = value
        return action
      },
      perform: async () => { performed += 1 },
    }
    const browser = {
      execute: async (fn, ...args) => fn(...args),
      action: () => action,
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => { drainCalls += 1 },
      defaultTimeout: () => 100,
    })

    await new Locator([{ selector: '[data-cut-action="accept-op"]' }]).hover()

    assert.deepEqual(pointerMove, { x: 50, y: 35 })
    assert.equal(performed, 1)
    assert.equal(drainCalls, 1)
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
})

test('WebdriverIO locator honors a requested element-relative click position', async () => {
  let pointerMove = null
  let performed = 0
  const recordedActions = []
  const attributes = [
    { name: 'data-cut-action', value: 'generated-compare-backdrop' },
    { name: 'data-cut-generated-compare-backdrop', value: '' },
  ]
  const node = {
    ...fakeNode('Close comparison'),
    attributes,
    getAttribute: (name) => attributes.find((attribute) => attribute.name === name)?.value ?? null,
    getBoundingClientRect: () => ({ x: 10, y: 20, width: 800, height: 600 }),
    scrollIntoView: () => {},
  }
  const previousDocument = globalThis.document
  globalThis.document = {
    querySelectorAll: (selector) => selector === '[data-cut-generated-compare-backdrop]' ? [node] : [],
  }

  try {
    const action = {
      move: (value) => {
        pointerMove = value
        return action
      },
      down: () => action,
      up: () => action,
      perform: async () => { performed += 1 },
    }
    const browser = {
      execute: async (fn, ...args) => fn(...args),
      action: () => action,
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => {},
      recordSuccessfulAction: async (actions) => { recordedActions.push(actions) },
      defaultTimeout: () => 100,
    })

    await new Locator([{ selector: '[data-cut-generated-compare-backdrop]' }])
      .click({ position: { x: 2, y: 3 } })

    assert.deepEqual(pointerMove, { x: 12, y: 23 })
    assert.equal(performed, 1)
    assert.deepEqual(recordedActions, [['generated-compare-backdrop']])
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
})

test('WebdriverIO locator holds keyboard modifiers through the pointer click', async () => {
  let performed = null
  let releases = 0
  const node = {
    ...fakeNode('second word'),
    getBoundingClientRect: () => ({ x: 10, y: 20, width: 20, height: 10 }),
    scrollIntoView: () => {},
  }
  const previousDocument = globalThis.document
  globalThis.document = {
    querySelectorAll: (selector) => selector === '[data-cut-word="1"]' ? [node] : [],
  }

  try {
    const browser = {
      capabilities: { platformName: 'macos' },
      execute: async (fn, ...args) => fn(...args),
      performActions: async (actions) => { performed = actions },
      releaseActions: async () => { releases += 1 },
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => {},
      defaultTimeout: () => 100,
    })

    await new Locator([{ selector: '[data-cut-word="1"]' }]).click({ modifiers: ['Shift'] })

    assert.deepEqual(performed, [
      {
        type: 'key',
        id: 'shellx-cut-modifiers',
        actions: [
          { type: 'keyDown', value: '\uE008' },
          { type: 'pause', duration: 0 },
          { type: 'pause', duration: 0 },
          { type: 'pause', duration: 0 },
          { type: 'keyUp', value: '\uE008' },
        ],
      },
      {
        type: 'pointer',
        id: 'shellx-cut-pointer',
        parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pause', duration: 0 },
          { type: 'pointerMove', x: 20, y: 25, duration: 100, origin: 'viewport' },
          { type: 'pointerDown', button: 0 },
          { type: 'pointerUp', button: 0 },
          { type: 'pause', duration: 0 },
        ],
      },
    ])
    assert.equal(releases, 1)
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
})

test('WebdriverIO forced context click reports the target center', async () => {
  let received = null
  const node = {
    ...fakeNode('Trim end'),
    getBoundingClientRect: () => ({
      left: 40,
      top: 60,
      width: 80,
      height: 20,
    }),
    dispatchEvent: (event) => {
      received = event
      return true
    },
  }
  const previousDocument = globalThis.document
  const previousMouseEvent = globalThis.MouseEvent
  globalThis.document = {
    querySelectorAll: (selector) => selector === '[data-cut-clip="c1"]' ? [node] : [],
  }
  globalThis.MouseEvent = class {
    constructor(type, init) {
      this.type = type
      Object.assign(this, init)
    }
  }

  try {
    const browser = {
      execute: async (fn, ...args) => fn(...args),
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => {},
      defaultTimeout: () => 100,
    })

    await new Locator([{ selector: '[data-cut-clip="c1"]' }])
      .click({ button: 'right', force: true })

    assert.equal(received.type, 'contextmenu')
    assert.equal(received.clientX, 80)
    assert.equal(received.clientY, 70)
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
    if (previousMouseEvent === undefined) delete globalThis.MouseEvent
    else globalThis.MouseEvent = previousMouseEvent
  }
})

test('WebdriverIO locator preserves hover by not scrolling an already visible control', async () => {
  let scrollCalls = 0
  const node = {
    ...fakeNode('Accept'),
    getBoundingClientRect: () => ({
      x: 10,
      y: 20,
      left: 10,
      top: 20,
      right: 90,
      bottom: 50,
      width: 80,
      height: 30,
    }),
    scrollIntoView: () => { scrollCalls += 1 },
  }
  const previousDocument = globalThis.document
  globalThis.document = {
    documentElement: { clientWidth: 200, clientHeight: 200 },
    elementFromPoint: () => node,
    querySelectorAll: (selector) => selector === '[data-cut-action="accept-op"]' ? [node] : [],
  }

  try {
    const browser = {
      execute: async (fn, ...args) => fn(...args),
    }
    const Locator = createWebdriverIoLocatorFactory({
      browser,
      installInstrumentation: async () => {},
      drainEvents: async () => {},
      defaultTimeout: () => 100,
    })

    await new Locator([{ selector: '[data-cut-action="accept-op"]' }]).scrollIntoViewIfNeeded()

    assert.equal(scrollCalls, 0)
  } finally {
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
})
