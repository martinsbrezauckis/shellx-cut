// ui/public-tests/grade-drawer.test.ts — Grade drawer receipt lifecycle +
// slider precision (run: `npm run test:lib`, or `tsx public-tests/grade-drawer.test.ts`).
//
// Regression coverage for receipt persistence and slider precision:
//
// 1. RECEIPT SELF-CLEAR. Applying a grade triggers a project snapshot refresh;
//    the snapshot carries the just-applied grade, so gradeSeedKey changes and
//    the slider-seed effect re-runs. That effect ALSO reset the `result`
//    receipt state — the receipt vanished moments after every apply. Contract:
//    the receipt persists across snapshot refreshes of the SAME clip and is
//    cleared only when the selection moves to a different clip (drawer close
//    unmounts the component; a new apply clears/replaces it inside fire()).
//    react-dom/server cannot run effects, so the wiring is pinned at SOURCE
//    level: the seed effect must not touch result/err, and a
//    separate [clipId]-keyed effect must own clearing them.
//
// 2. QUANTIZATION. step=0.05 made grades applied by the agent API at finer
//    precision unreachable from the UI (could not be reproduced or fine-tuned
//    by hand). Contract: contrast/brightness/saturation/gamma step 0.01;
//    temperature keeps its designed 100 K step (Kelvin scale; the engine
//    rounds to an integer). Proven on the REAL rendered markup via
//    react-dom/server — the component's CSS import is neutralized with a
//    module load hook so tsx can import it (dynamic imports keep hook
//    registration ordered before the component loads, same discipline as
//    panel-persist-guard.test.ts).

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { registerHooks } from 'node:module'
import { resolve } from 'node:path'

// Neutralize CSS imports (drawer.css) — tsx/node has no CSS loader.
registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith('.css')) return { format: 'module', source: 'export default {}', shortCircuit: true }
    return nextLoad(url, context)
  },
})

const { createElement } = await import('react')
const { renderToStaticMarkup } = await import('react-dom/server')
const { default: GradeDrawer } = await import('../src/panels/Grade/index')
type Project = import('../src/lib/clientModel').Project

const root = resolve(import.meta.dirname, '..')
const source = readFileSync(resolve(root, 'src/panels/Grade/index.tsx'), 'utf8')

// ---- fixture: one video track with one MEDIA clip ('asset' in clip) --------
/** Minimal project snapshot carrying a media clip with the given grade.
 *  Shape mirrors exactly what findMediaGrade() walks (tracks[].clips[]). */
function projectWith(grade: Record<string, number | string> | null): Project {
  return {
    tracks: [{ clips: [{ id: 'c1', asset: 'a1', ...(grade ? { grade } : {}) }] }],
  } as unknown as Project
}

const render = (project: Project | null, clipId: string | null) =>
  renderToStaticMarkup(createElement(GradeDrawer, { project, clipId }))

/** Pull one <input data-cut-grade-input="attr" …> tag from rendered markup. */
function sliderTag(html: string, attr: string): string {
  const m = html.match(new RegExp(`<input[^>]*data-cut-grade-input="${attr}"[^>]*>`))
  assert.ok(m, `slider [${attr}] renders`)
  return m[0]
}

/** Pull the live readout text for one slider. */
function readout(html: string, attr: string): string {
  const m = html.match(new RegExp(`data-cut-grade-val="${attr}"[^>]*>([^<]*)<`))
  assert.ok(m, `readout [${attr}] renders`)
  return m[1]
}

// ---- 1. slider precision: 0.01 step on the eq sliders, 100 K on temp -------
{
  // API-precision grade: 1.13 is representable at step 0.01, NOT at 0.05.
  const html = render(projectWith({ contrast: 1.13, brightness: -0.07, saturation: 1, gamma: 1, temperature_k: 5600 }), 'c1')

  for (const attr of ['contrast', 'brightness', 'saturation', 'gamma']) {
    assert.match(sliderTag(html, attr), /step="0\.01"/, `[${attr}] slider steps 0.01 so API-precision grades stay reachable from the UI`)
  }
  assert.match(sliderTag(html, 'temperature_k'), /step="100"/, 'temperature keeps its designed 100 K step (engine rounds Kelvin to int)')

  // The seeded input holds the clip's EXACT stored value (no snap-on-seed).
  assert.match(sliderTag(html, 'contrast'), /value="1\.13"/, 'API-precision stored grade seeds the slider exactly')
  assert.match(sliderTag(html, 'brightness'), /value="-0\.07"/, 'negative fine-precision brightness seeds exactly')

  // Readouts stay human-readable at 2 decimals (parseFloat-compatible for the
  // automated UI checks, which read them numerically).
  assert.equal(readout(html, 'contrast'), '1.13', 'contrast readout shows the fine-precision value')
  assert.match(readout(html, 'brightness'), /^-0\.07$/, 'brightness readout shows the fine-precision value')
}

{
  // Ungraded clip: neutral seeds render, formatted at a consistent 2 decimals.
  const html = render(projectWith(null), 'c1')
  assert.equal(readout(html, 'contrast'), '1.00', 'neutral contrast readout is 2-decimal formatted')
  assert.equal(readout(html, 'brightness'), '0.00', 'neutral brightness readout is 2-decimal formatted')
}

// ---- 2. receipt lifecycle wiring (source contract) -------------------------
{
  // Locate the slider-seed effect by its dependency anchor.
  const seedDep = source.indexOf('}, [gradeSeedKey])')
  assert.notEqual(seedDep, -1, 'slider-seed effect keyed on gradeSeedKey exists')
  const seedEffect = source.slice(source.lastIndexOf('useEffect', seedDep), seedDep)
  assert.equal(seedEffect.includes('setResult'), false,
    'seed effect must NOT clear the receipt — a same-clip snapshot refresh (the one apply itself triggers) re-runs it')
  assert.equal(seedEffect.includes('setErr'), false,
    'seed effect must NOT clear the error state either (same self-wipe path)')

  // A dedicated effect keyed on the SELECTED CLIP owns receipt clearing.
  const clipDep = source.indexOf('}, [clipId])')
  assert.notEqual(clipDep, -1, 'receipt-clearing effect keyed on clipId exists — receipt clears when selection moves to a different clip')
  const clearEffect = source.slice(source.lastIndexOf('useEffect', clipDep), clipDep)
  assert.equal(clearEffect.includes('setResult(null)'), true, 'clip-change effect clears the receipt')
  assert.equal(clearEffect.includes('setErr(null)'), true, 'clip-change effect clears the error state')
  assert.equal(clearEffect.includes('setContrast'), false, 'clip-change effect does not double-seed sliders (seeding stays with gradeSeedKey)')
}

console.log('PASS grade drawer receipt lifecycle + 0.01 slider precision contract')
