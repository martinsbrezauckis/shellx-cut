; ShellX Cut NSIS update/install handoff.
;
; The app-side updater stops and reaps the exact child it owns. This hook is a
; second, installer-owned guard for manual installs and abnormal stale-engine
; cases: only cutd.exe whose executable path is exactly $INSTDIR\cutd.exe is
; terminated. If PowerShell cannot prove the path is quiet, installation aborts
; before NSIS can offer an unsafe skip-write path and record a mixed version.

; Resolve the helper at include time. NSIS embeds it in the signed installer.
!define SHELLX_CUT_UI_PRUNE_SOURCE "${__FILEDIR__}\ui-dist-prune.ps1"

!macro NSIS_HOOK_PREINSTALL
  ; Tauri's generated default finalizer signs the temporary uninstaller. Add a
  ; later, verification-only finalizer with an enforced zero exit so a failed
  ; signature verification aborts NSIS without a copied template or a second
  ; provider signing request. The generated template defines this command
  ; before it expands this macro; unsigned builds keep it empty.
  !if "${UNINSTALLERSIGNCOMMAND}" != ""
    !uninstfinalize '${UNINSTALLERSIGNCOMMAND} --verify-only' = 0
  !endif
  nsExec::ExecToStack 'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "& { $$target = [IO.Path]::GetFullPath($\'$INSTDIR\cutd.exe$\'); $$owned = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $$_.Name -and $$_.Name.Equals($\'cutd.exe$\', [StringComparison]::OrdinalIgnoreCase) -and $$_.ExecutablePath -and [IO.Path]::GetFullPath($$_.ExecutablePath).Equals($$target, [StringComparison]::OrdinalIgnoreCase) }); foreach ($$process in $$owned) { Stop-Process -Id $$process.ProcessId -Force -ErrorAction Stop }; if ($$owned.Count -gt 0) { Start-Sleep -Milliseconds 500 }; $$remaining = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $$_.Name -and $$_.Name.Equals($\'cutd.exe$\', [StringComparison]::OrdinalIgnoreCase) -and $$_.ExecutablePath -and [IO.Path]::GetFullPath($$_.ExecutablePath).Equals($$target, [StringComparison]::OrdinalIgnoreCase) }); if ($$remaining.Count -ne 0) { exit 42 } }"'
  Pop $0
  Pop $1
  StrCmp $0 "0" shellx_cut_engine_quiet
  IfSilent +2
  MessageBox MB_OK|MB_ICONSTOP "ShellX Cut could not stop its background engine. Close ShellX Cut and try the installer again. No files were replaced."
  SetErrorLevel 1
  Abort
shellx_cut_engine_quiet:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; This runs after the generated running-app check and all package writes.
  ; The new UI identity sidecar lists the exact packaged file hashes, so only
  ; old-only files are pruned. A cancelled install never reaches this hook.
  System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_INSTALL_ROOT", t "$INSTDIR") i .r0'
  StrCmp $0 "0" shellx_cut_ui_prune_failed
  InitPluginsDir
  File "/oname=$PLUGINSDIR\shellx-cut-ui-prune.ps1" "${SHELLX_CUT_UI_PRUNE_SOURCE}"
  nsExec::ExecToStack 'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\shellx-cut-ui-prune.ps1"'
  Pop $0
  Pop $1
  System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_INSTALL_ROOT", p 0) i .r1'
  StrCmp $0 "0" shellx_cut_ui_prune_done
shellx_cut_ui_prune_failed:
  IfSilent +2
  MessageBox MB_OK|MB_ICONSTOP "ShellX Cut could not finish its UI asset update. Check the install folder and try the installer again."
  SetErrorLevel 1
  Abort
shellx_cut_ui_prune_done:
!macroend
