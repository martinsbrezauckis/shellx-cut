// Exhaustive conditional Transcript/Reel/Timeline action coverage.
//
// The normal transcript section runs against real STT and remains the release
// proof for provider-backed behavior. This deterministic companion uses the
// product's installed `?mock=1` surface to prove every local state transition
// and exact request shape that would otherwise become content-dependent or
// unreachable when a fixture contains no fillers, retakes, removed words, or
// missing-perception state.

import { resolveCoverageAppUrl } from './fullCoverageAppUrl.mjs'

export function createTranscriptActionCoverage({
  app,
  probe,
  captureVerbResp,
  sleep,
  closeOverlays,
}) {
  const surface = 'transcript-actions'

  function mockUrl(activeApp, { transcriptMissing = false, kinetic = false } = {}) {
    const url = new URL(activeApp)
    url.searchParams.set('mock', '1')
    if (transcriptMissing) url.searchParams.set('mockTranscriptMissing', '1')
    if (kinetic) url.searchParams.set('mockKinetic', '1')
    return url.toString()
  }

  async function resetMock(page, activeApp, options = {}) {
    await page.evaluate(() => localStorage.removeItem('cut.layout.v1'))
    await page.goto(mockUrl(activeApp, options), { waitUntil: 'domcontentloaded' })
    await sleep(150)
    await closeOverlays(page)
    await page.locator('[data-cut-left-tab="transcript"]').waitFor({
      state: 'visible',
      timeout: 8000,
    })
    await page.locator('[data-cut-left-tab="transcript"]').click()
    await page.locator('[data-cut-panel="transcript"]').waitFor({
      state: 'visible',
      timeout: 8000,
    })
  }

  async function calls(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__cutMock?.calls?.() ?? [])))
  }

  async function callCount(page) {
    return (await calls(page)).length
  }

  async function callSince(page, name, before) {
    return (await calls(page)).slice(before).find((entry) => entry.name === name)
  }

  function matchesArgs(actual, expected) {
    return Object.entries(expected).every(
      ([key, value]) => JSON.stringify(actual?.[key]) === JSON.stringify(value),
    )
  }

  async function selectSpan(first, last, sameWord) {
    await first.click()
    if (sameWord) return
    // WKWebView's WebDriver loses modifier state between its key and pointer
    // sources. The first endpoint remains a real native click; dispatch the
    // second mousedown with the exact Shift state consumed by the product.
    await last.dispatchEvent('mousedown', {
      button: 0,
      buttons: 1,
      shiftKey: true,
    })
    await last.dispatchEvent('mouseup', {
      button: 0,
      buttons: 0,
      shiftKey: true,
    })
  }

  async function selectTimelineSpan(page, from, to) {
    const first = page.locator(`[data-cut-timeline-word="${from}"]`)
    const last = page.locator(`[data-cut-timeline-word="${to}"]`)
    await selectSpan(first, last, to === from)
    await page.locator('[data-cut-timeline-cutbar]').waitFor({
      state: 'visible',
      timeout: 5000,
    })
  }

  async function selectSourceSpan(page, from, to) {
    await page.keyboard.press('Escape')
    await page.locator('.tx__cut-toolbar').waitFor({
      state: 'detached',
      timeout: 5000,
    }).catch(() => {})
    const first = page.locator(`[data-cut-word="a1:${from}"]`)
    const last = page.locator(`[data-cut-word="a1:${to}"]`)
    await selectSpan(first, last, to === from)
    await page.locator('[data-cut-action="add-to-reel"]').waitFor({
      state: 'visible',
      timeout: 5000,
    })
  }

  async function selectSourceAction(page, from, to, action) {
    await page.keyboard.press('Escape')
    await page.locator('.tx__cut-toolbar').waitFor({
      state: 'detached',
      timeout: 5000,
    }).catch(() => {})
    await page.locator('[data-cut-left-tab="transcript"]').click()
    await page.locator('[data-cut-panel="transcript"]').waitFor({ state: 'visible', timeout: 5000 })
    await page.locator('[data-cut-action="view-source"]').click()
    await page.locator('[data-cut-action="view-source"].tx__viewbtn--on').waitFor({
      state: 'visible',
      timeout: 12_000,
    })
    const first = page.locator(`[data-cut-word="a1:${from}"]`)
    const last = page.locator(`[data-cut-word="a1:${to}"]`)
    await first.waitFor({ state: 'visible', timeout: 5000 })
    await first.scrollIntoViewIfNeeded()
    await last.scrollIntoViewIfNeeded()
    await selectSpan(first, last, to === from)
    const toolbar = page.locator('.tx__cut-toolbar')
    if (!await toolbar.isVisible().catch(() => false)) {
      // A native WKWebView click may deliver its document-level mouseup after
      // the Shift endpoint and clear the provisional range. Retry only when
      // the product did not mount its selection toolbar, using the exact
      // mousedown/Shift-mousedown/mouseup state machine the component consumes.
      await first.dispatchEvent('mousedown', { button: 0, buttons: 1 })
      await last.dispatchEvent('mousedown', {
        button: 0,
        buttons: 1,
        shiftKey: true,
      })
      await last.dispatchEvent('mouseup', {
        button: 0,
        buttons: 0,
        shiftKey: true,
      })
    }
    await toolbar.waitFor({ state: 'visible', timeout: 12_000 })
    await page.locator(`[data-cut-action="${action}"]`).waitFor({
      state: 'visible',
      timeout: 12_000,
    })
  }

  async function ensureTools(page) {
    if (await page.locator('[data-cut-tools-menu]').count()) return
    await page.locator('[data-cut-action="tools-menu"]').click()
    await page.locator('[data-cut-tools-menu]').waitFor({
      state: 'visible',
      timeout: 5000,
    })
  }

  async function run(page) {
    const activeApp = await resolveCoverageAppUrl(page, app)
    await resetMock(page, activeApp)
    const panel = page.locator('[data-cut-panel="transcript"]').first()

    try {
      // PROGRAM loads an EDL-aware transcript and gives the timeline controls a
      // deterministic, same-clip word range.
      let before = await callCount(page)
      await probe(page, {
        surface,
        name: 'view-program',
        actionId: 'view-program',
        sel: panel.locator('[data-cut-action="view-program"]'),
        group: panel,
        groupName: 'transcript-program',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="view-program"]').click()
          await panel.locator('[data-cut-timeline-word="0"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.timeline', before)
          return {
            ok: JSON.stringify(call?.args) === '{}'
              && await panel.locator('[data-cut-transcript-view="program"]').count() === 1
              && await panel.locator('[data-cut-timeline-word]').count() === 12,
            detail: `transcript.timeline args=${JSON.stringify(call?.args)}; timeline words=${await panel.locator('[data-cut-timeline-word]').count()}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'view-selected-clip',
        actionId: 'view-clip',
        sel: panel.locator('[data-cut-action="view-clip"]'),
        group: panel,
        groupName: 'transcript-selected-clip',
        doClick: async () => {
          await panel.locator('[data-cut-action="view-clip"]').click()
          await panel.locator('[data-cut-timeline-empty]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await panel.locator('[data-cut-transcript-view="clip"]').count() === 1
            && (await panel.locator('[data-cut-timeline-empty]').textContent())?.includes('Select a clip'),
          detail: `scope=${await panel.locator('[data-cut-transcript-view]').getAttribute('data-cut-transcript-view')}; empty="${(await panel.locator('[data-cut-timeline-empty]').textContent())?.trim()}"`,
        }),
      })

      await panel.locator('[data-cut-action="view-program"]').click()
      await panel.locator('[data-cut-timeline-word="0"]').waitFor({
        state: 'visible',
        timeout: 5000,
      })

      await selectTimelineSpan(page, 0, 1)
      await probe(page, {
        surface,
        name: 'timeline-clear-selection',
        actionId: 'timeline-clear-sel',
        sel: panel.locator('[data-cut-action="timeline-clear-sel"]'),
        group: panel.locator('[data-cut-timeline-cutbar]'),
        groupName: 'transcript-timeline-selection',
        doClick: async () => {
          await panel.locator('[data-cut-action="timeline-clear-sel"]').click()
          await panel.locator('[data-cut-timeline-cutbar]').waitFor({
            state: 'detached',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await panel.locator('[data-cut-timeline-cutbar]').count() === 0
            && await panel.locator('.txv__w--sel').count() === 0,
          detail: `cut bar=${await panel.locator('[data-cut-timeline-cutbar]').count()}; selected words=${await panel.locator('.txv__w--sel').count()}`,
        }),
      })

      await selectTimelineSpan(page, 0, 1)
      await probe(page, {
        surface,
        name: 'timeline-cut-words',
        actionId: 'timeline-cut-words',
        sel: panel.locator('[data-cut-action="timeline-cut-words"]'),
        group: panel.locator('[data-cut-timeline-cutbar]'),
        groupName: 'transcript-timeline-cut',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="timeline-cut-words"]').click()
          await panel.locator('[data-cut-timeline-cutbar]').waitFor({
            state: 'detached',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.cut_words', before)
          const expected = {
            asset: 'a1',
            word_range: [0, 1],
            clip: 'c1',
            rationale: 'transcript (timeline view) cut',
          }
          return {
            ok: matchesArgs(call?.args, expected),
            detail: `transcript.cut_words args=${JSON.stringify(call?.args)}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'view-source',
        actionId: 'view-source',
        sel: panel.locator('[data-cut-action="view-source"]'),
        group: panel,
        groupName: 'transcript-source',
        doClick: async () => {
          await panel.locator('[data-cut-action="view-source"]').click()
          await panel.locator('[data-cut-word="a1:3"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await panel.locator('[data-cut-transcript-view="source"]').count() === 1
            && await panel.locator('[data-cut-word]').count() > 20,
          detail: `source words=${await panel.locator('[data-cut-word]').count()}`,
        }),
      })

      const transcriptSearch = panel.locator('[data-cut-transcript-search]')
      await probe(page, {
        surface,
        name: 'search-transcript',
        actionId: 'transcript-search',
        sel: transcriptSearch,
        group: panel,
        groupName: 'transcript-search',
        doClick: async () => {
          before = await callCount(page)
          await transcriptSearch.fill('today')
          await transcriptSearch.press('Enter')
          await panel.locator('[data-cut-search-note]').waitFor({ state: 'visible', timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.search', before)
          const note = (await panel.locator('[data-cut-search-note]').textContent())?.trim() || ''
          return {
            ok: call?.args?.asset === 'a1' && call?.args?.query === 'today' && /match/i.test(note),
            detail: `transcript.search args=${JSON.stringify(call?.args)}; note="${note}"`,
          }
        },
      })

      const restore = panel.locator('[data-cut-action="restore"][data-cut-op="op_000002"]')
      const removed = panel.locator('[data-cut-removed="op_000002"]')
      await removed.scrollIntoViewIfNeeded()
      await restore.waitFor({ state: 'visible', timeout: 5000 })
      let restoreResponse = null
      await probe(page, {
        surface,
        name: 'restore-removed-words',
        actionId: 'restore',
        sel: restore,
        group: restore,
        groupName: 'transcript-removed-words',
        doClick: async () => {
          before = await callCount(page)
          restoreResponse = await captureVerbResp(
            page,
            'edit.restore',
            () => restore.click(),
            12_000,
          )
          await removed.waitFor({ state: 'detached', timeout: 12_000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'edit.restore', before)
          const restored = await panel.locator('[data-cut-removed="op_000002"]').count() === 0
          return {
            // The installed WebView adapter can miss the synthetic mock
            // Response even though the exact call and resulting DOM transition
            // are both observable. An explicit not-ok response still fails;
            // otherwise the exact request plus confirmed restoration is the
            // stronger product result.
            ok: restoreResponse?.ok !== false
              && matchesArgs(call?.args, { op_id: 'op_000002' })
              && restored,
            detail: `edit.restore response=${restoreResponse?.ok ?? 'not-observed'} args=${JSON.stringify(call?.args)}; restored=${restored}`,
          }
        },
      })

      await selectSourceAction(page, 3, 4, 'ignore-words')
      await probe(page, {
        surface,
        name: 'ignore-source-words',
        actionId: 'ignore-words',
        sel: panel.locator('[data-cut-action="ignore-words"]'),
        group: panel,
        groupName: 'transcript-source-actions',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="ignore-words"]').click()
          await panel.locator('[data-cut-word-ignored]').first().waitFor({ state: 'attached', timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.ignore_words', before)
          return {
            ok: JSON.stringify(call?.args?.word_range) === JSON.stringify([3, 4])
              && call?.args?.remove !== true
              && await panel.locator('[data-cut-word-ignored]').count() >= 2,
            detail: `ignore args=${JSON.stringify(call?.args)}; ignored words=${await panel.locator('[data-cut-word-ignored]').count()}`,
          }
        },
      })
      await selectSourceAction(page, 3, 4, 'unignore-words')
      await probe(page, {
        surface,
        name: 'unignore-source-words',
        actionId: 'unignore-words',
        sel: panel.locator('[data-cut-action="unignore-words"]'),
        group: panel,
        groupName: 'transcript-source-actions',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="unignore-words"]').click()
          await panel.locator('[data-cut-word-ignored]').first().waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.ignore_words', before)
          return {
            ok: call?.args?.remove === true && await panel.locator('[data-cut-word-ignored]').count() === 0,
            detail: `unignore args=${JSON.stringify(call?.args)}; ignored words=${await panel.locator('[data-cut-word-ignored]').count()}`,
          }
        },
      })

      await selectSourceAction(page, 5, 6, 'mute-words')
      await probe(page, {
        surface,
        name: 'mute-source-words',
        actionId: 'mute-words',
        sel: panel.locator('[data-cut-action="mute-words"]'),
        group: panel,
        groupName: 'transcript-source-actions',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="mute-words"]').click()
          await panel.locator('[data-cut-word-muted]').first().waitFor({ state: 'attached', timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.mute_words', before)
          return {
            ok: JSON.stringify(call?.args?.word_range) === JSON.stringify([5, 6])
              && await panel.locator('[data-cut-word-muted]').count() >= 2,
            detail: `mute args=${JSON.stringify(call?.args)}; muted words=${await panel.locator('[data-cut-word-muted]').count()}`,
          }
        },
      })
      await selectSourceAction(page, 5, 6, 'unmute-words')
      await probe(page, {
        surface,
        name: 'unmute-source-words',
        actionId: 'unmute-words',
        sel: panel.locator('[data-cut-action="unmute-words"]'),
        group: panel,
        groupName: 'transcript-source-actions',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="unmute-words"]').click()
          await panel.locator('[data-cut-word-muted]').first().waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'edit.mute_range', before)
          return {
            ok: call?.args?.clip === 'c2'
              && Array.isArray(call?.args?.remove_ms)
              && await panel.locator('[data-cut-word-muted]').count() === 0,
            detail: `unmute args=${JSON.stringify(call?.args)}; muted words=${await panel.locator('[data-cut-word-muted]').count()}`,
          }
        },
      })

      await selectSourceAction(page, 12, 13, 'cut-words')
      await probe(page, {
        surface,
        name: 'cut-source-words',
        actionId: 'cut-words',
        sel: panel.locator('[data-cut-action="cut-words"]'),
        group: panel,
        groupName: 'transcript-source-actions',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="cut-words"]').click()
          await sleep(250)
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.cut_words', before)
          return {
            ok: JSON.stringify(call?.args?.word_range) === JSON.stringify([12, 13])
              && await panel.locator('[data-cut-removed]').count() >= 1,
            detail: `cut args=${JSON.stringify(call?.args)}; removed groups=${await panel.locator('[data-cut-removed]').count()}`,
          }
        },
      })

      await ensureTools(page)
      const aggressiveness = panel.locator('[data-cut-aggressiveness]')
      await probe(page, {
        surface,
        name: 'silence-aggressiveness',
        actionId: 'aggressiveness',
        sel: aggressiveness,
        group: panel.locator('[data-cut-tools-menu]'),
        groupName: 'transcript-tools-silence',
        doClick: async () => {
          await aggressiveness.selectOption('natural')
          await sleep(80)
        },
        assertResult: async () => ({
          ok: await aggressiveness.inputValue() === 'natural'
            && !(await panel.locator('[data-cut-action="silence-pass"]').isDisabled()),
          detail: `aggressiveness=${await aggressiveness.inputValue()}; silence enabled=${!(await panel.locator('[data-cut-action="silence-pass"]').isDisabled())}`,
        }),
      })
      await probe(page, {
        surface,
        name: 'silence-pass',
        actionId: 'silence-pass',
        sel: panel.locator('[data-cut-action="silence-pass"]'),
        group: panel.locator('[data-cut-tools-menu]'),
        groupName: 'transcript-tools-silence',
        doClick: async () => {
          const silencePass = panel.locator('[data-cut-action="silence-pass"]')
          await silencePass.scrollIntoViewIfNeeded()
          await page.waitForFunction(() => {
            const control = document.querySelector('[data-cut-action="silence-pass"]')
            if (!(control instanceof HTMLButtonElement) || control.disabled) return false
            const rect = control.getBoundingClientRect()
            const hit = document.elementFromPoint(
              rect.left + rect.width / 2,
              rect.top + rect.height / 2,
            )
            return hit === control || control.contains(hit)
          }, undefined, { timeout: 5000 })
          before = await callCount(page)
          await silencePass.click()
          await page.waitForFunction((start) => (
            (window.__cutMock?.calls?.() ?? [])
              .slice(start)
              .some((entry) => entry.name === 'transcript.remove_silences')
          ), before, { timeout: 5000 })
          await page.waitForFunction(() => (
            document.querySelector('.tx__pass-note')
              ?.textContent
              ?.includes('silence pass: 1 cuts') === true
          ), undefined, { timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.remove_silences', before)
          const copy = (await panel.textContent())?.replace(/\s+/g, ' ') || ''
          return {
            ok: call?.args?.aggressiveness === 'natural' && copy.includes('silence pass: 1 cuts'),
            detail: `remove_silences args=${JSON.stringify(call?.args)}; completion visible=${copy.includes('silence pass: 1 cuts')}`,
          }
        },
      })

      for (const pass of [
        {
          name: 'filler-pass',
          actionId: 'filler-pass',
          selector: '[data-cut-action="filler-pass"]',
          verb: 'transcript.remove_fillers',
          expectedNote: 'filler pass: 2 cuts',
        },
        {
          name: 'retakes-pass',
          actionId: 'retakes-pass',
          selector: '[data-cut-action="retakes-pass"]',
          verb: 'transcript.remove_retakes',
          expectedNote: 'retakes pass: 1 cuts',
        },
        {
          name: 'generate-captions',
          actionId: 'generate-captions',
          selector: '[data-cut-action="generate-captions"]',
          verb: 'captions.generate',
          expectedNote: 'captions generated',
        },
      ]) {
        await ensureTools(page)
        const menu = panel.locator('[data-cut-tools-menu]')
        await probe(page, {
          surface,
          name: pass.name,
          actionId: pass.actionId,
          sel: menu.locator(pass.selector),
          group: menu,
          groupName: `transcript-tools-${pass.name}`,
          doClick: async () => {
            before = await callCount(page)
            await menu.locator(pass.selector).click()
            await sleep(100)
          },
          assertResult: async () => {
            const call = await callSince(page, pass.verb, before)
            const copy = (await panel.textContent())?.replace(/\s+/g, ' ') ?? ''
            return {
              ok: JSON.stringify(call?.args) === '{}' && copy.includes(pass.expectedNote),
              detail: `${pass.verb} args=${JSON.stringify(call?.args)}; note="${pass.expectedNote}" visible=${copy.includes(pass.expectedNote)}`,
            }
          },
        })
      }

      await ensureTools(page)
      await probe(page, {
        surface,
        name: 'generate-chapters',
        actionId: 'generate-chapters',
        sel: panel.locator('[data-cut-action="generate-chapters"]'),
        group: panel.locator('[data-cut-tools-menu]'),
        groupName: 'transcript-tools-chapters',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="generate-chapters"]').click()
          await sleep(150)
        },
        assertResult: async () => {
          const callsSince = (await calls(page)).slice(before)
          const chapters = callsSince.find((entry) => entry.name === 'transcript.chapters')
          const markers = callsSince.filter((entry) => entry.name === 'edit.add_marker')
          const copy = (await panel.textContent())?.replace(/\s+/g, ' ') || ''
          return {
            ok: chapters?.args?.asset === 'a1' && markers.length === 2 && copy.includes('chapters: 2 found, 2 markers added'),
            detail: `chapters args=${JSON.stringify(chapters?.args)}; markers=${markers.length}; completion visible=${copy.includes('chapters: 2 found, 2 markers added')}`,
          }
        },
      })

      await ensureTools(page)
      await page.keyboard.press('Escape')
      await panel.locator('[data-cut-tools-menu]').waitFor({
        state: 'detached',
        timeout: 5000,
      })
      await ensureTools(page)
      const reelModeAction = panel.locator('[data-cut-action="reel-mode"]')
      await reelModeAction.scrollIntoViewIfNeeded()
      await probe(page, {
        surface,
        name: 'reel-mode',
        actionId: 'reel-mode',
        sel: reelModeAction,
        group: reelModeAction,
        groupName: 'transcript-reel-mode',
        doClick: async () => {
          await reelModeAction.focus()
          await reelModeAction.click({ force: true })
          await panel.locator('[data-cut-reel]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
          await page.keyboard.press('Escape')
        },
        assertResult: async () => ({
          ok: await panel.locator('[data-cut-reel]').isVisible()
            && await panel.locator('[data-cut-reel-empty]').isVisible(),
          detail: `reel visible=${await panel.locator('[data-cut-reel]').isVisible()}; empty tray=${await panel.locator('[data-cut-reel-empty]').isVisible()}`,
        }),
      })

      await selectSourceSpan(page, 3, 4)
      await probe(page, {
        surface,
        name: 'add-selection-to-reel',
        actionId: 'add-to-reel',
        sel: panel.locator('[data-cut-action="add-to-reel"]'),
        group: panel,
        groupName: 'transcript-add-to-reel',
        doClick: async () => {
          await panel.locator('[data-cut-action="add-to-reel"]').click()
          await panel.locator('[data-cut-reel-span="3-4"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await panel.locator('[data-cut-reel-span="3-4"]').count() === 1
            && (await panel.locator('[data-cut-reel-span="3-4"]').textContent())?.includes('today I'),
          detail: `span 3-4="${(await panel.locator('[data-cut-reel-span="3-4"]').textContent())?.replace(/\s+/g, ' ').trim()}"`,
        }),
      })

      await selectSourceSpan(page, 5, 6)
      await panel.locator('[data-cut-action="add-to-reel"]').click()
      await panel.locator('[data-cut-reel-span="5-6"]').waitFor({
        state: 'visible',
        timeout: 5000,
      })
      await probe(page, {
        surface,
        name: 'remove-span-from-reel',
        actionId: 'reel-remove',
        sel: panel.locator('[data-cut-reel-span="3-4"] [data-cut-action="reel-remove"]'),
        group: panel.locator('[data-cut-reel]'),
        groupName: 'transcript-reel-remove',
        doClick: async () => {
          await panel.locator('[data-cut-reel-span="3-4"] [data-cut-action="reel-remove"]').click()
          await panel.locator('[data-cut-reel-span="3-4"]').waitFor({
            state: 'detached',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await panel.locator('[data-cut-reel-span]').count() === 1
            && await panel.locator('[data-cut-reel-span="5-6"]').count() === 1,
          detail: `remaining spans=${await panel.locator('[data-cut-reel-span]').count()}; 5-6=${await panel.locator('[data-cut-reel-span="5-6"]').count()}`,
        }),
      })

      await probe(page, {
        surface,
        name: 'assemble-reel',
        actionId: 'assemble-reel',
        sel: panel.locator('[data-cut-action="assemble-reel"]'),
        group: panel.locator('[data-cut-reel]'),
        groupName: 'transcript-reel-assemble',
        doClick: async () => {
          before = await callCount(page)
          await panel.locator('[data-cut-action="assemble-reel"]').click()
          await panel.locator('[data-cut-reel-empty]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const call = await callSince(page, 'transcript.assemble', before)
          const expected = {
            asset: 'a1',
            word_ranges: [[5, 6]],
            rationale: 'highlight reel',
          }
          const note = await panel.locator('[data-cut-reel-note]').textContent()
          return {
            ok: matchesArgs(call?.args, expected)
              && note?.includes('Reel: 1 span'),
            detail: `transcript.assemble args=${JSON.stringify(call?.args)}; note="${note?.trim()}"`,
          }
        },
      })

      await selectSourceSpan(page, 7, 8)
      await panel.locator('[data-cut-action="add-to-reel"]').click()
      await panel.locator('[data-cut-reel-span="7-8"]').waitFor({
        state: 'visible',
        timeout: 5000,
      })
      await probe(page, {
        surface,
        name: 'clear-reel',
        actionId: 'reel-clear',
        sel: panel.locator('[data-cut-action="reel-clear"]'),
        group: panel.locator('[data-cut-reel]'),
        groupName: 'transcript-reel-clear',
        doClick: async () => {
          await panel.locator('[data-cut-action="reel-clear"]').click()
          await panel.locator('[data-cut-reel-empty]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await panel.locator('[data-cut-reel-span]').count() === 0
            && await panel.locator('[data-cut-action="reel-clear"]').count() === 0,
          detail: `spans=${await panel.locator('[data-cut-reel-span]').count()}; Clear action=${await panel.locator('[data-cut-action="reel-clear"]').count()}`,
        }),
      })

      await resetMock(page, activeApp, { kinetic: true })
      const kineticTranscript = page.locator('[data-cut-panel="transcript"]').first()
      await ensureTools(page)
      await probe(page, {
        surface,
        name: 'open-kinetic-captions',
        actionId: 'open-kinetic',
        sel: kineticTranscript.locator('[data-cut-action="open-kinetic"]'),
        group: kineticTranscript.locator('[data-cut-tools-menu]'),
        groupName: 'transcript-tools-kinetic',
        doClick: async () => {
          await kineticTranscript.locator('[data-cut-action="open-kinetic"]').click()
          await page.locator('[data-cut-kinetic]').waitFor({ state: 'visible', timeout: 5000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-kinetic-cuecount]').count() === 1,
          detail: `kinetic cue-count visible=${await page.locator('[data-cut-kinetic-cuecount]').count() === 1}`,
        }),
      })
      const kinetic = page.locator('[data-cut-kinetic]').first()
      await probe(page, {
        surface,
        name: 'kinetic-position',
        actionId: 'kinetic-position',
        sel: page.locator('[data-cut-kinetic-position]'),
        group: kinetic,
        groupName: 'kinetic-drawer',
        doClick: async () => { await page.locator('[data-cut-kinetic-position]').selectOption('center') },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-kinetic-position]').inputValue() === 'center',
          detail: 'kinetic position=center',
        }),
      })
      const replaceKinetic = page.locator('[data-cut-kinetic-replace]')
      const replaceBefore = await replaceKinetic.isChecked()
      await probe(page, {
        surface,
        name: 'kinetic-replace-static',
        actionId: 'kinetic-replace',
        sel: replaceKinetic,
        group: kinetic,
        groupName: 'kinetic-drawer',
        doClick: async () => { await replaceKinetic.click() },
        assertResult: async () => ({
          ok: await replaceKinetic.isChecked() !== replaceBefore,
          detail: `replace static ${replaceBefore}→${await replaceKinetic.isChecked()}`,
        }),
      })
      await probe(page, {
        surface,
        name: 'kinetic-apply',
        actionId: 'kinetic-apply',
        sel: page.locator('[data-cut-kinetic-apply]'),
        group: kinetic,
        groupName: 'kinetic-drawer',
        doClick: async () => {
          before = await callCount(page)
          await page.locator('[data-cut-kinetic-apply]').click()
          await page.locator('[data-cut-kinetic-result]').waitFor({ state: 'visible', timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'captions.kinetic', before)
          return {
            ok: call?.args?.position === 'center'
              && call?.args?.replace_static === !replaceBefore
              && await page.locator('[data-cut-kinetic-result-cues]').textContent() === '2',
            detail: `captions.kinetic args=${JSON.stringify(call?.args)}; cues=${await page.locator('[data-cut-kinetic-result-cues]').textContent()}`,
          }
        },
      })
      await probe(page, {
        surface,
        name: 'kinetic-close',
        actionId: 'kinetic-close',
        sel: page.locator('[data-cut-kinetic-close]'),
        group: kinetic,
        groupName: 'kinetic-drawer',
        doClick: async () => {
          await page.locator('[data-cut-kinetic-close]').click()
          await page.locator('[data-cut-kinetic]').waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-kinetic]').count() === 0,
          detail: 'Kinetic drawer closed through visible Close action',
        }),
      })

      // A second deterministic fixture strips the transcript and reports STT
      // missing, making the first-run installation CTA reachable.
      await resetMock(page, activeApp, { transcriptMissing: true })
      const setupPanel = page.locator('[data-cut-panel="transcript"]').first()
      await setupPanel.locator('[data-cut-perception-setup]').waitFor({
        state: 'visible',
        timeout: 8000,
      })
      await probe(page, {
        surface,
        name: 'install-captions-from-transcript',
        actionId: 'setup-perception',
        sel: setupPanel.locator('[data-cut-action="setup-perception"]'),
        group: setupPanel.locator('[data-cut-perception-setup]'),
        groupName: 'transcript-perception-setup',
        doClick: async () => {
          before = await callCount(page)
          await setupPanel.locator('[data-cut-action="setup-perception"]').click()
          await setupPanel.locator('[data-cut-perception-setup-progress]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const call = await callSince(page, 'system.setup_perception', before)
          const progress = await setupPanel.locator('[data-cut-perception-setup-progress]').textContent()
          return {
            ok: matchesArgs(call?.args, { warm_model: true })
              && !!progress?.includes('starting'),
            detail: `system.setup_perception args=${JSON.stringify(call?.args)}; progress="${progress?.trim()}"`,
          }
        },
      })
    } finally {
      await page.goto(activeApp, { waitUntil: 'domcontentloaded' }).catch(() => {})
      await sleep(250)
    }
  }

  return { run }
}
