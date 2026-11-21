; ShellX Cut NSIS update/install handoff.
;
; The app-side updater stops and reaps the exact child it owns. This hook is a
; second, installer-owned guard for manual installs and abnormal stale-engine
; cases: only cutd.exe whose executable path is exactly $INSTDIR\cutd.exe is
; terminated. If PowerShell cannot prove the path is quiet, installation aborts
; before NSIS can offer an unsafe skip-write path and record a mixed version.

; Resolve the helper at include time. NSIS embeds it in the signed installer.
!define SHELLX_CUT_UI_PRUNE_SOURCE "${__FILEDIR__}\ui-dist-prune.ps1"
!define SHELLX_CUT_OBSOLETE_PRUNE_SOURCE "${__FILEDIR__}\obsolete-owned-resources-prune.ps1"
!define SHELLX_CUT_UNINSTALL_CLEANUP_SOURCE "${__FILEDIR__}\uninstall-owned-files.ps1"

; The uninstaller's first page says what goes and what stays. This file is
; included before the generated pages, so MUI uses this text on that page.
!define MUI_UNCONFIRMPAGE_TEXT_TOP "Removes ShellX Cut and the tools it downloaded (FFmpeg, captions). Your projects, Library, recordings and the Microsoft Visual C++ runtime stay. Tick the box below to also delete its settings and project list."

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
  nsExec::ExecToStack 'powershell.exe -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$PLUGINSDIR\shellx-cut-ui-prune.ps1"'
  Pop $0
  Pop $1
  StrCmp $0 "0" shellx_cut_ui_prune_succeeded
  System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_INSTALL_ROOT", p 0) i .r1'
  Goto shellx_cut_ui_prune_failed
shellx_cut_ui_prune_succeeded:
  File "/oname=$PLUGINSDIR\shellx-cut-obsolete-prune.ps1" "${SHELLX_CUT_OBSOLETE_PRUNE_SOURCE}"
  nsExec::ExecToStack 'powershell.exe -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$PLUGINSDIR\shellx-cut-obsolete-prune.ps1"'
  Pop $0
  Pop $1
  DetailPrint $1
  System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_INSTALL_ROOT", p 0) i .r1'
  StrCmp $0 "0" shellx_cut_ui_prune_done
shellx_cut_ui_prune_failed:
  IfSilent +2
  MessageBox MB_OK|MB_ICONSTOP "ShellX Cut could not finish its installer resource cleanup. Check the install folder and try the installer again."
  SetErrorLevel 1
  Abort
shellx_cut_ui_prune_done:
!macroend

; Uninstall: remove what the app itself created.
;
; Besides the packaged files (which the generated uninstaller already deletes)
; the app writes downloads and run files into its folder: the video tools
; (tools), the captions Python and its environment and cache
; (perception\python, perception\.venv, perception\uv-cache), speech models,
; the premium background-removal environment and model, the work folder,
; engine.addr and tools-doctor.json. uninstall-owned-files.ps1 lists them and
; removes them; with "Delete the application data" ticked it also removes the
; settings, the project list and the remembered export folder. Projects, Library copies and recordings stay,
; and so does Microsoft's Visual C++ runtime: it belongs to Windows.
; The script is embedded and called by path: an inline -Command longer than
; NSIS's 1,024-character string limit is cut off and never runs.
;
; An update (the updater, or the installer run over an existing install) never
; runs this uninstaller, so downloaded tools survive updates without a new
; download. Only an explicit uninstall, including "uninstall before installing",
; reaches this hook; the /UPDATE guard is a second safeguard.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    SetDetailsView show
    InitPluginsDir
    File "/oname=$PLUGINSDIR\shellx-cut-uninstall-cleanup.ps1" "${SHELLX_CUT_UNINSTALL_CLEANUP_SOURCE}"
    System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_INSTALL_ROOT", t "$INSTDIR") i .r0'
    System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_DELETE_APP_DATA", t "$DeleteAppDataCheckboxState") i .r0'
    nsExec::ExecToLog 'powershell.exe -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$PLUGINSDIR\shellx-cut-uninstall-cleanup.ps1"'
    Pop $0
    StrCmp $0 "0" +2
    DetailPrint "ShellX Cut could not finish removing its downloaded files (code $0). The folders listed above can be deleted by hand."
    System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_INSTALL_ROOT", p 0) i .r2'
    System::Call 'Kernel32::SetEnvironmentVariable(t "SHELLX_CUT_DELETE_APP_DATA", p 0) i .r2'
    ; Remove the now-empty folders; RMDir without /r never deletes content.
    RMDir "$INSTDIR\perception\judge\adapters"
    RMDir "$INSTDIR\perception\judge"
    RMDir "$INSTDIR\perception"
    RMDir "$INSTDIR\matte"
    RMDir "$INSTDIR"
    RMDir "$LOCALAPPDATA\ShellX Cut\perception"
    RMDir "$LOCALAPPDATA\ShellX Cut\matte"
    RMDir "$LOCALAPPDATA\ShellX Cut"
  ${EndIf}
!macroend
