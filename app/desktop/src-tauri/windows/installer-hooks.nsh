; ShellX Cut NSIS update/install handoff.
;
; The app-side updater stops and reaps the exact child it owns. This hook is a
; second, installer-owned guard for manual installs and abnormal stale-engine
; cases: only cutd.exe whose executable path is exactly $INSTDIR\cutd.exe is
; terminated. If PowerShell cannot prove the path is quiet, installation aborts
; before NSIS can offer an unsafe skip-write path and record a mixed version.

!macro NSIS_HOOK_PREINSTALL
  nsExec::ExecToStack 'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "& { $$target = [IO.Path]::GetFullPath($\'$INSTDIR\cutd.exe$\'); $$owned = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $$_.Name -and $$_.Name.Equals($\'cutd.exe$\', [StringComparison]::OrdinalIgnoreCase) -and $$_.ExecutablePath -and [IO.Path]::GetFullPath($$_.ExecutablePath).Equals($$target, [StringComparison]::OrdinalIgnoreCase) }); foreach ($$process in $$owned) { Stop-Process -Id $$process.ProcessId -Force -ErrorAction Stop }; if ($$owned.Count -gt 0) { Start-Sleep -Milliseconds 500 }; $$remaining = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $$_.Name -and $$_.Name.Equals($\'cutd.exe$\', [StringComparison]::OrdinalIgnoreCase) -and $$_.ExecutablePath -and [IO.Path]::GetFullPath($$_.ExecutablePath).Equals($$target, [StringComparison]::OrdinalIgnoreCase) }); if ($$remaining.Count -ne 0) { exit 42 } }"'
  Pop $0
  Pop $1
  StrCmp $0 "0" shellx_cut_engine_quiet
  MessageBox MB_OK|MB_ICONSTOP "ShellX Cut could not stop its background engine. Close ShellX Cut and try the installer again. No files were replaced."
  Abort
shellx_cut_engine_quiet:
!macroend
