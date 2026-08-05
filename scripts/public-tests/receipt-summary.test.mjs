#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { withBooleanReceiptSummary } from "../lib/receipt-summary.mjs";

const receipt = withBooleanReceiptSummary(
  {
    cdp: "http://127.0.0.1:9223",
    results: [
      { name: "first", ok: true },
      { name: "second", ok: false },
    ],
  },
  {
    version: "0.6.105",
    generatedAt: "2026-06-30T00:00:00.000Z",
  },
);

assert.equal(receipt.ok, false);
assert.equal(receipt.version, "0.6.105");
assert.equal(receipt.generatedAt, "2026-06-30T00:00:00.000Z");
assert.deepEqual(receipt.summary, { total: 2, pass: 1, fail: 1 });

const allPass = withBooleanReceiptSummary(
  { results: [{ name: "first", ok: true }] },
  { version: "0.6.105", generatedAt: "2026-06-30T00:00:00.000Z" },
);
assert.equal(allPass.ok, true);
assert.deepEqual(allPass.summary, { total: 1, pass: 1, fail: 0 });

const harness = readFileSync(resolve("scripts/windows/cdp-cut-verify-0650-uiux.mjs"), "utf8");
assert.match(harness, /withBooleanReceiptSummary/, "Windows UI/UX harness writes a top-level ok receipt envelope");
assert.doesNotMatch(
  harness,
  /CUT_EXPECTED_VERSION\s*\|\|\s*['"]0\.6\.\d+['"]/,
  "Windows UI/UX harness must derive its default expected version from repo metadata, not a stale hardcoded release",
);
assert.match(
  harness,
  /tauri\.conf\.json/,
  "Windows UI/UX harness must read the current app version from Tauri metadata when CUT_EXPECTED_VERSION is unset",
);
assert.match(
  harness,
  /windowsEnvironmentPath\('USERPROFILE'/,
  "Windows UI/UX harness must resolve the native user profile instead of publishing a workstation-specific path",
);
assert.match(
  harness,
  /CUT_WINDOWS_TEMP/,
  "Windows UI/UX harness must resolve or accept the native temporary directory",
);
assert.doesNotMatch(
  harness,
  /const MEDIA\s*=.*C:\\\\Users/,
  "Windows UI/UX harness must not embed a workstation-specific media path",
);

const installSmoke = readFileSync(resolve("scripts/windows/install-cut-current.ps1"), "utf8");
assert.doesNotMatch(
  installSmoke,
  /\[string\]\$ExpectedVersion\s*=\s*['"]0\.6\.\d+['"]/,
  "Windows install smoke must not default ExpectedVersion to a stale hardcoded release",
);
assert.match(
  installSmoke,
  /tauri\.conf\.json/,
  "Windows install smoke must read the current app version from Tauri metadata when -ExpectedVersion is omitted",
);
assert.match(
  installSmoke,
  /\[switch\]\$AllowUnsignedSmoke/,
  "Windows install smoke exposes an explicit unsigned local-package override",
);
assert.match(
  installSmoke,
  /SIG_UNSIGNED_ALLOWED/,
  "Windows install smoke visibly records when unsigned local package smoke is allowed",
);
for (const [variable, path] of [
  ["venv", String.raw`perception\.venv`],
  ["sttSettings", String.raw`perception\stt.json`],
  ["tools", "tools"],
  ["matte", "matte"],
  ["plugins", "plugins.json"],
]) {
  assert.ok(
    installSmoke.includes(`$${variable} = Join-Path $installRoot "${path}"`),
    `Windows clean-install smoke must resolve mutable runtime state: ${path}`,
  );
  assert.ok(
    installSmoke.includes(`Move-IfExists -Source $${variable} -Destination $${variable}Stash`),
    `Windows clean-install smoke must stash mutable runtime state: ${path}`,
  );
  assert.ok(
    installSmoke.includes(`Restore-IfExists -Stash $${variable}Stash -Destination $${variable}`),
    `Windows clean-install smoke must restore mutable runtime state: ${path}`,
  );
}
assert.ok(
  installSmoke.indexOf("Move-IfExists -Source $matte") <
    installSmoke.indexOf('Invoke-Native -FilePath $uninstallExe'),
  "Windows clean-install smoke must stash mutable runtimes before invoking the uninstaller",
);
assert.match(
  installSmoke,
  /Write-Host "INSTALLING"\s+try \{[\s\S]+Invoke-Native -FilePath \$setup[\s\S]+\} finally \{[\s\S]+Restore-IfExists -Stash \$matteStash/,
  "Windows clean-install smoke must restore mutable runtimes even when installation fails",
);

const aiHarness = readFileSync(resolve("scripts/windows/installed-ai-workflows.mjs"), "utf8");
assert.match(
  aiHarness,
  /CUT_DUB_TRANSLATE_BACKEND=auto\|cli\|local[\s\S]+default auto: CLI agent; local only when no CLI is installed/,
  "AI workflow harness documents the product default translation path",
);
assert.match(
  aiHarness,
  /process\.env\.CUT_DUB_TRANSLATE_BACKEND \|\| 'auto'/,
  "AI workflow harness defaults to CLI-first auto translation",
);
assert.match(
  aiHarness,
  /dubResult\.translate_backend === 'local' && !cliInstalled/,
  "AI workflow harness must accept auto-mode local translation only when no CLI is installed",
);
assert.match(
  aiHarness,
  /translate_warnings/,
  "AI workflow harness must inspect audio.dub translation fallback warnings",
);

console.log("PASS receipt-summary");
