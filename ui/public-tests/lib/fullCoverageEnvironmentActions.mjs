// Deterministic coverage for conditional Settings environment actions.
//
// Setup/download buttons are machine-state dependent and must not provision
// multi-gigabyte runtimes during every UI sweep. This lane intercepts only the
// environment verbs, advances bounded jobs to done, records exact arguments,
// and restores fetch/window.open/listeners before returning. The release setup
// checks still validate real installed dependencies separately.

export function createEnvironmentActionCoverage({
  probe,
  verb,
  sleep,
  closeOverlays,
}) {
  const surface = 'environment-actions'

  async function waitFor(check, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs
    let last = null
    while (Date.now() < deadline) {
      try {
        last = await check()
        if (last) return last
      } catch {}
      await sleep(90)
    }
    return last
  }

  async function installFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (!target.__fcvEnvironmentOriginalFetch) target.__fcvEnvironmentOriginalFetch = window.fetch
      if (!target.__fcvEnvironmentOriginalOpen) target.__fcvEnvironmentOriginalOpen = window.open
      const originalFetch = target.__fcvEnvironmentOriginalFetch
      target.__fcvEnvironmentFixture = {
        doctorCalls: 0,
        fetchCalls: [],
        perceptionCalls: [],
        matteCalls: [],
        jobCalls: [],
        chatPrompts: [],
        manualUrls: [],
        failTransportVerb: null,
        jobSeq: 0,
        pending: {},
        statuses: {
          ffmpeg: 'missing',
          perception: 'missing',
          matte: 'missing',
          matte_premium: 'missing',
          dub: 'missing',
          diarize: 'missing',
        },
      }
      const fixture = target.__fcvEnvironmentFixture
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      const report = () => {
        const status = fixture.statuses
        const card = (id, kind, hint = null, details = {}) => ({
          id,
          kind,
          status: status[id],
          source: status[id] === 'ok' ? 'fixture' : 'missing',
          hint,
          details,
        })
        return {
          schema: 'shellx-cut/doctor-environment-fixture@1',
          scanned_at: new Date().toISOString(),
          os: 'linux',
          arch: 'fixture',
          app_version: 'fixture',
          addr: '127.0.0.1:fixture',
          essential_ok: status.ffmpeg === 'ok',
          cards: [
            card('ffmpeg', 'tool', 'Install the verified fixture build.', {
              resolved: status.ffmpeg === 'ok' ? '/fixture/ffmpeg' : null,
            }),
            card('gpu-encode', 'tool', null, {
              hardware_available: false,
              resolved: status.ffmpeg === 'ok' ? '/fixture/ffmpeg' : null,
            }),
            card('perception', 'perception', 'Install local captions and transcription.', {
              tier: status.perception === 'ok' ? 'full' : 'missing',
              stt_model: 'nemo-parakeet-tdt-0.6b-v3',
            }),
            card('matte', 'matte', 'Install standard background removal.'),
            card('matte_premium', 'matte', 'Install premium background removal.'),
            card('dub', 'service', 'Connect the optional dubbing runtime.', {
              model: 'OmniVoice fixture',
              powers: 'audio.dub',
              runner_available: true,
            }),
            card('diarize', 'service', 'Connect the optional speaker runtime.', {
              model: 'Sortformer fixture',
              powers: 'media.diarize',
              runner_available: true,
            }),
            {
              id: 'project-disk',
              kind: 'disk',
              status: 'ok',
              source: 'fixture',
              details: { free_human: '50 GB' },
            },
          ],
        }
      }
      const startJob = (kind) => {
        fixture.jobSeq += 1
        const jobId = `fixture-env-${fixture.jobSeq}`
        fixture.pending[jobId] = kind
        return envelope({ ok: true, result: { job_id: jobId } })
      }

      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}

        if (pathname === '/api/verb/system.doctor') {
          fixture.doctorCalls += 1
          return envelope({ ok: true, result: report() })
        }
        if (pathname === '/api/verb/system.fetch_tool') {
          const body = requestArgs(options)
          fixture.fetchCalls.push(body)
          if (fixture.failTransportVerb === 'system.fetch_tool') {
            fixture.failTransportVerb = null
            throw new Error('deterministic environment transport failure')
          }
          return startJob('ffmpeg')
        }
        if (pathname === '/api/verb/system.setup_perception') {
          const body = requestArgs(options)
          fixture.perceptionCalls.push(body)
          return startJob('perception')
        }
        if (pathname === '/api/verb/system.setup_matte') {
          const body = requestArgs(options)
          fixture.matteCalls.push(body)
          return startJob(body.model === 'matanyone' ? 'matte_premium' : 'matte')
        }
        if (pathname === '/api/verb/jobs.status') {
          const body = requestArgs(options)
          fixture.jobCalls.push(body)
          const kind = fixture.pending[body.job_id]
          if (kind) {
            fixture.statuses[kind] = 'ok'
            delete fixture.pending[body.job_id]
          }
          return envelope({
            ok: true,
            result: {
              job_id: body.job_id,
              state: 'done',
              progress: 1,
              message: `${kind || 'setup'} ready`,
            },
          })
        }
        return originalFetch(...args)
      }

      target.__fcvEnvironmentChatListener = (event) => {
        fixture.chatPrompts.push(event?.detail?.prompt || '')
      }
      document.addEventListener('cut:open-chat', target.__fcvEnvironmentChatListener)
      window.open = (url) => {
        fixture.manualUrls.push(String(url || ''))
        return null
      }
    })
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvEnvironmentFixture)))
  }

  async function setFixture(page, patch) {
    await page.evaluate((next) => {
      const fixture = window.__fcvEnvironmentFixture
      if (next.statuses) Object.assign(fixture.statuses, next.statuses)
      Object.assign(fixture, { ...next, statuses: fixture.statuses })
    }, patch)
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvEnvironmentChatListener) {
        document.removeEventListener('cut:open-chat', target.__fcvEnvironmentChatListener)
      }
      if (target.__fcvEnvironmentOriginalFetch) window.fetch = target.__fcvEnvironmentOriginalFetch
      if (target.__fcvEnvironmentOriginalOpen) window.open = target.__fcvEnvironmentOriginalOpen
      delete target.__fcvEnvironmentChatListener
      delete target.__fcvEnvironmentOriginalFetch
      delete target.__fcvEnvironmentOriginalOpen
      delete target.__fcvEnvironmentFixture
    })
  }

  async function refreshDoctor(page) {
    await page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:refresh-doctor')))
    await sleep(140)
  }

  async function openCategory(page, category) {
    if ((await page.locator('[data-cut-environment]').count()) === 0) {
      await page.locator('[data-cut-setup-btn]').first().click()
      await page.locator('[data-cut-environment]').first().waitFor({
        state: 'visible',
        timeout: 10_000,
      })
    }
    await page.locator(`[data-cut-settings-category="${category}"]`).first().click()
    await page.locator(`[data-cut-settings-body="${category}"]`).first().waitFor({
      state: 'visible',
      timeout: 8000,
    })
    return page.locator('[data-cut-environment]').first()
  }

  async function setupAction(page, panel, {
    name,
    actionId,
    selector,
    expectedStatus,
    assertCall,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: panel,
      groupName: 'environment-settings',
      doClick: async () => {
        await control.click()
        await waitFor(async () => (
          (await page.locator(`[data-cut-env-card="${expectedStatus}"]`).first().getAttribute('data-cut-env-status')) === 'ok'
        ), 12_000)
      },
      assertResult: async () => {
        const fixture = await fixtureState(page)
        const status = await page.locator(`[data-cut-env-card="${expectedStatus}"]`).first().getAttribute('data-cut-env-status')
        return {
          ok: status === 'ok' && assertCall(fixture),
          detail: `${expectedStatus} status=${status}; calls=${JSON.stringify({
            fetch: fixture.fetchCalls,
            perception: fixture.perceptionCalls,
            matte: fixture.matteCalls,
            jobs: fixture.jobCalls,
          })}`,
        }
      },
    })
  }

  async function run(page) {
    await closeOverlays(page)
    await installFixture(page)
    try {
      await refreshDoctor(page)
      await page.locator('[data-cut-wizard-dismiss]').first().click().catch(() => {})
      let panel = await openCategory(page, 'video-performance')

      await setFixture(page, { failTransportVerb: 'system.fetch_tool' })
      const install = page.locator('[data-cut-env-download="ffmpeg"]').first()
      await probe(page, {
        surface,
        name: 'environment-ffmpeg-transport-failure',
        actionId: 'env-download',
        sel: install,
        group: panel,
        groupName: 'environment-settings',
        doClick: async () => {
          await install.click()
          await page.locator('[data-cut-env-card="ffmpeg"] [data-cut-env-error]').first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const error = await page.locator('[data-cut-env-card="ffmpeg"] [data-cut-env-error]').first().textContent().catch(() => '')
          return {
            ok: /server unreachable/.test(error || '')
              && await page.locator('[data-cut-env-download="ffmpeg"]').first().isVisible(),
            detail: `setup error="${error || ''}"; Install returned for retry`,
          }
        },
      })
      await setupAction(page, panel, {
        name: 'environment-install-ffmpeg',
        actionId: 'env-download',
        selector: '[data-cut-env-download="ffmpeg"]',
        expectedStatus: 'ffmpeg',
        assertCall: (fixture) => fixture.fetchCalls.at(-1)?.tool === 'ffmpeg'
          && fixture.jobCalls.at(-1)?.job_id?.startsWith('fixture-env-'),
      })

      panel = await openCategory(page, 'ai-transcription')
      await setupAction(page, panel, {
        name: 'environment-install-perception',
        actionId: 'env-setup-perception',
        selector: '[data-cut-env-setup-perception="perception"]',
        expectedStatus: 'perception',
        assertCall: (fixture) => fixture.perceptionCalls.at(-1)?.warm_model === true,
      })
      await setupAction(page, panel, {
        name: 'environment-install-matte-standard',
        actionId: 'env-setup-matte',
        selector: '[data-cut-env-setup-matte="matte"]',
        expectedStatus: 'matte',
        assertCall: (fixture) => fixture.matteCalls.at(-1)?.model === 'rvm',
      })
      await setupAction(page, panel, {
        name: 'environment-install-matte-premium',
        actionId: 'env-setup-matte',
        selector: '[data-cut-env-setup-matte="matte_premium"]',
        expectedStatus: 'matte_premium',
        assertCall: (fixture) => fixture.matteCalls.at(-1)?.model === 'matanyone'
          && fixture.matteCalls.at(-1)?.accept_noncommercial === true,
      })

      panel = await openCategory(page, 'services-integrations')
      const connect = page.locator('[data-cut-env-service-connect="dub"]').first()
      await probe(page, {
        surface,
        name: 'environment-service-connect',
        actionId: 'env-service-connect',
        sel: connect,
        group: panel,
        groupName: 'environment-services',
        doClick: async () => {
          await connect.click()
          await sleep(90)
        },
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-env-service-setup="dub"]').first().getAttribute('open')) !== null,
          detail: 'dub connection steps opened',
        }),
      })

      let chatCount = (await fixtureState(page)).chatPrompts.length
      const setupHelp = page.locator('[data-cut-env-service-chat="dub"]').first()
      await probe(page, {
        surface,
        name: 'environment-service-setup-help',
        actionId: 'env-service-chat',
        sel: setupHelp,
        group: panel,
        groupName: 'environment-services',
        doClick: async () => {
          await setupHelp.click()
          await waitFor(async () => (await fixtureState(page)).chatPrompts.length > chatCount)
        },
        assertResult: async () => {
          const prompt = (await fixtureState(page)).chatPrompts.at(-1) || ''
          return {
            ok: /connect OmniVoice TTS/.test(prompt),
            detail: `Agent Chat prompt="${prompt}"`,
          }
        },
      })

      // Opening Agent Chat intentionally closes Settings, so reopen the same
      // category before testing its remaining human controls.
      panel = await openCategory(page, 'services-integrations')
      await page.locator('[data-cut-env-service-connect="dub"]').first().click()
      await waitFor(async () => (
        (await page.locator('[data-cut-env-service-setup="dub"]').first().getAttribute('open')) !== null
      ))
      const doctorCalls = (await fixtureState(page)).doctorCalls
      const rescan = page.locator('[data-cut-env-service-rescan="dub"]').first()
      await probe(page, {
        surface,
        name: 'environment-service-rescan',
        actionId: 'env-service-rescan',
        sel: rescan,
        group: panel,
        groupName: 'environment-services',
        doClick: async () => {
          await rescan.click()
          await waitFor(async () => (await fixtureState(page)).doctorCalls > doctorCalls)
        },
        assertResult: async () => {
          const after = (await fixtureState(page)).doctorCalls
          return { ok: after > doctorCalls, detail: `doctor calls ${doctorCalls}→${after}` }
        },
      })

      await setFixture(page, { statuses: { dub: 'ok' } })
      await refreshDoctor(page)
      await waitFor(async () => (
        (await page.locator('[data-cut-env-card="dub"]').first().getAttribute('data-cut-env-status')) === 'ok'
      ))
      chatCount = (await fixtureState(page)).chatPrompts.length
      const useChat = page.locator('button[data-cut-env-service-primary="dub"]').first()
      await probe(page, {
        surface,
        name: 'environment-service-use-in-chat',
        actionId: 'env-service-primary',
        sel: useChat,
        group: panel,
        groupName: 'environment-services',
        doClick: async () => {
          await useChat.click()
          await waitFor(async () => (await fixtureState(page)).chatPrompts.length > chatCount)
        },
        assertResult: async () => {
          const prompt = (await fixtureState(page)).chatPrompts.at(-1) || ''
          return {
            ok: /Dub the timeline audio into Latvian/.test(prompt),
            detail: `ready-service prompt="${prompt}"`,
          }
        },
      })
      panel = await openCategory(page, 'services-integrations')
      chatCount = (await fixtureState(page)).chatPrompts.length
      const readyChat = page.locator('button[data-cut-env-service-primary="dub"]').first()
      await probe(page, {
        surface,
        name: 'environment-service-ready-chat',
        actionId: 'env-service-chat',
        sel: readyChat,
        group: panel,
        groupName: 'environment-services',
        doClick: async () => {
          await readyChat.click()
          await waitFor(async () => (await fixtureState(page)).chatPrompts.length > chatCount)
        },
        assertResult: async () => ({
          ok: (await fixtureState(page)).chatPrompts.length > chatCount,
          detail: 'ready Use in Chat dispatched a second prompt',
        }),
      })

      await openCategory(page, 'services-integrations')
      await page.locator('[data-cut-environment-close]').first().click()
      await page.locator('[data-cut-environment]').first().waitFor({
        state: 'detached',
        timeout: 8000,
      })
      await verb('ui.open', { panel: 'wizard' })
      const wizard = page.locator('[data-cut-wizard]').first()
      await wizard.waitFor({ state: 'visible', timeout: 8000 })
      const guide = page.locator('[data-cut-setup-manual]').first()
      await probe(page, {
        surface,
        name: 'environment-setup-guide',
        actionId: 'setup-manual',
        sel: guide,
        group: wizard,
        groupName: 'environment-wizard',
        doClick: async () => {
          await guide.click()
          await waitFor(async () => (await fixtureState(page)).manualUrls.length > 0)
        },
        assertResult: async () => {
          const url = (await fixtureState(page)).manualUrls.at(-1) || ''
          return {
            ok: /docs\.theshellx\.com\/manual\/cut/.test(url)
              && /feature=cut\.preview\.ffmpeg_setup/.test(url),
            detail: `manual URL=${url}`,
          }
        },
      })
      await page.locator('[data-cut-wizard-dismiss]').first().click()
      await sleep(80)
    } finally {
      await restoreFixture(page)
      await closeOverlays(page)
    }
  }

  return { run }
}
