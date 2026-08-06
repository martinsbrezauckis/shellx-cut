// ui/public-tests/clip-context-menu.test.ts — clip right-click menu class gates
// (run: `npm run test:lib`, or `tsx public-tests/clip-context-menu.test.ts`).
//
// Red-proof for three 0.6.106 hotfix defects from the 2026-08-06 context-menu
// audit (release-studio page-build/CONTEXT_MENU_AUDIT_2026-08-06.md), all
// proven failing against the pre-fix ClipContextMenu before the gates landed:
//
// 1. DETACH-AUDIO GATE. "Detach audio" rendered on EVERY video clip — stills
//    and silent clips included, where edit.detach_audio can only reject
//    (cut_core DetachReject::NoAudio). Contract: the entry renders iff the
//    clip's ASSET carries an audio stream (probe.has_audio — the exact fact
//    the engine's planner accepts on, cut_core::plan_detach_audio). NOT gated
//    on a linked timeline sibling: the verb's primary case is a video clip
//    whose asset HAS audio but whose audio is not on the timeline yet (plain
//    edit.insert → silent render); a sibling-exists gate would hide the entry
//    exactly there. Both sides are pinned below.
//
// 2. TITLE/SHAPE CLASS FILTER. layout.ts kinds every non-audio clip 'video',
//    so a title card got the full ~30-entry media menu (Stabilize / Blur
//    faces / Speed / J-L cuts on rendered text). Contract: clips on a
//    `title*`-named track (the Inspector's own title/shape routing convention)
//    keep only the entries whose verbs genuinely apply to a rendered overlay
//    (clipboard / split / trim / remove / transform / transition / fades /
//    remove-track) and never render the footage-class entries.
//
// 3. ADD-TRANSITION ADJACENCY IN EDITORIAL TIME. The enable-gate tested LAID
//    adjacency while the action (crossfadeAdjacent, c68b449c) resolves the
//    seam in EDITORIAL time — on a seam that already has a crossfade the
//    neighbour's laid start sits inside this clip's tail, so the menu showed
//    the entry disabled even though the action would succeed. Contract: the
//    gate mirrors crossfadeAdjacent's editorial-neighbour scan exactly.
//
// Plus the c68b449c disclosure nit: "Remove clip" also removes the exact
// linked audio counterpart — the tooltip must SAY so when (and only when)
// that propagation will happen.
//
// Method: render the real component via react-dom/server (same discipline as
// grade-drawer.test.ts) and assert on data-cut-ctx entry presence + the
// disabled attribute in the produced markup. No CSS import exists in the
// component chain today; the loader hook stays registered so a future CSS
// import cannot break the suite.

import { strict as assert } from 'node:assert'
import { registerHooks } from 'node:module'

registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith('.css')) return { format: 'module', source: 'export default {}', shortCircuit: true }
    return nextLoad(url, context)
  },
})

const { createElement } = await import('react')
const { renderToStaticMarkup } = await import('react-dom/server')
const { default: ClipContextMenu } = await import('../src/panels/Timeline/ClipContextMenu')
type Project = import('../src/lib/client').Project
type LaidItem = import('../src/panels/Timeline/layout').LaidItem

// ---- fixtures --------------------------------------------------------------

/** Project with a base video track, a title overlay track, and a base audio
 * track. Assets cover every detach-gate class: muxed (has_audio true), silent
 * video, a still image, and a title render (title_shape.rs probes its .mov
 * with has_audio:false). */
const project = {
  tracks: [
    { id: 'v1', kind: 'video', clips: [] },
    { id: 'title1', kind: 'video', clips: [] },
    { id: 'a1t', kind: 'audio', clips: [] },
  ],
  assets: {
    muxed: { path: '/m/muxed.mp4', probe: { kind: 'video', has_audio: true, has_video: true } },
    silent: { path: '/m/silent.mp4', probe: { kind: 'video', has_audio: false, has_video: true } },
    still: { path: '/m/photo.jpg', probe: { kind: 'image' } },
    titlemov: { path: '/m/title1.mov', probe: { kind: 'video', has_audio: false, has_video: true } },
  },
} as unknown as Project

/** Media LaidItem shorthand (laid == editorial unless stated). */
function clip(
  id: string, kind: 'video' | 'audio', trackId: string, asset: string,
  startMs: number, durMs: number, extra: Partial<LaidItem> = {},
): LaidItem {
  return {
    id, kind, trackId, startMs, durMs,
    editorialStartMs: startMs, label: asset, asset,
    srcInMs: 0, srcOutMs: durMs, ...extra,
  }
}

const noop = () => {}
function renderMenu(allItems: LaidItem[], itemId: string, atMs = 100): string {
  return renderToStaticMarkup(createElement(ClipContextMenu, {
    menu: { x: 0, y: 0, itemId, atMs },
    project,
    allItems,
    selectedClipIds: [],
    assetPick: null,
    setAssetPick: noop,
    clipboardHasContent: false,
    onClose: noop,
    onCopyClip: () => true,
    onCutClip: noop,
    onPasteClip: noop,
    onPasteAttributes: noop,
    onOpenTrim: noop,
    onSelect: noop,
    onSeek: noop,
    removeItemById: noop,
    removeTrackById: noop,
    splitItemAt: noop,
    fadeItem: noop,
    trimItemTo: noop,
    reverseItem: noop,
    freezeItem: noop,
    stabilizeItem: noop,
    speedItem: noop,
    crossfadeAdjacent: noop,
    muteItem: noop,
    cleanVoiceItem: noop,
    blurFacesItem: noop,
    detachAudioItem: noop,
    splitEditItem: noop,
    replaceClipSource: noop,
    fitToFillAdjacent: noop,
    nestSelection: noop,
  }))
}

/** Full <button …> open tag for one data-cut-ctx key, or null when absent. */
function btn(html: string, key: string): string | null {
  const m = html.match(new RegExp(`<button[^>]*data-cut-ctx="${key}"[^>]*>`))
  return m ? m[0] : null
}
const present = (html: string, key: string) => btn(html, key) !== null
/** renderToStaticMarkup emits the boolean attribute as `disabled=""`. */
const isDisabled = (tag: string) => /\sdisabled(=""|\s|>)/.test(tag)

// ---- 1. detach-audio gates on the asset's audio stream ---------------------
{
  // EXTRACT case: muxed asset, audio NOT on the timeline (plain insert).
  // This is where the verb does its real work — the entry MUST stay.
  const soloMux = [clip('cv', 'video', 'v1', 'muxed', 0, 4000)]
  assert.ok(present(renderMenu(soloMux, 'cv'), 'detach-audio'),
    'detach-audio renders for a muxed video whose audio is NOT on the timeline (the extract case)')

  // Muxed with the linked sibling already placed: informational no-op, still
  // an audio-bearing clip — entry stays (matches the engine accept).
  const pairedMux = [
    clip('cv', 'video', 'v1', 'muxed', 0, 4000),
    clip('ca', 'audio', 'a1t', 'muxed', 0, 4000),
  ]
  assert.ok(present(renderMenu(pairedMux, 'cv'), 'detach-audio'),
    'detach-audio renders for a muxed video with its linked audio placed')

  // Silent video: probe.has_audio false → the verb can only reject (NoAudio).
  const silent = [clip('cs', 'video', 'v1', 'silent', 0, 4000)]
  assert.equal(present(renderMenu(silent, 'cs'), 'detach-audio'), false,
    'detach-audio is absent for a silent video clip (probe.has_audio false)')

  // Still image: no audio stream at all.
  const still = [clip('ci', 'video', 'v1', 'still', 0, 4000, { isImage: true })]
  assert.equal(present(renderMenu(still, 'ci'), 'detach-audio'), false,
    'detach-audio is absent for a still-image clip')
}

// ---- 2. title/shape clips get the class-filtered menu ----------------------
{
  // TWO adjacent titles: pre-fix this also lit the J/L video-seam path and the
  // laid seam, so every filtered entry below was genuinely rendering before.
  const titles = [
    clip('t1', 'video', 'title1', 'titlemov', 0, 2000),
    clip('t2', 'video', 'title1', 'titlemov', 2000, 2000),
  ]
  const html = renderMenu(titles, 't1', 500)

  // Menu identifies its class for the rig lanes / debug API.
  assert.match(html, /data-cut-clip-kind="title"/, 'title menu is tagged data-cut-clip-kind="title"')

  // Footage-class entries must NOT render on a rendered title/shape overlay.
  const filtered = [
    'split-edit-j', 'split-edit-l',                                   // A/V-pair seam ops — titles carry no audio
    'replace', 'fit-to-fill', 'nest',                                 // source-media swaps — would sever title.update identity
    'speed-half', 'speed-normal', 'speed-double', 'speed-custom',     // retiming rendered text
    'reverse', 'freeze',                                              // footage time ops
    'color-grade', 'crop', 'stabilize',                               // footage picture ops (crop = same panel as transform)
    'blur-faces',                                                     // face detection on text
    'detach-audio',                                                   // title .mov probes has_audio:false
    'gain', 'mute', 'clean-voice',                                    // audio group — no audio stream
  ]
  for (const key of filtered) {
    assert.equal(present(html, key), false, `title menu excludes footage-class entry [${key}]`)
  }

  // Entries whose verbs genuinely apply to a title overlay must stay.
  const kept = [
    'copy', 'cut', 'paste', 'paste-attributes',   // clipboard is clip-agnostic
    'split', 'trim-tools', 'trim-start', 'trim-end',
    'remove', 'remove-gap',
    'transform',                                  // position/scale/opacity of the overlay
    'add-transition',                             // dissolve between adjacent titles is real editorial
    'fade-in', 'fade-out',                        // title fades are bread-and-butter
    'remove-track',                               // title tracks are overlays
  ]
  for (const key of kept) {
    assert.ok(present(html, key), `title menu keeps applicable entry [${key}]`)
  }

  // The kept transition entry is also ENABLED here (t2 is editorial-adjacent).
  const trans = btn(html, 'add-transition')
  assert.ok(trans && !isDisabled(trans), 'title add-transition is enabled on the adjacent-title seam')
}

// ---- 3. add-transition adjacency is EDITORIAL, not laid --------------------
{
  // A 0..4000 + B with a 500ms crossfade INTO it: editorial seam at 4000,
  // laid start pulled back to 3500. Laid adjacency fails on BOTH sides of the
  // seam (the pre-fix disabled bug); editorial adjacency holds on both.
  const faded = [
    clip('cA', 'video', 'v1', 'muxed', 0, 4000),
    clip('cB', 'video', 'v1', 'muxed', 3500, 4000, { editorialStartMs: 4000, xfadeInMs: 500 }),
  ]
  for (const id of ['cA', 'cB']) {
    const tag = btn(renderMenu(faded, id), 'add-transition')
    assert.ok(tag, `[${id}] add-transition renders`)
    assert.equal(isDisabled(tag!), false,
      `[${id}] add-transition is ENABLED across an already-crossfaded seam (editorial adjacency; laid adjacency wrongly disabled it)`)
  }

  // Control: a lone clip has no seam in either time base — stays disabled.
  const lone = [clip('cL', 'video', 'v1', 'muxed', 0, 4000)]
  const loneTag = btn(renderMenu(lone, 'cL'), 'add-transition')
  assert.ok(loneTag && isDisabled(loneTag), 'lone clip: add-transition stays disabled (no seam in any time base)')
}

// ---- 4. Remove discloses linked-audio propagation --------------------------
{
  // Exact linked pair (same asset + src window + laid span — linkedSiblings
  // criteria): removeItemById WILL remove both halves → tooltip says so.
  const pairedMux = [
    clip('cv', 'video', 'v1', 'muxed', 0, 4000),
    clip('ca', 'audio', 'a1t', 'muxed', 0, 4000),
  ]
  const pairedHtml = renderMenu(pairedMux, 'cv')
  for (const key of ['remove', 'remove-gap']) {
    const tag = btn(pairedHtml, key)
    assert.ok(tag && /linked audio/.test(tag), `[${key}] tooltip discloses the linked-audio removal`)
  }

  // No exact counterpart → no propagation → the tooltip must not claim it.
  const soloMux = [clip('cv', 'video', 'v1', 'muxed', 0, 4000)]
  const soloHtml = renderMenu(soloMux, 'cv')
  for (const key of ['remove', 'remove-gap']) {
    const tag = btn(soloHtml, key)
    assert.ok(tag && !/linked audio/.test(tag), `[${key}] tooltip stays plain without a linked counterpart`)
  }
}

console.log('PASS clip context menu class gates: detach-audio asset gate + title filter + editorial transition adjacency + linked-remove disclosure')
