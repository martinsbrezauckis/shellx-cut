import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { revealRegisteredSource } from '../src/lib/tauri'
import { sourceNavigationRequest } from '../src/app/sourceNavigation'

const host = globalThis as unknown as Record<string, unknown>
const priorWindow = Object.getOwnPropertyDescriptor(host, 'window')
const invokes: Array<{ command: string; args: Record<string, unknown> | undefined }> = []

try {
  Object.defineProperty(host, 'window', {
    configurable: true,
    value: {
      __TAURI__: {
        core: {
          invoke: async (command: string, args?: Record<string, unknown>) => {
            invokes.push({ command, args })
            return { status: 'revealed', message: 'Revealed the registered source file in File Explorer' }
          },
        },
        event: { listen: async () => () => {} },
      },
    },
  })
  assert.deepEqual(await revealRegisteredSource(' asset-registered '), {
    status: 'revealed', message: 'Revealed the registered source file in File Explorer',
  })
  assert.deepEqual(invokes, [{
    command: 'reveal_registered_source',
    args: { assetId: 'asset-registered' },
  }], 'the renderer sends only the registered asset identity')

  Object.defineProperty(host, 'window', { configurable: true, value: {} })
  const browser = await revealRegisteredSource('asset-registered')
  assert.deepEqual(browser, {
    status: 'refused', message: 'Open the desktop app to reveal the registered source file',
  })
  assert.equal(invokes.length, 1, 'browser refusal must not invoke any native command')
} finally {
  if (priorWindow) Object.defineProperty(host, 'window', priorWindow)
  else delete host.window
}

assert.deepEqual(sourceNavigationRequest({ destination: 'project', assetId: ' asset-registered ' }), {
  destination: 'project', assetId: 'asset-registered',
})
assert.equal(sourceNavigationRequest({ destination: 'file', assetId: 'asset-registered' }), null)
assert.equal(sourceNavigationRequest({ destination: 'library', assetId: '' }), null)

const sourceMonitor = readFileSync(new URL('../src/panels/Assets/SourceMonitor.tsx', import.meta.url), 'utf8')
const clipMenu = readFileSync(new URL('../src/panels/Timeline/ClipContextMenuSections.tsx', import.meta.url), 'utf8')
const assets = readFileSync(new URL('../src/panels/Assets/index.tsx', import.meta.url), 'utf8')
const libraryCard = readFileSync(new URL('../src/panels/Library/LibraryCard.tsx', import.meta.url), 'utf8')
for (const action of ['reveal-source-project', 'reveal-source-library', 'reveal-source-file']) {
  assert.match(sourceMonitor, new RegExp(`data-cut-action="${action}"`), `Source Monitor exposes ${action}`)
  assert.match(clipMenu, new RegExp(`data-cut-action="${action}"`), `timeline context menu exposes ${action}`)
}
assert.match(assets, /data-cut-asset-selected/, 'Project Assets has a visible exact reveal marker')
assert.match(libraryCard, /data-cut-library-card/, 'Library reveal targets its existing keyboard-aware cards')

console.log('PASS identity-only source navigation and browser refusal contract')
