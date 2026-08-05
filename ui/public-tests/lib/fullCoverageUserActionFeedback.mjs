// Deterministic native-sweep coverage for the shared human-action failure
// surface. Real transport/error injection is covered separately; this module
// owns the two conditional controls that only exist while a failure is shown.

export function createUserActionFeedbackCoverage({
  probe,
  sleep,
  closeOverlays,
}) {
  const surface = 'user-action-feedback'

  async function showFeedback(page, detail) {
    await page.evaluate((next) => {
      document.dispatchEvent(new CustomEvent('cut:user-action-feedback', {
        detail: next,
      }))
    }, detail)
    await page.locator('[data-cut-user-action-feedback]').waitFor({
      state: 'visible',
      timeout: 5_000,
    })
  }

  async function run(page) {
    await closeOverlays(page)
    await showFeedback(page, {
      message: 'The video runtime is missing. Install it in Settings.',
      setupSurface: 'settings-video-performance',
    })

    const setupAlert = page.locator('[data-cut-user-action-feedback]').first()
    await probe(page, {
      surface,
      name: 'user-action-open-setup',
      sel: page.locator('[data-cut-user-action-open-setup]').first(),
      group: setupAlert,
      groupName: 'setup-action',
      doClick: async () => {
        await page.locator('[data-cut-user-action-open-setup]').first().click()
        await page.locator('[data-cut-environment]').waitFor({
          state: 'visible',
          timeout: 12_000,
        })
      },
      assertResult: async () => {
        const category = await page.locator('[data-cut-settings-body]').first()
          .getAttribute('data-cut-settings-body').catch(() => '')
        const alertCount = await page.locator('[data-cut-user-action-feedback]').count()
        return {
          ok: category === 'video-performance' && alertCount === 0,
          detail: `settings category=${category || 'missing'} feedback remaining=${alertCount}`,
        }
      },
    })

    await page.locator('[data-cut-environment-close]').first().click().catch(() => {})
    await sleep(150)
    await showFeedback(page, {
      message: 'Could not lock the selected track.',
    })

    const dismissAlert = page.locator('[data-cut-user-action-feedback]').first()
    await probe(page, {
      surface,
      name: 'user-action-dismiss',
      sel: page.locator('[data-cut-user-action-dismiss]').first(),
      group: dismissAlert,
      groupName: 'dismiss-action',
      doClick: async () => {
        await page.locator('[data-cut-user-action-dismiss]').first().click()
      },
      assertResult: async () => {
        const remaining = await page.locator('[data-cut-user-action-feedback]').count()
        return {
          ok: remaining === 0,
          detail: `feedback remaining=${remaining}`,
        }
      },
    })
  }

  return { run }
}
