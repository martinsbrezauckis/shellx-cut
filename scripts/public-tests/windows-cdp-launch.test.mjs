#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { buildInstalledCutCdpLaunchScript } from "../lib/windows-cdp-launch.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

const ps = buildInstalledCutCdpLaunchScript({
  installDir: "C:\\Users\\Example\\AppData\\Local\\ShellX Cut",
  cdpPort: 9333,
});
const psWithEnv = buildInstalledCutCdpLaunchScript({
  installDir: "C:\\Users\\Example\\AppData\\Local\\ShellX Cut",
  cdpPort: 9333,
  env: {
    CUTD_GENERATE_PROMPT_ADAPTER: "C:\\fixtures\\generate-prompt-adapter.py",
    CUTD_GENERATE_STORYBOARD_ADAPTER: "C:\\fixtures\\generate-storyboard-adapter.py",
  },
});
const psWithReservedOverride = buildInstalledCutCdpLaunchScript({
  cdpPort: 9333,
  env: {
    SHELLX_CUT_WEBVIEW2_DEBUG_PORT: "4444",
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--disable-web-security",
    WEBVIEW2_USER_DATA_FOLDER: "C:\\ambient-profile",
  },
});

assert.match(ps, /SHELLX_CUT_WEBVIEW2_DEBUG_PORT = '9333'/, "CDP launch must use the shell's narrow WebView2 debug-port opt-in");
assert.match(ps, /Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS/, "CDP launch must clear ambient generic WebView2 arguments");
assert.match(ps, /Remove-Item Env:WEBVIEW2_USER_DATA_FOLDER/, "CDP launch must clear ambient generic WebView2 profile overrides");
assert.match(ps, /shellx-cut\.exe/, "CDP launch must target the installed ShellX Cut executable");
assert.match(ps, /Get-Process shellx-cut,cutd/, "CDP launch must clean only ShellX Cut and cutd processes");
assert.match(ps, /\$env:PATH = \[Environment\]::GetEnvironmentVariable/, "CDP launch must refresh PATH for installed-app child tools");
assert.doesNotMatch(ps, /--remote-debugging-port/, "the launcher must pass a validated port, not arbitrary browser arguments");
assert.doesNotMatch(psWithReservedOverride, /4444|disable-web-security|ambient-profile/, "generic child environment entries must not override WebView2 launch controls");
assert.match(psWithReservedOverride, /SHELLX_CUT_WEBVIEW2_DEBUG_PORT = '9333'/, "the validated port remains authoritative");
assert.match(psWithEnv, /\$env:CUTD_GENERATE_PROMPT_ADAPTER = 'C:\\fixtures\\generate-prompt-adapter\.py'/, "CDP launch can inject Generate prompt adapter fixtures into the child environment");
assert.match(psWithEnv, /\$env:CUTD_GENERATE_STORYBOARD_ADAPTER = 'C:\\fixtures\\generate-storyboard-adapter\.py'/, "CDP launch can inject Generate storyboard adapter fixtures into the child environment");

const launcher = readFileSync(resolve(ROOT, "scripts/windows/launch-installed-cdp.mjs"), "utf8");
assert.match(launcher, /launchInstalledCutWithCdp/, "Windows launcher CLI must reuse the shared CDP launch helper");
assert.match(launcher, /--with-generate-fixtures/, "Windows launcher CLI must expose deterministic Generate fixtures for installed full-coverage runs");
assert.match(launcher, /CUTD_GENERATE_PROMPT_ADAPTER/, "Windows launcher CLI must set the Generate prompt adapter fixture");
assert.match(launcher, /CUTD_GENERATE_STORYBOARD_ADAPTER/, "Windows launcher CLI must set the Generate storyboard adapter fixture");
assert.match(launcher, /\/json\/list/, "Windows launcher CLI must wait for the CDP page list");
assert.match(launcher, /\/api\/verbs/, "Windows launcher CLI must wait for the installed cutd verb API");

console.log("PASS windows-cdp-launch");
