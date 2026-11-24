# ShellX Cut uninstall cleanup: removes what the app itself created.
#
# The NSIS uninstaller embeds this file and runs it after it has removed the
# packaged files (installer-hooks.nsh, NSIS_HOOK_POSTUNINSTALL). It is a file,
# not an inline -Command: NSIS cuts every string at 1,024 characters, and the
# earlier inline command was longer, so PowerShell never ran it.
#
# Input (environment):
#   SHELLX_CUT_INSTALL_ROOT     the install folder ($INSTDIR)
#   SHELLX_CUT_DELETE_APP_DATA  1 when the person ticked "Delete the application data"
#
# Always removed (downloads and run files, in the install folder and in the
# per-user data folder %LOCALAPPDATA%\ShellX Cut when that is a different one,
# plus the engine startup log shellx-cut-engine.log in the current user's temporary folder,
# only when that folder has no link above it and the log is a plain file the user owns):
#   tools (FFmpeg, uv), perception\.venv, perception\python, perception\uv-cache,
#   perception\models, matte\matanyone, matte\rvm.onnx, work, engine.addr,
#   tools-doctor.json and bytecode caches beside the packaged Python helpers.
# Removed only with the box ticked: the rest of the data folder (settings) and,
# in %USERPROFILE%\.shellx-cut, the project list, the microphone choice, the remembered export
# folder (export-folder.json) and the notes about interrupted work (jobs), plus the
# private export staging (export-staging, in the data folder) and the notes that say which
# recording renders Cut made (recording-render-origins).
# Never removed: projects (and projects.damaged*.json, the old data kept by design), the Library copies and recordings in
# %USERPROFILE%\.shellx-cut, and any uv or Python outside these folders (earlier
# versions installed Captions' Python under %APPDATA%\uv, which other programs
# may share); the script says so in the uninstaller's details.
#
# A link (symlink or junction) is removed as a link; what it points to is never
# entered or deleted. Each line printed here appears in the uninstall details.

$ErrorActionPreference = 'Stop'
$script:cleanupCount = 0
$script:cleanupClock = [Diagnostics.Stopwatch]::StartNew()

function Write-Line([string]$text) { [Console]::Out.WriteLine($text) }

# nsExec streams these lines into the visible uninstall details.
function Write-RemovalProgress {
  $script:cleanupCount += 1
  if ($script:cleanupClock.ElapsedMilliseconds -ge 1000) {
    Write-Line ('Removing downloaded files... ' + $script:cleanupCount + ' items processed.')
    $script:cleanupClock.Restart()
  }
}

function Test-Link([string]$path) {
  $item = Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
  return ($item -and (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0))
}

# Long paths: the Python environments go deeper than 260 characters.
function Get-Long([string]$path) {
  if ($path.StartsWith('\\?\')) { return $path }
  if ($path.StartsWith('\\')) { return '\\?\UNC\' + $path.Substring(2) }
  return '\\?\' + $path
}

# Delete a file or folder tree without following links. Returns the number of
# items that could not be removed.
function Remove-Tree([string]$path) {
  $long = Get-Long $path
  $info = New-Object IO.DirectoryInfo($long)
  if (-not $info.Exists) {
    $file = New-Object IO.FileInfo($long)
    if (-not $file.Exists) { return 0 }
    try { $file.Attributes = 'Normal'; $file.Delete(); return 0 } catch { Write-Line ('Could not remove ' + $path + ': ' + $_.Exception.Message); return 1 }
  }
  if (($info.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    try { [IO.Directory]::Delete($long, $false); return 0 } catch { Write-Line ('Could not remove link ' + $path + ': ' + $_.Exception.Message); return 1 }
  }
  $failed = 0
  foreach ($child in $info.EnumerateFileSystemInfos()) {
    if (($child.Attributes -band [IO.FileAttributes]::Directory) -ne 0) {
      $failed += Remove-Tree $child.FullName
    } else {
      try { $child.Attributes = 'Normal'; $child.Delete() } catch { $failed += 1; if ($failed -le 3) { Write-Line ('Could not remove ' + $child.FullName.Replace('\\?\', '') + ': ' + $_.Exception.Message) } }
    }
    Write-RemovalProgress
  }
  try { [IO.Directory]::Delete($long, $false) } catch { $failed += 1 }
  return $failed
}

# True when no folder between $root and $root\$relative (exclusive) is a link.
function Test-SafeParents([string]$root, [string]$relative) {
  $parts = $relative.Split('\')
  $current = $root
  for ($n = 0; $n -lt $parts.Count - 1; $n++) {
    $current = Join-Path $current $parts[$n]
    if ((Test-Path -LiteralPath $current) -and (Test-Link $current)) { return $false }
  }
  return $true
}

function Remove-Owned([string]$root, [string]$relative) {
  $path = Join-Path $root $relative
  if (-not (Test-Path -LiteralPath $path)) { return }
  if (-not (Test-SafeParents $root $relative)) { Write-Line ('Kept (inside a linked folder): ' + $path); return }
  Write-Line ('Removing: ' + $path + ' (large folders can take several minutes)')
  $failed = Remove-Tree $path
  if ($failed -eq 0) { Write-Line ('Removed: ' + $path) } else { Write-Line ('Partly removed (' + $failed + ' items in use or protected): ' + $path) }
}

# True when the file or folder at $path is owned by the user running this script
# (compared by account id). An owner that cannot be read counts as not owned.
function Test-OwnedByCurrentUser([string]$path) {
  try {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $owner = (Get-Acl -LiteralPath $path).GetOwner([Security.Principal.SecurityIdentifier])
    if (-not $owner) { return $false }
    return [bool](($id.User -and $owner.Equals($id.User)) -or ($id.Owner -and $owner.Equals($id.Owner)))
  } catch { return $false }
}

# True when no folder above $full, up to the drive root, is a link. A folder
# that cannot be read counts as a link.
function Test-NoLinkAbove([string]$full) {
  $current = [IO.Path]::GetDirectoryName($full)
  while ($current) {
    $item = Get-Item -LiteralPath $current -Force -ErrorAction SilentlyContinue
    if (-not $item -or (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) { return $false }
    $current = [IO.Path]::GetDirectoryName($current)
  }
  return $true
}

function Get-Root([string]$path) {
  if (-not $path) { return $null }
  try { $full = [IO.Path]::GetFullPath($path).TrimEnd('\') } catch { return $null }
  if ($full.Length -lt 4 -or -not (Test-Path -LiteralPath $full -PathType Container)) { return $null }
  if (Test-Link $full) { Write-Line ('Kept (the folder is a link): ' + $full); return $null }
  return $full
}

if (-not $env:SHELLX_CUT_INSTALL_ROOT) { Write-Line 'ShellX Cut cleanup: no install folder given'; exit 3 }
try { $given = [IO.Path]::GetFullPath($env:SHELLX_CUT_INSTALL_ROOT).TrimEnd('\') } catch { $given = '' }
if ($given.Length -lt 4) { Write-Line ('ShellX Cut cleanup: refused install folder ' + $env:SHELLX_CUT_INSTALL_ROOT); exit 3 }
$installRoot = Get-Root $given
$deleteData = $env:SHELLX_CUT_DELETE_APP_DATA -eq '1'
$dataRoot = $null
if ($env:LOCALAPPDATA) { $dataRoot = Get-Root (Join-Path $env:LOCALAPPDATA 'ShellX Cut') }

$roots = @()
if ($installRoot) { $roots += $installRoot }
if ($dataRoot -and ($roots -notcontains $dataRoot)) { $roots += $dataRoot }

# Downloads and run files: removed on every uninstall.
$owned = @('tools', 'perception\.venv', 'perception\python', 'perception\uv-cache', 'perception\models',
           'perception\__pycache__', 'perception\judge\__pycache__', 'perception\judge\adapters\__pycache__',
           'matte\matanyone', 'matte\rvm.onnx', 'work', 'engine.addr', 'tools-doctor.json')
foreach ($root in $roots) {
  foreach ($relative in $owned) { Remove-Owned $root $relative }
}

# The desktop shell writes the engine's stderr startup log to the current
# user's temporary folder; remove only that log file on every uninstall. The
# folder must be the user's own (GetTempPath, or TEMP/TMP when they are that
# same folder or a folder this user owns), no folder above it may be a link,
# and the log itself must be a plain file this user owns. Anything that cannot
# be verified stays.
$tempRoots = @()
$userTemp = $null
try { $userTemp = Get-Root ([IO.Path]::GetTempPath()) } catch { }
foreach ($candidate in @($userTemp, $env:TEMP, $env:TMP)) {
  $tempRoot = Get-Root $candidate
  if (-not $tempRoot -or ($tempRoots -contains $tempRoot)) { continue }
  $sameFolder = $userTemp -and ($tempRoot -eq $userTemp)
  if (-not $sameFolder -and -not (Test-OwnedByCurrentUser $tempRoot)) { Write-Line ('Kept (not a temporary folder of this user): ' + $tempRoot); continue }
  if (-not (Test-NoLinkAbove $tempRoot)) { Write-Line ('Kept (a folder above the temporary folder is a link): ' + $tempRoot); continue }
  $tempRoots += $tempRoot
}
foreach ($tempRoot in $tempRoots) {
  $log = Join-Path $tempRoot 'shellx-cut-engine.log'
  if (-not (Test-Path -LiteralPath $log)) { continue }
  if (Test-Link $log) { Write-Line ('Kept (the log is a link): ' + $log); continue }
  if (-not (Test-OwnedByCurrentUser $log)) { Write-Line ('Kept (not created by this user): ' + $log); continue }
  Remove-Owned $tempRoot 'shellx-cut-engine.log'
}

if ($deleteData) {
  # Settings and everything else the app keeps in its data folder.
  # export-folder.json: a pre-release build wrote it here, beside the tools; it now lives in
  # %USERPROFILE%\.shellx-cut with the other state files.
  foreach ($relative in @('perception', 'matte', 'plugins.json', 'ffmpeg-override', 'export-folder.json', 'export-staging')) {
    foreach ($root in $roots) { Remove-Owned $root $relative }
  }
  if ($env:USERPROFILE) {
    $state = Join-Path $env:USERPROFILE '.shellx-cut'
    if ((Test-Path -LiteralPath $state) -and -not (Test-Link $state)) {
      # jobs\running holds the notes about work a quit interrupted (shown at the next start).
      foreach ($relative in @('projects.json', 'projects.last-good.json', 'projects.json.lock', 'microphone-preference.json', 'export-folder.json', 'jobs', 'recording-render-origins')) { Remove-Owned $state $relative }
      foreach ($relative in @('library', 'recordings')) {
        $kept = Join-Path $state $relative
        if (Test-Path -LiteralPath $kept) { Write-Line ('Kept (your media): ' + $kept) }
      }
      if (-not (Get-ChildItem -LiteralPath $state -Force -ErrorAction SilentlyContinue)) { [IO.Directory]::Delete($state); Write-Line ('Removed: ' + $state) }
    }
  }
}

# Folders the uninstaller does not own: say that they stay.
$documents = [Environment]::GetFolderPath('MyDocuments')
if ($documents) {
  $projects = Join-Path $documents 'ShellX Cut Projects'
  if (Test-Path -LiteralPath $projects) { Write-Line ('Kept (your projects): ' + $projects) }
}
$shared = @()
if ($env:APPDATA) { $shared += (Join-Path $env:APPDATA 'uv') }
if ($env:LOCALAPPDATA) { $shared += (Join-Path $env:LOCALAPPDATA 'uv') }
if ($env:USERPROFILE) { $shared += (Join-Path $env:USERPROFILE '.local\bin\python3.12.exe') }
foreach ($path in $shared) {
  if (Test-Path -LiteralPath $path) { Write-Line ('Kept (shared uv/Python, may be used by other programs; earlier ShellX Cut versions put Captions there): ' + $path) }
}
# Microsoft's Visual C++ runtime (installed system-wide for captions) belongs
# to Windows and other programs: never removed. Read the 64-bit registry view,
# since NSIS may start a 32-bit PowerShell.
try {
  $hklm = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
  foreach ($arch in @('x64', 'arm64')) {
    $key = $hklm.OpenSubKey('SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\' + $arch)
    if ($key -and $key.GetValue('Installed') -eq 1) {
      Write-Line ('Kept (Microsoft Visual C++ runtime ' + $key.GetValue('Version') + ', part of Windows and used by other programs)')
    }
  }
} catch { }
exit 0
