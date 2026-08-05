// Host-picker action proof shared by empty and populated Assets surfaces.

export function createAssetsPickerProbe({
  probe,
  captureVerbResp,
  awaitImportJobs,
  waitForState,
  sleep,
  basenameHostPath,
  nativePickerClickNa,
  nativeOsActionsEnabled,
}) {
  return async function pickerProbe(page, {
    name,
    actionId,
    selector,
    panel,
    browserEvidence,
    selectPath = '',
    selectVerb = 'media.import',
    selectAsset = '',
    surface = 'assets',
    groupName = 'assets-panel',
  }) {
    // Several workspaces reuse data-cut-import-cta. Scope the control to the
    // visible Assets panel so a hidden sibling cannot absorb the interaction.
    const control = panel.locator(selector).first()
    let selectedResponse = null
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: panel,
      groupName,
      clickNa: nativePickerClickNa,
      nativeAction: {
        mode: selectPath && nativeOsActionsEnabled ? 'select' : 'cancel',
        path: selectPath,
        useDoClick: !!selectPath && nativeOsActionsEnabled,
        verifyResult: !!selectPath && nativeOsActionsEnabled,
      },
      doClick: async () => {
        if (selectPath && nativeOsActionsEnabled) {
          selectedResponse = await captureVerbResp(
            page,
            selectVerb,
            () => control.click(),
            60_000,
          )
          if (selectedResponse?.ok) await awaitImportJobs(selectedResponse)
        } else {
          await control.click()
        }
        await sleep(180)
      },
      assertResult: async () => {
        if (selectPath && nativeOsActionsEnabled) {
          if (selectVerb === 'media.relink') {
            const project = selectAsset
              ? await waitForState(
                (value) => basenameHostPath(value.assets?.[selectAsset]?.path) ===
                  basenameHostPath(selectPath),
                12_000,
              )
              : null
            return {
              ok: !!selectedResponse?.ok
                && selectedResponse?.result?.asset === selectAsset
                && !!project,
              detail: `media.relink ok=${selectedResponse?.ok}; selected=${basenameHostPath(selectPath)}; asset=${selectedResponse?.result?.asset || 'missing'}; projectState=${!!project}`,
            }
          }
          const asset = selectedResponse?.result?.asset_id || ''
          const project = asset
            ? await waitForState((value) => !!value.assets?.[asset], 12_000)
            : null
          return {
            ok: !!selectedResponse?.ok && !!asset && !!project,
            detail: `media.import ok=${selectedResponse?.ok}; selected=${basenameHostPath(selectPath)}; asset=${asset || 'missing'}; projectState=${!!project}`,
          }
        }
        return {
          ok: await browserEvidence(page),
          detail: 'browser fallback explains that the desktop app owns the native picker',
        }
      },
    })
  }
}
