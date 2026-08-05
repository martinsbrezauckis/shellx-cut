// Focused browser proof for the confirmed ui.* command contract.
//
// RUN:
//   npm run dev -- --host 127.0.0.1 --port 5208
//   SWEEP_APP=http://127.0.0.1:5208 npm run verify-ui-control

import { chromium } from 'playwright'
import { spawn } from 'node:child_process'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import readline from 'node:readline'

const APP = process.env.SWEEP_APP || 'http://127.0.0.1:5208'
const CUTD = process.env.SWEEP_CUTD || APP
const results = []
const check = (name, ok, detail = '') => {
  results.push({ name, ok: Boolean(ok), detail })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

class McpClient {
  constructor(child) {
    this.child = child
    this.nextId = 1
    this.pending = new Map()
    this.stderr = ''
    child.stderr.on('data', (chunk) => { this.stderr += chunk })
    this.lines = readline.createInterface({ input: child.stdout })
    this.lines.on('line', (line) => {
      let message
      try {
        message = JSON.parse(line)
      } catch {
        return
      }
      const pending = this.pending.get(message.id)
      if (!pending) return
      this.pending.delete(message.id)
      pending.resolve(message)
    })
  }

  request(method, params) {
    const id = this.nextId++
    return new Promise((resolveRequest, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`MCP ${method} timed out: ${this.stderr}`))
      }, 15_000)
      this.pending.set(id, {
        resolve: (message) => {
          clearTimeout(timer)
          resolveRequest(message)
        },
      })
      this.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`)
    })
  }

  async close() {
    this.lines.close()
    this.child.stdin.end()
    if (this.child.exitCode === null) {
      await Promise.race([
        new Promise((resolveExit) => this.child.once('exit', resolveExit)),
        new Promise((resolveWait) => setTimeout(resolveWait, 1_000)),
      ])
    }
    if (this.child.exitCode === null) this.child.kill('SIGTERM')
  }
}

async function main() {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  let mcp
  try {
    const projectName = `ui-control-${Date.now()}`
    const projectParent = mkdtempSync(join(tmpdir(), 'shellx-cut-ui-control-project.'))
    const setupVerb = async (verb, args) => {
      const response = await fetch(`${CUTD}/api/verb/${verb}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(args),
      })
      return response.json()
    }
    const created = await setupVerb('project.create', {
      name: projectName,
      dir: join(projectParent, `${projectName}.cutproj`),
    })
    if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created)}`)
    const title = await setupVerb('title.add', { text: 'Control contract', range_ms: [0, 2_000] })
    if (!title.ok) throw new Error(`title.add failed: ${JSON.stringify(title)}`)
    const clipId = title.result.clip_id

    await page.goto(APP, { waitUntil: 'networkidle' })
    await page.locator('[data-cut-app-root]').waitFor({ state: 'visible', timeout: 10_000 })

    const command = async (verb, args) => {
      const response = await fetch(`${CUTD}/api/verb/${verb}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(args),
      })
      const envelope = await response.json()
      return { envelope, result: envelope.result }
    }

    const { result: settings } = await command('ui.open', { panel: 'settings-agent-control' })
    check('settings-agent-control-applied', settings.applied === true, JSON.stringify(settings.error))
    check('settings-agent-control-committed', await page.locator('[data-cut-settings-body="agent-control"]').count() === 1)
    check('settings-result-is-revisioned', settings.state?.state_revision > 0)
    check('settings-result-names-surface-and-selector', settings.surface === 'settings-agent-control'
      && settings.selector === '[data-cut-settings-body="agent-control"]')

    const agentInfoResponse = await fetch(`${CUTD}/api/agent`)
    const agentInfo = await agentInfoResponse.json()
    check('agent-discovery-names-exact-installed-mcp-command',
      agentInfo.schema === 'shellx-cut/agent-docs/2'
      && agentInfo.runtime?.mcp_proxy?.command === agentInfo.runtime?.executable
      && JSON.stringify(agentInfo.runtime?.mcp_proxy?.args) === JSON.stringify(['mcp']))
    await page.locator('[data-cut-agent-control-test]').click()
    await page.locator('[data-cut-agent-control-test-result]').filter({ hasText: 'same engine confirmed' })
      .waitFor({ state: 'visible', timeout: 20_000 })
    check('settings-mcp-self-test-confirms-same-engine',
      (await page.locator('[data-cut-agent-control-test-result]').textContent())?.includes('same engine confirmed'))

    const { envelope: settingsNoopEnvelope, result: settingsNoop } = await command('ui.open', { panel: 'settings-agent-control' })
    check('already-open-surface-is-not-success', settingsNoop.applied === false
      && settingsNoop.error?.code === 'conflict' && settingsNoopEnvelope.ok === false)

    const { result: color } = await command('ui.open', { panel: 'color' })
    check('right-color-tab-applied', color.applied === true)
    check('right-color-tab-observable', color.state?.right?.active_tab === 'color'
      && color.state?.right?.collapsed === false)

    const { result: playhead } = await command('ui.playhead', { at_ms: 1_234 })
    check('playhead-ack-after-state-commit', playhead.applied === true
      && playhead.state?.playhead_ms === 1_234)
    const { result: playheadNoop } = await command('ui.playhead', { at_ms: 1_234 })
    check('playhead-noop-is-explicit-rejection', playheadNoop.applied === false
      && playheadNoop.error?.code === 'conflict')

    const restTransport = await command('ui.playhead', { at_ms: 1_432 })
    const mcpChild = spawn(agentInfo.runtime.executable, ['mcp'], {
      env: {
        ...process.env,
        CUTD_PROXY_ADDR: new URL(CUTD).host,
        CUTD_PROXY_ACTOR: 'agent:test:ui-control-transport-parity',
      },
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    mcp = new McpClient(mcpChild)
    const initialized = await mcp.request('initialize', {
      protocolVersion: '2025-06-18',
      capabilities: {},
      clientInfo: { name: 'ui-control-transport-parity', version: '1' },
    })
    check('mcp-protocol-version-negotiates', initialized.result?.protocolVersion === '2025-06-18')
    const mcpTransport = await mcp.request('tools/call', {
      name: 'ui_playhead',
      arguments: { at_ms: 1_532 },
    })
    const mcpEnvelope = mcpTransport.result?.structuredContent
    check('rest-and-mcp-return-the-same-applied-envelope',
      restTransport.envelope.ok === true
      && mcpEnvelope?.ok === true
      && restTransport.result?.applied === true
      && mcpEnvelope?.result?.applied === true
      && restTransport.result?.verb === mcpEnvelope?.result?.verb
      && JSON.stringify(Object.keys(restTransport.result).sort()) === JSON.stringify(Object.keys(mcpEnvelope.result).sort())
      && restTransport.result?.state?.schema === mcpEnvelope?.result?.state?.schema
      && mcpEnvelope?.result?.state?.playhead_ms === 1_532)

    const { result: selection } = await command('ui.select', { clip_ids: [clipId] })
    check('selection-ack-after-state-commit', selection.applied === true
      && JSON.stringify(selection.state?.selected_clip_ids) === JSON.stringify([clipId]))
    const { result: missingSelection } = await command('ui.select', { clip_ids: ['not-a-clip'] })
    check('missing-selection-is-not-applied', missingSelection.applied === false
      && missingSelection.error?.code === 'not_found')

    const { result: highlight } = await command('ui.highlight', { panel: 'preview', duration_ms: 0 })
    check('highlight-visible-before-ack', highlight.applied === true
      && await page.locator('[data-cut-highlight]').count() === 1)
    const { result: clear } = await command('ui.highlight', { clear: true })
    check('highlight-clear-confirmed', clear.applied === true
      && await page.locator('[data-cut-highlight]').count() === 0)
    const { result: missingHighlight } = await command('ui.highlight', { selector: '[data-cut-does-not-exist]' })
    check('missing-highlight-target-is-not-applied', missingHighlight.applied === false
      && missingHighlight.error?.code === 'not_found')

    const { envelope: unknown } = await command('ui.open', { panel: 'not-registered' })
    check('unknown-surface-is-rejected-before-relay', unknown.ok === false
      && unknown.error?.code === 'invalid_args')

    check('state-exposes-shared-surface-inventory',
      Array.isArray(color.state?.available_surface_ids)
      && color.state.available_surface_ids.includes('command-palette')
      && color.state.agent_openable_surface_ids.includes('settings-agent-control'))
    check('state-project-identity-is-path-safe',
      color.state?.project?.name === projectName
      && !JSON.stringify(color.state.project).includes('path'))

    const verbsResponse = await fetch(`${CUTD}/api/verbs`)
    const verbs = await verbsResponse.json()
    const openIds = verbs.verbs.find((verb) => verb.name === 'ui.open')
      ?.args?.properties?.panel?.enum ?? []
    const opened = []
    for (const panel of openIds) {
      // review and review-ops intentionally address the same OPS tab. Move to
      // another review tab so both stable ids prove their own applied route.
      if (panel === 'review-ops') await command('ui.open', { panel: 'receipts' })
      const response = await command('ui.open', { panel })
      if (response.envelope.ok === true && response.result?.applied === true) opened.push(panel)
      else console.log(`DETAIL ui.open ${panel}: ${JSON.stringify(response.envelope)}`)
    }
    check('every-schema-ui-open-surface-applies',
      opened.length === openIds.length,
      `${opened.length}/${openIds.length}`)
  } finally {
    if (mcp) await mcp.close()
    await browser.close()
  }

  const failed = results.filter((result) => !result.ok)
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`)
  if (failed.length) process.exitCode = 1
}

await main()
