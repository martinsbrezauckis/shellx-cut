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
assert.ok(queueModal.indexOf('await onPreflight(') < queueModal.indexOf("callVerb('render.queue'"), 'queue submission remains inside the shared gate')

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

const blocked: PregateReport = { pass: false, risks: [{ kind: 'empty_tail', severity: 'high' }] }
assert.equal(await run(blocked), 'warning')
assert.equal(enqueued.length, 0, 'high-risk preflight never submits either profile')
assert.ok(pending, 'blocked queue displays existing warning')
const blockedHtml = renderToStaticMarkup(React.createElement(PreflightWarning, {
  report: blocked, actionLabel: 'rendering queued deliveries', onCancel: () => {}, onContinue: () => {}, overModal: true,
}))
assert.match(blockedHtml, /data-cut-pregate-blocked="true"/)
assert.match(blockedHtml, /data-cut-pregate-continue="true" disabled=""/, 'existing warning refuses Continue for high risk')

pending = null
const warning: PregateReport = { pass: true, risks: [{ kind: 'silent_output', severity: 'med' }] }
assert.equal(await run(warning), 'warning')
assert.equal(enqueued.length, 0, 'medium warning requires an explicit Continue before all profiles')
assert.ok(pending)
const warningHtml = renderToStaticMarkup(React.createElement(PreflightWarning, {
  report: warning, actionLabel: 'rendering queued deliveries', onCancel: () => {}, onContinue: () => {}, overModal: true,
}))
assert.match(warningHtml, /data-cut-pregate-blocked="false"/)
assert.doesNotMatch(warningHtml, /data-cut-pregate-continue="true" disabled=""/)
const acknowledged = pending as (() => Promise<void>) | null
await acknowledged?.()
assert.deepEqual(enqueued, [jobs], 'acknowledgment submits the original two profiles together once')

pending = null
assert.equal(await run({ pass: true, risks: [] }, true), 'blocked')
assert.equal(enqueued.length, 1, 'missing FFmpeg refuses without a queue submission')
assert.match(notes.at(-1) ?? '', /Install FFmpeg/)
assert.equal(await run({ pass: true, risks: [] }), 'started')
assert.equal(enqueued.length, 2, 'clean preflight submits once without warning')
console.log('queue preflight: two-profile refusal, acknowledgment, FFmpeg guard, clean path')
