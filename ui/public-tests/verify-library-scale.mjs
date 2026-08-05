// verify-library-scale.mjs — deterministic browser proof that the dedicated
// Library workspace renders one bounded page at 1k and 10k collection sizes.
//
// The test intercepts the UI's verb transport with a faithful in-memory
// library.list implementation. It measures the real production React surface,
// including lazy chunk load, and exercises native keyboard paging plus
// filter→page reset. This is UI scalability evidence, not installed-engine I/O
// proof; the Rust query/page helpers have separate unit coverage.
//
// RUN: SWEEP_APP=http://127.0.0.1:5208 node public-tests/verify-library-scale.mjs

import { chromium } from 'playwright'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const APP = process.env.SWEEP_APP || 'http://127.0.0.1:5208'
const HERE = dirname(fileURLToPath(import.meta.url))
const OUTPUT = join(HERE, '..', '..', 'output', 'playwright')
const PAGE_SIZE = 100
let catalog = []
const listRequests = []

function makeCatalog(total) {
  return Array.from({ length: total }, (_, index) => ({
    id: `asset-${String(index).padStart(5, '0')}`,
    type: index % 3 === 0 ? 'video' : index % 3 === 1 ? 'audio' : 'image',
    name: `asset-${String(index).padStart(5, '0')}.mp4`,
    src_path: `/missing/asset-${index}.mp4`,
    tags: index % 10 === 0 ? ['favorite-ten'] : [],
    favorite: index % 10 === 0,
    added_ms: total - index,
    source: 'user',
    media_ok: true,
  }))
}

function listResult(args) {
  let matches = catalog
  if (Array.isArray(args.ids)) {
    const wanted = new Set(args.ids)
    matches = matches.filter((item) => wanted.has(item.id))
  }
  if (args.type) matches = matches.filter((item) => item.type === args.type)
  if (args.tag) matches = matches.filter((item) => item.tags.includes(args.tag))
  if (args.q) {
    const needle = String(args.q).toLowerCase()
    matches = matches.filter((item) => (
      item.name.toLowerCase().includes(needle)
      || item.tags.some((tag) => tag.toLowerCase().includes(needle))
    ))
  }
  if (args.collection === 'favorites') matches = matches.filter((item) => item.favorite)
  if (args.collection === 'missing') matches = matches.filter((item) => !item.media_ok)
  const offset = Number.isInteger(args.offset) ? args.offset : 0
  const limit = Number.isInteger(args.limit) ? args.limit : PAGE_SIZE
  const items = matches.slice(offset, offset + limit)
  return {
    items,
    folders: [],
    tags: ['favorite-ten'],
    total: matches.length,
    offset,
    limit,
    next_offset: offset + items.length < matches.length ? offset + items.length : null,
  }
}

async function openCatalog(page, total) {
  console.log(`opening ${total.toLocaleString()}-item catalog`)
  catalog = makeCatalog(total)
  listRequests.length = 0
  await page.reload({ waitUntil: 'domcontentloaded' })
  await page.locator('[data-cut-library-btn]').waitFor({ state: 'visible', timeout: 10_000 })
  const started = performance.now()
  await page.locator('[data-cut-library-btn]').click()
  console.log(`clicked Library for ${total.toLocaleString()}`)
  const status = page.locator('[data-cut-library-page-status]')
  await status.waitFor({ state: 'visible', timeout: 10_000 })
  await status.filter({ hasText: `of ${total}` }).waitFor({ state: 'visible', timeout: 10_000 })
  await page.locator('[data-cut-library-card]').nth(PAGE_SIZE - 1).waitFor({ state: 'visible', timeout: 10_000 })
  console.log(`rendered first ${PAGE_SIZE} of ${total.toLocaleString()}`)
  if (total === 10_000) {
    mkdirSync(OUTPUT, { recursive: true })
    await page.screenshot({
      path: join(OUTPUT, 'library-workspace-paged-1100x680.png'),
      fullPage: false,
    })
  }
  const elapsedMs = Math.round(performance.now() - started)
  const cardCount = await page.locator('[data-cut-library-card]').count()
  const heapBytes = await page.evaluate(() => (
    /** @type {{memory?: {usedJSHeapSize?: number}}} */ (performance).memory?.usedJSHeapSize ?? null
  ))
  const firstStatus = await status.textContent()
  const previous = page.locator('[data-cut-library-page-prev]')
  const next = page.locator('[data-cut-library-page-next]')
  const boundary = {
    previousDisabled: await previous.isDisabled(),
    nextDisabled: await next.isDisabled(),
  }

  const firstCard = page.locator('[data-cut-library-card]').first()
  await firstCard.focus()
  await page.keyboard.press('ArrowDown')
  const arrowDownId = await page.evaluate(() => (
    document.activeElement?.getAttribute('data-cut-library-card')
  ))
  await page.keyboard.press('End')
  const endId = await page.evaluate(() => (
    document.activeElement?.getAttribute('data-cut-library-card')
  ))
  await page.keyboard.press('Home')
  const homeId = await page.evaluate(() => (
    document.activeElement?.getAttribute('data-cut-library-card')
  ))
  const favorite = page.locator('[data-cut-library-fav]').first()
  await favorite.focus()
  await page.keyboard.press('ArrowDown')
  const childControlStayedFocused = await favorite.evaluate((element) => document.activeElement === element)

  await next.focus()
  await page.keyboard.press('Enter')
  await status.filter({ hasText: `101–200 of ${total}` }).waitFor({ state: 'visible', timeout: 5_000 })
  await page.waitForFunction(
    () => document.querySelector('[data-cut-library-card]')?.getAttribute('data-cut-library-card') === 'asset-00100',
    null,
    { timeout: 5_000 },
  )
  const secondPageFirst = await page.locator('[data-cut-library-card]').first().getAttribute('data-cut-library-card')
  await previous.focus()
  await page.keyboard.press('Enter')
  await status.filter({ hasText: `1–100 of ${total}` }).waitFor({ state: 'visible', timeout: 5_000 })

  const maxRequestedLimit = Math.max(...listRequests.map((args) => args.limit ?? PAGE_SIZE))
  return {
    total,
    elapsedMs,
    heapBytes,
    cardCount,
    firstStatus,
    secondPageFirst,
    boundary,
    keyboard: { arrowDownId, endId, homeId, childControlStayedFocused },
    maxRequestedLimit,
  }
}

async function main() {
  const browser = await chromium.launch({ args: ['--enable-precise-memory-info'] })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  page.on('pageerror', (error) => console.error(`PAGE ERROR: ${error.message}`))
  await page.route('**/api/library-poster**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="9"><rect width="16" height="9" fill="#20242c"/></svg>',
    })
  })
  await page.route('**/api/verb/**', async (route) => {
    const name = new URL(route.request().url()).pathname.split('/').pop()
    let args = {}
    try {
      args = route.request().postDataJSON() ?? {}
    } catch {
      // Empty/non-JSON requests are treated as the verb's default arguments.
    }
    // Keep the non-Library boot responses faithful enough for the production
    // shell to settle. Returning a generic object here used to make App.resync
    // treat project.ops as present and then call `.map` on an absent `ops`
    // array; the resulting remount race could masquerade as a Library scale
    // failure before the workspace ever opened.
    let result = null
    if (name === 'library.list') {
      listRequests.push(args)
      result = listResult(args)
    } else if (name === 'project.state') {
      result = null
    } else if (name === 'project.list') {
      result = { projects: [] }
    } else if (name === 'project.ops') {
      result = { ops: [] }
    } else if (name === 'jobs.list') {
      result = { jobs: [] }
    } else if (name === 'system.doctor') {
      result = {
        schema: 'shellx-cut/doctor/1',
        scanned_at: new Date(0).toISOString(),
        os: 'test',
        arch: 'test',
        app_version: '0.6.105',
        cards: [],
        essential_ok: true,
      }
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ok: true, result }),
    })
  })
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  console.log(`loaded ${APP}`)
  await page.locator('[data-cut-library-btn]').waitFor({ state: 'visible', timeout: 10_000 })

  const evidence = [await openCatalog(page, 1_000), await openCatalog(page, 10_000)]

  // From page two, changing a filter must query offset 0 and show the one exact
  // 10k-tail match—proof that filter changes cannot strand users on an empty page.
  await page.locator('[data-cut-library-page-next]').click()
  await page.locator('[data-cut-library-page-status]').filter({ hasText: '101–200 of 10000' })
    .waitFor({ state: 'visible', timeout: 5_000 })
  await page.locator('[data-cut-library-search]').fill('asset-09999')
  await page.locator('[data-cut-library-page-status]').filter({ hasText: '1–1 of 1' })
    .waitFor({ state: 'visible', timeout: 5_000 })
  await page.waitForFunction(
    () => document.querySelector('[data-cut-library-card]')?.getAttribute('data-cut-library-card') === 'asset-09999',
    null,
    { timeout: 5_000 },
  )
  const filteredId = await page.locator('[data-cut-library-card]').getAttribute('data-cut-library-card')
  const finalRequest = listRequests.at(-1)

  const failures = []
  for (const row of evidence) {
    if (row.cardCount !== PAGE_SIZE) failures.push(`${row.total}: rendered ${row.cardCount}, expected ${PAGE_SIZE}`)
    if (!row.boundary.previousDisabled || row.boundary.nextDisabled) failures.push(`${row.total}: wrong first-page button boundaries`)
    if (row.secondPageFirst !== 'asset-00100') failures.push(`${row.total}: keyboard Next reached ${row.secondPageFirst}`)
    if (row.keyboard.arrowDownId !== 'asset-00001') failures.push(`${row.total}: ArrowDown reached ${row.keyboard.arrowDownId}`)
    if (row.keyboard.endId !== 'asset-00099') failures.push(`${row.total}: End reached ${row.keyboard.endId}`)
    if (row.keyboard.homeId !== 'asset-00000') failures.push(`${row.total}: Home reached ${row.keyboard.homeId}`)
    if (!row.keyboard.childControlStayedFocused) failures.push(`${row.total}: row navigation hijacked a child button`)
    if (row.maxRequestedLimit > PAGE_SIZE) failures.push(`${row.total}: UI requested limit ${row.maxRequestedLimit}`)
    if (row.elapsedMs > 5_000) failures.push(`${row.total}: first page took ${row.elapsedMs}ms`)
  }
  if (filteredId !== 'asset-09999' || finalRequest?.offset !== 0) {
    failures.push(`filter reset: id=${filteredId}, offset=${finalRequest?.offset}`)
  }

  const report = {
    generated_at: new Date().toISOString(),
    evidence_kind: 'production UI with deterministic mocked library.list',
    page_size: PAGE_SIZE,
    evidence,
    filter_reset: { filteredId, requestOffset: finalRequest?.offset ?? null },
    failures,
  }
  mkdirSync(OUTPUT, { recursive: true })
  writeFileSync(join(OUTPUT, 'library-scale-paging.json'), `${JSON.stringify(report, null, 2)}\n`)
  console.log(JSON.stringify(report, null, 2))
  await browser.close()
  process.exit(failures.length ? 1 : 0)
}

main().catch((error) => {
  console.error(error)
  process.exit(2)
})
