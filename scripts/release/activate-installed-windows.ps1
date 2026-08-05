param(
  [string]$ExpectedProcessName = "shellx-cut"
)

$ErrorActionPreference = "Stop"
$sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
$matches = @(
  Get-Process $ExpectedProcessName -ErrorAction SilentlyContinue |
    Where-Object SessionId -eq $sessionId
)
if ($matches.Count -ne 1) {
  throw "Expected one interactive $ExpectedProcessName process; found $($matches.Count)."
}
$process = $matches[0]
for ($attempt = 0; $attempt -lt 40 -and $process.MainWindowHandle -eq 0; $attempt++) {
  Start-Sleep -Milliseconds 100
  $process.Refresh()
}
if ($process.MainWindowHandle -eq 0) {
  throw "Installed $ExpectedProcessName process $($process.Id) has no main window handle."
}

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class CutUnattendedFocus {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool attach);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int command);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern void SwitchToThisWindow(IntPtr hWnd, bool altTab);
}
"@

$target = [IntPtr]::new($process.MainWindowHandle.ToInt64())
$foreground = [CutUnattendedFocus]::GetForegroundWindow()
[uint32]$foregroundPid = 0
$foregroundThread = [CutUnattendedFocus]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
[uint32]$targetPid = 0
$targetThread = [CutUnattendedFocus]::GetWindowThreadProcessId($target, [ref]$targetPid)
$currentThread = [CutUnattendedFocus]::GetCurrentThreadId()
if ($targetPid -ne $process.Id) {
  throw "Validated ShellX Cut process/window mismatch: process=$($process.Id) window=$targetPid."
}

$attachedForeground = $false
$attachedTarget = $false
try {
  if ($foregroundThread -ne 0 -and $foregroundThread -ne $currentThread) {
    $attachedForeground = [CutUnattendedFocus]::AttachThreadInput(
      $currentThread,
      $foregroundThread,
      $true
    )
  }
  if ($targetThread -ne 0 -and $targetThread -ne $currentThread) {
    $attachedTarget = [CutUnattendedFocus]::AttachThreadInput(
      $currentThread,
      $targetThread,
      $true
    )
  }
  $null = [CutUnattendedFocus]::ShowWindowAsync($target, 9)
  $null = [CutUnattendedFocus]::BringWindowToTop($target)
  $null = [CutUnattendedFocus]::SetForegroundWindow($target)
  $null = [CutUnattendedFocus]::SetFocus($target)
  [CutUnattendedFocus]::SwitchToThisWindow($target, $true)
} finally {
  if ($attachedTarget) {
    $null = [CutUnattendedFocus]::AttachThreadInput($currentThread, $targetThread, $false)
  }
  if ($attachedForeground) {
    $null = [CutUnattendedFocus]::AttachThreadInput($currentThread, $foregroundThread, $false)
  }
}

Start-Sleep -Milliseconds 400
$landedWindow = [CutUnattendedFocus]::GetForegroundWindow()
[uint32]$landedPid = 0
$null = [CutUnattendedFocus]::GetWindowThreadProcessId($landedWindow, [ref]$landedPid)
$landedProcess = Get-Process -Id $landedPid -ErrorAction SilentlyContinue
if ($landedPid -ne $process.Id -or $landedProcess.ProcessName -ne $ExpectedProcessName) {
  throw "Unattended focus did not land on exact $ExpectedProcessName process $($process.Id); foreground=$landedPid/$($landedProcess.ProcessName)."
}

[pscustomobject]@{
  requestedPid = $process.Id
  foregroundPid = $landedPid
  foregroundProcess = $landedProcess.ProcessName
  sessionId = $sessionId
} | ConvertTo-Json -Compress
