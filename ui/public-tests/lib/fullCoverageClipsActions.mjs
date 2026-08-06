// Exhaustive Repurpose into shorts action coverage.
//
// Candidate discovery can depend on a transcript/perception runtime and a real
// social bundle is already rendered and manifest-validated by the residual
// verb-level gate. This module supplies deterministic envelopes for those two
// expensive boundaries while retaining the installed Clips UI, exact request
// arguments, job polling, platform state, and package-result rendering.

export function createClipsActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'clips-actions'
  const candidateAtMs = 1000
  const candidateDurMs = 3500

  async function installFixture(page, assetId) {
    await page.evaluate(({ fixtureAsset, atMs, durMs }) => {
      const target = window
      if (!target.__fcvClipsOriginalFetch) target.__fcvClipsOriginalFetch = window.fetch
      const originalFetch = target.__fcvClipsOriginalFetch
      target.__fcvClipsFixture = {
        assetId: fixtureAsset,
        candidateCalls: [],
        bundleCalls: [],
        statusCalls: [],
      }
      const fixture = target.__fcvClipsFixture
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      const packageResult = {
        bundle_id: 'fcv_bundle_1000_4500',
        range_ms: [atMs, atMs + durMs],
        status: 'ready',
        pass: true,
        issues: [],
        warnings: [],
        platforms: [{
          aspect: '9:16',
          width: 1080,
          height: 1920,
          path: '/tmp/fcv_bundle_1000_4500/9x16/clip.mp4',
          hash: 'sha256:fcv-clip',
          caption_path: null,
          caption_hash: null,
          vtt_path: null,
          vtt_hash: null,
          caption_count: 0,
          thumb: null,
          thumb_hash: null,
          receipt_id: 'receipt_fcv_9x16',
          pass: true,
          duration_ms: durMs,
        }],
        receipt_ids: ['receipt_fcv_9x16'],
        manifest_path: '/tmp/fcv_bundle_1000_4500/manifest.json',
        manifest_hash: 'sha256:fcv-manifest',
      }

      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}

        if (pathname === '/api/verb/clip.candidates') {
          fixture.candidateCalls.push(requestArgs(options))
          return envelope({
            ok: true,
            result: {
              candidates: [{
                asset: fixtureAsset,
                word_range: [2, 10],
                at_ms: atMs,
                dur_ms: durMs,
                hook_score: 0.84,
                retention_score: 0.76,
                score: 0.81,
                reason: 'Clear opening and a focused, self-contained idea.',
                transcript_excerpt: 'A strong opening that becomes one useful short.',
              }],
              count: 1,
              scoring: 'heuristic',
            },
          })
        }
        if (pathname === '/api/verb/render.bundle') {
          const body = requestArgs(options)
          fixture.bundleCalls.push(body)
          return envelope({
            ok: true,
            result: {
              job_id: 'job_fcv_clips_bundle',
              bundle_id: packageResult.bundle_id,
            },
          })
        }
        if (pathname === '/api/verb/jobs.status') {
          const body = requestArgs(options)
          if (body.job_id === 'job_fcv_clips_bundle') {
            fixture.statusCalls.push(body)
            return envelope({
              ok: true,
              result: {
                job_id: body.job_id,
                kind: 'bundle',
                state: 'done',
                progress: 1,
                created_ts: '2026-07-29T00:00:00Z',
                updated_ts: '2026-07-29T00:00:01Z',
                result: packageResult,
              },
            })
          }
        }
        return originalFetch(...args)
      }
    }, { fixtureAsset: assetId, atMs: candidateAtMs, durMs: candidateDurMs })
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvClipsFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvClipsOriginalFetch) window.fetch = target.__fcvClipsOriginalFetch
      delete target.__fcvClipsOriginalFetch
      delete target.__fcvClipsFixture
    })
  }

  async function toggleTwice(control) {
    const states = []
    await control.click()
    states.push(await control.getAttribute('data-cut-on'))
    await control.click()
    states.push(await control.getAttribute('data-cut-on'))
    return states
  }

  async function run(page) {
    const created = await freshProject(page, 'clips_actions', primaryMedia)
    await closeOverlays(page)
    if (!created.assetId) throw new Error('Clips coverage needs an imported video asset')
    await installFixture(page, created.assetId)
    try {
      const open = page.locator('[data-cut-clips-btn]').first()
      const topbar = page.locator('[data-cut-panel="topbar"]').first()
      await probe(page, {
        surface,
        name: 'clips-open-from-topbar',
        actionId: 'clips-btn',
        sel: open,
        group: topbar,
        groupName: 'clips-topbar-entry',
        doClick: async () => {
          await open.click()
          await page.locator(`[data-cut-clip-card="${candidateAtMs}"]`).first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: await page.locator('[data-cut-clips-open="true"]').first().isVisible()
              && fixture.candidateCalls.length === 1
              && fixture.candidateCalls[0].count === 6,
            detail: `drawer=${await page.locator('[data-cut-clips-open="true"]').first().isVisible()}; candidate calls=${fixture.candidateCalls.length}; count=${fixture.candidateCalls[0]?.count}`,
          }
        },
      })

      const drawer = page.locator('[data-cut-clips-open="true"]').first()
      const close = page.locator('[data-cut-clips-close]').first()
      await probe(page, {
        surface,
        name: 'clips-close-and-return',
        actionId: 'clips-close',
        sel: close,
        group: drawer,
        groupName: 'clips-candidate',
        doClick: async () => {
          await close.click()
          await page.locator('[data-cut-clips-open="true"]').waitFor({
            state: 'detached',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-clips-open="true"]').count() === 0,
          detail: `open drawers=${await page.locator('[data-cut-clips-open="true"]').count()}`,
        }),
      })

      await open.click()
      await page.locator(`[data-cut-clip-card="${candidateAtMs}"]`).first().waitFor({
        state: 'visible',
        timeout: 8000,
      })
      const reopenedDrawer = page.locator('[data-cut-clips-open="true"]').first()
      const platformCases = [
        ['9:16', 'vertical'],
        ['1:1', 'square'],
        ['16:9', 'widescreen'],
      ]
      for (const [aspect, label] of platformCases) {
        const control = page.locator(`[data-cut-clips-platform="${aspect}"]`).first()
        let observed = []
        await probe(page, {
          surface,
          name: `clips-platform-${label}-off-on`,
          actionId: 'clips-platform',
          sel: control,
          group: reopenedDrawer,
          groupName: 'clips-platforms',
          doClick: async () => { observed = await toggleTwice(control) },
          assertResult: async () => ({
            ok: observed[0] === 'false'
              && observed[1] === 'true'
              && await control.getAttribute('aria-pressed') === 'true',
            detail: `${aspect} states=${observed.join('→')}; aria-pressed=${await control.getAttribute('aria-pressed')}`,
          }),
        })
      }

      // Leave one platform selected, then prove the final selection cannot be
      // removed. This is a separate action because an always-nonempty bundle is
      // part of the user-facing control contract, not merely component state.
      await page.locator('[data-cut-clips-platform="1:1"]').first().click()
      await page.locator('[data-cut-clips-platform="16:9"]').first().click()
      const lastPlatform = page.locator('[data-cut-clips-platform="9:16"]').first()
      await probe(page, {
        surface,
        name: 'clips-platform-keeps-one-output',
        actionId: 'clips-platform',
        sel: lastPlatform,
        group: reopenedDrawer,
        groupName: 'clips-platform-last-option',
        doClick: async () => {
          await lastPlatform.click()
          await page.locator('[data-cut-clips-error]').filter({ hasText: 'Choose at least one platform.' }).waitFor({
            state: 'visible',
            timeout: 3000,
          })
        },
        assertResult: async () => ({
          ok: await lastPlatform.getAttribute('data-cut-on') === 'true'
            && await lastPlatform.getAttribute('aria-pressed') === 'true',
          detail: `last data-cut-on=${await lastPlatform.getAttribute('data-cut-on')}; aria-pressed=${await lastPlatform.getAttribute('aria-pressed')}`,
        }),
      })

      const make = page.locator(`[data-cut-clip-make="${candidateAtMs}"]`).first()
      await probe(page, {
        surface,
        name: 'clips-make-selected-package',
        actionId: 'clip-make',
        sel: make,
        group: reopenedDrawer,
        groupName: 'clips-ready-to-render',
        doClick: async () => {
          await make.click()
          await page.locator('[data-cut-package-status="ready"]').first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const call = fixture.bundleCalls[0]
          const manifest = page.locator('[data-cut-bundle-manifest]').first()
          const manifestHref = (await manifest.getAttribute('href')) || ''
          const packageText = await page.locator('[data-cut-clip-bundle]').first().textContent()
          // The manifest link must name the EXACT file the bundle wrote.
          //
          // This used to assert the `/api/export/<rel>` prefix. `5d0c2dff`
          // deliberately stopped forcing every export into that project-relative
          // shape: an absolute path with no `exports/` segment — this fixture's
          // /tmp bundle, and every Save-As target — became either a 404 or, when
          // the folder's own path contained an `exports/` segment, a BARE NAME
          // that silently resolved to a stale same-named file inside the project.
          // So the old prefix assertion was asserting the bug that fix removed;
          // the exact `/api/export-file?path=` shape is the contract now. Both
          // shapes stay covered by the exportUrl unit tests (lib.test.ts).
          const manifestNames = decodeURIComponent(manifestHref.split('path=')[1] || '')
          return {
            ok: fixture.bundleCalls.length === 1
              && call?.candidate?.at_ms === candidateAtMs
              && call?.candidate?.dur_ms === candidateDurMs
              && JSON.stringify(call?.platforms) === JSON.stringify(['9:16'])
              && call?.rationale === 'user: social bundle from Clips'
              && fixture.statusCalls.length >= 1
              && packageText?.includes('Package ready')
              && packageText?.includes('9:16')
              && manifestHref.includes('/api/export-file?path=')
              && manifestNames === '/tmp/fcv_bundle_1000_4500/manifest.json',
            detail: `bundle calls=${fixture.bundleCalls.length}; args=${JSON.stringify(call)}; status calls=${fixture.statusCalls.length}; package="${packageText?.replace(/\s+/g, ' ').trim()}"; manifest=${manifestHref}`,
          }
        },
      })

      const completedClose = page.locator('[data-cut-clips-close]').first()
      await probe(page, {
        surface,
        name: 'clips-close-completed-package',
        actionId: 'clips-close',
        sel: completedClose,
        group: page.locator('[data-cut-clips-open="true"]').first(),
        groupName: 'clips-package-ready',
        doClick: async () => {
          await completedClose.click()
          await page.locator('[data-cut-clips-open="true"]').waitFor({
            state: 'detached',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-clips-open="true"]').count() === 0,
          detail: `completed package closed; open drawers=${await page.locator('[data-cut-clips-open="true"]').count()}`,
        }),
      })
    } finally {
      await restoreFixture(page)
    }
  }

  return { run }
}
