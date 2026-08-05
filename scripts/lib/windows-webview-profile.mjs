import { spawnSync } from "node:child_process";

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
$deadline = [DateTime]::UtcNow.AddSeconds(10)
do {
  $live = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" |
    Where-Object { $_.CommandLine -like ("*" + $token + "*") })
  if ($live.Count -eq 0) { break }
  Start-Sleep -Milliseconds 250
} while ([DateTime]::UtcNow -lt $deadline)
if ($live.Count -gt 0) {
  $live | ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Milliseconds 500
}
$remaining = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" |
  Where-Object { $_.CommandLine -like ("*" + $token + "*") })
if ($remaining.Count -gt 0) {
  throw "WebView2 profile processes did not exit for token $token"
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
