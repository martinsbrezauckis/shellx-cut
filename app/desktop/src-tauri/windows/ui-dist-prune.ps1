$ErrorActionPreference = 'Stop'

$raw = $env:SHELLX_CUT_INSTALL_ROOT
if ($raw -notmatch '^[A-Za-z]:\\') { throw 'Install root must be a local absolute path' }
$root = [IO.Path]::GetFullPath($raw).TrimEnd('\')
if ($root -match '^[A-Za-z]:$' -or
    -not [string]::Equals($raw.TrimEnd('\'), $root, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Install root must be canonical and below a local drive'
}
$target = [IO.Path]::Combine($root, 'ui-dist')
if (-not [string]::Equals([IO.Path]::GetDirectoryName($target), $root, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'UI path escaped install root'
}

$owner = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
function Assert-RealOwned([string]$path) {
  $item = Get-Item -LiteralPath $path -Force -ErrorAction Stop
  if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse path: $path" }
  if ((Get-Acl -LiteralPath $path -ErrorAction Stop).GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $owner) {
    throw "Foreign owner: $path"
  }
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
if (-not (Assert-RealOwned $target).PSIsContainer) { throw 'UI root is not a directory' }

$files = New-Object 'System.Collections.Generic.List[object]'
$directories = New-Object 'System.Collections.Generic.List[object]'
function Visit-Tree([string]$path, [string]$relative) {
  $item = Assert-RealOwned $path
  if ($item.PSIsContainer) {
    $directories.Add([pscustomobject]@{ path = $path; relative = $relative })
    foreach ($child in Get-ChildItem -LiteralPath $path -Force -ErrorAction Stop) {
      $childRelative = if ($relative) { "$relative/$($child.Name)" } else { $child.Name }
      Visit-Tree $child.FullName $childRelative
    }
  } else {
    $files.Add([pscustomobject]@{ path = $path; relative = $relative })
  }
}
Visit-Tree $target ''

$identityPath = [IO.Path]::Combine($target, '.shellx-cut-ui-identity.json')
if (-not ($files | Where-Object { $_.path -eq $identityPath })) { throw 'Packaged UI manifest is missing' }
$identity = Get-Content -LiteralPath $identityPath -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
if ($identity.schema -ne 'shellx-cut/ui-dist-identity@1' -or
    $null -eq $identity.dist -or $null -eq $identity.dist.files) { throw 'Packaged UI manifest is invalid' }
$expected = New-Object 'System.Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in $identity.dist.files) {
  $name = [string]$entry.path
  $sha = [string]$entry.sha256
  if ($name -notmatch '^[A-Za-z0-9._/-]+$' -or
      @($name.Split('/') | Where-Object { $_ -eq '' -or $_ -eq '.' -or $_ -eq '..' }).Count -ne 0 -or
      $name -eq '.shellx-cut-ui-identity.json' -or
      $sha -cnotmatch '^[a-f0-9]{64}$' -or $expected.ContainsKey($name)) {
    throw "Packaged UI manifest contains an unsafe or duplicate path: $name"
  }
  $expected.Add($name, $sha)
}
if (-not $expected.ContainsKey('index.html')) { throw 'Packaged UI manifest has no index.html' }
foreach ($entry in $expected.GetEnumerator()) {
  $file = @($files | Where-Object { [string]::Equals($_.relative, $entry.Key, [StringComparison]::OrdinalIgnoreCase) })
  if ($file.Count -ne 1) { throw "Packaged UI file missing: $($entry.Key)" }
  $actual = (Get-FileHash -LiteralPath $file[0].path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
  if ($actual -ne $entry.Value) { throw "Packaged UI file hash mismatch: $($entry.Key)" }
}

# The preflight above checks the whole tree before any old file is removed.
foreach ($file in $files) {
  if ($file.relative -eq '.shellx-cut-ui-identity.json' -or $expected.ContainsKey($file.relative)) { continue }
  $null = Assert-RealOwned $file.path
  Remove-Item -LiteralPath $file.path -Force -ErrorAction Stop
}
foreach ($directory in @($directories | Where-Object { $_.relative } | Sort-Object { $_.relative.Split('/').Count } -Descending)) {
  $null = Assert-RealOwned $directory.path
  if (@(Get-ChildItem -LiteralPath $directory.path -Force -ErrorAction Stop).Count -eq 0) {
    Remove-Item -LiteralPath $directory.path -Force -ErrorAction Stop
  }
}
