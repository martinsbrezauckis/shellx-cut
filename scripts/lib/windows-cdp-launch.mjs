import { spawnSync } from "node:child_process";

export const DEFAULT_CDP_PORT = 9223;

export function psSingleQuote(value) {
  return `'${String(value).replace(/'/g, "''")}'`;
}

export function normalizeCdpPort(value = DEFAULT_CDP_PORT) {
  const port = Number(value);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`Invalid CDP port: ${value}`);
  }
  return port;
}

export function buildInstalledCutCdpLaunchScript({
  installDir = "",
  cdpPort = DEFAULT_CDP_PORT,
  stopExisting = true,
  env = {},
} = {}) {
  const port = normalizeCdpPort(cdpPort);
  const rootLine = installDir
    ? `$root = ${psSingleQuote(installDir)}`
    : '$root = Join-Path $env:LOCALAPPDATA "ShellX Cut"';
  const stopLine = stopExisting
    ? "Get-Process shellx-cut,cutd -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue"
    : "# keeping existing ShellX Cut processes";
  const envLines = Object.entries(env)
    .filter(([name, value]) =>
      /^[A-Z0-9_]+$/.test(name) &&
      ![
        "SHELLX_CUT_WEBVIEW2_DEBUG_PORT",
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "WEBVIEW2_USER_DATA_FOLDER",
      ].includes(name) &&
      value != null &&
      String(value) !== "")
    .map(([name, value]) => `$env:${name} = ${psSingleQuote(value)}`)
    .join("\n");

  return `
$ErrorActionPreference = "Stop"
${rootLine}
$exe = Join-Path $root "shellx-cut.exe"
if (-not (Test-Path -LiteralPath $exe)) { throw "Installed ShellX Cut executable not found: $exe" }
${stopLine}
Start-Sleep -Milliseconds 500
$env:PATH = [Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [Environment]::GetEnvironmentVariable('PATH','User')
Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
Remove-Item Env:WEBVIEW2_USER_DATA_FOLDER -ErrorAction SilentlyContinue
${envLines}
$env:SHELLX_CUT_WEBVIEW2_DEBUG_PORT = ${psSingleQuote(port)}
$p = Start-Process -FilePath $exe -PassThru
Write-Host ("LAUNCHED_PID=" + $p.Id)
`.trim();
}

export function launchInstalledCutWithCdp(options = {}) {
  const script = buildInstalledCutCdpLaunchScript(options);
  return spawnSync(
    "powershell.exe",
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
    { encoding: "utf8" },
  );
}
