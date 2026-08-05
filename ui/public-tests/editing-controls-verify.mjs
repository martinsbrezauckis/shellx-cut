// editing-controls-verify.mjs — runtime verification of the core editing
// editing-control batch on a LIVE cutd + real browser. The rig-portable RESULT gate
// for the eight features: every check drives the real UI (or the verb) and
// asserts the effect in project state / DOM / render output — never ok:true.
//
// Run (any rig):  CUTD_ADDR=127.0.0.1:7801 CUT_REPO=$HOME/shellx-cut \
//                   node ui/public-tests/editing-controls-verify.mjs
// Requires: a running cutd serving ui/dist at CUTD_ADDR, testdata/ in
// CUT_REPO, playwright in ui/node_modules. Creates + leaves one throwaway
// project named editing-controls-verify-<ts> (cheap; delete via project.delete).
// Exit 0 = all PASS. Callers: rig passes (Linux / Windows CDP / macOS),
// release checklists.

import { chromium } from 'playwright'

const ADDR = process.env.CUTD_ADDR ?? '127.0.0.1:6161'
const REPO = process.env.CUT_REPO ?? `${process.env.HOME}/shellx-cut`
// Installed-app mode: CUT_CDP_URL attaches to a running shell (WebView2/CDP)
// instead of launching a headless chromium; CUT_CLIP overrides the import path
// (the SERVER resolves it — on Windows pass a C:/ path the installed cutd can read).
const CDP = process.env.CUT_CDP_URL ?? ''
const CLIP = process.env.CUT_CLIP ?? `${REPO}/testdata/silent_screen.mp4`
const results = []
const check = (n, ok, note = '') => { results.push(ok); console.log(`${ok ? 'PASS' : 'FAIL'} ${n}${note ? ' — ' + note : ''}`) }
const verb = (name, args = {}) =>
  fetch(`http://${ADDR}/api/verb/${name}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(args) }).then((r) => r.json())
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const state = async () => (await verb('project.state', {})).result
const intersectionArea = (a, b) => {
  if (!a || !b) return null
  const width = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x))
  const height = Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y))
  return width * height
}
const srcOf = async (id) => {
  for (const t of (await state()).tracks) for (const c of t.clips) if (c.id === id) return [c.src_in_ms, c.src_out_ms]
  return null
}

// ---------- setup: project + clip + 3-way split -----------------------------
const proj = await verb('project.create', { name: `editing-controls-verify-${Date.now()}`, settings: { width: 1280, height: 720, fps: 30 } })
if (!proj.ok) throw new Error('project.create failed: ' + JSON.stringify(proj.error))
await verb('media.import', { path: CLIP })
let vt = null
for (let i = 0; i < 120; i++) {
  vt = (await state()).tracks.find((t) => t.kind === 'video')
  if (vt?.clips?.length) break
  await sleep(500)
}
if (!vt?.clips?.length) throw new Error('import never placed a clip')
await verb('edit.trim', { clip: vt.clips[0].id, src_in_ms: 2000, src_out_ms: 18000 })
await verb('edit.split', { track: vt.id, at_ms: 5000 })
await verb('edit.split', { track: vt.id, at_ms: 10000 })
const ids = (await state()).tracks.find((t) => t.id === vt.id).clips.map((c) => c.id)
check('setup: 3 clips staged', ids.length === 3, ids.join(','))
const [src, mid, last] = ids

let browser, page
if (CDP) {
  browser = await chromium.connectOverCDP(CDP)
  const ctx = browser.contexts()[0]
  page = ctx.pages().find((pg) => /127\.0\.0\.1:\d+/.test(pg.url())) || ctx.pages()[0]
  await page.goto(`http://${ADDR}/`, { waitUntil: 'networkidle' }) // fresh state
} else {
  browser = await chromium.launch()
  page = await browser.newPage({ viewport: { width: 1500, height: 950 } })
  await page.goto(`http://${ADDR}/`, { waitUntil: 'networkidle' })
}
await page.waitForSelector(`[data-cut-clip="${mid}"]`, { timeout: 20000 })

// ---------- marquee -------------------------------------------------------
{
  const first = await page.locator(`[data-cut-clip="${src}"]`).boundingBox()
  const third = await page.locator(`[data-cut-clip="${last}"]`).boundingBox()
  const laneBottom = Math.max(first.y + first.height, third.y + third.height)
  await page.mouse.move(third.x + third.width - 4, laneBottom + 40)
  await page.mouse.down()
  await page.mouse.move(first.x + 4, first.y + first.height / 2, { steps: 8 })
  check('marquee rectangle visible mid-drag', (await page.locator('[data-cut-marquee]').count()) === 1)
  await page.mouse.up()
  await sleep(300)
  check('marquee live-selected the crossed clips', (await page.locator('.tl-clip--selected').count()) >= 3)
  check('marquee cleared on release', (await page.locator('[data-cut-marquee]').count()) === 0)
}

// ---------- trim popover + Alt+arrow slip --------------------------------
{
  await page.locator(`[data-cut-clip="${mid}"]`).click()
  await sleep(250)
  await page.locator(`[data-cut-clip="${mid}"]`).click({ button: 'right' })
  await page.locator('[data-cut-ctx="trim-tools"]').click()
  await page.waitForSelector('[data-cut-trim-popover]', { timeout: 5000 })
  check('trim popover opens (3 rows)', (await page.locator('[data-cut-trim-row]').count()) === 3)
  const pre = await srcOf(mid)
  await page.locator('[data-cut-trim-step="slip:10"]').click()
  await sleep(700)
  const post = await srcOf(mid)
  check('popover slip +10f = +330ms', post[0] === pre[0] + 330 && post[1] === pre[1] + 330, `Δ=${post[0] - pre[0]}`)
  const rollPre = await srcOf(mid)
  await page.locator('[data-cut-trim-step="roll:1"]').click()
  await sleep(700)
  check('popover roll +1f moved this clip\'s out-point', (await srcOf(mid))[1] === rollPre[1] + 33)
  await page.locator('[data-cut-trim-close]').click()
  await sleep(200)
  const pre2 = await srcOf(mid)
  // clear the marquee's multi-selection first — clicking an already-selected
  // clip KEEPS the whole selection (NLE convention), and the Alt+arrow slip
  // guard requires exactly ONE selected clip.
  await page.keyboard.press('Escape')
  await sleep(150)
  await page.locator(`[data-cut-clip="${mid}"]`).click()
  await sleep(250)
  await page.keyboard.press('Alt+ArrowRight')
  await sleep(700)
  check('Alt+→ slips +1 frame', (await srcOf(mid))[0] === pre2[0] + 33)
  // REST slide sanity (UI steppers share the same verb path)
  const a1 = await srcOf(src), c1 = await srcOf(last)
  const sd = await verb('edit.slide_edit', { clip: mid, by_ms: 300 })
  check('slide_edit ok + neighbors absorbed', sd.ok && (await srcOf(src))[1] === a1[1] + 300 && (await srcOf(last))[0] === c1[0] + 300)
}

// ---------- paste attributes ----------------------------------------------
{
  await verb('edit.grade', { clip: src, contrast: 1.25, saturation: 0.8 })
  await verb('edit.speed', { clip: src, factor: 1.5 })
  // single-select the source (Ctrl+C warns + no-ops on a multi-selection)
  await page.keyboard.press('Escape')
  await sleep(150)
  await page.locator(`[data-cut-clip="${src}"]`).click()
  await sleep(250)
  await page.keyboard.press('Control+c')
  await sleep(250)
  await page.locator(`[data-cut-clip="${last}"]`).click()
  await sleep(250)
  await page.keyboard.press('Control+Alt+v')
  await page.waitForSelector('[data-cut-paste-attributes]', { timeout: 5000 })
  check('Ctrl+Alt+V opens the dialog (5 checkboxes)', (await page.locator('[data-cut-pa-check]').count()) === 5)
  await page.locator('[data-cut-pa-check="effects"]').uncheck()
  await page.locator('[data-cut-pa-apply]').click()
  await sleep(1500)
  const st = await state()
  const tgt = st.tracks.flatMap((t) => t.clips).find((c) => c.id === last)
  check('apply landed grade+speed on the target', tgt?.grade?.contrast === 1.25 && tgt?.speed === 1.5, JSON.stringify({ g: tgt?.grade?.contrast, s: tgt?.speed }))
}

// ---------- default transition (Ctrl/Cmd+T) -------------------------------
{
  const opsBefore = (await verb('project.ops', {})).result.ops.length
  await page.locator(`[data-cut-clip="${src}"]`).click() // seam src→mid is nearest
  await sleep(250)
  await page.keyboard.press('Control+t')
  await sleep(900)
  const ops = (await verb('project.ops', {})).result.ops
  const xf = ops.slice(opsBefore).find((o) => o.verb === 'edit.crossfade')
  check('Ctrl+T applied the 500ms dissolve', xf?.args?.duration_ms === 500 && xf?.args?.transition === 'dissolve', JSON.stringify(xf?.args ?? {}))
}

// ---------- marker rename + color -----------------------------------------
{
  await verb('edit.add_marker', { at_ms: 1500, label: 'rig check' })
  await sleep(600)
  const mk = (await state()).markers.find((m) => m.label === 'rig check')
  await page.locator(`[data-cut-marker="${mk.id}"]`).click({ button: 'right' })
  await page.waitForSelector('[data-cut-marker-menu]', { timeout: 5000 })
  await page.locator('[data-cut-marker-color-swatch="teal"]').click()
  await sleep(700)
  check('swatch click landed color=teal', (await state()).markers.find((m) => m.id === mk.id)?.color === 'teal')
  await page.locator(`[data-cut-marker="${mk.id}"]`).click({ button: 'right' })
  const input = page.locator('[data-cut-marker-rename-input]')
  await input.fill('rig renamed')
  await input.press('Enter')
  await sleep(700)
  check('rename via menu landed', (await state()).markers.find((m) => m.id === mk.id)?.label === 'rig renamed')
  await page.locator(`[data-cut-marker="${mk.id}"]`).click({ button: 'right' })
  const noteInput = page.locator('[data-cut-marker-note-input]')
  await noteInput.fill('watch the beat cut')
  await page.locator('[data-cut-marker-ctx="note-commit"]').click()
  await sleep(700)
  check('note via menu landed', (await state()).markers.find((m) => m.id === mk.id)?.note === 'watch the beat cut')
}

// ---------- mixer pan ------------------------------------------------------
{
  await page.locator('[data-cut-mixer-btn]').click()
  await page.waitForSelector('[data-cut-mixer-pan]', { timeout: 8000 })
  const slider = page.locator('[data-cut-mixer-pan]').first()
  const tid = await slider.getAttribute('data-cut-mixer-pan')
  await slider.evaluate((el) => {
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set
    set.call(el, '-0.4')
    el.dispatchEvent(new Event('input', { bubbles: true }))
    el.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))
  })
  await sleep(800)
  const t = (await state()).tracks.find((t) => t.id === tid)
  check('pan slider landed pan=-0.4 in state', t?.pan === -0.4, `pan=${t?.pan}`)
  await page.keyboard.press('Escape')
}

// ---------- fullscreen + guides ----------------------------------------
{
  const toolsStrip = page.locator('[data-cut-action="expand-rail"]')
  const transport = page.locator('[data-cut-transport]')
  const fullscreen = page.locator('[data-cut-action="fullscreen-toggle"]')
  const stripBox = await toolsStrip.boundingBox()
  const transportBox = await transport.boundingBox()
  const fullscreenBox = await fullscreen.boundingBox()
  const transportOverlap = intersectionArea(stripBox, transportBox)
  const fullscreenOverlap = intersectionArea(stripBox, fullscreenBox)
  check('collapsed Tools strip stays outside preview transport', transportOverlap === 0, `overlap=${transportOverlap}px²`)
  check('collapsed Tools strip stays outside Full Screen control', fullscreenOverlap === 0, `overlap=${fullscreenOverlap}px²`)
  const gbtn = page.locator('[data-cut-action="cycle-guides"]')
  while ((await gbtn.getAttribute('data-cut-guides')) !== 'both') await gbtn.click()
  check('guides render (4 lines + 2 safe rects)', (await page.locator('.pv-guides line').count()) === 4 && (await page.locator('.pv-guides rect').count()) === 2)
  while ((await gbtn.getAttribute('data-cut-guides')) !== 'off') await gbtn.click()
  await fullscreen.click()
  await sleep(300)
  check('fullscreen targets the preview panel', await page.evaluate(() => document.fullscreenElement?.classList?.contains('pv-root') ?? false))
  await fullscreen.click()
  await sleep(300)
  check('toggle exits + attr honest', (await page.locator('[data-cut-panel="preview"]').getAttribute('data-cut-fullscreen')) === 'false')
}

// ---------- additional engine results and UI surfaces ---------
// Engine checks assert real state/results (the per-feature deep interaction
// proofs ran at feature time); the later checks assert the live UI surface engages.
{
  // offline report — the staged source must be online + referenced.
  const chk = await verb('media.check', {})
  check('media.check reports the source online', chk.ok && chk.result?.offline_count === 0 && (chk.result?.assets ?? []).some((a) => a.exists && a.referenced > 0), `offline=${chk.result?.offline_count}`)

  // non-destructive mute — SOURCE-time range lands in state, clear removes;
  // when the rig clip has no audio stream, the video-clip REFUSAL is the check.
  const at = (await state()).tracks.find((t) => t.kind === 'audio' && t.clips.some((c) => c.asset))
  if (at) {
    const ac = at.clips.find((c) => c.asset)
    const add = await verb('edit.mute_range', { clip: ac.id, range_ms: [ac.src_in_ms + 200, ac.src_in_ms + 700] })
    const got = (await state()).tracks.find((t) => t.id === at.id).clips.find((c) => c.id === ac.id)
    const cleared = await verb('edit.mute_range', { clip: ac.id, clear: true })
    check('mute_range add lands in state + clear removes', add.ok && (got.mute_ranges ?? []).length === 1 && cleared.ok, JSON.stringify(got.mute_ranges))
  } else {
    const ref = await verb('edit.mute_range', { clip: mid, range_ms: [0, 500] })
    check('mute_range machinery live (video-clip refusal — clip has no audio)', !ref.ok && ref.error?.code === 'invalid_args', ref.error?.code)
  }

  // smart bins — save, LIVE membership, delete.
  await verb('media.bin_save', { name: 'rig bin', kind: 'video' })
  const bl = await verb('media.bin_list', {})
  const bin = (bl.result?.bins ?? []).find((b) => b.name === 'rig bin')
  const bd = await verb('media.bin_delete', { name: 'rig bin' })
  check('smart bin lists live members + deletes', !!bin && bin.match_count >= 1 && bd.ok, `matches=${bin?.match_count}`)

  // caption style gallery — 6 built-ins; apply creates the target style key.
  const ls = await verb('captions.list_styles', {})
  const ap = await verb('captions.apply_style', { name: 'broadcast yellow', ref: 'rig_style' })
  const st4 = await state()
  check('caption gallery: built-ins + apply lands in state', (ls.result?.presets ?? []).filter((p) => p.builtin).length === 6 && ap.ok && st4.caption_styles?.rig_style?.color === '#ffe14d', `color=${st4.caption_styles?.rig_style?.color}`)

  // trim tool — the T key engages it (toolbar + scroller attr), Esc resets.
  await page.keyboard.press('Escape')
  await sleep(200)
  await page.keyboard.press('t')
  await sleep(300)
  check('trim tool engages via T', (await page.locator('[data-cut-tool="trim"]').getAttribute('data-cut-trim-tool')) === 'slip' && (await page.locator('[data-cut-trimtool="slip"]').count()) === 1)
  await page.keyboard.press('Escape')
  await sleep(200)

  // hover-scrub thumbnail — the Assets card renders the filmstrip strip.
  await page.locator('[data-cut-left-tab="assets"]').click()
  await sleep(500)
  const thumb = page.locator('[data-cut-asset-thumb]').first()
  const thumbBg = (await thumb.count()) ? await thumb.evaluate((el) => getComputedStyle(el).backgroundImage) : ''
  check('hover-scrub thumbnail renders the strip', /filmstrip\//.test(thumbBg), thumbBg.slice(0, 60))

  // keymap editor — mounts in Settings with every remappable action listed.
  await verb('ui.open', { panel: 'environment' })
  await page.locator('[data-cut-settings-category="editing"]').click()
  await page.waitForSelector('[data-cut-keymap-editor]', { timeout: 8000 })
  check('keymap editor mounts with all actions', (await page.locator('[data-cut-keymap-row]').count()) >= 15)
  await page.locator('[data-cut-environment-close]').click()
  await sleep(300)

  // Dedicated Library → explicit Insert at playhead. The workspace replaces
  // the old narrow tab, so timeline drag cannot be the primary path while the
  // timeline is intentionally hidden. Seed one item, drive the visible Insert
  // action, assert clips grow, then clean up the Library entry.
  {
    const added = await verb('library.add', { path: CLIP, name: 'ts-gate-library-insert' })
    const libId = added.result?.item?.id
    await page.locator('[data-cut-library-btn]').click()
    await page.waitForSelector('[data-cut-library-workspace]', { timeout: 8000 })
    await page.waitForSelector(`[data-cut-library-card="${libId}"]`, { timeout: 8000 })
    const before = ((await verb('project.state', {})).result?.tracks ?? []).flatMap((t) => t.clips ?? []).length
    await page.locator(`[data-cut-library-insert="${libId}"]`).click()
    let after = before
    for (let i = 0; i < 30; i++) {
      await sleep(1000)
      after = ((await verb('project.state', {})).result?.tracks ?? []).flatMap((t) => t.clips ?? []).length
      if (after > before) break
    }
    check('Library inserts at the playhead', after > before, `clips ${before}->${after}`)
    if (libId) await verb('library.remove', { id: libId })
    await page.locator('[data-cut-library-close]').click()
    // Drain the dropped clip's import chain before the render check: on the
    // Windows rig, render.final racing a live import chain lost a segrender
    // segment at concat. This gate models a completed import rather than that race,
    // matching a user whose import has finished.
    for (let i = 0; i < 90; i++) {
      const jl = await verb('jobs.list', {})
      const active = (jl.result?.jobs ?? []).filter((j) => j.state === 'queued' || j.state === 'running')
      if (!active.length) break
      await sleep(1000)
    }
    await sleep(300)
  }
}

// ---------- real render through the edited timeline --------------------------
{
  const r = await verb('render.final', {})
  let done = false, out = null
  if (r.ok && r.result?.job_id) {
    for (let i = 0; i < 300; i++) {
      const s = await verb('jobs.status', { job_id: r.result.job_id })
      const res = s.result ?? {}
      if (res.progress === 1 || res.state === 'done') { done = true; out = res.result ?? {}; break }
      if (res.state === 'failed') { out = res; break }
      await sleep(1000)
    }
  }
  check('render.final completes on the edited timeline', done && !!out?.path, (out?.path ?? JSON.stringify(out ?? {})).slice(-60))
}

await browser.close()
const fails = results.filter((x) => !x).length
console.log(`\nediting-controls-verify: ${results.length - fails}/${results.length} PASS, ${fails} FAIL`)
process.exit(fails ? 1 : 0)
