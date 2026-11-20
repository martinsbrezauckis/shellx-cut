$ErrorActionPreference = 'Stop'

# Exact retired installer resources. Current resource maps contain none of
# these paths; modified or unrelated files must remain untouched.
$obsolete = @(
  @{ path = 'perception/requirements-full.linux-x86_64.lock'; bytes = 213880; sha256 = 'd0ca7d0b813b04c67f0b687c1c9d523af7567a981e4101f5518eedd42198c246' }
  @{ path = 'perception/requirements-full.macos-aarch64.lock'; bytes = 204842; sha256 = '3b40276bb35c9e62a237a0366fb7e7d94699d0ade18e990a3210159858274cc8' }
  @{ path = 'perception/requirements-full.windows-x86_64.lock'; bytes = 214239; sha256 = 'f60ce6ea50db3ceafc408c9be808e2dd7d4b26b76524b077749f8f0ac666812f' }
  @{ path = 'perception/judge/__pycache__/judge.cpython-312.pyc'; bytes = 39931; sha256 = 'e63fe31506e6d2aa9453a309c59cca33e866026a528229bdc23d079562ffd945' }
  @{ path = 'perception/judge/adapters/__pycache__/antigravity_judge.cpython-312.pyc'; bytes = 20714; sha256 = '7f8ee486824b5fc2215024b81fa549d310d566208be90d3c304ed6593b85f45b' }
  @{ path = 'perception/judge/adapters/__pycache__/cli_judge.cpython-312.pyc'; bytes = 38091; sha256 = '8c025c415db1e4dee9a21094eece838a2cbae871a9eadd5a5f76eb8a0b3ae729' }
  @{ path = 'perception/judge/adapters/__pycache__/codex_judge.cpython-312.pyc'; bytes = 24770; sha256 = '484fa8f802f6124a9fcf66b162835820fe4d236bb487c0c74ccfba71566bd5af' }
  @{ path = 'perception/judge/adapters/__pycache__/diagnostics.cpython-312.pyc'; bytes = 2318; sha256 = '21b950b99bbd795e5506d1cacdebb8951f7e3c55e8614efdb07c85942d0aac0d' }
  @{ path = 'perception/judge/adapters/__pycache__/grok_judge.cpython-312.pyc'; bytes = 25046; sha256 = '15a655556ca10dbf80d64a25f87fa08637e8522a8ce3dd166d22f23e7f48dd57' }
  @{ path = 'perception/judge/adapters/__pycache__/ladder_judge.cpython-312.pyc'; bytes = 16449; sha256 = '7dcc7e2c60e2354a158b62f4975155b619eb91883653f5c291e5287544fcf55c' }
)

$raw = $env:SHELLX_CUT_INSTALL_ROOT
if ($raw -notmatch '^[A-Za-z]:\\') { throw 'Install root must be a local absolute path' }
$root = [IO.Path]::GetFullPath($raw).TrimEnd('\')
if ($root -match '^[A-Za-z]:$' -or
    -not [string]::Equals($raw.TrimEnd('\'), $root, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Install root must be canonical and below a local drive'
}
$owner = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$administrators = 'S-1-5-32-544'
$localAppData = [IO.Path]::GetFullPath([Environment]::GetFolderPath('LocalApplicationData')).TrimEnd('\')
$underLocalAppData = $root.StartsWith($localAppData + '\', [StringComparison]::OrdinalIgnoreCase)

function Assert-RealOwned([string]$path) {
  $item = Get-Item -LiteralPath $path -Force -ErrorAction Stop
  if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse path: $path" }
  $acl = Get-Acl -LiteralPath $path -ErrorAction Stop
  $actualOwner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
  if ($actualOwner -eq $owner) { return $item }
  if ($actualOwner -ne $administrators -or -not $underLocalAppData) { throw "Foreign owner: $path" }
  $hasFullControl = @($acl.Access | Where-Object {
    if ($_.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) { return $false }
    try { $sid = $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { return $false }
    $sid -eq $owner -and (($_.FileSystemRights -band [Security.AccessControl.FileSystemRights]::FullControl) -eq [Security.AccessControl.FileSystemRights]::FullControl)
  }).Count -gt 0
  if (-not $hasFullControl) { throw "Foreign owner: $path" }
  return $item
}

$cursor = $root
while ($cursor) {
  $node = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
  if (($node.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse ancestor: $cursor" }
  $parent = [IO.Path]::GetDirectoryName($cursor)
  if (-not $parent -or $parent -eq $cursor) { break }
  $cursor = $parent
}
if (-not (Assert-RealOwned $root).PSIsContainer) { throw 'Install root is not a directory' }

function Inspect-Obsolete($entry) {
  $parts = $entry.path.Split('/')
  $path = $root
  for ($index = 0; $index -lt $parts.Count; $index++) {
    $path = [IO.Path]::Combine($path, $parts[$index])
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    $item = Assert-RealOwned $path
    if ($index -lt $parts.Count - 1 -and -not $item.PSIsContainer) { throw "Non-directory parent: $path" }
  }
  if ($item -isnot [IO.FileInfo] -or $item.PSIsContainer) { throw "Not a regular file: $path" }
  if ($item.Length -ne $entry.bytes) { return $null }
  $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256 -ErrorAction Stop).Hash
  if (-not [string]::Equals($hash, $entry.sha256, [StringComparison]::OrdinalIgnoreCase)) { return $null }
  return $path
}

# Inspect every literal candidate before the first unlink. Absent or changed
# files are intentionally retained; unsafe paths stop the installer cleanup.
$approved = New-Object 'System.Collections.Generic.List[object]'
foreach ($entry in $obsolete) {
  $path = Inspect-Obsolete $entry
  if ($path) { $approved.Add([pscustomobject]@{ path = $path; entry = $entry }) }
  else {
    $literal = [IO.Path]::Combine($root, $entry.path.Replace('/', '\'))
    if (Test-Path -LiteralPath $literal) { Write-Output "Preserved changed obsolete file: $($entry.path)" }
    else { Write-Output "Absent obsolete file: $($entry.path)" }
  }
}
foreach ($candidate in $approved) {
  if (-not [string]::Equals((Inspect-Obsolete $candidate.entry), $candidate.path, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Obsolete file changed during cleanup: $($candidate.path)"
  }
  Remove-Item -LiteralPath $candidate.path -Force -ErrorAction Stop
  Write-Output "Removed exact obsolete file: $($candidate.entry.path)"
}

# These are the only directory names the retired cache files could empty.
foreach ($relative in @('perception/judge/adapters/__pycache__', 'perception/judge/__pycache__')) {
  $parts = $relative.Split('/')
  $path = $root
  foreach ($part in $parts) {
    $path = [IO.Path]::Combine($path, $part)
    if (-not (Test-Path -LiteralPath $path)) { break }
    $null = Assert-RealOwned $path
  }
  if ((Test-Path -LiteralPath $path) -and
      (Assert-RealOwned $path).PSIsContainer -and
      @(Get-ChildItem -LiteralPath $path -Force -ErrorAction Stop).Count -eq 0) {
    Remove-Item -LiteralPath $path -Force -ErrorAction Stop
  }
}
