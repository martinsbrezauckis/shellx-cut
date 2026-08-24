import assert from 'node:assert/strict'
import {
  normalizeProviderCatalog,
  normalizeProviderHits,
  preferredProvider,
  providerAllowsEmptyQuery,
  providerLabel,
  providerNeedsDirectory,
  safeExternalUrl,
} from '../src/panels/Stock/providerCatalog'
import {
  beginStockFetch,
  beginStockSearch,
  finishStockFetch,
  finishStockSearch,
  initialStockRequestState,
  invalidateStockRequests,
  isCurrentStockFetch,
  isCurrentStockSearch,
} from '../src/panels/Stock/requestState'

const catalog = normalizeProviderCatalog([
  { name: 'local_folder', kinds: ['audio', 'image', 'video'], needs_key: false, network: false, note: 'A selected folder.' },
  { name: 'stickers', kinds: ['image'], needs_key: false, network: false, note: 'Offline shapes.' },
  { name: 'openverse', kinds: ['audio', 'image'], needs_key: false, network: true, note: 'Creative Commons.' },
  { name: 'unknown_future_provider', kinds: ['video'], needs_key: false, network: true, note: 'Cannot build a schema-valid request.' },
  { name: 'nasa', kinds: ['not-a-kind'], needs_key: false, network: true, note: 'Malformed.' },
])

assert.deepEqual(catalog.map((provider) => provider.name), ['local_folder', 'stickers', 'openverse'])
assert.equal(preferredProvider(catalog)?.name, 'openverse')
assert.equal(providerLabel('archive_org'), 'Internet Archive')
assert.equal(providerNeedsDirectory('local_folder'), true)
assert.equal(providerNeedsDirectory('stickers'), false)
assert.equal(providerAllowsEmptyQuery('local_folder'), true)
assert.equal(providerAllowsEmptyQuery('stickers'), true)
assert.equal(providerAllowsEmptyQuery('nasa'), false)

const stickers = catalog.find((provider) => provider.name === 'stickers')
assert.ok(stickers)
const stickerHits = normalizeProviderHits([
  {
    provider: 'stickers', id: 'arrow_right', title: 'Arrow (right)', kind: 'image',
    license: 'cc0', license_url: 'https://creativecommons.org/publicdomain/zero/1.0/',
    source_url: null, attribution: '"Arrow (right)" — CC0', requires_attribution: false,
  },
  {
    provider: 'openverse', id: 'crossed-provider', title: 'Wrong source', kind: 'image',
    license: 'cc-by', attribution: 'Wrong source', requires_attribution: true,
  },
  {
    provider: 'stickers', id: 'bad-kind', title: 'Bad kind', kind: 'video',
    license: 'cc0', attribution: 'Bad kind', requires_attribution: false,
  },
], stickers)
assert.equal(stickerHits.length, 1, 'only a source-matching, source-supported hit is importable')
assert.equal(stickerHits[0]?.id, 'arrow_right')
assert.equal(safeExternalUrl('https://example.test/license'), 'https://example.test/license')
assert.equal(safeExternalUrl('javascript:alert(1)'), null)
assert.equal(safeExternalUrl('file:///private/source'), null)

// A source switch while an old request is still in flight must not leave the
// replacement surface disabled when that old promise settles later.
const firstSearch = beginStockSearch(initialStockRequestState)
const firstSearchEpoch = firstSearch.searchEpoch
const firstFetch = beginStockFetch(firstSearch, 'arrow_right')
assert.ok(firstFetch)
const firstFetchEpoch = firstFetch.fetchEpoch
const switchedProvider = invalidateStockRequests(firstFetch)
assert.equal(switchedProvider.searching, false, 'source switch immediately re-enables Search')
assert.equal(switchedProvider.fetchingId, null, 'source switch immediately clears Import busy state')
assert.equal(isCurrentStockSearch(switchedProvider, firstSearchEpoch), false, 'late search response is stale')
assert.equal(isCurrentStockFetch(switchedProvider, firstFetchEpoch), false, 'late import response is stale')
assert.equal(finishStockSearch(switchedProvider, firstSearchEpoch), switchedProvider, 'late search completion cannot overwrite new state')
assert.equal(finishStockFetch(switchedProvider, firstFetchEpoch), switchedProvider, 'late import completion cannot overwrite new state')

// Two hit handlers can run before React has had an opportunity to repaint all
// Import buttons. Admission happens synchronously in the controller, so only
// the first handler receives a request token and its successful finish remains
// the one the panel can publish as Added.
let singleFlight = initialStockRequestState
const firstImport = beginStockFetch(singleFlight, 'arrow_right')
assert.ok(firstImport)
singleFlight = firstImport
const secondImport = beginStockFetch(singleFlight, 'circle')
assert.equal(secondImport, null, 'a queued different-hit handler is refused before assets.fetch can dispatch')
const firstImportFinished = finishStockFetch(singleFlight, firstImport.fetchEpoch)
assert.equal(firstImportFinished.fetchingId, null, 'the admitted first import still finishes normally')
assert.equal(isCurrentStockFetch(firstImportFinished, firstImport.fetchEpoch), true, 'the admitted first completion remains current')

console.log('PASS stock provider catalog')
