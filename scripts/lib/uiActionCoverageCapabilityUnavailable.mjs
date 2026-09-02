import { readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'

// These controls are compiled as the future Region picker, but cannot mount in
// the current Record runtime before the private foreground bridge receives
// compiled/native qualification. Keep the exception deliberately small and
// auditable; this is not a generic escape hatch for merely hidden controls.
export const CAPABILITY_UNAVAILABLE_ACTIONS = Object.freeze({
  'rec-region-display': regionPickerUnavailableMetadata(),
  'record-region-cancel': regionPickerUnavailableMetadata(),
  'record-region-cancel-footer': regionPickerUnavailableMetadata(),
  'record-region-use': regionPickerUnavailableMetadata(),
  'record-region-use-last': regionPickerUnavailableMetadata(),
})

function regionPickerUnavailableMetadata() {
  return {
    owner: 'ui/src/panels/Record/RegionPickerOverlay.tsx',
    reason: 'Region capture is not exposed before compiled/native foreground qualification.',
    boundaryTest: 'ui/public-tests/record-region-picker.test.ts',
    predicate: 'record-region-picker-unavailable-v1',
  }
}

const CAPABILITY_UNAVAILABLE_PREDICATES = Object.freeze({
  'record-region-picker-unavailable-v1': {
    source: 'ui/src/panels/Record/index.tsx',
    required: [
      'const regionPickerCapability = REGION_PICKER_UNAVAILABLE',
      "regionPickerOpen && regionPickerCapability.availability === 'available'",
    ],
    boundaryRequired: [
      'REGION_PICKER_UNAVAILABLE',
      'unavailable Region is not advertised as a dead control',
    ],
  },
})

function sourceTextFor(relativePath, files, repoRoot) {
  const matched = files.find((file) => relative(repoRoot, file).replaceAll('\\', '/') === relativePath)
  if (matched) return readFileSync(matched, 'utf8')
  try {
    return readFileSync(resolve(repoRoot, relativePath), 'utf8')
  } catch {
    return ''
  }
}

/**
 * A capability-unavailable entry is valid only while the inventory still owns
 * exactly that source, the runtime keeps its unavailable predicate, and the
 * named boundary test proves the control is absent. This intentionally makes
 * an exception stale as soon as someone wires a native-sweep reference.
 */
export function classifyCapabilityUnavailableActions({
  actions,
  repoRoot,
  sourceFiles = [],
  testFiles = [],
  registry = CAPABILITY_UNAVAILABLE_ACTIONS,
  testSourceReferencesAction,
} = {}) {
  const unavailable = []
  const invalid = []
  const actionsById = new Map(actions.map((action) => [action.id, action]))
  for (const [id, metadata] of Object.entries(registry)) {
    const action = actionsById.get(id)
    const predicate = CAPABILITY_UNAVAILABLE_PREDICATES[metadata.predicate]
    const ownerMatches = action?.owners?.every((owner) => owner.startsWith(`${metadata.owner}:`)) === true
    const predicateSource = predicate
      ? sourceTextFor(predicate.source, sourceFiles, repoRoot)
      : ''
    const predicateMatches = Boolean(predicate)
      && predicate.required.every((snippet) => predicateSource.includes(snippet))
    const boundarySource = sourceTextFor(metadata.boundaryTest, testFiles, repoRoot)
    const boundaryMatches = Boolean(predicate)
      && testSourceReferencesAction(boundarySource, id)
      && predicate.boundaryRequired.every((snippet) => boundarySource.includes(snippet))
    const staleNativeReference = action?.referencedByNativeSweep === true
    if (!action || !ownerMatches || !predicateMatches || !boundaryMatches || staleNativeReference) {
      invalid.push({
        id,
        metadata,
        missingAction: !action,
        ownerMatches,
        predicateMatches,
        boundaryMatches,
        staleNativeReference,
      })
      continue
    }
    unavailable.push({ id, ...metadata })
  }
  return { unavailable, invalid }
}
