// Installed-WebView action coverage for the complete topbar Sequence switcher.
// The separate sequence lifecycle gate remains the persistence/reopen proof;
// this module makes every visible control part of the final all-actions sweep
// and verifies exact requests plus live project state.

export function createSequenceSwitcherActionCoverage({
  probe,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'sequence-switcher-actions'

  async function captureRequest(page, pathname, action) {
    let payload = null
    const onRequest = (request) => {
      let requestPath = ''
      try { requestPath = new URL(request.url()).pathname } catch { return }
      if (requestPath !== pathname) return
      try { payload = request.postDataJSON() } catch {}
    }
    page.on('request', onRequest)
    try {
      await action()
      // Native confirmation dialogs resolve outside the WebView. The accepted
      // request can therefore reach the page bridge after the click promise and
      // state poll have completed. Drain while the listener is still attached
      // so exact payload proof is not lost as a stale queued event.
      const deadline = Date.now() + 2_000
      do {
        await page.flushEvents?.()
        if (payload !== null) break
        await sleep(80)
      } while (Date.now() < deadline)
    } finally {
      page.off('request', onRequest)
    }
    return payload
  }

  async function openMenu(page) {
    const trigger = page.locator('[data-cut-sequence-trigger]').first()
    if ((await trigger.getAttribute('aria-expanded')) !== 'true') {
      await trigger.click()
    }
    await page.locator('[data-cut-sequence-menu]').first()
      .waitFor({ state: 'visible', timeout: 8_000 })
    await page.locator('[data-cut-sequence-row]').first()
      .waitFor({ state: 'visible', timeout: 8_000 })
    return page.locator('[data-cut-sequence-menu]').first()
  }

  async function run(page) {
    await freshProject(page, 'sequence_switcher_actions', primaryMedia)
    await closeOverlays(page)

    const trigger = page.locator('[data-cut-sequence-trigger]').first()
    let listResponse = null
    await probe(page, {
      surface,
      name: 'sequence-trigger',
      actionId: 'sequence-trigger',
      sel: trigger,
      group: page.locator('[data-cut-sequences]').first(),
      groupName: 'sequence-trigger',
      doClick: async () => {
        listResponse = await captureVerbResp(
          page,
          'project.sequence_list',
          () => openMenu(page),
          20_000,
        )
      },
      assertResult: async () => {
        const menu = page.locator('[data-cut-sequence-menu]').first()
        const expanded = await trigger.getAttribute('aria-expanded')
        const active = await page.locator('[data-cut-sequence-switch][aria-checked="true"]').count()
        const listed = listResponse?.result
        return {
          ok: listResponse?.ok
            && Array.isArray(listed?.sequences)
            && listed.sequences.length === 1
            && listed.active_sequence === 'seq1'
            && await menu.isVisible()
            && expanded === 'true'
            && active === 1,
          detail: `listOk=${listResponse?.ok}; sequences=${listed?.sequences?.length ?? 'missing'}; active=${listed?.active_sequence || 'missing'}; menu visible=${await menu.isVisible()}; expanded=${expanded}; one active row=${active}`,
        }
      },
    })

    let menu = page.locator('[data-cut-sequence-menu]').first()
    const newButton = page.locator('[data-cut-sequence-new]').first()
    await probe(page, {
      surface,
      name: 'sequence-new',
      actionId: 'sequence-new',
      sel: newButton,
      group: menu,
      groupName: 'sequence-menu',
      doClick: async () => {
        await newButton.click()
        await page.locator('[data-cut-sequence-create]').first()
          .waitFor({ state: 'visible', timeout: 5_000 })
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-sequence-create]').first().isVisible(),
        detail: 'new-sequence form mounted',
      }),
    })

    let form = page.locator('[data-cut-sequence-create]').first()
    const nameInput = page.locator('[data-cut-sequence-name]').first()
    await probe(page, {
      surface,
      name: 'sequence-name',
      actionId: 'sequence-name',
      sel: nameInput,
      group: form,
      groupName: 'sequence-create',
      doClick: async () => { await nameInput.fill('Cancelled draft') },
      assertResult: async () => ({
        ok: await nameInput.inputValue() === 'Cancelled draft',
        detail: `name=${await nameInput.inputValue()}`,
      }),
    })

    const duplicate = page.locator('[data-cut-sequence-from="active"]').first()
    await probe(page, {
      surface,
      name: 'sequence-from',
      actionId: 'sequence-from',
      sel: duplicate,
      group: form,
      groupName: 'sequence-create',
      doClick: async () => { await duplicate.click() },
      assertResult: async () => ({
        ok: await duplicate.evaluate((element) => element.classList.contains('is-selected')),
        detail: `Duplicate selected=${await duplicate.evaluate((element) => element.classList.contains('is-selected'))}`,
      }),
    })

    const cancel = page.locator('[data-cut-sequence-create-cancel]').first()
    await probe(page, {
      surface,
      name: 'sequence-create-cancel',
      actionId: 'sequence-create-cancel',
      sel: cancel,
      group: form,
      groupName: 'sequence-create',
      doClick: async () => {
        await cancel.click()
        await page.locator('[data-cut-sequence-create]').first()
          .waitFor({ state: 'detached', timeout: 5_000 })
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-sequence-create]').count() === 0,
        detail: `create form count=${await page.locator('[data-cut-sequence-create]').count()}`,
      }),
    })

    await page.locator('[data-cut-sequence-new]').first().click()
    form = page.locator('[data-cut-sequence-create]').first()
    await form.waitFor({ state: 'visible', timeout: 5_000 })
    await page.locator('[data-cut-sequence-name]').first().fill('Review')
    await page.locator('[data-cut-sequence-from="active"]').first().click()
    const submit = page.locator('[data-cut-sequence-create-submit]').first()
    await probe(page, {
      surface,
      name: 'sequence-create-submit',
      actionId: 'sequence-create-submit',
      sel: submit,
      group: form,
      groupName: 'sequence-create-submit',
      doClick: async () => {
        probe._sequenceCreateArgs = await captureRequest(
          page,
          '/api/verb/project.sequence_create',
          () => submit.click(),
        )
        await page.locator('[data-cut-sequence-menu]').first()
          .waitFor({ state: 'detached', timeout: 8_000 })
      },
      assertResult: async () => {
        const created = await waitForState((project) =>
          project.active_sequence === 'seq2'
          && project.sequences?.some((sequence) => sequence.id === 'seq2' && sequence.name === 'Review'),
        12_000)
        const args = probe._sequenceCreateArgs
        const exact = args?.name === 'Review'
          && args?.from === 'active'
          && args?.rationale === 'user: create active sequence'
        const label = await page.locator('[data-cut-sequence-active="seq2"]').textContent()
        return {
          ok: !!created && exact && label === 'Review',
          detail: `seq2 active=${!!created}; exact create=${exact}; label=${label}; args=${JSON.stringify(args)}`,
        }
      },
    })

    menu = await openMenu(page)
    const switchMain = page.locator('[data-cut-sequence-switch="seq1"]').first()
    await probe(page, {
      surface,
      name: 'sequence-switch',
      actionId: 'sequence-switch',
      sel: switchMain,
      group: menu,
      groupName: 'sequence-switch',
      doClick: async () => {
        probe._sequenceSwitchArgs = await captureRequest(
          page,
          '/api/verb/project.sequence_switch',
          () => switchMain.click(),
        )
        await page.locator('[data-cut-sequence-menu]').first()
          .waitFor({ state: 'detached', timeout: 8_000 })
      },
      assertResult: async () => {
        const switched = await waitForState((project) => (project.active_sequence ?? 'seq1') === 'seq1', 12_000)
        const args = probe._sequenceSwitchArgs
        const exact = args?.id === 'seq1' && args?.rationale === 'user: switch sequence'
        return {
          ok: !!switched && exact,
          detail: `Main active=${!!switched}; exact switch=${exact}; args=${JSON.stringify(args)}`,
        }
      },
    })

    menu = await openMenu(page)
    const rename = page.locator('[data-cut-sequence-rename="seq2"]').first()
    await probe(page, {
      surface,
      name: 'sequence-rename',
      actionId: 'sequence-rename',
      sel: rename,
      group: menu,
      groupName: 'sequence-menu',
      doClick: async () => {
        await rename.click()
        await page.locator('[data-cut-sequence-rename-input="seq2"]').first()
          .waitFor({ state: 'visible', timeout: 5_000 })
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-sequence-rename-input="seq2"]').first().isVisible(),
        detail: 'rename form mounted for seq2',
      }),
    })

    const renameInput = page.locator('[data-cut-sequence-rename-input="seq2"]').first()
    const renameForm = page.locator('[data-cut-sequence-row="seq2"]').first()
    await probe(page, {
      surface,
      name: 'sequence-rename-input',
      actionId: 'sequence-rename-input',
      sel: renameInput,
      group: renameForm,
      groupName: 'sequence-rename-form',
      doClick: async () => { await renameInput.fill('Review cut') },
      assertResult: async () => ({
        ok: await renameInput.inputValue() === 'Review cut',
        detail: `name=${await renameInput.inputValue()}`,
      }),
    })

    const renameSave = page.locator('[data-cut-sequence-rename-save="seq2"]').first()
    await probe(page, {
      surface,
      name: 'sequence-rename-save',
      actionId: 'sequence-rename-save',
      sel: renameSave,
      group: renameForm,
      groupName: 'sequence-rename-form',
      doClick: async () => {
        probe._sequenceRenameArgs = await captureRequest(
          page,
          '/api/verb/project.sequence_rename',
          () => renameSave.click(),
        )
        await page.locator('[data-cut-sequence-row="seq2"]').filter({ hasText: 'Review cut' })
          .waitFor({ state: 'visible', timeout: 8_000 })
      },
      assertResult: async () => {
        const renamed = await waitForState((project) =>
          project.sequences?.some((sequence) => sequence.id === 'seq2' && sequence.name === 'Review cut'),
        12_000)
        const args = probe._sequenceRenameArgs
        const exact = args?.id === 'seq2'
          && args?.name === 'Review cut'
          && args?.rationale === 'user: rename sequence'
        return {
          ok: !!renamed && exact,
          detail: `state renamed=${!!renamed}; exact rename=${exact}; args=${JSON.stringify(args)}`,
        }
      },
    })

    const remove = page.locator('[data-cut-sequence-delete="seq2"]').first()
    await probe(page, {
      surface,
      name: 'sequence-delete',
      actionId: 'sequence-delete',
      sel: remove,
      group: menu,
      groupName: 'sequence-delete',
      nativeAction: { mode: 'accept', useDoClick: true, verifyResult: true },
      doClick: async () => {
        const accept = (dialog) => { void dialog.accept() }
        page.on('dialog', accept)
        try {
          probe._sequenceDeleteArgs = await captureRequest(
            page,
            '/api/verb/project.sequence_delete',
            async () => {
              await remove.click()
              // The native rfd confirmation resolves asynchronously. Keep the
              // request listener attached until the accepted deletion reaches
              // live project state; an 80 ms post-click grace only covers the
              // browser-confirm path and can miss a real host TaskDialog.
              await waitForState((project) =>
                !project.sequences?.some((sequence) => sequence.id === 'seq2'),
              12_000)
            },
          )
          await waitForState((project) =>
            !project.sequences?.some((sequence) => sequence.id === 'seq2'),
          12_000)
        } finally {
          page.off('dialog', accept)
        }
      },
      assertResult: async () => {
        const current = await state()
        const deleted = !current.sequences?.some((sequence) => sequence.id === 'seq2')
        const args = probe._sequenceDeleteArgs
        const exact = args?.id === 'seq2' && args?.rationale === 'user: delete sequence'
        return {
          ok: deleted && exact,
          detail: `seq2 deleted=${deleted}; exact delete=${exact}; args=${JSON.stringify(args)}`,
        }
      },
    })
  }

  return { run }
}
