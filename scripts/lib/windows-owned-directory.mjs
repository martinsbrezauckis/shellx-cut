import { win32 } from 'node:path'

const LOCAL_DRIVE = /^[A-Za-z]:\\/

function quotedPowerShell(value) {
  return `'${String(value).replaceAll("'", "''")}'`
}

function localDrivePath(value, label) {
  const raw = String(value || '').trim()
  const normalized = win32.normalize(raw).replace(/[\\/]+$/, '')
  if (!LOCAL_DRIVE.test(normalized) || normalized.startsWith('\\\\')) {
    throw new Error(`${label} must be a Windows local drive path: ${raw || '(empty)'}`)
  }
  return normalized
}

export function assertWindowsOwnedDirectChild({ parentWin, targetWin, label = 'Windows owned directory' }) {
  const parent = localDrivePath(parentWin, `${label} parent`)
  const target = localDrivePath(targetWin, `${label} target`)
  if (win32.dirname(target).toLowerCase() !== parent.toLowerCase()) {
    throw new Error(`${label} must be an exact direct child of its owned parent`)
  }
  return { parent, target }
}

export function windowsOwnedPathPowerShell() {
  return [
    'function Assert-DirectOwnedChild([string]$parent,[string]$target){',
    '  $parentFull=[System.IO.Path]::GetFullPath($parent).TrimEnd("\\")',
    '  $targetFull=[System.IO.Path]::GetFullPath($target)',
    '  if(-not [string]::Equals([System.IO.Path]::GetDirectoryName($targetFull),$parentFull,[System.StringComparison]::OrdinalIgnoreCase)){throw "owned path is not a direct child: $targetFull"}',
    '}',
    'function Assert-NoReparseAncestors([string]$path){',
    '  $full=[System.IO.Path]::GetFullPath($path)',
    '  $root=[System.IO.Path]::GetPathRoot($full)',
    '  $cursor=$root',
    '  foreach($part in $full.Substring($root.Length).Split(@("\\"),[System.StringSplitOptions]::RemoveEmptyEntries)){',
    '    $cursor=[System.IO.Path]::Combine($cursor,$part)',
    '    $item=Get-Item -LiteralPath $cursor -Force -ErrorAction SilentlyContinue',
    '    if($null -eq $item){break}',
    '    if(($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned path contains a reparse ancestor: $cursor"}',
    '  }',
    '}',
    'function Assert-RealDirectory([string]$path){',
    '  $item=Get-Item -LiteralPath $path -Force -ErrorAction Stop',
    '  if(-not $item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned path is not a real directory: $path"}',
    '}',
    'function Assert-RealFile([string]$path){',
    '  $item=Get-Item -LiteralPath $path -Force -ErrorAction Stop',
    '  if($item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned path is not a real file: $path"}',
    '}',
    'function Test-MissingPathException([System.Exception]$error){',
    '  $cursor=$error',
    '  while($null -ne $cursor){',
    '    if($cursor -is [System.IO.FileNotFoundException] -or $cursor -is [System.IO.DirectoryNotFoundException]){return $true}',
    '    $cursor=$cursor.InnerException',
    '  }',
    '  return $false',
    '}',
    'function Remove-OwnedTreeTolerant([string]$path){',
    '  $entries=@()',
    '  try {$entries=@([System.IO.Directory]::EnumerateFileSystemEntries($path))} catch {if(Test-MissingPathException $_.Exception){return};throw}',
    '  foreach($entry in $entries){',
    '    try {',
    '      $attributes=[System.IO.File]::GetAttributes($entry)',
    '      if($attributes -band [System.IO.FileAttributes]::ReparsePoint){throw "owned directory contains a reparse entry: $entry"}',
    '      if($attributes -band [System.IO.FileAttributes]::Directory){',
    '        Remove-OwnedTreeTolerant $entry',
    '        try {[System.IO.Directory]::Delete($entry,$false)} catch {if(-not (Test-MissingPathException $_.Exception)){throw}}',
    '      } else {',
    '        try {[System.IO.File]::SetAttributes($entry,[System.IO.FileAttributes]::Normal)} catch {if(-not (Test-MissingPathException $_.Exception)){throw}}',
    '        try {[System.IO.File]::Delete($entry)} catch {if(-not (Test-MissingPathException $_.Exception)){throw}}',
    '      }',
    '    } catch {if(-not (Test-MissingPathException $_.Exception)){throw}}',
    '  }',
    '}',
    'function Remove-ExactOwnedDirectory([string]$parent,[string]$target){',
    '  Assert-DirectOwnedChild $parent $target',
    '  Assert-NoReparseAncestors $parent',
    '  Assert-RealDirectory $parent',
    '  Assert-NoReparseAncestors $target',
    '  for($attempt=1;$attempt -le 20;$attempt++){',
    '    $item=Get-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue',
    '    if($null -eq $item){break}',
    '    Assert-RealDirectory $target',
    '    Assert-NoReparseAncestors $parent',
    '    Assert-RealDirectory $parent',
    '    Assert-NoReparseAncestors $target',
    '    Assert-RealDirectory $target',
    '    try {',
    '      [System.IO.Directory]::Delete([System.IO.Path]::GetFullPath($target),$true)',
    '    } catch {',
    '      $remaining=Get-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue',
    '      if($null -eq $remaining){break}',
    '      if(Test-MissingPathException $_.Exception){',
    '        Remove-OwnedTreeTolerant $target',
    '        try {[System.IO.Directory]::Delete([System.IO.Path]::GetFullPath($target),$false)} catch {if(-not (Test-MissingPathException $_.Exception) -and -not ($_.Exception -is [System.IO.IOException])){throw}}',
    '        $remaining=Get-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue',
    '        if($null -eq $remaining){break}',
    '      }',
    '      if($attempt -eq 20){throw}',
    '      Start-Sleep -Milliseconds 250',
    '    }',
    '  }',
    '  $remaining=Get-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue',
    '  if($null -ne $remaining){throw "owned directory cleanup failed: $target"}',
    '  Assert-NoReparseAncestors $parent',
    '  Assert-RealDirectory $parent',
    '}',
  ].join(';')
}

export function removeWindowsOwnedDirectory({ parentWin, targetWin, captureSync, label }) {
  if (typeof captureSync !== 'function') throw new Error(`${label || 'Windows owned directory'} cleanup requires PowerShell capture`)
  const { parent, target } = assertWindowsOwnedDirectChild({ parentWin, targetWin, label })
  const script = [
    '$ErrorActionPreference="Stop"',
    `$parent=${quotedPowerShell(parent)}`,
    `$target=${quotedPowerShell(target)}`,
    windowsOwnedPathPowerShell(),
    'Remove-ExactOwnedDirectory $parent $target',
  ].join(';')
  captureSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script])
}
