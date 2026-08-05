// Chromium accessibility-tree gate for every surface addressable through
// ui.open. This complements action/effect coverage: a control can work for a
// pointer and still be unusable to assistive technology when it has no name.
//
// RUN:
//   SWEEP_APP=http://127.0.0.1:6171 npm run verify-accessibility-surfaces

import { chromium } from 'playwright'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const APP = process.env.SWEEP_APP || 'http://127.0.0.1:5208'
const CUTD = process.env.SWEEP_CUTD || APP
const INTERACTIVE_ROLES = new Set([
  'button',
  'checkbox',
  'combobox',
  'link',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'radio',
  'searchbox',
  'slider',
  'spinbutton',
  'switch',
  'tab',
  'textbox',
  'treeitem',
])

async function postVerb(name, args) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-cut-actor': 'agent:test:accessibility-surface',
    },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(15_000),
  })
  return response.json()
}

function attributeMap(attributes = []) {
  const out = {}
  for (let index = 0; index < attributes.length; index += 2) {
    out[attributes[index]] = attributes[index + 1]
  }
  return out
}

function nodeIdentity(node) {
  const attrs = attributeMap(node.attributes)
  const cut = Object.entries(attrs).find(([name]) => name.startsWith('data-cut'))
  return {
    element: node.nodeName.toLowerCase(),
    selector: cut ? `[${cut[0]}${cut[1] ? `="${cut[1]}"` : ''}]` : null,
    type: attrs.type || null,
    role: attrs.role || null,
  }
}

async function scanSurface(page, cdp, surface) {
  const tree = await cdp.send('Accessibility.getFullAXTree')
  const failures = []
  for (const node of tree.nodes) {
    if (node.ignored || !node.backendDOMNodeId) continue
    const role = String(node.role?.value || '')
    const name = String(node.name?.value || '').trim()
    const focusable = node.properties?.some(
      (property) => property.name === 'focusable' && property.value?.value === true,
    )
    const disabled = node.properties?.some(
      (property) => property.name === 'disabled' && property.value?.value === true,
    )
    const unnamedControl = focusable && INTERACTIVE_ROLES.has(role) && !name
    const unreachableControl = !focusable && !disabled && INTERACTIVE_ROLES.has(role)
    const unnamedDialog = (role === 'dialog' || role === 'alertdialog') && !name
    if (!unnamedControl && !unnamedDialog && !unreachableControl) continue
    try {
      const described = await cdp.send('DOM.describeNode', {
        backendNodeId: node.backendDOMNodeId,
        depth: 0,
        pierce: true,
      })
      failures.push({
        surface,
        kind: unnamedDialog
          ? 'unnamed-dialog'
          : unreachableControl
            ? 'enabled-control-not-focusable'
            : 'unnamed-control',
        accessibilityRole: role,
        ...nodeIdentity(described.node),
      })
    } catch {
      failures.push({
        surface,
        kind: unnamedDialog
          ? 'unnamed-dialog'
          : unreachableControl
            ? 'enabled-control-not-focusable'
            : 'unnamed-control',
        accessibilityRole: role,
        element: 'remounted-before-description',
        selector: null,
        type: null,
        role: null,
      })
    }
  }

  const duplicateIds = await page.evaluate(() => {
    const ids = Array.from(document.querySelectorAll('[id]'), (element) => element.id)
      .filter(Boolean)
    return Array.from(new Set(ids.filter((id, index) => ids.indexOf(id) !== index)))
  })
  for (const id of duplicateIds) {
    failures.push({
      surface,
      kind: 'duplicate-id',
      accessibilityRole: null,
      element: null,
      selector: `#${id}`,
      type: null,
      role: null,
    })
  }
  return failures
}

async function main() {
  const projectParent = mkdtempSync(join(tmpdir(), 'shellx-cut-a11y.'))
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  const failures = []
  let ownsProject = false
  let safeToRemoveProject = true
  let projectPath = ''
  try {
    const projectName = `accessibility-${Date.now()}`
    projectPath = join(projectParent, `${projectName}.cutproj`)
    const created = await postVerb('project.create', {
      name: projectName,
      dir: projectPath,
    })
    if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)
    ownsProject = true
    const title = await postVerb('title.add', {
      text: 'Accessibility surface',
      range_ms: [0, 2_000],
    })
    if (!title.ok) throw new Error(`title.add failed: ${JSON.stringify(title.error)}`)

    await page.goto(APP, { waitUntil: 'networkidle' })
    await page.locator('[data-cut-app-root]').waitFor({ state: 'visible', timeout: 10_000 })
    const cdp = await page.context().newCDPSession(page)
    await cdp.send('Accessibility.enable')
    await cdp.send('DOM.enable')

    const registry = await fetch(`${CUTD}/api/verbs`, {
      signal: AbortSignal.timeout(15_000),
    }).then((response) => response.json())
    const surfaceIds = registry.verbs
      .find((verb) => verb.name === 'ui.open')
      ?.args?.properties?.panel?.enum
    if (!Array.isArray(surfaceIds) || surfaceIds.length === 0) {
      throw new Error('ui.open surface registry is missing')
    }

    let opened = 0
    for (const surface of surfaceIds) {
      // `review` and `review-ops` intentionally share the Ops destination.
      if (surface === 'review-ops') await postVerb('ui.open', { panel: 'receipts' })
      const response = await postVerb('ui.open', { panel: surface })
      if (!response.ok || response.result?.applied !== true) {
        failures.push({
          surface,
          kind: 'surface-did-not-open',
          accessibilityRole: null,
          element: null,
          selector: response.result?.selector || null,
          type: null,
          role: null,
          error: response.error?.message || 'ui.open did not confirm the surface',
        })
        continue
      }
      opened += 1
      if (response.result.selector) {
        await page.locator(response.result.selector).waitFor({ state: 'visible', timeout: 5_000 })
      }
      failures.push(...await scanSurface(page, cdp, surface))
    }

    const receipt = {
      schema: 'shellx-cut/accessibility-surface-verify@1',
      ok: failures.length === 0,
      surfaces: surfaceIds.length,
      opened,
      failures,
    }
    process.stdout.write(`${JSON.stringify(receipt, null, 2)}\n`)
    if (!receipt.ok || opened !== surfaceIds.length) process.exitCode = 1
  } finally {
    if (ownsProject) {
      try {
        const closed = await postVerb('project.close', {})
        if (!closed.ok) {
          safeToRemoveProject = false
          process.stderr.write(`accessibility-surface-verify: could not close disposable project: ${JSON.stringify(closed.error)}\n`)
          process.exitCode = 1
        }
        if (safeToRemoveProject) {
          const forgotten = await postVerb('project.forget', { path: projectPath })
          if (!forgotten.ok) {
            safeToRemoveProject = false
            process.stderr.write(`accessibility-surface-verify: could not forget disposable project: ${JSON.stringify(forgotten.error)}\n`)
            process.exitCode = 1
          }
        }
      } catch (error) {
        safeToRemoveProject = false
        process.stderr.write(`accessibility-surface-verify: could not retire disposable project: ${error.message || error}\n`)
        process.exitCode = 1
      }
    }
    await browser.close()
    if (safeToRemoveProject) rmSync(projectParent, { recursive: true, force: true })
  }
}

main().catch((error) => {
  process.stderr.write(`accessibility-surface-verify: ${error.stack || error}\n`)
  process.exitCode = 1
})
