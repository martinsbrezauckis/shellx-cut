import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

const source = readFileSync('ui/public-tests/full-coverage-verify.mjs', 'utf8')

function actionRow(name) {
  const marker = `name: '${name}'`
  const start = source.indexOf(marker)
  assert.notEqual(start, -1, `missing full-coverage action row: ${name}`)
  const next = source.indexOf('\n  await probe(page, {', start + marker.length)
  return source.slice(start, next === -1 ? source.length : next)
}

function requires(name, patterns) {
  const row = actionRow(name)
  for (const pattern of patterns) {
    assert.match(row, pattern, `${name} lost effect proof ${pattern}`)
  }
  assert.doesNotMatch(
    row,
    /assertResult:\s*async \(\) => \(\{\s*ok:\s*!!probe\._r\?\.ok\s*,/,
    `${name} regressed to an ok:true-only assertion`,
  )
}

test('library rows re-read the stored item after the UI response', () => {
  requires('library.favorite', [
    /waitForLibraryItem/,
    /Boolean\(candidate\.favorite\) === probe\._favoriteExpected/,
    /Boolean\(probe\._r\?\.result\?\.item\?\.favorite\) === probe\._favoriteExpected/,
  ])
  requires('library.move', [
    /waitForLibraryItem/,
    /candidate\.folder === folder/,
    /result\?\.item\?\.folder === folder/,
    /inputValue/,
  ])
})

test('project save and history rows prove durable or visible state', () => {
  requires('project.save(Ctrl+S)', [
    /statSync\(localPath\)\.size/,
    /JSON\.parse\(readFileSync\(localPath/,
    /markerPersisted/,
    /stateMatches/,
    /pathMatches/,
  ])
  requires('project.undo(Review Undo button)', [
    /waitForState/,
    /label === 'fcv-m3'/,
    /redo_available === true/,
  ])
  requires('project.redo(Review Redo button)', [
    /waitForState/,
    /label === 'fcv-m3'/,
    /undo_available === true/,
  ])
})

test('comment apply proves executed steps, review artifact, and addressed state', () => {
  requires('comment.apply(Apply button)', [
    /waitForState/,
    /comment\.status === 'addressed'/,
    /result\?\.applied/,
    /applied\.every\(\(step\) => step\.ok === true/,
    /result\?\.diff != null/,
  ])
})

test('nest action proves preview, interchange, persistence, and reopened composition', () => {
  requires('nest-selection', [
    /'render\.preview'/,
    /previewBytes > 0/,
    /'export\.otio'/,
    /otioReferencesNest/,
    /'project\.save'/,
    /'project\.close'/,
    /'project\.open'/,
    /reopenedNested/,
    /bakedAssetStayedEphemeral/,
    /'render\.frame'/,
    /frameBytes > 0/,
  ])
})
