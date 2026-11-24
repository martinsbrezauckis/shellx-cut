import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'
import { sealedPlaywrightChromiumLaunchOptions } from './lib/sealedPlaywrightChromium.mjs'

test('Chat tab migrates local history, shows live agent-started conversations, short unreadable reasons, and supports human continuation', { timeout: 120_000 }, async () => {
  let server
  let browser
  let onStrayRejection
  const strayRejection = new Promise((_resolve, reject) => {
    onStrayRejection = (reason) => reject(reason instanceof Error ? reason : new Error(String(reason)))
  })
  process.on('unhandledRejection', onStrayRejection)
  const running = run()
  try {
    await Promise.race([running, strayRejection])
  } finally {
    process.off('unhandledRejection', onStrayRejection)
    await Promise.race([running.catch(() => {}), new Promise((resolve) => setTimeout(resolve, 20_000).unref())])
    await Promise.allSettled([browser?.close(), server?.close()])
  }

  async function run() {
    server = await createServer({
      root: fileURLToPath(new URL('..', import.meta.url)),
      configFile: false,
      plugins: [react(), {
        name: 'chat-conversations-fixture',
        configureServer(vite) {
          vite.middlewares.use('/__chat_conversations__', (_request, response) => {
            response.setHeader('content-type', 'text/html')
            response.end(`<div id="root"></div><script type="module">
              import RefreshRuntime from '/@react-refresh'
              RefreshRuntime.injectIntoGlobalHook(window)
              window.$RefreshReg$ = () => {}
              window.$RefreshSig$ = () => type => type
              window.__vite_plugin_react_preamble_installed__ = true
            </script><script type="module" src="/public-tests/fixtures/agent-chat-conversations.tsx"></script>`)
          })
        },
      }],
      server: { host: '127.0.0.1', port: 0, watch: null },
    })
    await server.listen()
    browser = await chromium.launch({ headless: true, ...sealedPlaywrightChromiumLaunchOptions() })
    const page = await browser.newPage({ viewport: { width: 1280, height: 820 } })
    const errors = []
    page.on('pageerror', (error) => errors.push(String(error)))

    // Engine state simulated in test harness implementing the verb contract.
    const conversations = new Map([
      ['chat_unknown_99', {
        id: 'chat_unknown_99',
        title: 'Future version chat',
        origin: 'unknown',
        readable: false,
        unreadable_reason: 'Saved in a newer format (version 99) and cannot be opened here.',
        running: false,
        turn_count: 0,
        updated_at: '2026-11-24T00:00:00Z',
        turns: [],
      }],
    ])
    let migratedCalls = 0
    let listNote = null
    const continuedRequests = []

    const migrateBodies = []
    await page.addInitScript(() => {
      localStorage.setItem('cut.chatAgent', 'claude')
      // The 0.6.121 browser history (real stored format); seeded once, so a
      // reload sees what the app itself left behind.
      if (!localStorage.getItem('shellx-cut:agent-chat-history:v1')) {
        localStorage.setItem('shellx-cut:agent-chat-history:v1', JSON.stringify({
          schema: 'shellx-cut/agent-chat-history/1',
          projects: [{
            identity: {
              schema: 'shellx-cut/project-identity/1',
              origin_path_sha256: `sha256:${'a'.repeat(64)}`,
              project_name: 'Chat Conversations Fixture',
            },
            session: {
              log: [
                { id: 'legacy-u1', role: 'user', text: 'Legacy trim request' },
                { id: 'legacy-a1', role: 'agent', agent: 'claude', text: 'Trimmed opening clip.', ok: true, actions: [] },
              ],
              input: '',
              attachments: [],
              target: null,
            },
          }],
        }))
      }

      // Controllable WS client so test can push chat_updated frames like cutd does.
      const clients = []
      window.__pushWsEvent = (payload) => {
        const msg = JSON.stringify(payload)
        for (const ws of clients) {
          ws.onmessage?.({ data: msg })
        }
      }
      class TestWebSocket {
        static OPEN = 1
        readyState = 1
        onopen = null
        onmessage = null
        onclose = null
        onerror = null
        constructor() {
          clients.push(this)
          queueMicrotask(() => this.onopen?.({}))
        }
        close() {
          this.readyState = 3
        }
        send() {}
      }
      window.WebSocket = TestWebSocket
    })

    await page.route('**/api/verb/**', async (route) => {
      const name = new URL(route.request().url()).pathname.split('/').at(-1)
      const body = route.request().postDataJSON() ?? {}
      const fulfill = (result) => route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ok: true, result }),
      })

      if (name === 'system.doctor') {
        return fulfill({
          cards: [{ id: 'claude', kind: 'judge', status: 'ok', details: { found: true, installed: true, chat: { ready: true, status: 'ok' } } }],
        })
      }
      if (name === 'agent.chat_migrate') {
        migratedCalls += 1
        migrateBodies.push(body)
        const entries = Array.isArray(body.conversations) ? body.conversations : []
        const created = []
        for (const entry of entries) {
          const id = `chat_migrated_${created.length + 1}`
          if (!conversations.has(id)) {
            conversations.set(id, {
              id,
              title: entry.title || 'Legacy trim request',
              origin: 'human',
              readable: true,
              unreadable_reason: null,
              running: false,
              turn_count: Array.isArray(entry.turns) ? entry.turns.length : 0,
              updated_at: '2026-11-24T00:01:00Z',
              turns: entry.turns ?? [],
            })
            created.push(id)
          }
        }
        return fulfill({ migrated: created.length, conversation_ids: created })
      }
      if (name === 'agent.chat_list') {
        const list = [...conversations.values()].map(({ turns: _t, ...meta }) => meta)
        return fulfill(listNote ? { conversations: list, truncated: true, not_shown: 2, note: listNote } : { conversations: list })
      }
      if (name === 'agent.chat_get') {
        const conv = conversations.get(body.conversation_id)
        if (!conv) {
          return route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify({ ok: false, error: { code: 'not_found', message: 'Conversation not found' } }),
          })
        }
        return fulfill({ conversation: conv })
      }
      if (name === 'agent.chat') {
        continuedRequests.push(body)
        const id = body.conversation_id || 'chat_human_new'
        const conv = conversations.get(id)
        if (conv) {
          conv.turns = [
            ...conv.turns,
            { id: 'u-follow', role: 'user', text: body.message, origin: 'human' },
            { id: 'a-follow', role: 'agent', agent: body.agent || 'claude', text: 'Applied follow-up fade.', status: 'ok', actions: [{ op_id: 'op_0002', verb: 'edit.fade' }] },
          ]
          conv.turn_count = conv.turns.length
        }
        return fulfill({
          ok: true,
          conversation_id: id,
          origin: conv?.origin ?? 'human',
          agent: body.agent || 'claude',
          reply: 'Applied follow-up fade.',
          actions: [{ op_id: 'op_0002', verb: 'edit.fade' }],
          attachments: [],
          evidence_ids: [],
          cost_usd: 0,
        })
      }
      return fulfill({})
    })

    // Pictures of the Chat tab with an agent-started conversation, both themes
    // (written only when CUT_CHAT_SHOT_DIR is set).
    const shoot = async (step) => {
      const shotDir = process.env.CUT_CHAT_SHOT_DIR
      if (!shotDir) return
      for (const theme of ['dark', 'light']) {
        await page.evaluate((t) => { document.documentElement.dataset.theme = t }, theme)
        await page.waitForTimeout(400) // let colour transitions finish
        await page.screenshot({ path: `${shotDir}/chat-agent-${step}-${theme}.png` })
      }
      await page.evaluate(() => { delete document.documentElement.dataset.theme })
      await page.waitForTimeout(400)
    }

    await page.goto(new URL('/__chat_conversations__', server.resolvedUrls.local[0]).href)
    await page.locator('[data-cut-chat]').waitFor({ timeout: 10_000 })

    // 1. Legacy conversation migrated and visible alongside unknown-version short reason.
    const list = page.locator('[data-cut-chat-conversation-list]')
    await list.waitFor({ timeout: 5_000 })
    assert.ok(migratedCalls >= 1, 'legacy localStorage history must migrate via agent.chat_migrate on first read')
    await page.waitForFunction(() => document.querySelector('[data-cut-chat-conversation-list]')?.textContent?.includes('Legacy trim request'))
    assert.ok(
      (await list.textContent())?.includes('Saved in a newer format (version 99)'),
      'unreadable conversation shows short plain reason',
    )

    const sent = JSON.stringify(migrateBodies[0] ?? {})
    assert.ok(sent.includes('Legacy trim request') && sent.includes('Trimmed opening clip.'), `both legacy turns reach the project record: ${sent}`)
    const local = await page.evaluate(() => ({
      history: localStorage.getItem('shellx-cut:agent-chat-history:v1') ?? '',
      kept: localStorage.getItem('shellx-cut:agent-chat-engine-kept:v1') ?? '',
    }))
    assert.ok(local.history.includes('Legacy trim request'), 'the browser copy is never deleted by the migration')
    assert.ok(local.kept.length > 2, 'the project is marked as kept in the project after the copy')

    // An unreadable conversation opens to its short reason; it cannot be continued.
    await page.locator('[data-cut-chat-conversation="chat_unknown_99"]').click()
    await page.locator('[data-cut-chat-unreadable-detail]').waitFor({ timeout: 5_000 })
    assert.ok(((await page.locator('[data-cut-chat-unreadable-detail]').textContent()) ?? '').includes('Saved in a newer format (version 99)'))
    assert.equal(await page.locator('[data-cut-chat-input]').isDisabled(), true, 'an unreadable conversation cannot take a new message')
    // Its one button leaves the unreadable conversation and starts a new one.
    await page.locator('[data-cut-chat-unreadable-detail] [data-cut-chat-new]').click()
    await page.locator('[data-cut-chat-unreadable-detail]').waitFor({ state: 'detached', timeout: 5_000 })
    assert.equal(await page.locator('[data-cut-chat-conversation="chat_unknown_99"]').getAttribute('aria-current'), null, 'the unreadable conversation is no longer the open one')

    // 2. External agent command starts a conversation -> appears live in list while running.
    conversations.set('chat_agent_1', {
      id: 'chat_agent_1',
      title: 'Agent CLI marker request',
      origin: 'agent',
      readable: true,
      unreadable_reason: null,
      running: true,
      turn_count: 2,
      updated_at: '2026-11-24T00:05:00Z',
      turns: [
        { id: 'u-agent-1', role: 'user', text: 'Add a marker at 1 second', origin: 'agent' },
        { id: 'a-agent-1', role: 'agent', agent: 'claude', text: '', status: 'running', actions: [{ op_id: 'op_0001', verb: 'edit.add_marker' }] },
      ],
    })
    await page.evaluate(() => window.__pushWsEvent({
      type: 'chat_updated',
      conversation_id: 'chat_agent_1',
      state: 'running',
      turn_id: 'turn-live-1',
    }))

    const agentRow = page.locator('[data-cut-chat-conversation="chat_agent_1"]')
    await agentRow.waitFor({ timeout: 5_000 })
    const rowText = (await agentRow.textContent()) ?? ''
    assert.ok(rowText.includes('Started by an agent'), `expected "Started by an agent" badge, got: ${rowText}`)
    assert.ok(/running|working/i.test(rowText), `expected live running status on list item, got: ${rowText}`)

    // 3. Open the agent-started conversation while it runs and observe live user message + live action.
    await agentRow.click()
    await page.waitForFunction(() => {
      const log = document.querySelector('[data-cut-chat-log]')?.textContent ?? ''
      return log.includes('Add a marker at 1 second') && log.includes('edit.add_marker')
    })

    await shoot('running')

    // 4. Turn finishes and reply streams into the opened conversation.
    const runningConv = conversations.get('chat_agent_1')
    runningConv.running = false
    runningConv.turns[1] = {
      id: 'a-agent-1',
      role: 'agent',
      agent: 'claude',
      text: 'Added live marker.',
      status: 'ok',
      actions: [{ op_id: 'op_0001', verb: 'edit.add_marker' }],
    }
    await page.evaluate(() => window.__pushWsEvent({
      type: 'chat_updated',
      conversation_id: 'chat_agent_1',
      state: 'completed',
      turn_id: 'turn-live-1',
    }))
    await page.waitForFunction(() => document.querySelector('[data-cut-chat-log]')?.textContent?.includes('Added live marker.'))

    // 5. Person continues the agent-started conversation -> sends conversation_id and keeps origin marker.
    await page.locator('[data-cut-chat-input]').fill('Continue by hand')
    await page.locator('[data-cut-chat-send]').click()
    await page.waitForFunction(() => document.querySelector('[data-cut-chat-log]')?.textContent?.includes('Applied follow-up fade.'))
    assert.equal(continuedRequests.length, 1)
    assert.equal(continuedRequests[0].conversation_id, 'chat_agent_1')
    assert.equal(continuedRequests[0].message, 'Continue by hand')
    assert.ok(
      ((await agentRow.textContent()) ?? '').includes('Started by an agent'),
      'human continuation must preserve Started by an agent badge',
    )

    await shoot('continued')

    // 6. Reopen the app: the list comes from the project again, no second copy.
    await page.reload()
    await page.locator('[data-cut-chat-conversation="chat_agent_1"]').waitFor({ timeout: 10_000 })
    await page.locator('[data-cut-chat-conversation^="chat_migrated_"]').first().waitFor({ timeout: 5_000 })
    assert.equal(migratedCalls, 1, 'the browser history is copied into the project only once')
    assert.equal(await page.locator('[data-cut-chat-conversation^="chat_migrated_"]').count(), 1)
    assert.ok(((await page.locator('[data-cut-chat-conversation="chat_agent_1"]').textContent()) ?? '').includes('Started by an agent'))

    // 7. Truncated conversation list note appears when backend reports more conversations not shown.
    listNote = '2 more conversations not shown'
    await page.evaluate(() => window.__pushWsEvent({
      type: 'chat_updated',
      conversation_id: 'chat_agent_1',
      state: 'completed',
      turn_id: 'turn-live-1',
    }))
    await page.locator('[data-cut-chat-conversation-more-note]').waitFor({ timeout: 5_000 })
    assert.equal(
      (await page.locator('[data-cut-chat-conversation-more-note]').textContent())?.trim(),
      '2 more conversations not shown',
    )
    assert.deepEqual(errors, [])
  }
})
