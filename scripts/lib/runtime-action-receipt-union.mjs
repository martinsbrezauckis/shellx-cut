import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}

function readJson(path, label) {
  const bytes = readFileSync(path)
  let value
  try {
    value = JSON.parse(bytes.toString('utf8'))
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${path}: ${error.message}`)
  }
  return { bytes, value }
}

export function buildRuntimeActionReceiptUnion({
  receiptFiles,
  manifestFile,
  generatedAt = new Date().toISOString(),
}) {
  if (!Array.isArray(receiptFiles) || receiptFiles.length === 0) {
    throw new Error('at least one full-coverage result receipt is required')
  }

  const manifestPath = resolve(manifestFile)
  const manifestRead = readJson(manifestPath, 'action manifest')
  const expected = [...new Set((manifestRead.value.actions || []).map(String))].sort()
  if (expected.length === 0 || expected.length !== manifestRead.value.actionCount) {
    throw new Error(`action manifest count is invalid: ${manifestPath}`)
  }

  const observedSet = new Set()
  const inputs = receiptFiles.map((file) => {
    const path = resolve(file)
    const receiptRead = readJson(path, 'result receipt')
    const actions = receiptRead.value.runtimeSourceActionManifest?.observed
    if (!Array.isArray(actions)) {
      throw new Error(`result receipt has no runtimeSourceActionManifest.observed array: ${path}`)
    }
    for (const action of actions) observedSet.add(String(action))
    return {
      path,
      sha256: sha256(receiptRead.bytes),
      actionTotal: new Set(actions.map(String)).size,
      receiptOk: receiptRead.value.ok === true,
      generatedAt: receiptRead.value.generatedAt ?? null,
    }
  })

  const observed = [...observedSet].sort()
  const expectedSet = new Set(expected)
  const missing = expected.filter((action) => !observedSet.has(action))
  const unexpected = observed.filter((action) => !expectedSet.has(action))
  const matchesExpected = missing.length === 0 && unexpected.length === 0

  return {
    schema: 'shellx-cut/runtime-action-receipt-union@1',
    generatedAt,
    manifest: {
      path: manifestPath,
      sha256: sha256(manifestRead.bytes),
      actionTotal: expected.length,
    },
    inputs,
    runtimeSourceActionManifest: {
      algorithm: 'sha256',
      sha256: sha256(JSON.stringify(observed)),
      total: observed.length,
      observed,
      expectedSha256: sha256(JSON.stringify(expected)),
      expectedTotal: expected.length,
      missing,
      unexpected,
      matchesExpected,
    },
    ok: matchesExpected,
  }
}
