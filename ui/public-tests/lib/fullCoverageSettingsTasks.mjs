// Task-level Settings routes that temporarily leave/remount the drawer.

export function createSettingsTaskCoverage({ probe, verb, captureVerbResp, sleep }) {
  async function waitFor(locator, state = 'visible', timeout = 20_000) {
    await locator.waitFor({ state, timeout })
  }

  async function open(page, category) {
    if ((await page.locator('[data-cut-environment]').count()) === 0) {
      await page.locator('[data-cut-setup-btn]').click()
      await waitFor(page.locator('[data-cut-environment]'))
    }
    await page.locator(`[data-cut-settings-category="${category}"]`).click()
    await waitFor(page.locator(`[data-cut-settings-body="${category}"]`))
    return page.locator('[data-cut-environment]').first()
  }

  async function returnFromRecord(page) {
    await page.locator('[data-cut-mode="edit"]').click()
    await sleep(100)
  }

  async function run(page, surface = 'settings') {
    let panel = await open(page, 'overview')
    await probe(page, {
      surface, name: 'settings-overview-recording', actionId: 'settings-overview-action:recording',
      sel: page.locator('[data-cut-settings-overview-action="recording"]'),
      group: panel, groupName: 'settings-overview',
      doClick: async () => {
        await page.locator('[data-cut-settings-overview-action="recording"]').click()
        await waitFor(page.locator('[data-cut-environment]'), 'detached')
      },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-mode="record"]').getAttribute('aria-selected')) === 'true',
        detail: 'overview Recording task opened the Record workspace',
      }),
    })
    await returnFromRecord(page)

    panel = await open(page, 'recording')
    await probe(page, {
      surface, name: 'settings-open-record-workspace', actionId: 'settings-open-recording',
      sel: page.locator('[data-cut-settings-open-recording]'),
      group: panel, groupName: 'settings-recording',
      doClick: async () => {
        await page.locator('[data-cut-settings-open-recording]').click()
        await waitFor(page.locator('[data-cut-environment]'), 'detached')
      },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-mode="record"]').getAttribute('aria-selected')) === 'true',
        detail: 'Recording category opened the Record workspace',
      }),
    })
    await returnFromRecord(page)

    panel = await open(page, 'agent-control')
    await waitFor(page.locator('[data-cut-agent-control-test]'))
    let mcpResponse = null
    await probe(page, {
      surface, name: 'settings-agent-mcp-test', actionId: 'agent-control-test',
      sel: page.locator('[data-cut-agent-control-test]'),
      group: panel, groupName: 'settings-agent-control',
      doClick: async () => {
        mcpResponse = await captureVerbResp(
          page,
          'system.mcp_test',
          () => page.locator('[data-cut-agent-control-test]').click(),
          20_000,
        )
        const result = page.locator('[data-cut-agent-control-test-result]')
        for (let index = 0; index < 120; index += 1) {
          const text = await result.textContent().catch(() => '')
          if (/MCP connected|self-test failed|Could not reach/i.test(text)) break
          await sleep(250)
        }
      },
      assertResult: async () => {
        const text = await page.locator('[data-cut-agent-control-test-result]').textContent().catch(() => '')
        const result = mcpResponse?.result
        const exact = mcpResponse?.ok
          && result?.schema === 'shellx-cut/mcp-self-test/1'
          && result?.mode === 'proxy'
          && result?.read_only === true
          && result?.ping === true
          && result?.same_engine === true
          && result?.tools > 0
          && result?.tools === result?.expected_tools
          && result?.tools_list_bytes > 0
          && result?.tools_list_bytes <= result?.tools_list_max_bytes
          && result?.command?.[0] === result?.executable
          && result?.command?.[1] === 'mcp'
          && !!result?.protocol_version
          && !!result?.proxy_addr
        return {
          ok: !!exact && /MCP connected/i.test(text),
          detail: `${text.trim() || 'no MCP self-test result'}; schema=${result?.schema || 'missing'} mode=${result?.mode || 'missing'} ping=${result?.ping} tools=${result?.tools}/${result?.expected_tools} bytes=${result?.tools_list_bytes}/${result?.tools_list_max_bytes} sameEngine=${result?.same_engine}`,
        }
      },
    })

    const created = await verb('project.create', {
      name: `fcv_settings_output_${Math.random().toString(36).slice(2, 7)}`,
    })
    const outputDir = created.result?.path || ''
    await page.locator('[data-cut-environment-close]').click()
    await waitFor(page.locator('[data-cut-environment]'), 'detached')
    await page.evaluate((dir) => localStorage.setItem('cut.outputDir', dir), outputDir)
    panel = await open(page, 'general')
    const clear = page.locator('[data-cut-export-default-clear]')
    await waitFor(clear)
    await sleep(500)
    await probe(page, {
      surface, name: 'settings-export-folder-clear', actionId: 'export-default-clear',
      sel: clear, group: panel, groupName: 'settings-general',
      doClick: async () => {
        await clear.click()
        for (let index = 0; index < 80; index += 1) {
          if ((await page.locator('[data-cut-export-default-note]').textContent().catch(() => ''))
            .includes('Using each project')) break
          await sleep(150)
        }
      },
      assertResult: async () => {
        const stored = await page.evaluate(() => localStorage.getItem('cut.outputDir'))
        const note = await page.locator('[data-cut-export-default-note]').textContent().catch(() => '')
        return {
          ok: stored === null && /Using each project exports folder/.test(note),
          detail: `stored=${stored ?? 'none'} note="${note.trim()}"`,
        }
      },
    })
  }

  return run
}
