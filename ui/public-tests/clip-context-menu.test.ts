// ui/public-tests/clip-context-menu.test.ts — clip right-click menu class gates
// (run: `npm run test:lib`, or `tsx public-tests/clip-context-menu.test.ts`).
//
// Regression coverage for three context-menu contracts:
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
// 2. TITLE/SHAPE COMPACT MENU. layout.ts now distinguishes generated overlays
//    from footage. Their rendered media must never regain generic source,
//    clipboard, speed, picture, privacy, audio, transition, or fade actions;
//    only Inspector edit, Transform, Split, Remove, and Remove track apply.
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
import { readFileSync } from 'node:fs'
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
const { layoutTrack } = await import('../src/panels/Timeline/layout')
const { clipContextMenuContract, exactTimelineAudioTarget } = await import('../src/panels/Timeline/ClipContextMenuModel')
type Project = import('../src/lib/client').Project
type LaidItem = import('../src/panels/Timeline/layout').LaidItem
type TimelineContentClass = import('../src/panels/Timeline/layout').TimelineContentClass

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
    contentClass: extra.contentClass ?? (extra.isImage ? 'still' : kind),
    editorialStartMs: startMs, label: asset, asset,
    srcInMs: 0, srcOutMs: durMs, ...extra,
  }
}

function nonMedia(
  id: string,
  contentClass: Extract<TimelineContentClass, 'caption' | 'gap'>,
  trackId: string,
): LaidItem {
  return {
    id,
    kind: contentClass,
    contentClass,
    trackId,
    startMs: 0,
    editorialStartMs: 0,
    durMs: 1000,
    label: contentClass === 'caption' ? 'Caption text' : '',
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

// ---- 0. Layout owns the semantic clip class --------------------------------
{
  const video = layoutTrack({
    id: 'v1', kind: 'video', clips: [{ id: 'v', asset: 'muxed', src_in_ms: 0, src_out_ms: 1000 }],
  } as never)[0]
  const audio = layoutTrack({
    id: 'a1t', kind: 'audio', clips: [{ id: 'a', asset: 'muxed', src_in_ms: 0, src_out_ms: 1000 }],
  } as never)[0]
  const still = layoutTrack({
    id: 'v1', kind: 'video', clips: [{ id: 's', asset: 'still', src_in_ms: 0, src_out_ms: 1000 }],
  } as never, new Set(['still']))[0]
  const overlays = layoutTrack({
    id: 'title1', kind: 'video', clips: [
      { id: 't', asset: 'titlemov', src_in_ms: 0, src_out_ms: 1000, title_text: 'Hello' },
      { id: 'sh', asset: 'titlemov', src_in_ms: 0, src_out_ms: 1000, shape_kind: 'rectangle' },
    ],
  } as never)
  const caption = layoutTrack({
    id: 'cap1', kind: 'caption', clips: [{ id: 'cap', text: 'Hello', range_ms: [0, 1000] }],
  } as never)[0]
  const gap = layoutTrack({
    id: 'v1', kind: 'video', clips: [{ kind: 'gap', duration_ms: 1000 }],
  } as never)[0]

  assert.deepEqual(
    [video.contentClass, audio.contentClass, still.contentClass, overlays[0].contentClass, overlays[1].contentClass, caption.contentClass, gap.contentClass],
    ['video', 'audio', 'still', 'title', 'shape', 'caption', 'gap'],
    'layout projects every timeline item into its authoritative UI content class',
  )
}

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

// ---- 1b. Exhaustive class contract: hide invalid, disable valid-but-unready
{
  const muxed = clip('mv', 'video', 'v1', 'muxed', 0, 1000)
  const audio = clip('au', 'audio', 'a1t', 'muxed', 0, 1000)
  const still = clip('st', 'video', 'v1', 'still', 0, 1000, { isImage: true })
  const title = clip('ti', 'video', 'title1', 'titlemov', 0, 1000, { contentClass: 'title' })
  const shape = clip('sh', 'video', 'title1', 'titlemov', 0, 1000, { contentClass: 'shape' })
  const caption = nonMedia('cap', 'caption', 'cap1')
  const gap = nonMedia('gap:v1:0', 'gap', 'v1')
  const cases: Array<{
    name: string
    item: LaidItem
    expected: { surface: 'media' | 'caption' | 'none'; detach: 'visible' | 'hidden'; sourceEdits: boolean; speed: boolean }
  }> = [
    { name: 'muxed video', item: muxed, expected: { surface: 'media', detach: 'visible', sourceEdits: true, speed: true } },
    { name: 'audio', item: audio, expected: { surface: 'media', detach: 'hidden', sourceEdits: true, speed: true } },
    { name: 'still', item: still, expected: { surface: 'media', detach: 'hidden', sourceEdits: true, speed: false } },
    { name: 'title', item: title, expected: { surface: 'media', detach: 'hidden', sourceEdits: false, speed: false } },
    { name: 'shape', item: shape, expected: { surface: 'media', detach: 'hidden', sourceEdits: false, speed: false } },
    { name: 'caption', item: caption, expected: { surface: 'caption', detach: 'hidden', sourceEdits: false, speed: false } },
    { name: 'gap', item: gap, expected: { surface: 'none', detach: 'hidden', sourceEdits: false, speed: false } },
  ]

  for (const { name, item, expected } of cases) {
    const contract = clipContextMenuContract(item, project, [item])
    assert.equal(contract.surface, expected.surface, `[${name}] uses the intended menu surface`)
    assert.equal(contract.detachAudio.visibility, expected.detach, `[${name}] detach-audio visibility is class-owned`)
    assert.equal(contract.allowsSourceEdits, expected.sourceEdits, `[${name}] source edits follow clip ownership`)
    assert.equal(contract.allowsSpeedEdits, expected.speed, `[${name}] speed eligibility follows clip ownership`)
    if (expected.surface === 'media') {
      assert.equal(contract.addTransition.visibility, 'visible', `[${name}] transition remains discoverable for a media clip`)
      assert.equal(contract.addTransition.enabled, false, `[${name}] transition is disabled without an adjacent editorial seam`)
      assert.match(contract.addTransition.reason, /adjacent media clip/, `[${name}] disabled transition explains its timeline precondition`)
    } else {
      assert.equal(contract.addTransition.visibility, 'hidden', `[${name}] transition is hidden when the class has no media menu`)
    }
  }

  const joined = [muxed, clip('mv-next', 'video', 'v1', 'muxed', 800, 1000, { editorialStartMs: 1000, xfadeInMs: 200 })]
  const transition = clipContextMenuContract(muxed, project, joined).addTransition
  assert.deepEqual(
    { visibility: transition.visibility, enabled: transition.enabled, atMs: transition.atMs },
    { visibility: 'visible', enabled: true, atMs: 1000 },
    'a valid crossfaded seam stays visible and enabled at its EDITORIAL (not laid) boundary',
  )
}

// ---- 2. Exact linked audio means exact source window and laid span ----------
{
  const video = clip('cv', 'video', 'v1', 'muxed', 0, 4000)
  const independentlyTrimmedAudio = clip('ca', 'audio', 'a1t', 'muxed', 0, 4000, { srcInMs: 500, srcOutMs: 4500 })
  const exactAudioA = clip('ca2', 'audio', 'a1t', 'muxed', 0, 4000)
  const exactAudioB = clip('ca3', 'audio', 'a1t', 'muxed', 0, 4000)

  // Both adversarial fixtures retain the legacy loose-pair keys (asset +
  // start), but only the second has an exact linked counterpart. The first
  // must not expose actions that would otherwise call audio verbs on `cv`.
  const trimmedItems = [video, independentlyTrimmedAudio]
  const trimmedContract = clipContextMenuContract(video, project, trimmedItems)
  assert.equal(exactTimelineAudioTarget(video, trimmedItems), null,
    'independently trimmed same-asset/start audio is not an exact linked counterpart')
  assert.equal(trimmedContract.hasTimelineAudio, false,
    'near-match audio does not make a video timeline-audio eligible')
  const trimmedHtml = renderMenu(trimmedItems, video.id)
  for (const key of ['gain', 'mute', 'clean-voice']) {
    assert.equal(present(trimmedHtml, key), false, `near-match audio action [${key}] is hidden`)
  }
  assert.ok(present(trimmedHtml, 'detach-audio'), 'the valid detach path remains available for muxed footage')

  const ambiguousItems = [video, exactAudioA, exactAudioB]
  assert.equal(exactTimelineAudioTarget(video, ambiguousItems), null,
    'multiple audio candidates are ambiguous even when one is otherwise exact')
  assert.equal(clipContextMenuContract(video, project, ambiguousItems).hasTimelineAudio, false,
    'ambiguous candidates keep audio actions hidden')

  const actionsSource = readFileSync(new URL('../src/panels/Timeline/useTimelineClipActions.ts', import.meta.url), 'utf8')
  assert.match(actionsSource, /const target = audioHalfOf\(itemId\)\n\s*if \(!target\) return\n\s*void runUserVerb\('edit\.gain'/,
    'mute dispatch returns before calling edit.gain when no exact audio target exists')
  assert.match(actionsSource, /const target = audioHalfOf\(itemId\)\n\s*if \(!target\) return\n\s*void runUserVerb\('audio\.cleanup_voice'/,
    'clean-voice dispatch returns before calling audio.cleanup_voice when no exact audio target exists')
}

// ---- 3. title/shape clips get the compact generated-overlay menu -----------
{
  const titles = [
    clip('t1', 'video', 'title1', 'titlemov', 0, 2000, { contentClass: 'title' }),
    clip('t2', 'video', 'title1', 'titlemov', 2000, 2000, { contentClass: 'title' }),
  ]
  const html = renderMenu(titles, 't1', 500)

  // Menu identifies its class for automation and the debug API.
  assert.match(html, /data-cut-clip-kind="title"/, 'title menu is tagged data-cut-clip-kind="title"')

  // Generated overlay identity lives in title/shape data, not the rendered
  // media file. All generic clipboard/footage entries remain absent.
  const filtered = [
    'copy', 'cut', 'paste', 'paste-attributes',
    'trim-tools', 'trim-start', 'trim-end', 'remove-gap',
    'split-edit-j', 'split-edit-l', 'replace', 'fit-to-fill', 'nest',
    'speed-time', 'speed-half', 'speed-normal', 'speed-double', 'speed-custom', 'reverse', 'freeze',
    'color-grade', 'crop', 'stabilize', 'blur-faces',
    'detach-audio', 'gain', 'mute', 'clean-voice',
    'add-transition', 'fade-in', 'fade-out',
  ]
  for (const key of filtered) {
    assert.equal(present(html, key), false, `title menu excludes footage-class entry [${key}]`)
  }

  // Only the approved overlay actions remain: select + Inspector, Transform,
  // Split, Remove, and overlay-track removal.
  const kept = [
    'overlay-edit', 'transform', 'split', 'remove', 'remove-track',
  ]
  for (const key of kept) {
    assert.ok(present(html, key), `title menu keeps applicable entry [${key}]`)
  }

  assert.equal((html.match(/data-cut-ctx=/g) ?? []).length, 5, 'title menu contains only its five approved overlay actions')

  const shapes = [
    clip('s1', 'video', 'title1', 'titlemov', 0, 2000, { contentClass: 'shape' }),
    clip('s2', 'video', 'title1', 'titlemov', 2000, 2000, { contentClass: 'shape' }),
  ]
  const shapeHtml = renderMenu(shapes, 's1', 500)
  assert.match(shapeHtml, /data-cut-clip-kind="shape"/, 'shape menu carries its distinct semantic class')
  for (const key of filtered) {
    assert.equal(present(shapeHtml, key), false, `shape menu excludes footage-class entry [${key}]`)
  }
  for (const key of kept) {
    assert.ok(present(shapeHtml, key), `shape menu keeps applicable entry [${key}]`)
  }
  assert.equal((shapeHtml.match(/data-cut-ctx=/g) ?? []).length, 5, 'shape menu contains only its five approved overlay actions')
}

// ---- 4. add-transition adjacency is EDITORIAL, not laid --------------------
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

// ---- 5. media grouping remains honest and compact --------------------------
{
  const muxed = clip('mv', 'video', 'v1', 'muxed', 0, 1000)
  const linkedAudio = clip('ma', 'audio', 'a1t', 'muxed', 0, 1000)
  const html = renderMenu([muxed, linkedAudio], muxed.id)
  assert.match(html, /Transitions &amp; fades/, 'transition and fade actions share one editorial section')
  assert.match(html, /Picture[\s\S]*Blur faces/, 'privacy blur is inside the Picture section')
  assert.equal(/>Privacy</.test(html), false, 'privacy has no detached section')
  assert.ok(present(html, 'speed-time'), 'speed/time is one expandable parent action')
  assert.equal(present(html, 'speed-half'), false, 'speed presets stay collapsed until Speed & time opens')
  const audioStart = html.indexOf('>Audio<')
  assert.ok(audioStart >= 0 && html.indexOf('data-cut-ctx="detach-audio"', audioStart) >= 0,
    'Detach audio is inside the one Audio section')
}

// ---- 6. caption and gap preserve their deliberately compact/no-menu routes --
{
  const caption = nonMedia('cap-1', 'caption', 'cap1')
  const captionHtml = renderMenu([caption], 'cap-1', 200)
  assert.match(captionHtml, /data-cut-clip-kind="caption"/, 'caption keeps its dedicated compact menu class')
  for (const key of ['caption-edit', 'caption-seek', 'remove']) {
    assert.ok(present(captionHtml, key), `caption menu keeps applicable entry [${key}]`)
  }
  for (const key of ['detach-audio', 'add-transition', 'color-grade', 'gain', 'speed-half']) {
    assert.equal(present(captionHtml, key), false, `caption menu hides invalid media entry [${key}]`)
  }

  const gap = nonMedia('gap:v1:0', 'gap', 'v1')
  assert.equal(renderMenu([gap], gap.id), '', 'gap has no clip context menu; its dedicated future surface remains separate scope')
}

// ---- 7. Remove discloses linked-audio propagation --------------------------
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

console.log('PASS clip context menu contracts: exact linked audio + compact overlays + editorial transitions + grouped media actions')
