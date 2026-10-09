import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { PregateReport } from '../src/lib/client'
import PreflightWarning from '../src/topbar/PreflightWarning'
import { runVideoPreflightAction } from '../src/topbar/videoPreflight'

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
assert.match(blockedHtml, /13066ms frozen of source/)
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
console.log('queue preflight: two-profile explicit high-risk override, acknowledgment, FFmpeg guard, clean path')
