// This renderer deliberately uses only Windows PowerShell 5.1 and .NET
// Framework APIs. Keep it small: it protects the native profile handoff that
// the installed Windows harness uses before it can start the application.
export function renderWindowsPowerShell51CanonicalLocalDrivePathPredicate() {
  return [
    'function Test-CanonicalWindowsLocalDrivePath([string]$path){',
    '  if([string]::IsNullOrWhiteSpace($path) -or $path -cne $path.Trim()){return $false}',
    "  if($path -notmatch '^[A-Za-z]:\\\\'){return $false}",
    '  $tail=$path.Substring(3)',
    '  if($tail.Length -eq 0){return $false}',
    "  foreach($segment in $tail.Split([char]'\\')){",
    '    if([string]::IsNullOrWhiteSpace($segment) -or $segment -eq \'.\' -or $segment -eq \'..\'){return $false}',
    "    if($segment -match '[:*?\"<>|/]'){return $false}",
    "    if($segment -match '[. ]$'){return $false}",
    "    if($segment -match '(?i)~[0-9]+(?:[.][^\\\\]+)?$'){return $false}",
    '  }',
    '  try {',
    '    $canonical=[System.IO.Path]::GetFullPath($path)',
    '    if(-not [string]::Equals($canonical,$path,[System.StringComparison]::OrdinalIgnoreCase)){return $false}',
    '    $drive=New-Object -TypeName System.IO.DriveInfo -ArgumentList ($path.Substring(0,2))',
    '    return $drive.DriveType -eq [System.IO.DriveType]::Fixed',
    '  } catch {return $false}',
    '}',
  ].join('\n')
}
