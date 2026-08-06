// Update-surface actions (Settings > About + the topbar update button) nested
// inside the installed/native Settings sweep.
//
// The shell's update snapshot crosses the narrow window.__TAURI__ bridge
// (get_update_state / update_check_now / update_install_now + the
// cut:update-state event — app/desktop/src-tauri/src/update_state.rs). A page
// fixture stages the bridge — creating a minimal one in a plain browser run,
// or wrapping ONLY the three update commands over the real invoke in the
// installed shell — so the sweep can actuate the whole surface
// deterministically: check → available → topbar button → install request →
// honest failure line. The install replies are scripted (a decline, then a
// failure) because a REAL install would restart the app mid-sweep; the true
// download+restart path is proven on a rig against a staged feed
// (SHELLX_CUT_UPDATE_FEED_URL — see update-check-disclosure-contract.test.mjs).

export function createSettingsUpdateCoverage({ probe, sleep, openSettings, waitFor }) {
  async function installUpdateFixture(page) {
    await page.evaluate(() => {
      const target = window
      const fx = {
        invokes: [],
        installReplies: [],
        snapshot: null,
        originalCore: target.__TAURI__?.core || null,
        originalInvoke: target.__TAURI__?.core?.invoke || null,
        // Tauri 2.11 freezes the window.__TAURI__ namespace, so the real
        // dispatcher to wrap is window.__TAURI_INTERNALS__.invoke (which
        // __TAURI__.core.invoke delegates to). Kept separately from
        // originalInvoke so restore puts back exactly the layer we patched.
        originalInternalsInvoke: target.__TAURI_INTERNALS__?.invoke || null,
        patchedReal: false,
        // TRUE only when this fixture CREATED window.__TAURI__ (plain browser).
        // Restore may delete the bridge only in that case — see the comment on
        // restoreUpdateFixture for the collateral damage the flag prevents.
        createdStub: false,
      }
      const mk = (status, extra = {}) => ({
        schema: 'shellx-cut/update-state/1',
        status,
        version: null,
        current: '0.0.0-fixture',
        checked_at: null,
        error: null,
        checking: false,
        installing: false,
        supported: true,
        ...extra,
      })
      fx.snapshot = mk('none', { checked_at: Date.now() - 5 * 60_000 })
      const UPDATE_COMMANDS = ['get_update_state', 'update_check_now', 'update_install_now']
      const handle = async (cmd) => {
        fx.invokes.push(cmd)
        if (cmd === 'get_update_state') return fx.snapshot
        if (cmd === 'update_check_now') {
          fx.snapshot = mk('available', { version: '9.9.9-fixture', checked_at: Date.now() })
          return fx.snapshot
        }
        // update_install_now — scripted reply; an error is also reflected into
        // the snapshot exactly like update_state.rs records install failures.
        const reply = fx.installReplies.shift() || { ok: false, cancelled: true }
        if (reply.error) fx.snapshot = { ...fx.snapshot, error: reply.error }
        return reply
      }
      // Patch order matters. Native shells (Tauri 2.11+): __TAURI__ is FROZEN,
      // so assigning core.invoke silently no-ops and the fixture would test
      // nothing (exactly the defect found in the 2026-08-06 macOS strict run —
      // 3 rows failed while the product behaved correctly). The unfrozen layer
      // is __TAURI_INTERNALS__.invoke — wrap that. Every patch is read back
      // and a patch that did not take THROWS, so a future freeze change fails
      // the row loudly instead of silently.
      const internals = target.__TAURI_INTERNALS__
      if (internals && typeof internals.invoke === 'function') {
        const orig = fx.originalInternalsInvoke
        try {
          internals.invoke = (cmd, args, opts) =>
            UPDATE_COMMANDS.includes(cmd) ? handle(cmd) : orig.call(internals, cmd, args, opts)
        } catch {
          // frozen/sealed namespace — assignment throws in strict mode
        }
        if (internals.invoke === orig) {
          // Tauri 2.11 seals BOTH __TAURI__ and __TAURI_INTERNALS__ in native
          // shells (proven independently on WKWebView and WebView2, 2026-08-06),
          // so no JS-level fake of the update bridge is possible there. Report
          // it as an unavailable fixture rather than throwing: a throw takes the
          // whole settings section down, which is strictly less informative than
          // three rows failing with this exact reason. Simulating an available
          // update on a native shell needs a STAGED FEED at app launch
          // (SHELLX_CUT_UPDATE_FEED_URL), which is the real end-to-end proof.
          fx.unavailable = 'native shell seals the Tauri bridge; a JS fixture cannot stage an available update (use a staged feed)'
        } else {
          fx.patchedReal = 'internals'
        }
      } else if (fx.originalInvoke) {
        // Legacy/unfrozen shells only — verified, never assumed.
        target.__TAURI__.core.invoke = (cmd, args) =>
          UPDATE_COMMANDS.includes(cmd)
            ? handle(cmd)
            : fx.originalInvoke.call(fx.originalCore, cmd, args)
        if (target.__TAURI__.core.invoke === fx.originalInvoke) {
          throw new Error('update fixture: __TAURI__.core.invoke assignment did not take (frozen namespace — expected __TAURI_INTERNALS__ path)')
        }
        fx.patchedReal = 'core'
      } else {
        // Plain browser: no bridge exists; install the minimal stub.
        target.__TAURI__ = {
          core: { invoke: (cmd) => handle(cmd) },
          event: { listen: async () => () => {} },
        }
        fx.createdStub = true
      }
      target.__fcvUpdateFixture = fx
      document.dispatchEvent(new CustomEvent('cut:refresh-update-state'))
    })
  }

  async function restoreUpdateFixture(page) {
    await page.evaluate(() => {
      const fx = window.__fcvUpdateFixture
      if (!fx) return
      // Undo EXACTLY what was installed, and nothing else. The unconditional
      // `delete window.__TAURI__` this replaces was the 2026-08-06
      // `settings-ffmpeg-change` failure on macOS AND Windows: when the native
      // shell seals the bridge the fixture patches nothing, so the else-branch
      // deleted the REAL shell namespace. isTauri() reads window.__TAURI__, so
      // every desktop-only helper silently degraded to a no-op for the rest of
      // the settings section — pickFfmpeg() returned null without ever asking
      // for a panel, and the row failed with "no native dialog appeared" while
      // the product was never reached. Linux was unaffected because its webview
      // does not seal the bridge, so the patch took and this branch never ran.
      if (fx.patchedReal === 'internals') window.__TAURI_INTERNALS__.invoke = fx.originalInternalsInvoke
      else if (fx.patchedReal === 'core') window.__TAURI__.core.invoke = fx.originalInvoke
      else if (fx.createdStub) delete window.__TAURI__
      delete window.__fcvUpdateFixture
      // Components clear (browser) or re-read the real shell state (installed).
      document.dispatchEvent(new CustomEvent('cut:refresh-update-state'))
    })
  }

  return async function runUpdateSurfaceCoverage(page, panel, S) {
    await openSettings(page, 'about')
    await installUpdateFixture(page)
    // A sealed native bridge makes the staged-update scenarios impossible; say
    // so once, loudly, so the run log names the reason instead of leaving three
    // bare "absent" rows for a reader to re-diagnose.
    const fixtureUnavailable = await page.evaluate(() => window.__fcvUpdateFixture?.unavailable || '')
    if (fixtureUnavailable) {
      console.log(`[full-coverage] update-surface fixture unavailable: ${fixtureUnavailable}`)
    }
    try {
      await waitFor(page.locator('[data-cut-about-update-panel]'), 'attached')

      await probe(page, {
        surface: S,
        name: 'settings-about-check-updates',
        actionId: 'about-check-updates',
        sel: page.locator('[data-cut-about-check-updates]'),
        group: panel,
        groupName: 'settings-about',
        doClick: async () => {
          await page.locator('[data-cut-about-check-updates]').click()
          await waitFor(page.locator('[data-cut-about-update-status="available"]'), 'attached')
          // The shell broadcasts cut:update-state to the topbar; the fixture
          // cannot reach the real Tauri event bus, so it uses the components'
          // documented re-sync hook instead (the cut:refresh-doctor idiom).
          await page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:refresh-update-state')))
          await waitFor(page.locator('[data-cut-update-btn]'), 'attached')
        },
        assertResult: async () => {
          const status = await page.locator('[data-cut-about-update-status]').textContent().catch(() => '')
          const checked = await page.locator('[data-cut-about-update-checked]').count()
          const notes = await page.locator('[data-cut-about-release-notes]').getAttribute('href').catch(() => '')
          const topbar = await page.locator('[data-cut-update-btn]').textContent().catch(() => '')
          const invoked = await page.evaluate(() => window.__fcvUpdateFixture.invokes.filter((c) => c === 'update_check_now').length)
          return {
            ok: invoked === 1
              && /ShellX Cut 9\.9\.9-fixture is available/.test(status || '')
              && checked === 1
              && notes === 'https://github.com/martinsbrezauckis/shellx-cut/releases/tag/v9.9.9-fixture'
              && /Update to v9\.9\.9-fixture/.test(topbar || ''),
            detail: `manual check invoked=${invoked}; status="${(status || '').trim().slice(0, 80)}"; checked-ago rows=${checked}; notes=${notes}; topbar="${(topbar || '').trim()}"`,
          }
        },
      })

      // Topbar button: the click must cross the bridge as an install request.
      // The scripted reply is a DECLINE (the confirm's "Later"), proving the
      // request→reply loop without restarting the app under the sweep.
      // The button lives OUTSIDE the Settings overlay (which intercepts
      // pointer events while open) — close Settings first, exactly like the
      // real user flow, then reopen About for the install action below.
      await page.locator('[data-cut-environment-close]').first().click()
      await waitFor(page.locator('[data-cut-environment]'), 'detached')
      await probe(page, {
        surface: S,
        name: 'topbar-update-button',
        actionId: 'update-btn',
        sel: page.locator('[data-cut-update-btn]'),
        group: page.locator('[data-cut-panel="topbar"]').first(),
        groupName: 'topbar-update',
        doClick: async () => {
          await page.locator('[data-cut-update-btn]').click()
          await sleep(150)
        },
        assertResult: async () => {
          const installs = await page.evaluate(() => window.__fcvUpdateFixture.invokes.filter((c) => c === 'update_install_now').length)
          const stillOffered = await page.locator('[data-cut-update-btn]').count()
          const enabled = stillOffered === 1 && !(await page.locator('[data-cut-update-btn]').isDisabled())
          return {
            ok: installs === 1 && stillOffered === 1 && enabled,
            detail: `install requests=${installs}; declined offer keeps the quiet button present=${stillOffered === 1} enabled=${enabled}`,
          }
        },
      })

      // About install action: scripted FAILURE reply — the honest error text
      // must reach the status line (honest-degradation contract).
      await openSettings(page, 'about')
      await waitFor(page.locator('[data-cut-about-update-panel]'), 'attached')
      await page.evaluate(() => {
        window.__fcvUpdateFixture.installReplies.push({ ok: false, error: 'fixture: install rejected' })
      })
      await probe(page, {
        surface: S,
        name: 'settings-about-install-update',
        actionId: 'about-install-update',
        sel: page.locator('[data-cut-about-install-update]'),
        group: panel,
        groupName: 'settings-about',
        doClick: async () => {
          await page.locator('[data-cut-about-install-update]').click()
          await sleep(200)
        },
        assertResult: async () => {
          const installs = await page.evaluate(() => window.__fcvUpdateFixture.invokes.filter((c) => c === 'update_install_now').length)
          const status = await page.locator('[data-cut-about-update-status]').textContent().catch(() => '')
          const honest = /Last attempt failed: fixture: install rejected/.test(status || '')
          return {
            ok: installs === 2 && honest,
            detail: `install requests=${installs}; failure surfaced honestly=${honest}; status="${(status || '').trim().slice(0, 100)}"`,
          }
        },
      })
    } finally {
      await restoreUpdateFixture(page)
      await sleep(120)
    }
  }
}
