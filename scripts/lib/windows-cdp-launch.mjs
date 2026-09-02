import { spawnSync } from "node:child_process";
import { renderWindowsPowerShell51CanonicalLocalDrivePathPredicate } from "./windows-powershell51-local-drive-path.mjs";

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

export function normalizeWindowsSessionId(value = "") {
  const sessionId = String(value ?? "").trim();
  if (sessionId && !/^[1-9]\d*$/.test(sessionId)) {
    throw new Error(`Invalid Windows interactive session id: ${value}`);
  }
  return sessionId;
}

function normalizeWindowsCodexWrapperSeal(value = null) {
  if (value == null) return null;
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).sort().join("|") !== "desktopSession|path|sha256|user") {
    throw new Error("Invalid Windows Codex wrapper seal");
  }
  const path = String(value.path ?? "");
  const sha256 = String(value.sha256 ?? "").toLowerCase();
  const user = String(value.user ?? "");
  const desktopSession = normalizeWindowsSessionId(value.desktopSession);
  if (!/^[A-Za-z0-9._-]+\\[A-Za-z0-9._-]+$/.test(user) || !/^[a-f0-9]{64}$/.test(sha256)) {
    throw new Error("Invalid Windows Codex wrapper seal identity");
  }
  const username = user.split("\\")[1];
  const escapedUser = username.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (!new RegExp(`^[A-Za-z]:\\\\Users\\\\${escapedUser}\\\\AppData\\\\Roaming\\\\npm\\\\codex[.]cmd$`, "i").test(path)) {
    throw new Error("Invalid Windows Codex wrapper seal path");
  }
  return { path, sha256, user, desktopSession };
}

export function buildWindowsCodexWrapperSealScript({ seal, phase = "pre", includeCanonicalLocalDrivePathPredicate = true } = {}) {
  const normalized = normalizeWindowsCodexWrapperSeal(seal);
  if (!normalized || !["pre", "post"].includes(phase)) {
    throw new Error("Invalid Windows Codex wrapper seal phase");
  }
  const localDrivePathPredicate = includeCanonicalLocalDrivePathPredicate
    ? renderWindowsPowerShell51CanonicalLocalDrivePathPredicate()
    : "";
  const username = normalized.user.split("\\")[1];
  const marker = `CODEX_WRAPPER_SEAL_${phase.toUpperCase()}`;
  return `
$ErrorActionPreference = "Stop"
${localDrivePathPredicate}
$expectedPath = ${psSingleQuote(normalized.path)}
$expectedHash = ${psSingleQuote(normalized.sha256)}
$expectedUser = ${psSingleQuote(normalized.user)}
$expectedSession = ${psSingleQuote(normalized.desktopSession)}
$actualUser = [Security.Principal.WindowsIdentity]::GetCurrent().Name
if ($actualUser -ne $expectedUser) { throw "Codex wrapper seal native user drift: expected=$expectedUser actual=$actualUser" }
$actualSession = [string](Get-Process -Id $PID).SessionId
if ($actualSession -ne $expectedSession) { throw "Codex wrapper seal desktop-session drift: expected=$expectedSession actual=$actualSession" }
$nativeUserProfile = [Environment]::GetFolderPath('UserProfile')
if (-not (Test-CanonicalWindowsLocalDrivePath $nativeUserProfile)) { throw 'Codex wrapper seal could not resolve the native user profile' }
$canonicalPath = Join-Path $nativeUserProfile 'AppData\\Roaming\\npm\\codex.cmd'
if ($nativeUserProfile -notmatch ${psSingleQuote('\\\\Users\\\\' + username.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '$')}) { throw 'Codex wrapper seal native user profile does not match the grant' }
if ($expectedPath -ne $canonicalPath) { throw "Codex wrapper seal path drift: expected=$expectedPath actual=$canonicalPath" }
$item = Get-Item -Force -LiteralPath $canonicalPath -ErrorAction Stop
if ($item.PSIsContainer -or (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) { throw 'Codex wrapper seal requires a regular non-reparse-point file' }
$actualHash = (Get-FileHash -LiteralPath $canonicalPath -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash) { throw "Codex wrapper seal ${phase} hash drift: expected=$expectedHash actual=$actualHash" }
Write-Host (${psSingleQuote(marker)} + '=' + $actualHash)
`.trim();
}

export function verifyWindowsCodexWrapperSeal({ seal, phase = "post" } = {}) {
  const script = buildWindowsCodexWrapperSealScript({ seal, phase });
  return spawnSync(
    "powershell.exe",
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
    { encoding: "utf8" },
  );
}

// The post-use verification is deliberately in finally semantics: a red
// provider row is precisely when a wrapper replacement must not be hidden.
// Keep the matrix error as the primary exception and attach a seal failure as
// additional evidence instead of replacing the action failure.
export async function runWithPostUseWindowsCodexWrapperSeal({ seal = null, run, verify = verifyWindowsCodexWrapperSeal } = {}) {
  if (typeof run !== "function" || typeof verify !== "function") {
    throw new Error("Windows Codex wrapper post-use sealing requires run and verify functions");
  }
  let primaryError = null;
  let postUseError = null;
  try {
    await run();
  } catch (error) {
    primaryError = error;
  } finally {
    if (seal) {
      try {
        const result = await verify({ seal, phase: "post" });
        if (!result || result.status !== 0) {
          postUseError = new Error(`controller-authorized Codex wrapper changed during installed run: ${result?.stderr || result?.stdout || "post-use seal did not return success"}`);
        }
      } catch (error) {
        postUseError = error;
      }
    }
  }
  if (primaryError) {
    if (postUseError) {
      primaryError.postUseCodexWrapperSealError = postUseError;
      primaryError.message = `${primaryError.message}\nPost-use Codex wrapper seal failure: ${postUseError.message}`;
    }
    throw primaryError;
  }
  if (postUseError) throw postUseError;
}

export function buildInstalledCutCdpLaunchScript({
  installDir = "",
  cdpPort = DEFAULT_CDP_PORT,
  stopExisting = true,
  expectedSessionId = "",
  env = {},
  codexWrapperSeal = null,
} = {}) {
  const port = normalizeCdpPort(cdpPort);
  const sessionId = normalizeWindowsSessionId(expectedSessionId);
  const preLaunchCodexWrapperSeal = codexWrapperSeal
    ? buildWindowsCodexWrapperSealScript({ seal: codexWrapperSeal, phase: "pre", includeCanonicalLocalDrivePathPredicate: false })
    : "# no controller-authorized Codex cmd wrapper requires sealing";
  const localDrivePathPredicate = renderWindowsPowerShell51CanonicalLocalDrivePathPredicate();
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
        "HOME",
        "USERPROFILE",
      ].includes(name) &&
      value != null &&
      String(value) !== "")
    .map(([name, value]) => `$env:${name} = ${psSingleQuote(value)}`)
    .join("\n");

  return `
$ErrorActionPreference = "Stop"
${localDrivePathPredicate}
${rootLine}
$exe = Join-Path $root "shellx-cut.exe"
if (-not (Test-Path -LiteralPath $exe)) { throw "Installed ShellX Cut executable not found: $exe" }
${stopLine}
Start-Sleep -Milliseconds 500
$env:PATH = [Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [Environment]::GetEnvironmentVariable('PATH','User')
$nativeUserProfile = [Environment]::GetFolderPath('UserProfile')
if (-not (Test-CanonicalWindowsLocalDrivePath $nativeUserProfile)) { throw 'Windows CDP launch could not resolve the interactive native user profile' }
# A scheduled Windows task enters WSL before reaching this launcher. Never let
# its Linux HOME cross into the native shell/cutd child: provider discovery and
# the provider-owned canonical login routing must use the actual Windows profile.
$env:USERPROFILE = $nativeUserProfile
$env:HOME = $nativeUserProfile
Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
Remove-Item Env:WEBVIEW2_USER_DATA_FOLDER -ErrorAction SilentlyContinue
${envLines}
$env:SHELLX_CUT_WEBVIEW2_DEBUG_PORT = ${psSingleQuote(port)}
${preLaunchCodexWrapperSeal}
$p = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru
Start-Sleep -Milliseconds 250
$started = Get-Process -Id $p.Id -ErrorAction Stop
if ($started.Path -ne $exe) { throw "Installed ShellX Cut launch resolved a different executable: $($started.Path)" }
${sessionId ? `if ([string]$started.SessionId -ne ${psSingleQuote(sessionId)}) { throw "Installed ShellX Cut launch escaped its required interactive session: expected=${sessionId} actual=$($started.SessionId)" }` : "# no interactive-session assertion requested"}
Write-Host ("LAUNCHED_PID=" + $p.Id)
Write-Host ("LAUNCHED_SESSION=" + $started.SessionId)
Write-Host ("LAUNCHED_EXE=" + $started.Path)
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
