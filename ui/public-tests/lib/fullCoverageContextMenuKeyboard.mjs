// Native keyboard context-menu diagnostics shared by the Timeline matrix.
// The coverage row records an unsupported transport distinctly from a failed
// Shift+F10 action, so this helper keeps that evidence intact across callers.
async function beginKeyboardMenuDiagnostics(page, menuSelector) {
  return page.evaluate((selector) => {
    const describe = (node) => {
      if (!(node instanceof Element)) return null
      const rect = node.getBoundingClientRect()
      return {
        tag: node.tagName.toLowerCase(),
        id: node.id || '',
        trackHeader: node.getAttribute('data-cut-track-header') || '',
        track: node.getAttribute('data-cut-track') || '',
        role: node.getAttribute('role') || '',
        label: node.getAttribute('aria-label') || '',
        visible: rect.width > 0 && rect.height > 0,
      }
    }
    const previous = globalThis.__shellxCutFcvKeyboardMenuDiagnostics
    previous?.stop?.()
    const keys = []
    const capture = (event) => {
      keys.push({
        type: event.type,
        key: event.key,
        code: event.code,
        shiftKey: event.shiftKey,
        target: describe(event.target),
        activeElement: describe(document.activeElement),
      })
    }
    document.addEventListener('keydown', capture, true)
    document.addEventListener('keyup', capture, true)
    globalThis.__shellxCutFcvKeyboardMenuDiagnostics = {
      menuSelector: selector,
      keys,
      stop: () => {
        document.removeEventListener('keydown', capture, true)
        document.removeEventListener('keyup', capture, true)
      },
    }
    return {
      activeElement: describe(document.activeElement),
      menu: describe(document.querySelector(selector)),
    }
  }, menuSelector).catch((error) => ({ captureError: String(error?.message || error) }))
}

async function endKeyboardMenuDiagnostics(page, menuSelector) {
  return page.evaluate((selector) => {
    const describe = (node) => {
      if (!(node instanceof Element)) return null
      const rect = node.getBoundingClientRect()
      return {
        tag: node.tagName.toLowerCase(),
        id: node.id || '',
        trackHeader: node.getAttribute('data-cut-track-header') || '',
        track: node.getAttribute('data-cut-track') || '',
        role: node.getAttribute('role') || '',
        label: node.getAttribute('aria-label') || '',
        visible: rect.width > 0 && rect.height > 0,
      }
    }
    const diagnostics = globalThis.__shellxCutFcvKeyboardMenuDiagnostics
    const keyEvents = diagnostics?.keys?.slice(-8) || []
    diagnostics?.stop?.()
    if (globalThis.__shellxCutFcvKeyboardMenuDiagnostics === diagnostics) {
      delete globalThis.__shellxCutFcvKeyboardMenuDiagnostics
    }
    return {
      captureInstalled: diagnostics?.menuSelector === selector,
      activeElement: describe(document.activeElement),
      menu: describe(document.querySelector(selector)),
      keyEvents,
    }
  }, menuSelector).catch((error) => ({ captureError: String(error?.message || error) }))
}

export async function openKeyboardContextMenu(page, target, menuSelector, dismiss) {
  await dismiss(page)
  await target.waitFor({ state: 'visible', timeout: 8_000 })
  await target.scrollIntoViewIfNeeded().catch(() => {})
  const before = await beginKeyboardMenuDiagnostics(page, menuSelector)
  const headerBefore = await target.evaluate((element) => {
    const rect = element.getBoundingClientRect()
    return {
      visible: rect.width > 0 && rect.height > 0,
      locked: element.closest('[data-cut-track]')?.getAttribute('data-cut-track-locked') || 'false',
      active: document.activeElement === element,
    }
  }).catch((error) => ({ captureError: String(error?.message || error) }))
  let pressError = ''
  try {
    await target.focus()
    await page.keyboard.press('Shift+F10')
  } catch (error) {
    pressError = String(error?.message || error)
  }
  const menu = page.locator(menuSelector).first()
  let menuWaitError = ''
  try {
    await menu.waitFor({ state: 'visible', timeout: 2_500 })
  } catch (error) {
    menuWaitError = String(error?.message || error)
  }
  const headerAfterFocus = await target.evaluate((element) => ({
    active: document.activeElement === element,
    locked: element.closest('[data-cut-track]')?.getAttribute('data-cut-track-locked') || 'false',
  })).catch((error) => ({ captureError: String(error?.message || error) }))
  const after = await endKeyboardMenuDiagnostics(page, menuSelector)
  const opened = !menuWaitError && await menu.isVisible().catch(() => false)
  return {
    menu: opened ? menu : null,
    diagnostics: { before, headerBefore, headerAfterFocus, pressError, menuWaitError, after },
  }
}
