import { spawnSync } from "node:child_process";
import { win32 } from "node:path";

export function normalizeWebviewProfileToken(value) {
  const token = String(value || "");
  if (!/^[A-Za-z0-9_-]{1,80}$/.test(token)) {
    throw new Error(`Invalid WebView2 profile token: ${value}`);
  }
  return token;
}

export function buildWebviewProfileReleaseScript(value) {
  const token = normalizeWebviewProfileToken(value);
  return `
$ErrorActionPreference = "Stop"
$token = '${token}'
$graceDeadline = [DateTime]::UtcNow.AddSeconds(10)
do {
  $live = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" |
    Where-Object { $_.CommandLine -like ("*" + $token + "*") })
  if ($live.Count -eq 0) { break }
  Start-Sleep -Milliseconds 250
} while ([DateTime]::UtcNow -lt $graceDeadline)
$quietMilliseconds = 2000
$forceDeadline = [DateTime]::UtcNow.AddSeconds(15)
$quietSince = $null
do {
  $live = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" |
    Where-Object { $_.CommandLine -like ("*" + $token + "*") })
  if ($live.Count -gt 0) {
    $quietSince = $null
    $live | ForEach-Object {
      Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
    }
  } else {
    $now = [DateTime]::UtcNow
    if ($null -eq $quietSince) {
      $quietSince = $now
    } elseif (($now - $quietSince).TotalMilliseconds -ge $quietMilliseconds) {
      break
    }
  }
  Start-Sleep -Milliseconds 250
} while ([DateTime]::UtcNow -lt $forceDeadline)
$remaining = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" |
  Where-Object { $_.CommandLine -like ("*" + $token + "*") })
$quietElapsed = if ($null -eq $quietSince) { 0 } else { ([DateTime]::UtcNow - $quietSince).TotalMilliseconds }
if ($remaining.Count -gt 0 -or $quietElapsed -lt $quietMilliseconds) {
  throw "WebView2 profile processes did not stay quiescent for token $token"
}
Write-Host "WEBVIEW2_PROFILE_RELEASED=$token"
`.trim();
}

export function releaseWindowsWebviewProfile(value) {
  const script = buildWebviewProfileReleaseScript(value);
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    { encoding: "utf8" },
  );
  if (result.status !== 0) {
    throw new Error(`WebView2 profile release failed: ${result.stderr || result.stdout}`);
  }
  return result.stdout.trim();
}

export function buildWindowsQualificationProfileRemovalScript({
  localAppData,
  runtimeLocalAppData,
}) {
  const base = win32.normalize(String(localAppData || "")).replace(/[\\/]+$/, "");
  const target = win32.normalize(String(runtimeLocalAppData || "")).replace(/[\\/]+$/, "");
  const remainder = win32.relative(base, target);
  const parts = remainder.split("\\").filter(Boolean);
  if (
    !win32.isAbsolute(base) ||
    !win32.isAbsolute(target) ||
    remainder.startsWith("..") ||
    parts.length !== 3 ||
    parts[0] !== "ShellX Cut Qualification" ||
    !/^windows-installed-[A-Za-z0-9_-]{1,119}$/.test(parts[1]) ||
    parts[2] !== "local-app-data"
  ) {
    throw new Error(`Invalid Windows qualification profile root: ${runtimeLocalAppData}`);
  }
  const runRoot = win32.dirname(target);
  const literal = `'${runRoot.replaceAll("'", "''")}'`;
  return `
$ErrorActionPreference = "Stop"
$path = ${literal}
if (Test-Path -LiteralPath $path) {
  $item = Get-Item -LiteralPath $path -Force
  if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw "Qualification profile root must be a real directory: $path"
  }
  Remove-Item -LiteralPath $path -Recurse -Force
}
Write-Host "WINDOWS_QUALIFICATION_PROFILE_REMOVED=$path"
`.trim();
}

export function removeWindowsQualificationProfile(options) {
  const script = buildWindowsQualificationProfileRemovalScript(options);
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    { encoding: "utf8" },
  );
  if (result.status !== 0) {
    throw new Error(`Windows qualification profile cleanup failed: ${result.stderr || result.stdout}`);
  }
  return result.stdout.trim();
}
