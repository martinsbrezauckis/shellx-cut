import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { assetUsesFromSequenceIndex } from '../src/panels/Assets/assetUses'
import type { SequenceIndexResult } from '../src/lib/client'

const index = {
  query: 'asset-target',
  asset: 'asset-target',
  kind: 'clip',
  status: 'all',
  total: 4,
  clip_count: 4,
  marker_count: 0,
  issue_count: 1,
  effect_clip_count: 0,
  truncated: true,
  results: [
    {
      kind: 'clip', sequence_id: 'seq-b', sequence_name: 'B sequence', active: false,
      id: 'b2', at_ms: 1000, end_ms: 2000, label: 'target', track_id: 'v1', track_kind: 'video',
      clip_kind: 'media', asset: 'asset-target', src_in_ms: 0, src_out_ms: 1000,
      effect_count: 0, effects: [], offline: true, track_visible: true, track_locked: false, track_muted: false, issues: ['offline'],
    },
    {
      kind: 'clip', sequence_id: 'seq-a', sequence_name: 'A sequence', active: true,
      id: 'a2', at_ms: 5000, end_ms: 6000, label: 'target', track_id: 'v2', track_kind: 'video',
      clip_kind: 'media', asset: 'asset-target', src_in_ms: 0, src_out_ms: 1000,
      effect_count: 0, effects: [], offline: false, track_visible: true, track_locked: false, track_muted: false, issues: [],
    },
    {
      kind: 'clip', sequence_id: 'seq-a', sequence_name: 'A sequence', active: true,
      id: 'a1', at_ms: 1000, end_ms: 2000, label: 'similarly named, different asset', track_id: 'a1', track_kind: 'audio',
      clip_kind: 'media', asset: 'asset-target', src_in_ms: 0, src_out_ms: 1000,
      effect_count: 0, effects: [], offline: false, track_visible: true, track_locked: false, track_muted: false, issues: [],
    },
    {
      kind: 'clip', sequence_id: 'seq-a', sequence_name: 'A sequence', active: true,
      id: 'wrong', at_ms: 1, end_ms: 2, label: 'target', track_id: 'v1', track_kind: 'video',
      clip_kind: 'media', asset: 'asset-other', src_in_ms: 0, src_out_ms: 1,
      effect_count: 0, effects: [], offline: false, track_visible: true, track_locked: false, track_muted: false, issues: [],
    },
  ],
} as SequenceIndexResult

const result = assetUsesFromSequenceIndex(index, 'asset-target')
assert.deepEqual(result.uses.map((use) => use.clipId), ['a1', 'a2', 'b2'],
  'All uses filters by exact stable asset id and sorts compact occurrence rows deterministically')
assert.equal(result.uses.at(-1)?.offline, true,
  'Sequence Index offline state is preserved so an unavailable occurrence cannot be opened')
assert.equal(result.truncated, true,
  'the bounded per-asset Sequence Index result remains visibly non-exhaustive')

const sourceMonitor = readFileSync(new URL('../src/panels/Assets/SourceMonitor.tsx', import.meta.url), 'utf8')
assert.match(sourceMonitor, /project\.sequence_index', \{ asset: asset\.id, kind: 'clip', limit: 500 \}/,
  'All uses sends the stable asset id as an exact server-side filter before the bounded result limit')
assert.match(sourceMonitor, /use\.offline[\s\S]*Relink this source before opening an offline occurrence/,
  'offline index rows stay visible but are disabled with a concise recovery reason')
assert.match(sourceMonitor, /callVerb\('ui\.state', \{\}\)[\s\S]*ui\.result\?\.playhead_ms !== use\.atMs/,
  'an already-satisfied playhead conflict is accepted only after authoritative UI-state verification')
assert.match(sourceMonitor, /catch \{\s*setUses\(null\)[\s\S]*setUsesNote\('Server unreachable'\)[\s\S]*finally \{\s*setUsesBusy\(false\)/,
  'a rejected All uses transport clears loading, reports a retryable error, and leaves Hide uses reachable')

console.log('PASS source-monitor All uses: exact asset filter, stable rows, transport recovery, offline refusal')
