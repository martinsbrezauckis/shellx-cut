// environment-services-verify.mjs - focused unsigned runtime verifier for
// Settings > Environment optional service cards. It proves that Dub/Diarize are
// model-runtime cards with real controls, not only explanatory prose.
//
// RUN:
//   SWEEP_CUTD=http://127.0.0.1:6161 SWEEP_APP=http://127.0.0.1:6161 \
//     node public-tests/environment-services-verify.mjs

import { chromium } from 'playwright'
import { mkdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6161'
const APP = process.env.SWEEP_APP || 'http://127.0.0.1:6161'
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 30000)
const EVIDENCE_DIR = process.env.CUT_SETTINGS_EVIDENCE_DIR || ''
const SERVICE_SELECTORS = {
  dubCard: '[data-cut-env-card="dub"]',
  diarizeCard: '[data-cut-env-card="diarize"]',
  dubConnect: '[data-cut-env-service-connect="dub"]',
  dubChat: '[data-cut-env-service-chat="dub"]',
  diarizeConnect: '[data-cut-env-service-connect="diarize"]',
  diarizeChat: '[data-cut-env-service-chat="diarize"]',
}

async function verb(name, args = {}) {
  const r = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:environment-services-verify' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(VERB_TIMEOUT_MS),
  })
  return r.json()
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitForUiOpen(panel, timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await verb('ui.open', { panel })
    if (last.ok) return last
    await sleep(250)
  }
  throw new Error(`ui.open ${panel} did not succeed: ${JSON.stringify(last)}`)
}

async function visibleText(locator) {
  return (await locator.innerText({ timeout: 3000 }).catch(() => '')).replace(/\s+/g, ' ').trim()
}

async function visibleCount(locator) {
  return locator.evaluateAll((elements) => elements.filter((element) =>
    !element.closest('details:not([open])') && element.getClientRects().length > 0,
  ).length)
}

async function waitForChatPrompt(page, expected, timeoutMs = 6000) {
  const deadline = Date.now() + timeoutMs
  let last = ''
  while (Date.now() < deadline) {
    const input = page.locator('[data-cut-chat-input], [data-cut-agent-chat-input], textarea').first()
    if ((await input.count()) > 0) {
      last = (await input.inputValue().catch(async () => visibleText(input))) || ''
      if (expected.test(last)) return last
    }
    await sleep(150)
  }
  return last
}

async function findClipped(page) {
  return page.evaluate(() => {
    const root = document.querySelector('[data-cut-environment]')
    if (!root) return [{ tag: 'missing-root', text: 'Environment panel not found' }]
    const visible = (el) => {
      const box = el.getBoundingClientRect()
      const cs = getComputedStyle(el)
      return box.width > 1 && box.height > 1 && cs.visibility !== 'hidden' && cs.display !== 'none'
    }
    return [...root.querySelectorAll('button,span,p,div,label,summary,select,code,dd,dt,strong,small,em')]
      .filter(visible)
      .filter((el) => !el.closest('details:not([open])'))
      .filter((el) => el.scrollWidth > el.clientWidth + 3 && getComputedStyle(el).overflowX === 'visible')
      .map((el) => ({
        tag: el.tagName.toLowerCase(),
        cls: String(el.className || ''),
        text: (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 140),
        client: el.clientWidth,
        scroll: el.scrollWidth,
      }))
      .slice(0, 20)
  })
}

async function findFragmentedTitles(page) {
  return page.evaluate(() => {
    const root = document.querySelector('[data-cut-environment]')
    if (!root) return [{ id: 'missing-root', text: 'Environment panel not found', lines: 0 }]
    return [...root.querySelectorAll('[data-cut-env-title]')]
      .map((el) => {
        const box = el.getBoundingClientRect()
        const style = getComputedStyle(el)
        const lineHeight = Number.parseFloat(style.lineHeight) || 16
        return {
          id: el.getAttribute('data-cut-env-title') || '',
          text: (el.textContent || '').replace(/\s+/g, ' ').trim(),
          lines: Math.round((box.height / lineHeight) * 10) / 10,
          width: Math.round(box.width),
        }
      })
      .filter((item) => item.text.length > 12 && item.lines > 2.25)
      .slice(0, 10)
  })
}

function check(name, ok, detail, out) {
  out.push({ name, ok: !!ok, detail })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name} ${detail ? `- ${detail}` : ''}`)
  if (!ok) throw new Error(`${name}: ${detail || 'failed'}`)
}

async function verifyService(page, id, expected, evidence) {
  const card = page.locator(`[data-cut-env-card="${id}"]`).first()
  await card.waitFor({ state: 'visible', timeout: 8000 })
  const cardText = await visibleText(card)
  const status = await card.getAttribute('data-cut-env-status')
  const detail = card.locator(`[data-cut-env-service="${id}"]`).first()
  await detail.waitFor({ state: 'visible', timeout: 5000 })

  const service = {
    id,
    status,
    text: cardText,
    requirement: await visibleText(detail.locator(`[data-cut-env-service-requirement="${id}"]`).first()),
    connector: await visibleText(detail.locator(`[data-cut-env-service-connector="${id}"]`).first()),
    runtime: await visibleText(detail.locator(`[data-cut-env-service-runtime="${id}"]`).first()),
    outcome: await visibleText(detail.locator(`[data-cut-env-service-outcome="${id}"]`).first()),
    setupCollapsed: (await detail.locator(`[data-cut-env-service-setup="${id}"][open]`).count()) === 0,
    setupSteps: await detail.locator(`[data-cut-env-service-setup-step="${id}"]`).count(),
    primary: await visibleCount(card.locator(`[data-cut-env-service-primary="${id}"]`)),
    connect: await visibleCount(card.locator(`[data-cut-env-service-connect="${id}"]`)),
    chat: await visibleCount(card.locator(`[data-cut-env-service-chat="${id}"]`)),
    rescan: await visibleCount(card.locator(`[data-cut-env-service-rescan="${id}"]`)),
  }
  evidence.services[id] = service

  check(
    `environment-${id}-outcome-first-card`,
    service.outcome.includes(expected.outcome) && !cardText.includes(expected.verb),
    service.outcome,
    evidence.checks,
  )
  check(
    `environment-${id}-setup-collapsed`,
    service.setupCollapsed && service.setupSteps >= 3,
    `collapsed=${service.setupCollapsed} steps=${service.setupSteps}`,
    evidence.checks,
  )
  check(
    `environment-${id}-primary-action`,
    status === 'ok'
      ? service.primary >= 1 && service.chat >= 1 && service.connect === 0
      : service.primary === 1 && service.connect === 1 && service.chat === 0 && service.rescan === 0,
    `status=${status} primary=${service.primary} connect=${service.connect} chat=${service.chat} rescan=${service.rescan}`,
    evidence.checks,
  )

  if (service.connect >= 1) {
    await card.locator(`[data-cut-env-service-connect="${id}"]`).click()
    await page.waitForSelector(`[data-cut-env-card="${id}"] [data-cut-env-service-setup="${id}"][open]`, { timeout: 3000 })
    const setupOpen = await card.locator(`[data-cut-env-service-setup="${id}"][open]`).count()
    check(`environment-${id}-connect-opens-setup`, setupOpen === 1, `open=${setupOpen}`, evidence.checks)
  } else {
    await card.locator(`[data-cut-env-service-setup-toggle="${id}"]`).click()
  }

  const runtimeName = await visibleText(card.locator(`[data-cut-env-service-setup="${id}"] .env-advanced-row`).first().locator('dd'))
  const capability = await visibleText(card.locator(`[data-cut-env-service-powered-by="${id}"]`))
  service.model = runtimeName
  service.capability = capability
  service.requirement = await visibleText(detail.locator(`[data-cut-env-service-requirement="${id}"]`).first())
  service.connector = await visibleText(detail.locator(`[data-cut-env-service-connector="${id}"]`).first())
  service.runtime = await visibleText(detail.locator(`[data-cut-env-service-runtime="${id}"]`).first())
  check(
    `environment-${id}-technical-details`,
    runtimeName.includes(expected.model) && capability.includes(expected.verb),
    `${runtimeName} | ${capability}`,
    evidence.checks,
  )
  check(
    `environment-${id}-connector-runtime-states`,
    /Connector/i.test(service.connector) && /External service/i.test(service.runtime),
    `${service.connector} | ${service.runtime}`,
    evidence.checks,
  )
  check(
    `environment-${id}-requirement-copy`,
    /External runtime required/i.test(service.requirement) && /Connector included/i.test(service.requirement),
    service.requirement,
    evidence.checks,
  )

  if (service.connect >= 1) {
    const expandedChat = await visibleCount(card.locator(`[data-cut-env-service-chat="${id}"]`))
    const expandedRescan = await visibleCount(card.locator(`[data-cut-env-service-rescan="${id}"]`))
    check(
      `environment-${id}-secondary-actions-stay-in-setup`,
      expandedChat === 1 && expandedRescan === 1,
      `chat=${expandedChat} rescan=${expandedRescan}`,
      evidence.checks,
    )
  }

  await card.locator(`[data-cut-env-service-chat="${id}"]`).click()
  const prompt = await waitForChatPrompt(page, expected.prompt)
  evidence.services[id].chatPrompt = prompt
  check(
    `environment-${id}-chat-prefills-agent`,
    expected.prompt.test(prompt),
    prompt.slice(0, 180),
    evidence.checks,
  )
}

async function main() {
  const evidence = { cutd: CUTD, app: APP, checks: [], doctor: {}, services: {} }
  const doctor = await verb('system.doctor', { refresh: true })
  if (!doctor.ok) throw new Error(`system.doctor failed: ${JSON.stringify(doctor.error)}`)
  const cards = doctor.result?.cards || []
  for (const id of ['dub', 'diarize']) {
    const card = cards.find((c) => c.id === id)
    evidence.doctor[id] = card
    check(
      `doctor-${id}-service-card`,
      card?.kind === 'service' && card?.details?.model && card?.details?.powers,
      JSON.stringify({ status: card?.status, model: card?.details?.model, powers: card?.details?.powers }),
      evidence.checks,
    )
  }

  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 920 } })
  const consoleErrors = []
  const badResponses = []
  page.on('console', (msg) => {
    if (msg.type() === 'error') consoleErrors.push(msg.text())
  })
  page.on('response', (resp) => {
    if (resp.status() >= 400) badResponses.push({ status: resp.status(), url: resp.url() })
  })

  try {
    await page.goto(APP, { waitUntil: 'networkidle' })
    evidence.debugOpen = await waitForUiOpen('environment')
    await page.waitForSelector('[data-cut-environment]', { timeout: 8000 })
    await page.locator('[data-cut-settings-category="services-integrations"]').click()
    const uiState = await verb('ui.state')
    evidence.uiState = uiState.result?.open_surface_ids ?? []
    check(
      'environment-debug-open-state',
      uiState.ok &&
      uiState.result?.schema === 'shellx-cut/ui-state/2' &&
      uiState.result?.open_surface_ids?.includes('environment'),
      JSON.stringify(evidence.uiState),
      evidence.checks,
    )

    await verifyService(page, 'dub', {
      model: 'OmniVoice TTS',
      verb: 'audio.dub',
      outcome: 'translated voice track',
      prompt: /Help me connect OmniVoice TTS|Dub the timeline audio into Latvian/i,
    }, evidence)

    await waitForUiOpen('environment')
    await page.waitForSelector('[data-cut-environment]', { timeout: 5000 })
    await page.locator('[data-cut-settings-category="services-integrations"]').click()
    await verifyService(page, 'diarize', {
      model: 'Sortformer v2',
      verb: 'media.diarize',
      outcome: 'speaker labels',
      prompt: /(Help me connect Sortformer v2|Label the speakers).*?(diarize|speaker)/i,
    }, evidence)

    await waitForUiOpen('environment')
    await page.waitForSelector('[data-cut-environment]', { timeout: 5000 })
    await page.locator('[data-cut-settings-category="services-integrations"]').click()
    const envText = await visibleText(page.locator('[data-cut-environment]').first())
    const clipped = await findClipped(page)
    const fragmentedTitles = await findFragmentedTitles(page)
    evidence.noStaleCopyText = envText.slice(0, 1200)
    evidence.clipped = clipped
    evidence.fragmentedTitles = fragmentedTitles
    check(
      'environment-service-copy-no-gpu-host',
      !/GPU host|remote GPU box|tunnel/i.test(envText),
      envText.slice(0, 240),
      evidence.checks,
    )
    check(
      'environment-service-no-major-overflow',
      clipped.length === 0,
      JSON.stringify(clipped.slice(0, 3)),
      evidence.checks,
    )
    check(
      'environment-titles-not-fragmented',
      fragmentedTitles.length === 0,
      JSON.stringify(fragmentedTitles.slice(0, 3)),
      evidence.checks,
    )
    const unexpectedResponses = badResponses.filter((resp) => {
      const knownEmptyFrameProbe = resp.status === 422 && /\/api\/frame\?/.test(resp.url)
      return !knownEmptyFrameProbe
    })
    const unexpectedConsole = consoleErrors.filter((msg) => {
      const knownEmptyFrameConsole = /Failed to load resource:.*status of 422/i.test(msg)
      return !knownEmptyFrameConsole
    })
    evidence.badResponses = badResponses
    evidence.consoleErrors = consoleErrors
    check(
      'environment-no-unexpected-http-errors',
      unexpectedResponses.length === 0,
      JSON.stringify(unexpectedResponses.slice(0, 3)),
      evidence.checks,
    )
    check('environment-console-clean', unexpectedConsole.length === 0, unexpectedConsole.slice(0, 3).join(' | '), evidence.checks)
    if (EVIDENCE_DIR) {
      mkdirSync(EVIDENCE_DIR, { recursive: true })
      await page.screenshot({ path: resolve(EVIDENCE_DIR, 'settings-services-1440x920.png'), fullPage: false })
    }
  } finally {
    await browser.close()
  }

  if (process.env.CUT_RECEIPT) {
    writeFileSync(process.env.CUT_RECEIPT, JSON.stringify(evidence, null, 2))
  }
  console.log(JSON.stringify({ ok: true, evidence }, null, 2))
}

main().catch((err) => {
  console.error(err?.stack || err)
  process.exit(1)
})
