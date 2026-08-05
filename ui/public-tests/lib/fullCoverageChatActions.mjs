// Deterministic Agent Chat action coverage.
//
// The normal `agent` section retains provider-backed end-to-end prompts. This
// companion drives the product's installed `?mock=1` App so every local prompt,
// attachment, preview, review, whole-turn revert, and retry control is proven
// without spending a subscription turn or depending on one provider's quota.

import { resolveCoverageAppUrl } from './fullCoverageAppUrl.mjs'

export function createChatActionCoverage({
  app,
  probe,
  sleep,
  closeOverlays,
  ensureRail,
}) {
  const surface = 'chat-actions'

  function mockUrl(activeApp) {
    const url = new URL(activeApp)
    url.searchParams.set('mock', '1')
    return url.toString()
  }

  async function resetMock(page, activeApp) {
    await page.evaluate(() => {
      localStorage.setItem('cut.layout.v1', JSON.stringify({
        txFrac: 0.4,
        tlH: 280,
        railW: 420,
        railCollapsed: false,
        railPinned: false,
        leftCollapsed: false,
        leftTab: 'projects',
        findSurface: 'find-media',
        workspaceMode: 'edit',
        rightTab: 'chat',
      }))
      localStorage.setItem('cut.chatAgent', 'claude')
      localStorage.removeItem('shellx-cut:reviewed:demo-cut')
    })
    await page.goto(mockUrl(activeApp), { waitUntil: 'domcontentloaded' })
    await sleep(150)
    await closeOverlays(page)
    await ensureRail(page)
    await page.locator('[data-cut-right-tab="chat"]').click()
    await page.locator('[data-cut-chat]').waitFor({
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

  function matchesRevertArgs(actual, expected) {
    return actual?.to === expected.to
      && actual?.if_tip === expected.if_tip
      && actual?.rationale === expected.rationale
  }

  async function sendTurn(page, message) {
    const chat = page.locator('[data-cut-chat]').first()
    const beforeReviews = await chat.locator('[data-cut-chat-review]').count()
    const beforeCalls = await callCount(page)
    await chat.locator('[data-cut-chat-input]').fill(message)
    await chat.locator('[data-cut-chat-send]').click()
    await chat.locator('[data-cut-chat-review]').nth(beforeReviews).waitFor({
      state: 'visible',
      timeout: 8000,
    })
    return {
      call: await callSince(page, 'agent.chat', beforeCalls),
      review: chat.locator('[data-cut-chat-review]').nth(beforeReviews),
    }
  }

  async function openAttachments(page) {
    const chat = page.locator('[data-cut-chat]').first()
    if (await chat.locator('[data-cut-chat-attachment-menu]').count()) return
    await chat.locator('[data-cut-chat-attach]').click()
    await chat.locator('[data-cut-chat-attachment-menu]').waitFor({
      state: 'visible',
      timeout: 5000,
    })
  }

  async function revealReviewCard(chat, review) {
    const chatLog = chat.locator('[data-cut-chat-log]').first()
    await chatLog.evaluate((log) => { log.scrollTop = log.scrollHeight })
    await review.waitFor({ state: 'visible', timeout: 5000 })
    await review.evaluate((card) => {
      const log = card.closest('[data-cut-chat-log]')
      if (log instanceof HTMLElement) {
        const cardRect = card.getBoundingClientRect()
        const logRect = log.getBoundingClientRect()
        const centeredTop = (logRect.height - cardRect.height) / 2
        log.scrollTop = Math.max(0, log.scrollTop + cardRect.top - logRect.top - centeredTop)
      }
    })
    const reviewVisibleInLog = await review.evaluate((card) => {
      const log = card.closest('[data-cut-chat-log]')
      if (!(log instanceof HTMLElement)) return false
      const cardRect = card.getBoundingClientRect()
      const logRect = log.getBoundingClientRect()
      return cardRect.width > 0
        && cardRect.height > 0
        && cardRect.top >= logRect.top
        && cardRect.bottom <= logRect.bottom
    })
    if (!reviewVisibleInLog) {
      throw new Error('Agent Chat review card is outside the visible chat log after scrolling')
    }
  }

  async function run(page) {
    const activeApp = await resolveCoverageAppUrl(page, app)
    await resetMock(page, activeApp)
    const chat = page.locator('[data-cut-chat]').first()

    try {
      await probe(page, {
        surface,
        name: 'open-prompt-library',
        actionId: 'chat-prompt-library',
        sel: chat.locator('[data-cut-chat-prompt-library]'),
        group: chat.locator('[data-cut-chat-chips]'),
        groupName: 'chat-prompt-library-trigger',
        doClick: async () => {
          await chat.locator('[data-cut-chat-prompt-library]').click()
          await chat.locator('[data-cut-chat-prompt-menu]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await chat.locator('[data-cut-chat-prompt-menu]').isVisible()
            && await chat.locator('[data-cut-chat-prompt-group]').count() === 4,
          detail: `prompt groups=${await chat.locator('[data-cut-chat-prompt-group]').count()}; menu visible=${await chat.locator('[data-cut-chat-prompt-menu]').isVisible()}`,
        }),
      })

      const prompt = chat.locator('[data-cut-chat-prompt="preflight-review"]')
      await probe(page, {
        surface,
        name: 'choose-library-prompt',
        actionId: 'chat-prompt',
        sel: prompt,
        group: chat.locator('[data-cut-chat-prompt-menu]'),
        groupName: 'chat-prompt-library',
        doClick: async () => {
          await prompt.click()
          await chat.locator('[data-cut-chat-prompt-menu]').waitFor({
            state: 'detached',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const value = await chat.locator('[data-cut-chat-input]').inputValue()
          const focused = await chat.locator('[data-cut-chat-input]').evaluate((input) => document.activeElement === input)
          return {
            ok: value === 'Run pre-render checks for pacing, captions, delivery, and brand. Report the issues without changing the timeline.'
              && focused,
            detail: `prefill="${value}"; input focused=${focused}`,
          }
        },
      })
      await chat.locator('[data-cut-chat-input]').fill('')

      await probe(page, {
        surface,
        name: 'open-attachment-picker',
        actionId: 'chat-attach',
        sel: chat.locator('[data-cut-chat-attach]'),
        group: chat.locator('.chat__compose'),
        groupName: 'chat-attachment-trigger',
        doClick: async () => {
          await chat.locator('[data-cut-chat-attach]').click()
          await chat.locator('[data-cut-chat-attachment-menu]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await chat.locator('[data-cut-chat-attachment-menu]').isVisible()
            && await chat.locator('[data-cut-chat-attachment]').count() === 3,
          detail: `asset choices=${await chat.locator('[data-cut-chat-attachment]').count()}; picker visible=${await chat.locator('[data-cut-chat-attachment-menu]').isVisible()}`,
        }),
      })

      await probe(page, {
        surface,
        name: 'select-chat-attachment',
        actionId: 'chat-attachment',
        sel: chat.locator('[data-cut-chat-attachment="a1"]'),
        group: chat.locator('[data-cut-chat-attachment-menu]'),
        groupName: 'chat-attachment-menu',
        doClick: async () => {
          await chat.locator('[data-cut-chat-attachment="a1"]').click()
          await chat.locator('[data-cut-chat-attachments="1"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await chat.locator('[data-cut-chat-attachment="a1"]').getAttribute('aria-selected') === 'true'
            && (await chat.locator('[data-cut-chat-attachments="1"]').textContent())?.includes('talking_head.mp4'),
          detail: `a1 selected=${await chat.locator('[data-cut-chat-attachment="a1"]').getAttribute('aria-selected')}; chip="${(await chat.locator('[data-cut-chat-attachments="1"]').textContent())?.replace(/\s+/g, ' ').trim()}"`,
        }),
      })

      await page.keyboard.press('Escape')
      await chat.locator('[data-cut-chat-attachment-menu]').waitFor({
        state: 'detached',
        timeout: 5000,
      })
      await probe(page, {
        surface,
        name: 'remove-chat-attachment',
        actionId: 'chat-attachment-remove',
        sel: chat.locator('[data-cut-chat-attachment-remove="a1"]'),
        group: chat.locator('[data-cut-chat-attachments="1"]'),
        groupName: 'chat-attachment-chip',
        doClick: async () => {
          await chat.locator('[data-cut-chat-attachment-remove="a1"]').click()
          await chat.locator('[data-cut-chat-attachments]').waitFor({
            state: 'detached',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await chat.locator('[data-cut-chat-attachments]').count() === 0,
          detail: `attachment strip count=${await chat.locator('[data-cut-chat-attachments]').count()}`,
        }),
      })

      // Reattach a1 so the first reviewable turn also proves registered asset
      // IDs reach agent.chat and are rendered on the user turn.
      await openAttachments(page)
      await chat.locator('[data-cut-chat-attachment="a1"]').click()
      await page.keyboard.press('Escape')
      const firstMessage = 'Add a marker and keep this turn reviewable'
      const first = await sendTurn(page, firstMessage)
      if (
        first.call?.args?.message !== firstMessage
        || first.call?.args?.agent !== 'claude'
        || JSON.stringify(first.call?.args?.attachments) !== JSON.stringify(['a1'])
      ) {
        throw new Error(`first agent.chat args=${JSON.stringify(first.call?.args)}`)
      }
      await chat.locator('[data-cut-chat-turn-attachment="a1"]').waitFor({
        state: 'visible',
        timeout: 5000,
      })

      await probe(page, {
        surface,
        name: 'preview-chat-turn',
        actionId: 'chat-preview',
        sel: first.review.locator('[data-cut-chat-preview]'),
        group: first.review,
        groupName: 'chat-turn-review',
        doClick: async () => {
          await first.review.locator('[data-cut-chat-preview]').click()
          await page.locator('[data-cut-panel="preview"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const focused = await page.locator('[data-cut-panel="preview"]').evaluate((preview) => document.activeElement === preview)
          return {
            ok: focused,
            detail: `Preview panel focused=${focused}; composed surface="${await page.locator('[data-cut-preview-surface]').getAttribute('data-cut-preview-surface')}"`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'inspect-chat-turn-diff',
        actionId: 'chat-diff',
        sel: first.review.locator('[data-cut-chat-diff]'),
        group: first.review,
        groupName: 'chat-turn-diff',
        doClick: async () => {
          await first.review.locator('[data-cut-chat-diff]').click()
          await page.locator('[data-cut-review-tab="diff"][aria-selected="true"]').waitFor({
            state: 'visible',
            timeout: 8000,
          })
          await page.locator('[data-cut-diff]').waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-review-tab="diff"][aria-selected="true"]').isVisible()
            && await page.locator('[data-cut-diff-from]').inputValue() === 'op_000007'
            && await page.locator('[data-cut-diff-to]').inputValue() === 'op_000008',
          detail: `Diff selected from=${await page.locator('[data-cut-diff-from]').inputValue()} to=${await page.locator('[data-cut-diff-to]').inputValue()}`,
        }),
      })

      await probe(page, {
        surface,
        name: 'accept-chat-turn',
        actionId: 'chat-accept',
        sel: first.review.locator('[data-cut-chat-accept]'),
        group: first.review,
        groupName: 'chat-turn-accept',
        doClick: async () => {
          await first.review.locator('[data-cut-chat-accept]').click()
          await first.review.waitFor({ state: 'visible', timeout: 5000 })
        },
        assertResult: async () => {
          const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('shellx-cut:reviewed:demo-cut') || '{}'))
          return {
            ok: await first.review.getAttribute('data-cut-chat-review') === 'accepted'
              && stored.op_000008 === 'accepted',
            detail: `review state=${await first.review.getAttribute('data-cut-chat-review')}; stored op_000008=${stored.op_000008}`,
          }
        },
      })

      const secondMessage = 'Create a second reversible marker turn'
      const second = await sendTurn(page, secondMessage)
      await probe(page, {
        surface,
        name: 'revert-complete-chat-turn',
        actionId: 'chat-revert',
        sel: second.review.locator('[data-cut-chat-revert]'),
        group: second.review,
        groupName: 'chat-turn-revert',
        doClick: async () => {
          const before = await callCount(page)
          await second.review.locator('[data-cut-chat-revert]').click()
          await chat.locator('[data-cut-chat-review="reverted"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
          second.revertCall = await callSince(page, 'project.revert', before)
        },
        assertResult: async () => {
          const expected = {
            to: 'op_000008',
            if_tip: 'op_000009',
            rationale: 'revert Agent Chat turn turn_mock_2',
          }
          const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('shellx-cut:reviewed:demo-cut') || '{}'))
          return {
            ok: matchesRevertArgs(second.revertCall?.args, expected)
              && await second.review.getAttribute('data-cut-chat-review') === 'reverted'
              && stored.op_000009 === 'rejected',
            detail: `project.revert args=${JSON.stringify(second.revertCall?.args)}; state=${await second.review.getAttribute('data-cut-chat-review')}; marker=${stored.op_000009}`,
          }
        },
      })

      // A fresh pending turn proves Try again's complete behavior: atomic
      // revert first, then restore the original request and registered assets.
      await openAttachments(page)
      await chat.locator('[data-cut-chat-attachment="m1"]').click()
      await page.keyboard.press('Escape')
      const thirdMessage = 'Try this attached music adjustment again'
      const third = await sendTurn(page, thirdMessage)
      // Let the response-driven log/busy effects finish before the final
      // centering pass. Otherwise their newest-turn auto-scroll can race the
      // native geometry capture after we have already validated the card.
      await sleep(200)
      await revealReviewCard(chat, third.review)
      await probe(page, {
        surface,
        name: 'retry-complete-chat-turn',
        actionId: 'chat-retry',
        sel: third.review.locator('[data-cut-chat-retry]'),
        group: third.review,
        groupName: 'chat-turn-retry',
        doClick: async () => {
          const before = await callCount(page)
          await third.review.locator('[data-cut-chat-retry]').click()
          await chat.locator('[data-cut-chat-review="retry"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
          await chat.locator('[data-cut-chat-input]').waitFor({ state: 'visible', timeout: 5000 })
          third.revertCall = await callSince(page, 'project.revert', before)
        },
        assertResult: async () => {
          const expected = {
            to: 'op_000010',
            if_tip: 'op_000011',
            rationale: 'revert Agent Chat turn turn_mock_3',
          }
          return {
            ok: matchesRevertArgs(third.revertCall?.args, expected)
              && await third.review.getAttribute('data-cut-chat-review') === 'retry'
              && await chat.locator('[data-cut-chat-input]').inputValue() === thirdMessage
              && await chat.locator('[data-cut-chat-attachment-remove="m1"]').count() === 1,
            detail: `project.revert args=${JSON.stringify(third.revertCall?.args)}; state=${await third.review.getAttribute('data-cut-chat-review')}; input="${await chat.locator('[data-cut-chat-input]').inputValue()}"; restored m1=${await chat.locator('[data-cut-chat-attachment-remove="m1"]').count()}`,
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
