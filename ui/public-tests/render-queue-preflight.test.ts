import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { PregateReport } from '../src/lib/client'
import PreflightWarning from '../src/topbar/PreflightWarning'
import { resourceRisks, runVideoPreflightAction, withResourceRisks } from '../src/topbar/videoPreflight'

const here = dirname(fileURLToPath(import.meta.url))
const topbar = readFileSync(resolve(here, '../src/topbar/index.tsx'), 'utf8')
const queueModal = readFileSync(resolve(here, '../src/topbar/RenderQueueModal.tsx'), 'utf8')
assert.match(topbar, /<RenderQueueModal /)
assert.match(topbar, /onPreflight=\{runVideoPreflight\}/)
assert.match(queueModal, /await onPreflight\('rendering queued deliveries'/)
assert.ok(queueModal.indexOf('await onPreflight(') < queueModal.indexOf('await owner.submit('), 'queue submission remains inside the shared gate')

const jobs = [{ preset: 'standard', aspect: 'project' }, { preset: 'high', aspect: '9:16' }]
const enqueued: typeof jobs[] = []
const enqueue = async () => { enqueued.push(jobs) }
let pending: (() => Promise<void>) | null = null
const notes: string[] = []
const run = async (report: PregateReport, missing = false) => runVideoPreflightAction('rendering queued deliveries', enqueue, {
  ffmpegMissing: missing,
  check: async () => ({ ok: true, result: report }),
  showWarning: (_report, _label, action) => { pending = action },
  note: (message) => { notes.push(message) },
})

const blocked: PregateReport = { pass: false, summary: 'pregate FAIL — fix before spending the render', risks: [{ kind: 'black_or_frozen', severity: 'high', detail: '13066ms frozen of source', range_ms: [0, 29167] }] }
assert.equal(await run(blocked), 'warning')
assert.equal(enqueued.length, 0, 'high-risk preflight requires explicit acknowledgment before either profile')
assert.ok(pending, 'high-risk queue displays a review warning')
const blockedHtml = renderToStaticMarkup(React.createElement(PreflightWarning, {
  report: blocked, actionLabel: 'rendering queued deliveries', onCancel: () => {}, onContinue: () => {}, overModal: true,
}))
assert.match(blockedHtml, /data-cut-pregate-blocked="false"/)
assert.doesNotMatch(blockedHtml, /data-cut-pregate-continue="true" disabled=""/, 'quality predictions allow explicit override')
assert.match(blockedHtml, /Queue anyway/)
assert.match(blockedHtml, /Review before export/)
assert.match(blockedHtml, /screen recording/)
assert.match(blockedHtml, /0s-29.2s/)
assert.doesNotMatch(blockedHtml, /13066ms frozen of source|pregate FAIL|<code>/, 'Details uses plain check wording and the timeline range')
const exportHtml = renderToStaticMarkup(React.createElement(PreflightWarning, { report: blocked, actionLabel: 'exporting Video', onCancel: () => {}, onContinue: () => {} }))
assert.match(exportHtml, /Export anyway/)
await (pending as (() => Promise<void>) | null)?.()
assert.deepEqual(enqueued, [jobs], 'high-risk acknowledgment invokes the original two profiles once')

pending = null
const warning: PregateReport = { pass: true, risks: [{ kind: 'silent_output', severity: 'med' }] }
assert.equal(await run(warning), 'warning')
assert.equal(enqueued.length, 1, 'medium warning requires an explicit Continue before all profiles')
assert.ok(pending)
const warningHtml = renderToStaticMarkup(React.createElement(PreflightWarning, {
  report: warning, actionLabel: 'rendering queued deliveries', onCancel: () => {}, onContinue: () => {}, overModal: true,
}))
assert.match(warningHtml, /data-cut-pregate-blocked="false"/)
assert.doesNotMatch(warningHtml, /data-cut-pregate-continue="true" disabled=""/)
const acknowledged = pending as (() => Promise<void>) | null
await acknowledged?.()
assert.deepEqual(enqueued, [jobs, jobs], 'acknowledgment submits the original two profiles together once')

pending = null
assert.equal(await run({ pass: true, risks: [] }, true), 'blocked')
assert.equal(enqueued.length, 2, 'missing FFmpeg refuses without a queue submission')
assert.match(notes.at(-1) ?? '', /Install FFmpeg/)
assert.equal(await run({ pass: true, risks: [] }), 'started')
assert.equal(enqueued.length, 3, 'clean preflight submits once without warning')

// Item 82: the render plan's memory/disk warnings and a refusal become risks
// with their numbers, so the person sees them before the render starts.
const plan = {
  resources: { warnings: [{ code: 'memory_short', message: 'This render needs about 11.0 GB of memory and about 6.0 GB is free now.' }] },
  refused: { code: 'not_enough_space', message: 'not enough free space for this render on /vol: it needs about 9.0 GB, 1.0 GB is free', suggested_action: 'Free at least 8.5 GB on that drive.' },
}
assert.deepEqual(resourceRisks(plan).map((r) => r.kind), ['memory_short', 'not_enough_space'])
assert.match(resourceRisks(plan)[1].detail ?? '', /Free at least 8.5 GB/)
assert.deepEqual(resourceRisks({ resources: { warnings: [] } }), [])
const merged = withResourceRisks({ pass: true, risks: [] }, plan)
assert.equal(merged?.pass, false)
const resourceHtml = renderToStaticMarkup(React.createElement(PreflightWarning, {
  report: merged as PregateReport, actionLabel: 'rendering video', onCancel: () => {}, onContinue: () => {},
}))
assert.match(resourceHtml, /Not enough free memory/)
assert.match(resourceHtml, /Estimated memory needed: 11.0 GB\. Available: 6.0 GB\./)
assert.match(resourceHtml, /Estimated space needed: 9.0 GB\. Available: 1.0 GB\. Space to free: 8.5 GB\./)
assert.doesNotMatch(resourceHtml, /project\.set_output_dir/)
assert.match(resourceHtml, /Not enough free space/)
const topbarRender = topbar.slice(topbar.indexOf('const onRender'))
assert.match(topbarRender, /aspect !== 'project' \? undefined : renderArgs/, 'a single render passes its plan args to the preflight')
assert.match(topbar, /callVerb\('render.final', \{ \.\.\.renderPlanArgs, dry_run: true \}\)/)
console.log('queue preflight: two-profile explicit high-risk override, acknowledgment, FFmpeg guard, clean path, render resources')
