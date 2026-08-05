// full-surface.mjs — THE full UI/action test plan: run every effect-driven gate
// against ONE running stack and aggregate PASS/FAIL. Each gate drives the real UI
// (Playwright on data-cut-* hooks) AND asserts the engine effect, and exits non-zero
// on any failure, so this is a single go/no-go for "every action applies its result".
//
// Usage (start a stack first — cutd with the dev perception env, + vite pointed at it):
//   SWEEP_CUTD=http://127.0.0.1:6190 SWEEP_APP=http://localhost:5190 node public-tests/full-surface.mjs
//
// Gates:
//   interaction-verify  — behavior/interaction (undo, drag-no-leak, marker delete/seek, …)
//   release-verify      — the 24-tool effect gate (render-output proof per tool)
//   surface-sweep       — surface render/open health (drawers, tabs, workflows)
//   verify-audio-layer  — Mixer levels (gain/mute/solo/add-track) + Layer transform/crop/…
//   verify-grade-inspector — Grade contrast/gamma/WB/reset + Inspector stabilize/reverse/duck/…
//   verify-topbar-library  — Topbar tools + export formats + Library CRUD
import { spawnSync } from 'node:child_process'

const GATES = [
  'interaction-verify.mjs',
  'release-verify.mjs',
  'surface-sweep.mjs',
  'verify-audio-layer.mjs',
  'verify-grade-inspector.mjs',
  'verify-topbar-library.mjs',
]

let totalPass = 0
let totalFail = 0
let hardErr = 0
const rows = []

for (const g of GATES) {
  // maxBuffer bumped — release-verify/surface-sweep emit a lot; the 1 MB default
  // truncates + sets r.error, which silently hid the summary.
  const r = spawnSync('node', [`public-tests/${g}`], { encoding: 'utf8', env: process.env, cwd: process.cwd(), maxBuffer: 64 * 1024 * 1024 })
  const out = `${r.stdout || ''}${r.stderr || ''}`
  // Gates print "<P> PASS, <F> FAIL", "<P> PASS / <F> FAIL", or "<P> PASS · <F> FAIL"
  // (release-verify/surface-sweep use the · separator). Last occurrence wins.
  const matches = [...out.matchAll(/(\d+)\s+PASS[\s,/·•]+(\d+)\s+FAIL/gi)]
  const last = matches.at(-1)
  const pass = last ? Number(last[1]) : 0
  const fail = last ? Number(last[2]) : 0
  totalPass += pass
  totalFail += fail
  const ok = r.status === 0
  if (!ok) hardErr++
  const row = `${ok ? 'OK ' : 'ERR'}  ${g.padEnd(26)} ${String(pass).padStart(3)} PASS / ${String(fail).padStart(2)} FAIL${last ? '' : '  (no count parsed — see raw)'}`
  rows.push(row)
  console.log(row) // stream as each gate finishes — never a silent run
}

console.log('\n================ FULL SURFACE TEST ================')
rows.forEach((r) => console.log(r))
console.log('--------------------------------------------------')
console.log(`TOTAL  ${totalPass} PASS / ${totalFail} FAIL across ${GATES.length} gates  (${hardErr} gate(s) exited non-zero)`)
console.log('==================================================\n')
process.exit(totalFail > 0 || hardErr > 0 ? 1 : 0)
