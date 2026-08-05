// Native action coverage for every Sequence Index control. The fixture spans
// two sequences and carries effect/visibility/lock/mute facts so each filter
// drives the public project.sequence_index contract, not only local form state.

export function createSequenceIndexActionCoverage({
  probe,
  verb,
  state,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
}) {
  const surface = 'sequence-index-actions'

  async function chooseKind(page, panel, value) {
    const selector = `[data-cut-sequence-index-kind="${value}"]`
    const control = page.locator(selector).first()
    let response = null
    await probe(page, {
      surface,
      name: `sequence-index-kind-${value}`,
      actionId: 'sequence-index-kind',
      sel: control,
      group: panel,
      groupName: 'sequence-index',
      doClick: async () => {
        response = await captureVerbResp(
          page,
          'project.sequence_index',
          () => control.click(),
          20_000,
        )
      },
      assertResult: async () => ({
        ok: response?.ok
          && response.result?.kind === value
          && await control.getAttribute('aria-pressed') === 'true',
        detail: `kind=${response?.result?.kind || 'none'} pressed=${await control.getAttribute('aria-pressed')}`,
      }),
    })
  }

  async function chooseFilter(page, panel, {
    name,
    actionId,
    selector,
    value,
    resultKey,
    expected = value || null,
  }) {
    const control = page.locator(selector).first()
    let response = null
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: panel,
      groupName: 'sequence-index',
      doClick: async () => {
        response = await captureVerbResp(
          page,
          'project.sequence_index',
          () => control.selectOption(value),
          20_000,
        )
      },
      assertResult: async () => ({
        ok: response?.ok
          && (response.result?.[resultKey] ?? null) === expected
          && await control.inputValue() === value,
        detail: `${resultKey}=${JSON.stringify(response?.result?.[resultKey] ?? null)} value=${await control.inputValue()} total=${response?.result?.total ?? 'none'}`,
      }),
    })
  }

  async function reopenIndex(page) {
    await page.locator('[data-cut-left-tab="find"]').first().click()
    await page.locator('[data-cut-find-tab="sequence-index"]').first().click()
    const panel = page.locator('[data-cut-sequence-index]').first()
    await panel.waitFor({ state: 'visible', timeout: 12_000 })
    await page.locator('[data-cut-sequence-index-summary]').first()
      .waitFor({ state: 'visible', timeout: 20_000 })
    return panel
  }

  async function run(page) {
    const { assetId } = await freshProject(page, 'sequence_index_actions')
    const initial = await state()
    const videoTrack = initial.tracks.find((track) => track.kind === 'video')
    const audioTrack = initial.tracks.find((track) => track.kind === 'audio')
    const clip = videoTrack?.clips?.find((candidate) => candidate.asset === assetId)
    if (!videoTrack?.id || !clip?.id) throw new Error('Sequence Index fixture needs an imported video clip')

    const effect = await verb('edit.effect', {
      clip: clip.id,
      effects: [{ type: 'vignette', amount: 0.35 }],
      rationale: 'fcv: Sequence Index effect filter',
    })
    if (!effect.ok) throw new Error(`Sequence Index effect fixture failed: ${effect.error?.message || effect.error?.code}`)
    const firstMarker = await verb('edit.add_marker', {
      at_ms: 420,
      label: 'FCV Index main',
      note: 'main sequence source row',
      rationale: 'fcv: Sequence Index main marker',
    })
    if (!firstMarker.ok) throw new Error(`Sequence Index marker fixture failed: ${firstMarker.error?.message || firstMarker.error?.code}`)
    const second = await verb('project.sequence_create', {
      name: 'FCV Social',
      from: 'active',
      rationale: 'fcv: Sequence Index second sequence',
    })
    const secondId = second.result?.sequence?.id || second.result?.active_sequence
    if (!second.ok || !secondId) throw new Error(`Sequence Index sequence fixture failed: ${second.error?.message || second.error?.code}`)
    const secondMarker = await verb('edit.add_marker', {
      at_ms: 860,
      label: 'FCV Index social',
      note: 'social sequence target',
      rationale: 'fcv: Sequence Index second marker',
    })
    if (!secondMarker.ok) throw new Error(`Sequence Index second marker failed: ${secondMarker.error?.message || secondMarker.error?.code}`)
    for (const [name, args] of [
      ['edit.track_visible', { track: videoTrack.id, on: false }],
      ['edit.track_lock', { track: videoTrack.id, on: true }],
      ...(audioTrack?.id ? [['edit.mute', { track: audioTrack.id, on: true }]] : []),
    ]) {
      const changed = await verb(name, { ...args, rationale: 'fcv: Sequence Index status filter' })
      if (!changed.ok) throw new Error(`${name} fixture failed: ${changed.error?.message || changed.error?.code}`)
    }

    await closeOverlays(page)
    let panel = await reopenIndex(page)

    const query = page.locator('[data-cut-sequence-index-query]').first()
    await probe(page, {
      surface,
      name: 'sequence-index-query',
      actionId: 'sequence-index-query',
      sel: query,
      group: panel,
      groupName: 'sequence-index',
      doClick: async () => { await query.fill('FCV Index social') },
      assertResult: async () => ({
        ok: await query.inputValue() === 'FCV Index social',
        detail: `query=${await query.inputValue()}`,
      }),
    })

    const search = page.locator('[data-cut-sequence-index-search]').first()
    let searched = null
    await probe(page, {
      surface,
      name: 'sequence-index-search',
      actionId: 'sequence-index-search',
      sel: search,
      group: panel,
      groupName: 'sequence-index',
      doClick: async () => {
        searched = await captureVerbResp(
          page,
          'project.sequence_index',
          () => search.click(),
          20_000,
        )
      },
      assertResult: async () => ({
        ok: searched?.ok
          && searched.result?.query === 'FCV Index social'
          && searched.result?.results?.some((row) => row.label === 'FCV Index social'),
        detail: `query=${searched?.result?.query || 'none'} markers=${searched?.result?.marker_count ?? 'none'}`,
      }),
    })

    await query.fill('')
    await captureVerbResp(page, 'project.sequence_index', () => search.click(), 20_000)
    for (const value of ['clip', 'marker', 'all']) await chooseKind(page, panel, value)

    await chooseKind(page, panel, 'marker')
    await chooseKind(page, panel, 'clip')
    for (const value of ['video', 'audio', 'caption', '']) {
      await chooseFilter(page, panel, {
        name: `sequence-index-track-${value || 'all'}`,
        actionId: 'sequence-index-track',
        selector: '[data-cut-sequence-index-track]',
        value,
        resultKey: 'track_kind',
      })
    }

    for (const value of ['issues', 'offline', 'gaps', 'effects', 'hidden', 'locked', 'muted', 'all']) {
      await chooseFilter(page, panel, {
        name: `sequence-index-status-${value}`,
        actionId: 'sequence-index-status',
        selector: '[data-cut-sequence-index-status]',
        value,
        resultKey: 'status',
        expected: value,
      })
    }

    for (const value of ['seq1', secondId, '']) {
      await chooseFilter(page, panel, {
        name: `sequence-index-sequence-${value || 'all'}`,
        actionId: 'sequence-index-sequence',
        selector: '[data-cut-sequence-index-sequence]',
        value,
        resultKey: 'sequence',
      })
    }

    await page.evaluate(() => {
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: {
          writeText: async (value) => { window.__fcvSequenceIndexCsv = value },
        },
      })
    })
    const copy = page.locator('[data-cut-sequence-index-copy]').first()
    await probe(page, {
      surface,
      name: 'sequence-index-copy',
      actionId: 'sequence-index-copy',
      sel: copy,
      group: panel,
      groupName: 'sequence-index',
      doClick: async () => { await copy.click(); await sleep(80) },
      assertResult: async () => {
        const csv = await page.evaluate(() => window.__fcvSequenceIndexCsv || '')
        const status = await page.locator('.si__copy-status').textContent()
        return {
          ok: csv.startsWith('sequence,kind,label,at_ms') && /CSV copied/.test(status || ''),
          detail: `csvBytes=${csv.length} status=${status || 'none'}`,
        }
      },
    })

    await chooseKind(page, panel, 'marker')
    await chooseKind(page, panel, 'clip')
    await chooseFilter(page, panel, {
      name: 'sequence-index-source-filter-seq2',
      actionId: 'sequence-index-sequence',
      selector: '[data-cut-sequence-index-sequence]',
      value: secondId,
      resultKey: 'sequence',
    })
    const source = page.locator('button[data-cut-sequence-index-source]').first()
    await probe(page, {
      surface,
      name: 'sequence-index-source',
      actionId: 'sequence-index-source',
      sel: source,
      group: panel,
      groupName: 'sequence-index',
      doClick: async () => { await source.click(); await sleep(120) },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-left-tab="assets"]').getAttribute('aria-selected') === 'true'
          && await page.locator('[data-cut-source-monitor]').count() === 1,
        detail: `assetsSelected=${await page.locator('[data-cut-left-tab="assets"]').getAttribute('aria-selected')} monitor=${await page.locator('[data-cut-source-monitor]').count()}`,
      }),
    })
    await page.locator('[data-cut-source-monitor-close]').first().click()

    panel = await reopenIndex(page)
    await chooseKind(page, panel, 'marker')
    await chooseFilter(page, panel, {
      name: 'sequence-index-open-filter-seq1',
      actionId: 'sequence-index-sequence',
      selector: '[data-cut-sequence-index-sequence]',
      value: 'seq1',
      resultKey: 'sequence',
    })
    const open = page.locator('button[data-cut-sequence-index-open]').first()
    let playhead = null
    await probe(page, {
      surface,
      name: 'sequence-index-open',
      actionId: 'sequence-index-open',
      sel: open,
      group: panel,
      groupName: 'sequence-index',
      doClick: async () => {
        playhead = await captureVerbResp(page, 'ui.playhead', () => open.click(), 20_000)
      },
      assertResult: async () => {
        const project = await state()
        return {
          ok: playhead?.ok && (project.active_sequence || 'seq1') === 'seq1',
          detail: `playhead=${playhead?.ok} active=${project.active_sequence || 'seq1(default)'}`,
        }
      },
    })
  }

  return { run }
}
