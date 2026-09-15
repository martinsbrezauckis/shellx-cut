// Library Insert branch contract. This drives the real placement controller
// through its fetch boundary; it does not need a native runner or a test-only
// production timeout hook.
import { strict as assert } from 'node:assert'

import { insertLibraryItemAtPlayhead } from '../src/panels/Library/libraryPlacement'
import type { LibItem, Project } from '../src/lib/client'

const item: LibItem = {
  id: 'library-selected',
  type: 'video',
  name: 'Selected Library clip',
  src_path: '/fixtures/selected.mp4',
  tags: [],
  added_ms: 1,
  source: 'user',
}

function project({ empty = false, imported = false }: { empty?: boolean; imported?: boolean } = {}): Project {
  return {
    schema: 'shellx-cut/project/1',
    name: 'Library placement fixture',
    settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48_000 },
    assets: {
      base: { path: '/fixtures/base.mp4', hash: 'sha256:base' },
      ...(imported ? {
        'asset-from-library': {
          path: item.src_path!, hash: 'sha256:selected', probe: { has_audio: true },
        },
      } : {}),
    },
    tracks: [
      { id: 'v1', kind: 'video', clips: empty ? [] : [{ id: 'base-video', kind: 'video', asset: 'base', src_in_ms: 0, src_out_ms: 1_000 }] },
      { id: 'a1', kind: 'audio', clips: empty ? [] : [{ id: 'base-audio', kind: 'audio', asset: 'base', src_in_ms: 0, src_out_ms: 1_000 }] },
    ],
    markers: [],
    caption_styles: {},
    checkpoints: [],
  } as Project
}

type Call = { verb: string; args: Record<string, unknown> }

async function withVerbFetch<T>(
  responder: (verb: string, args: Record<string, unknown>) => unknown | Promise<unknown>,
  action: (calls: Call[]) => Promise<T>,
): Promise<T> {
  const originalFetch = globalThis.fetch
  const calls: Call[] = []
  globalThis.fetch = (async (url: string | URL | Request, init?: RequestInit) => {
    const verb = String(url).split('/api/verb/')[1]
    const args = typeof init?.body === 'string' ? JSON.parse(init.body) as Record<string, unknown> : {}
    calls.push({ verb, args })
    return { json: async () => responder(verb, args) } as Response
  }) as typeof fetch
  try {
    return await action(calls)
  } finally {
    globalThis.fetch = originalFetch
  }
}

async function withImmediateTimers<T>(action: () => Promise<T>): Promise<T> {
  const originalSetTimeout = globalThis.setTimeout
  globalThis.setTimeout = ((callback: (...args: unknown[]) => void, _ms?: number, ...args: unknown[]) => {
    queueMicrotask(() => callback(...args))
    return 0
  }) as unknown as typeof setTimeout
  try {
    return await action()
  } finally {
    globalThis.setTimeout = originalSetTimeout
  }
}

{
  await withVerbFetch((verb, args) => {
    if (verb === 'project.state') return { ok: true, result: project({ empty: true }) }
    if (verb === 'library.add_to_project') {
      assert.deepEqual(args, { id: item.id })
      return { ok: true, result: { asset_id: 'asset-first' } }
    }
    throw new Error(`empty timeline dispatched an unexpected verb: ${verb}`)
  }, async (calls) => {
    const result = await insertLibraryItemAtPlayhead({ item, project: project(), playheadMs: 777 })
    assert.deepEqual(result, {
      ok: true,
      projectChanged: true,
      message: '"Selected Library clip" is becoming the first timeline clip',
    })
    assert.deepEqual(calls, [
      { verb: 'project.state', args: {} },
      { verb: 'library.add_to_project', args: { id: item.id } },
    ], 'the first import relies on engine auto-placement and never sends a second placement')
  })
}

{
  let stateReads = 0
  await withVerbFetch((verb, args) => {
    if (verb === 'project.state') {
      stateReads += 1
      return { ok: true, result: stateReads === 1 ? project() : project({ imported: true }) }
    }
    if (verb === 'library.add_to_project') {
      assert.deepEqual(args, { id: item.id })
      return { ok: true, result: { asset_id: 'asset-from-library' } }
    }
    if (verb === 'edit.insert_linked') {
      assert.deepEqual(args, {
        asset: 'asset-from-library', at_ms: 1_235, video_track: 'v1', audio_track: 'a1',
        ripple: true, rationale: 'insert "Selected Library clip" from Library at the playhead',
      })
      return {
        ok: true,
        result: {
          video_clip_id: 'video-inserted', audio_clip_id: 'audio-inserted',
          video_track: 'v1', audio_track: 'a1', created_video_track: false, created_audio_track: false,
          at_ms: 1_235, src_range_ms: [0, 1_000], ripple: true,
        },
      }
    }
    throw new Error(`ready Library Insert dispatched an unexpected verb: ${verb}`)
  }, async (calls) => {
    const result = await insertLibraryItemAtPlayhead({ item, project: project(), playheadMs: 1_234.6 })
    assert.deepEqual(result, {
      ok: true,
      projectChanged: true,
      message: 'Inserted "Selected Library clip" at the playhead',
    })
    assert.deepEqual(calls.map((call) => call.verb), [
      'project.state', 'library.add_to_project', 'project.state', 'edit.insert_linked',
    ])
  })
}

{
  await withImmediateTimers(async () => {
    let stateReads = 0
    await withVerbFetch((verb, args) => {
      if (verb === 'project.state') {
        stateReads += 1
        return { ok: true, result: project() }
      }
      if (verb === 'library.add_to_project') {
        assert.deepEqual(args, { id: item.id })
        return { ok: true, result: { asset_id: 'asset-awaiting-analysis' } }
      }
      throw new Error(`analysis timeout must not dispatch placement: ${verb}`)
    }, async (calls) => {
      const result = await insertLibraryItemAtPlayhead({ item, project: project(), playheadMs: 50 })
      assert.equal(result.ok, false)
      assert.equal(result.projectChanged, true)
      assert.match(result.message, /media analysis did not finish/)
      assert.equal(stateReads, 61, 'one baseline plus the product\'s bounded sixty analysis reads')
      assert.equal(calls.some((call) => call.verb.startsWith('edit.')), false)
    })
  })
}
