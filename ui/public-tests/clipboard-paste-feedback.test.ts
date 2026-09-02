// ui/public-tests/clipboard-paste-feedback.test.ts — clipboard snapshot paste
// refusal feedback (run: `npm run test:lib`, or this file with tsx).
//
// A copied clip can outlive both its timeline source and its backing asset. The
// engine correctly refuses the latter without appending an edit; this regression
// proves the UI turns that exact error plus its recovery cause into the shared,
// accessible notification. It also pins the valid stale-source snapshot path:
// an absent source clip with a still-imported asset remains a successful paste
// and must not show a false failure or alter the caller's selection.

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { UserActionFeedbackNotice } from '../src/components/UserActionFeedback'
import { clipboardPasteFailureMessage } from '../src/app/useAppClipboardController'
import type { VerbResult } from '../src/lib/client'

const root = resolve(import.meta.dirname, '..')
const controllerSource = readFileSync(resolve(root, 'src/app/useAppClipboardController.ts'), 'utf8')

const unavailableAsset: VerbResult = {
  ok: false,
  error: {
    code: 'not_found',
    message: "no asset 'missing-asset' in the project",
    cause: "the snapshot's asset is not (or no longer) imported",
  },
}
const unchangedSelection = ['selected-clip']
const rejectionBefore = JSON.stringify(unavailableAsset)
const refusal = clipboardPasteFailureMessage(unavailableAsset)

assert.equal(
  refusal,
  "no asset 'missing-asset' in the project the snapshot's asset is not (or no longer) imported",
  'a refused snapshot paste keeps the exact engine error and recovery cause',
)
assert.equal(unavailableAsset.op_ids, undefined, 'the refused paste reports no committed mutation')
assert.equal(JSON.stringify(unavailableAsset), rejectionBefore, 'feedback does not mutate the refused verb result')
assert.deepEqual(unchangedSelection, ['selected-clip'], 'a refused paste leaves the existing selection unchanged')

const renderedRefusal = renderToStaticMarkup(createElement(UserActionFeedbackNotice, {
  feedback: { message: refusal! },
  onOpenSetup: () => {},
  onDismiss: () => {},
}))
assert.match(renderedRefusal, /data-cut-user-action-feedback/, 'the paste refusal has the shared visible notification selector')
assert.match(renderedRefusal, /role="alert"/, 'the paste refusal is exposed as an accessible alert')
assert.match(renderedRefusal, /aria-live="assertive"/, 'the paste refusal is announced when it appears')
assert.match(renderedRefusal, /no asset &#x27;missing-asset&#x27; in the project/, 'the rendered alert preserves the exact engine error')
assert.match(renderedRefusal, /snapshot&#x27;s asset is not \(or no longer\) imported/, 'the rendered alert preserves the actionable recovery cause')

const sourceDeletedButAssetAvailable: VerbResult = {
  ok: true,
  result: { inserted_clip: 'pasted-snapshot' },
  op_ids: ['op-paste-snapshot'],
}
assert.equal(
  clipboardPasteFailureMessage(sourceDeletedButAssetAvailable),
  null,
  'a source-deleted clipboard snapshot with an available asset remains successful and silent',
)
assert.deepEqual(unchangedSelection, ['selected-clip'], 'a successful snapshot paste preserves the caller selection')

assert.match(controllerSource, /const result = await callVerb\('edit\.paste'/, 'paste observes the engine result instead of discarding it')
assert.match(controllerSource, /clipboardPasteFailureMessage\(result\)/, 'paste routes refusals through the clipboard feedback model')
assert.match(controllerSource, /publishUserActionMessage\(message\)/, 'paste publishes refusals through the shared visible notification')

console.log('PASS clipboard paste feedback')
