param(
  [Parameter(Mandatory = $true)]
  [ValidateSet("state", "act", "focus")]
  [string]$Command,
  [Int64]$Handle = 0,
  [ValidateSet("accept", "cancel", "select")]
  [string]$Mode = "cancel",
  [string]$Path = "",
  [string]$ExpectedProcessName = "shellx-cut"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class CutNativeDialog {
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int command);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr hWnd, int controlId);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint message, IntPtr wParam, IntPtr lParam);
}
"@

if ($Command -eq "state") {
  $window = [CutNativeDialog]::GetForegroundWindow()
  $title = New-Object System.Text.StringBuilder 1024
  $class = New-Object System.Text.StringBuilder 256
  $null = [CutNativeDialog]::GetWindowText($window, $title, $title.Capacity)
  $null = [CutNativeDialog]::GetClassName($window, $class, $class.Capacity)
  [uint32]$processId = 0
  $null = [CutNativeDialog]::GetWindowThreadProcessId($window, [ref]$processId)
  $processName = if ($processId -gt 0) {
    (Get-Process -Id $processId -ErrorAction SilentlyContinue).ProcessName
  } else {
    ""
  }
  $windows = [System.Collections.Generic.List[object]]::new()
  $callback = [CutNativeDialog+EnumWindowsProc]{
    param([IntPtr]$candidate, [IntPtr]$lParam)
    if (-not [CutNativeDialog]::IsWindowVisible($candidate)) { return $true }
    $candidateTitle = New-Object System.Text.StringBuilder 1024
    $candidateClass = New-Object System.Text.StringBuilder 256
    $null = [CutNativeDialog]::GetWindowText($candidate, $candidateTitle, $candidateTitle.Capacity)
    $null = [CutNativeDialog]::GetClassName($candidate, $candidateClass, $candidateClass.Capacity)
    [uint32]$candidateProcessId = 0
    $null = [CutNativeDialog]::GetWindowThreadProcessId($candidate, [ref]$candidateProcessId)
    $candidateProcessName = if ($candidateProcessId -gt 0) {
      (Get-Process -Id $candidateProcessId -ErrorAction SilentlyContinue).ProcessName
    } else {
      ""
    }
    $windows.Add([pscustomobject]@{
      handle = $candidate.ToInt64()
      title = $candidateTitle.ToString()
      className = $candidateClass.ToString()
      processId = $candidateProcessId
      processName = $candidateProcessName
    })
    return $true
  }
  $null = [CutNativeDialog]::EnumWindows($callback, [IntPtr]::Zero)
  [pscustomobject]@{
    handle = $window.ToInt64()
    title = $title.ToString()
    className = $class.ToString()
    processId = $processId
    processName = $processName
    windows = $windows
  } | ConvertTo-Json -Compress -Depth 3
  exit 0
}

if ($Handle -le 0) { throw "A positive window handle is required." }
$window = [IntPtr]::new($Handle)
[uint32]$dialogProcessId = 0
$null = [CutNativeDialog]::GetWindowThreadProcessId($window, [ref]$dialogProcessId)
$dialogProcessName = if ($dialogProcessId -gt 0) {
  (Get-Process -Id $dialogProcessId -ErrorAction SilentlyContinue).ProcessName
} else {
  ""
}
if ($dialogProcessName -ne $ExpectedProcessName) {
  throw "Refusing native input: window process '$dialogProcessName' is not '$ExpectedProcessName'."
}
if ($Command -eq "focus") {
  if (-not [CutNativeDialog]::IsWindowVisible($window)) {
    throw "Refusing focus restore: ShellX Cut window $Handle is not visible."
  }
  $SW_RESTORE = 9
  for ($attempt = 0; $attempt -lt 10; $attempt++) {
    $null = [CutNativeDialog]::ShowWindow($window, $SW_RESTORE)
    $null = [CutNativeDialog]::SetForegroundWindow($window)
    Start-Sleep -Milliseconds 100
    if ([CutNativeDialog]::GetForegroundWindow() -eq $window) { exit 0 }
  }
  throw "ShellX Cut window $Handle did not regain foreground focus."
}
function Assert-ForegroundTarget {
  $foreground = [CutNativeDialog]::GetForegroundWindow()
  if ($foreground -ne $window) {
    throw "Refusing native input: ShellX Cut dialog $Handle is not the foreground window (foreground=$($foreground.ToInt64()))."
  }
}
Assert-ForegroundTarget
if ($Mode -eq "cancel") {
  Assert-ForegroundTarget
  [System.Windows.Forms.SendKeys]::SendWait("{ESC}")
  exit 0
}
$BM_CLICK = 0x00F5
if ($Mode -eq "accept") {
  # rfd uses a TaskDialog custom affirmative button (id 1000) whenever the
  # product supplies labels such as "Delete" / "Remove". Plain Ok/Cancel and
  # Yes/No dialogs use Win32 ids 1 and 6. TDM_CLICK_BUTTON is the supported
  # TaskDialog action; BM_CLICK remains the classic MessageBox fallback.
  $TDM_CLICK_BUTTON = 0x0400 + 102
  $affirmativeButtonIds = @(1000, 1, 6)
  foreach ($buttonId in $affirmativeButtonIds) {
    if (-not [CutNativeDialog]::IsWindow($window)) { exit 0 }
    Assert-ForegroundTarget
    $null = [CutNativeDialog]::PostMessage(
      $window,
      $TDM_CLICK_BUTTON,
      [IntPtr]::new($buttonId),
      [IntPtr]::Zero
    )
    Start-Sleep -Milliseconds 120
    if (-not [CutNativeDialog]::IsWindow($window)) { exit 0 }
    $button = [CutNativeDialog]::GetDlgItem($window, $buttonId)
    if ($button -ne [IntPtr]::Zero) {
      Assert-ForegroundTarget
      $null = [CutNativeDialog]::PostMessage(
        $button,
        $BM_CLICK,
        [IntPtr]::Zero,
        [IntPtr]::Zero
      )
      Start-Sleep -Milliseconds 120
    }
  }
  if (-not [CutNativeDialog]::IsWindow($window)) { exit 0 }
  throw "ShellX Cut confirmation dialog did not accept a custom OK, standard OK, or Yes action."
}
if ([string]::IsNullOrWhiteSpace($Path)) { throw "Select mode requires a path." }
$pickerPath = $Path
if ($pickerPath.StartsWith("\\?\UNC\", [System.StringComparison]::OrdinalIgnoreCase)) {
  $pickerPath = "\\" + $pickerPath.Substring(8)
} elseif ($pickerPath.StartsWith("\\?\", [System.StringComparison]::OrdinalIgnoreCase)) {
  $pickerPath = $pickerPath.Substring(4)
}
# Cross-host harness paths can be valid Win32 paths with a POSIX separator at
# the final join boundary. The Common Item Dialog filename field is stricter
# than CreateFile and may navigate without committing such a mixed path.
$pickerPath = $pickerPath.Replace("/", "\")
if ($pickerPath -match '[+^%~(){}\[\]]') {
  throw "The test selection path contains SendKeys metacharacters: $pickerPath"
}
Assert-ForegroundTarget
[System.Windows.Forms.SendKeys]::SendWait("%n")
Start-Sleep -Milliseconds 100
Assert-ForegroundTarget
[System.Windows.Forms.SendKeys]::SendWait("^a")
Start-Sleep -Milliseconds 100
Assert-ForegroundTarget
[System.Windows.Forms.SendKeys]::SendWait($pickerPath)
Assert-ForegroundTarget
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Milliseconds 400

# In FOS_PICKFOLDERS dialogs, entering a complete directory can navigate into
# it on the first Enter rather than activating "Select Folder". Open/Save
# dialogs can likewise leave the target selected without committing it.
# Activate the standard IDOK button directly instead of relying on a second
# focus-sensitive Enter. PostMessage is intentionally asynchronous: a
# cross-process SendMessage waits inside the dialog's selection callback and
# can deadlock the helper until its watchdog kills it.
for ($attempt = 0; $attempt -lt 2 -and [CutNativeDialog]::IsWindow($window); $attempt++) {
  $okButton = [CutNativeDialog]::GetDlgItem($window, 1)
  if ($okButton -ne [IntPtr]::Zero) {
    $null = [CutNativeDialog]::PostMessage($okButton, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
  } else {
    Assert-ForegroundTarget
    [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
  }
  Start-Sleep -Milliseconds 450
}
