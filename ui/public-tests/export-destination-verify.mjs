// export-destination-verify.mjs - lightweight unsigned runtime verifier for
// Settings > Default save folder plus Export menu Save As controls.
//
// RUN:
//   cd ui && SWEEP_CUTD=http://127.0.0.1:6161 SWEEP_APP=http://127.0.0.1:6161 \
//     node public-tests/export-destination-verify.mjs
//
// This is intentionally small enough to run between code batches before a
// signed release package. Installed WebView2 coverage remains in
// scripts/windows/cdp-cut-verify-0650-uiux.mjs.

import { chromium } from 'playwright'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6161'
const APP = process.env.SWEEP_APP || CUTD
const EXPECTED_VERSION = process.env.CUT_EXPECTED_VERSION || ''
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 30000)
const RECEIPT = process.env.CUT_RECEIPT || ''

const results = []
const evidence = {
  app: APP,
  cutd: CUTD,
  project: '',
  rowText: '',
  menuText: '',
  overflow: [],
}

function check(name, ok, detail = '') {
  const item = { name, ok: !!ok, detail }
  results.push(item)
  console.log(`${item.ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

async function verb(name, args = {}) {
  const r = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-cut-actor': 'human:ui:export-destination-verify',
    },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(VERB_TIMEOUT_MS),
  })
  return r.json()
}

async function main() {
  const tmp = mkdtempSync(join(tmpdir(), 'shellx-cut-export-dest-'))
  const suffix = Math.random().toString(36).slice(2, 8)
  const name = `export_dest_${suffix}`
  const projectDir = join(tmp, `${name}.cutproj`)
  evidence.project = projectDir

  const created = await verb('project.create', {
    name,
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  check('project.create', created?.ok === true, created?.ok ? projectDir : JSON.stringify(created?.error ?? created).slice(0, 240))
  if (created?.ok !== true) throw new Error('cannot continue without a project')

  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  try {
    await page.goto(APP, { waitUntil: 'networkidle' })
    await page.waitForTimeout(800)

    const doctor = await verb('system.doctor', { refresh: true })
    evidence.appVersion = doctor?.result?.app_version
    if (EXPECTED_VERSION) {
      check('dev-app-version', doctor?.result?.app_version === EXPECTED_VERSION, `app_version=${doctor?.result?.app_version}`)
    } else {
      check('dev-app-version-present', /^\d+\.\d+\.\d+/.test(String(doctor?.result?.app_version || '')), `app_version=${doctor?.result?.app_version}`)
    }

    const outputChip = page.locator('[data-cut-output-chip]')
    await outputChip.waitFor({ state: 'visible', timeout: 8000 })
    const outputChipText = ((await outputChip.textContent()) || '').replace(/\s+/g, ' ').trim()
    check('status-output-chip-visible', await outputChip.count() === 1, outputChipText)

    await outputChip.click()
    await page.waitForSelector('[data-cut-environment] [data-cut-export-default-folder]', { timeout: 8000 })
    check('statusbar-output-chip-opens-environment', await page.locator('[data-cut-environment] [data-cut-export-default-folder]').count() === 1)

    const envRow = page.locator('[data-cut-export-default-folder]')
    const heading = (await envRow.locator('[data-cut-export-default-heading]').textContent())?.trim()
    const pick = envRow.locator('[data-cut-export-default-pick]')
    const clear = envRow.locator('[data-cut-export-default-clear]')
    const clearVisible = await clear.isVisible()
    const clearDisabled = await clear.isDisabled()
    evidence.rowText = ((await envRow.textContent()) || '').replace(/\s+/g, ' ').trim()

    check('environment-export-row-heading', heading === 'Default save folder', heading)
    check('environment-export-pick-visible', await pick.isVisible())
    check('environment-export-clear-visible-disabled', clearVisible && clearDisabled, `visible=${clearVisible} disabled=${clearDisabled}`)
    check(
      'environment-export-row-copy-compact',
      evidence.rowText.length < 300 && /Save As can override one file/i.test(evidence.rowText),
      evidence.rowText,
    )

    await pick.click()
    const browserPickNote = await page.locator('[data-cut-export-default-note]').textContent({ timeout: 3000 }).catch(() => '')
    check('environment-export-pick-browser-feedback', /desktop app/i.test(browserPickNote || ''), (browserPickNote || '').trim())

    await page.locator('[data-cut-environment-close]').click()
    await page.waitForSelector('[data-cut-environment]', { state: 'detached', timeout: 5000 })

    await page.locator('[data-cut-settings-btn]').click()
    await page.locator('[data-cut-settings-category="general"]').click()
    await page.waitForSelector('[data-cut-environment] [data-cut-export-default-folder]', { timeout: 8000 })
    check('settings-button-opens-export-folder', await page.locator('[data-cut-environment] [data-cut-export-default-folder]').count() === 1)
    await page.locator('[data-cut-environment-close]').click()
    await page.waitForSelector('[data-cut-environment]', { state: 'detached', timeout: 5000 })

    const exportButton = page.locator('[data-cut-export-btn]')
    await exportButton.waitFor({ state: 'visible', timeout: 8000 })
    const exportButtonText = ((await exportButton.textContent()) || '').replace(/\s+/g, ' ').trim()
    check('topbar-export-button-visible-enabled', (await exportButton.count()) === 1 && !(await exportButton.isDisabled()), exportButtonText)

    await exportButton.click()
    await page.waitForSelector('[data-cut-export-menu]', { timeout: 8000 })
    evidence.menuText = ((await page.locator('[data-cut-export-menu]').textContent()) || '').replace(/\s+/g, ' ').trim()
    const chooseFolder = await page.locator('[data-cut-export-choose-folder]').count()
    const clearFolder = await page.locator('[data-cut-export-clear-folder]').count()
    const saveAsVideo = await page.locator('[data-cut-export-saveas-option="video"]').count()

    check('export-menu-folder-picker-visible', chooseFolder === 1, `choose=${chooseFolder}`)
    check('export-menu-clear-hidden-without-custom-folder', clearFolder === 0, `clear=${clearFolder}`)
    check('export-menu-save-as-video-control', saveAsVideo === 1, `saveAsVideo=${saveAsVideo}`)
    check('export-menu-copy-names-default-folder', /Default export folder|default export folder/i.test(evidence.menuText), evidence.menuText.slice(0, 180))

    evidence.overflow = await page.evaluate(() => {
      const offenders = []
      for (const el of document.querySelectorAll('[data-cut-export-default-folder], [data-cut-output-chip], [data-cut-export-menu]')) {
        const r = el.getBoundingClientRect()
        if (el.scrollWidth > Math.ceil(r.width) + 2 || el.scrollHeight > Math.ceil(r.height) + 2) {
          offenders.push({
            role: el.hasAttribute('data-cut-export-default-folder')
              ? 'default-folder'
              : el.hasAttribute('data-cut-output-chip')
                ? 'output-chip'
                : 'export-menu',
            scrollWidth: el.scrollWidth,
            width: Math.ceil(r.width),
            scrollHeight: el.scrollHeight,
            height: Math.ceil(r.height),
          })
        }
      }
      return offenders
    })
    check('no-obvious-export-settings-overflow', evidence.overflow.length === 0, JSON.stringify(evidence.overflow))
  } finally {
    await browser.close().catch(() => {})
    await verb('project.close', {}).catch(() => {})
    await verb('project.delete', { path: projectDir }).catch(() => {})
    await verb('project.forget', { path: projectDir }).catch(() => {})
    rmSync(tmp, { recursive: true, force: true })
  }

  const fail = results.filter((r) => !r.ok).length
  const pass = results.length - fail
  const receipt = { pass, fail, results, evidence }
  if (RECEIPT) writeFileSync(RECEIPT, `${JSON.stringify(receipt, null, 2)}\n`)
  console.log(`SUMMARY pass=${pass} fail=${fail}`)
  if (fail) process.exit(1)
}

main().catch((error) => {
  console.error(error?.stack || String(error))
  if (RECEIPT) writeFileSync(RECEIPT, `${JSON.stringify({ pass: 0, fail: 1, error: String(error?.stack || error), results, evidence }, null, 2)}\n`)
  process.exit(1)
})
