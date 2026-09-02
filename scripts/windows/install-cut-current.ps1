# ShellX Cut Windows install smoke helper.
#
# This is a local qualification harness for a built NSIS setup executable.
# It is not the end-user installer. The user installer remains:
#   ShellX Cut_<version>_x64-setup.exe

[CmdletBinding()]
param(
  [string]$SetupPath,
  [string]$ExpectedVersion = "",
  [int]$SmokePort = 6191,
  [switch]$SkipCleanInstall,
  [switch]$AllowUnsignedSmoke
)

$ErrorActionPreference = "Stop"

function Resolve-RepoRoot {
  return [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
}

function Resolve-SetupPath {
  param([string]$Candidate)

  if (-not [string]::IsNullOrWhiteSpace($Candidate)) {
    $resolved = Resolve-Path -LiteralPath $Candidate -ErrorAction Stop
    $item = Get-Item -LiteralPath $resolved.Path -ErrorAction Stop
    if ($item.PSIsContainer) { throw "SetupPath points to a directory: $($item.FullName)" }
    return $item.FullName
  }

  $repo = Resolve-RepoRoot
  $bundleDir = Join-Path $repo "app\desktop\src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis"
  $matches = @(Get-ChildItem -LiteralPath $bundleDir -Filter "ShellX Cut_*_x64-setup.exe" -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTimeUtc -Descending)
  if ($matches.Count -eq 0) {
    throw "No ShellX Cut NSIS setup executable found. Pass -SetupPath explicitly."
  }
  return $matches[0].FullName
}

function Resolve-ExpectedVersion {
  param([string]$Candidate)

  if (-not [string]::IsNullOrWhiteSpace($Candidate)) {
    return $Candidate
  }

  $repo = Resolve-RepoRoot
  $confPath = Join-Path $repo "app\desktop\src-tauri\tauri.conf.json"
  if (-not (Test-Path -LiteralPath $confPath)) {
    throw "Could not find Tauri config for expected version: $confPath"
  }
  $conf = Get-Content -Raw -LiteralPath $confPath | ConvertFrom-Json
  $version = [string]$conf.version
  if ([string]::IsNullOrWhiteSpace($version)) {
    throw "Could not read version from Tauri config: $confPath"
  }
  return $version
}

function Invoke-Native {
  param(
    [Parameter(Mandatory = $true)][string]$FilePath,
    [string[]]$Arguments = @()
  )

  $process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -Wait -PassThru -WindowStyle Hidden
  if ($null -eq $process) {
    throw "Command did not start: $FilePath $($Arguments -join ' ')"
  }
  if ($process.ExitCode -ne 0) {
    throw "Command failed with exit code $($process.ExitCode): $FilePath $($Arguments -join ' ')"
  }
}

function Stop-AppProcesses {
  foreach ($name in @("shellx-cut", "cutd")) {
    Get-Process -Name $name -ErrorAction SilentlyContinue |
      Stop-Process -Force -ErrorAction SilentlyContinue
  }
}

function Get-OwnedDescendantProcessIds {
  param([Parameter(Mandatory = $true)][int]$RootProcessId)

  $seen = @{}
  $seen[$RootProcessId] = $true
  $frontier = @($RootProcessId)
  $descendants = New-Object System.Collections.Generic.List[int]

  for ($depth = 0; $depth -lt 32 -and $frontier.Count -gt 0; $depth++) {
    $next = New-Object System.Collections.Generic.List[int]
    foreach ($parentProcessId in $frontier) {
      $children = @(Get-CimInstance Win32_Process -Filter ("ParentProcessId = {0}" -f $parentProcessId) -ErrorAction SilentlyContinue)
      foreach ($child in $children) {
        $childProcessId = [int]$child.ProcessId
        if ($childProcessId -le 0 -or $seen.ContainsKey($childProcessId)) { continue }
        $seen[$childProcessId] = $true
        $descendants.Add($childProcessId)
        $next.Add($childProcessId)
      }
    }
    $frontier = @($next.ToArray())
  }

  return @($descendants.ToArray())
}

function Stop-OwnedProcessTree {
  param([Parameter(Mandatory = $true)][System.Diagnostics.Process]$RootProcess)

  $rootProcessId = [int]$RootProcess.Id
  $descendantProcessIds = @(Get-OwnedDescendantProcessIds -RootProcessId $rootProcessId)
  try { $RootProcess.Refresh() } catch { }

  if (-not $RootProcess.HasExited) {
    $taskkill = Join-Path $env:SystemRoot "System32\taskkill.exe"
    # taskkill can report an already-exited descendant on stderr even after it
    # successfully stopped the root. Capture that diagnostic without allowing
    # the script-wide Stop preference to preempt the bounded survivor check.
    $previousErrorActionPreference = $ErrorActionPreference
    try {
      $ErrorActionPreference = "Continue"
      $taskkillOutput = (& $taskkill /PID ([string]$rootProcessId) /T /F 2>&1 | Out-String).Trim()
      $taskkillExitCode = $LASTEXITCODE
    } finally {
      $ErrorActionPreference = $previousErrorActionPreference
    }
    try { $RootProcess.Refresh() } catch { }
    if ($taskkillExitCode -ne 0 -and -not $RootProcess.HasExited) {
      throw "Could not stop installed smoke process tree rooted at PID ${rootProcessId}: $taskkillOutput"
    }
  }

  # If the root exited between discovery and taskkill, Windows can leave its
  # already-discovered import checks alive. Stop those exact owned descendants
  # leaves-first as a bounded fallback.
  for ($index = $descendantProcessIds.Count - 1; $index -ge 0; $index--) {
    Stop-Process -Id $descendantProcessIds[$index] -Force -ErrorAction SilentlyContinue
  }

  $ownedProcessIds = @($rootProcessId) + $descendantProcessIds
  $survivors = @()
  for ($attempt = 0; $attempt -lt 50; $attempt++) {
    $survivors = @($ownedProcessIds | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
    if ($survivors.Count -eq 0) { break }
    Start-Sleep -Milliseconds 100
  }
  if ($survivors.Count -gt 0) {
    throw "Installed smoke process tree did not stop: $($survivors -join ', ')"
  }

  Write-Host ("SMOKE_CUTD_TREE_STOPPED: root={0} descendants={1}" -f $rootProcessId, $descendantProcessIds.Count)
}

function Move-IfExists {
  param([string]$Source, [string]$Destination)

  if (Test-Path -LiteralPath $Source) {
    if (Test-Path -LiteralPath $Destination) {
      Remove-Item -LiteralPath $Destination -Recurse -Force
    }
    Move-Item -LiteralPath $Source -Destination $Destination -Force
  }
}

function Restore-IfExists {
  param([string]$Stash, [string]$Destination)

  if (Test-Path -LiteralPath $Stash) {
    $parent = Split-Path -Parent $Destination
    if (-not (Test-Path -LiteralPath $parent)) {
      New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    if (Test-Path -LiteralPath $Destination) {
      Remove-Item -LiteralPath $Destination -Recurse -Force
    }
    Move-Item -LiteralPath $Stash -Destination $Destination -Force
  }
}

if ($SmokePort -lt 1024 -or $SmokePort -gt 65535) {
  throw "SmokePort must be between 1024 and 65535."
}

$setup = Resolve-SetupPath -Candidate $SetupPath
$expected = Resolve-ExpectedVersion -Candidate $ExpectedVersion
$installRoot = Join-Path $env:LOCALAPPDATA "ShellX Cut"
$webViewRoot = Join-Path $env:LOCALAPPDATA "lv.shellx.cut"
$shellExe = Join-Path $installRoot "shellx-cut.exe"
$cutdExe = Join-Path $installRoot "cutd.exe"
$uninstallExe = Join-Path $installRoot "uninstall.exe"
$agentDocsRoot = Join-Path $installRoot "agent-docs"
$venv = Join-Path $installRoot "perception\.venv"
$venvStash = Join-Path $env:LOCALAPPDATA "ShellX Cut perception venv"
$sttSettings = Join-Path $installRoot "perception\stt.json"
$sttSettingsStash = Join-Path $env:LOCALAPPDATA "ShellX Cut perception stt.json"
$tools = Join-Path $installRoot "tools"
$toolsStash = Join-Path $env:LOCALAPPDATA "ShellX Cut tools"
$matte = Join-Path $installRoot "matte"
$matteStash = Join-Path $env:LOCALAPPDATA "ShellX Cut matte"
$plugins = Join-Path $installRoot "plugins.json"
$pluginsStash = Join-Path $env:LOCALAPPDATA "ShellX Cut plugins.json"

Write-Host "SETUP: $setup"
Write-Host "EXPECTED_VERSION: $expected"
Write-Host ("SETUP_SHA256: " + (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant())

Stop-AppProcesses
Start-Sleep -Seconds 2

if (-not $SkipCleanInstall) {
  Move-IfExists -Source $venv -Destination $venvStash
  Move-IfExists -Source $sttSettings -Destination $sttSettingsStash
  Move-IfExists -Source $tools -Destination $toolsStash
  Move-IfExists -Source $matte -Destination $matteStash
  Move-IfExists -Source $plugins -Destination $pluginsStash

  if (Test-Path -LiteralPath $uninstallExe) {
    Write-Host "UNINSTALLING_OLD"
    Invoke-Native -FilePath $uninstallExe -Arguments @("/S")
    Start-Sleep -Seconds 2
  }

  if (Test-Path -LiteralPath $installRoot) {
    Remove-Item -LiteralPath $installRoot -Recurse -Force
  }
  if (Test-Path -LiteralPath $webViewRoot) {
    Remove-Item -LiteralPath $webViewRoot -Recurse -Force
  }
}

Write-Host "INSTALLING"
try {
  Invoke-Native -FilePath $setup -Arguments @("/S")
  Start-Sleep -Seconds 3
} finally {
  Restore-IfExists -Stash $venvStash -Destination $venv
  Restore-IfExists -Stash $sttSettingsStash -Destination $sttSettings
  Restore-IfExists -Stash $toolsStash -Destination $tools
  Restore-IfExists -Stash $matteStash -Destination $matte
  Restore-IfExists -Stash $pluginsStash -Destination $plugins
}

if (-not (Test-Path -LiteralPath $shellExe)) { throw "Installed shell missing: $shellExe" }
if (-not (Test-Path -LiteralPath $cutdExe)) { throw "Installed engine missing: $cutdExe" }
if (-not (Test-Path -LiteralPath (Join-Path $installRoot "perception\instruments.py"))) {
  throw "Installed perception sidecar missing."
}

$repoRoot = Resolve-RepoRoot
$agentDocManifest = Join-Path $repoRoot "scripts\lib\agent-docs.mjs"
$node = (Get-Command node -ErrorAction Stop).Source
$agentDocPaths = @(& $node $agentDocManifest --paths)
if ($LASTEXITCODE -ne 0 -or $agentDocPaths.Count -eq 0) {
  throw "Could not read the canonical installed agent-doc manifest: $agentDocManifest"
}
foreach ($relative in $agentDocPaths) {
  $source = Join-Path $repoRoot $relative
  $installed = Join-Path $agentDocsRoot $relative
  if (-not (Test-Path -LiteralPath $installed -PathType Leaf)) {
    throw "Installed agent doc missing: $relative"
  }
  $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
  $installedHash = (Get-FileHash -LiteralPath $installed -Algorithm SHA256).Hash
  if ($sourceHash -ne $installedHash) {
    throw "Installed agent doc differs from candidate source: $relative"
  }
}
Write-Host ("AGENT_DOC_FILES_EXACT: " + $agentDocPaths.Count)

$versionInfo = (Get-Item -LiteralPath $shellExe).VersionInfo
Write-Host ("PRODUCT_VERSION: " + $versionInfo.ProductVersion)
if ($versionInfo.ProductVersion -notmatch ("^" + [regex]::Escape($expected))) {
  throw "Installed version is $($versionInfo.ProductVersion), expected $expected"
}

$sigShell = Get-AuthenticodeSignature -LiteralPath $shellExe
$sigCutd = Get-AuthenticodeSignature -LiteralPath $cutdExe
Write-Host ("SIG_SHELL: " + $sigShell.Status)
Write-Host ("SIG_CUTD: " + $sigCutd.Status)
if (-not $AllowUnsignedSmoke -and ($sigShell.Status -ne "Valid" -or $sigCutd.Status -ne "Valid")) {
  throw "Installed shell or engine signature is not valid."
} elseif ($AllowUnsignedSmoke -and ($sigShell.Status -ne "Valid" -or $sigCutd.Status -ne "Valid")) {
  Write-Host "SIG_UNSIGNED_ALLOWED: true"
}

$cutdVersion = (& $cutdExe --version 2>&1 | Out-String).Trim()
Write-Host ("CUTD_VERSION: " + $cutdVersion)
if ($cutdVersion -notmatch [regex]::Escape($expected)) {
  throw "cutd version is '$cutdVersion', expected $expected"
}

$engine = $null
$previousAgentDocsDir = [Environment]::GetEnvironmentVariable("SHELLX_CUT_AGENT_DOCS_DIR", "Process")
try {
  $env:SHELLX_CUT_AGENT_DOCS_DIR = $agentDocsRoot
  $engine = Start-Process -FilePath $cutdExe -ArgumentList @("serve", "--headless", "--addr", "127.0.0.1:$SmokePort") -PassThru -WindowStyle Hidden
  if ($null -eq $engine) {
    throw "Could not start installed engine smoke process."
  }
  Write-Host ("SMOKE_CUTD_PID: " + $engine.Id)

  $verbs = $null
  for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 1
    try {
      $response = Invoke-WebRequest -Uri "http://127.0.0.1:$SmokePort/api/verbs" -UseBasicParsing -TimeoutSec 5
      if ($response.StatusCode -eq 200) {
        $verbs = $response.Content
        break
      }
    } catch {
      $verbs = $null
    }
  }

  if (-not $verbs) { throw "cutd did not answer /api/verbs on port $SmokePort" }
  $verbObject = $verbs | ConvertFrom-Json
  $count = if ($verbObject -is [array]) { $verbObject.Count } elseif ($verbObject.verbs) { $verbObject.verbs.Count } else { 0 }
  Write-Host ("VERB_COUNT: " + $count)
  if ($verbs -notmatch "audio\.dub") { throw "audio.dub missing from installed verb registry." }

  $agentResponse = Invoke-WebRequest -Uri "http://127.0.0.1:$SmokePort/api/agent" -UseBasicParsing -TimeoutSec 5
  $agent = $agentResponse.Content | ConvertFrom-Json
  if ($agent.schema -ne "shellx-cut/agent-docs/2" -or -not $agent.docs_available) {
    throw "Installed /api/agent does not report bundled docs available."
  }
  if ([string]$agent.version -ne $expected) {
    throw "Installed /api/agent version is $($agent.version), expected $expected"
  }
  foreach ($relative in $agentDocPaths) {
    $encoded = ($relative -split "/" | ForEach-Object { [Uri]::EscapeDataString($_) }) -join "/"
    $apiCopy = [System.IO.Path]::GetTempFileName()
    try {
      $docResponse = Invoke-WebRequest -Uri "http://127.0.0.1:$SmokePort/api/agent-doc/$encoded" -UseBasicParsing -TimeoutSec 5 -OutFile $apiCopy -PassThru
      if ($docResponse.StatusCode -ne 200) { throw "Installed agent-doc endpoint failed: $relative" }
      $sourceHash = (Get-FileHash -LiteralPath (Join-Path $repoRoot $relative) -Algorithm SHA256).Hash
      $servedHash = (Get-FileHash -LiteralPath $apiCopy -Algorithm SHA256).Hash
      if ($sourceHash -ne $servedHash) {
        throw "Installed agent-doc endpoint differs from candidate source: $relative"
      }
    } finally {
      Remove-Item -LiteralPath $apiCopy -Force -ErrorAction SilentlyContinue
    }
  }
  Write-Host ("AGENT_DOC_API_EXACT: " + $agentDocPaths.Count)
} finally {
  if ($engine) {
    Stop-OwnedProcessTree -RootProcess $engine
  }
  if ($null -eq $previousAgentDocsDir) {
    Remove-Item Env:SHELLX_CUT_AGENT_DOCS_DIR -ErrorAction SilentlyContinue
  } else {
    $env:SHELLX_CUT_AGENT_DOCS_DIR = $previousAgentDocsDir
  }
}

Write-Host "INSTALL_SMOKE_PASS"
