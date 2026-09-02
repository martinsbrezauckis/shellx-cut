import assert from "node:assert/strict";
import test from "node:test";

import {
  buildWindowsQualificationProfileRemovalScript,
  buildWebviewProfileReleaseScript,
  normalizeWebviewProfileToken,
} from "../lib/windows-webview-profile.mjs";

test("WebView2 profile cleanup is bounded to one validated token", () => {
  const script = buildWebviewProfileReleaseScript("ShellXCutFinalAction-20260730");
  assert.match(script, /Name = 'msedgewebview2[.]exe'/);
  assert.match(script, /CommandLine -like/);
  assert.match(script, /ShellXCutFinalAction-20260730/);
  assert.match(script, /AddSeconds[(]10[)]/);
  assert.match(script, /quietMilliseconds = 2000/);
  assert.match(script, /AddSeconds[(]15[)]/);
  assert.match(script, /quietSince = [$]null/);
  assert.match(script, /quietElapsed -lt [$]quietMilliseconds/);
  assert.match(script, /Stop-Process -Id [$]_[.]ProcessId/);
  assert.doesNotMatch(script, /Get-Process[^]*Stop-Process -Force[^]*msedgewebview2/);
});

test("WebView2 profile cleanup rejects paths and broad tokens", () => {
  assert.equal(normalizeWebviewProfileToken("profile_1"), "profile_1");
  for (const value of ["", ".", "..", "profile/path", String.raw`C:\Temp`, "*"]) {
    assert.throws(() => normalizeWebviewProfileToken(value));
  }
});

test("workspace-backed Windows profile cleanup stays under one qualification run", () => {
  const script = buildWindowsQualificationProfileRemovalScript({
    localAppData: String.raw`C:\Users\User\AppData\Local`,
    runtimeLocalAppData: String.raw`C:\Users\User\AppData\Local\ShellX Cut Qualification\windows-installed-candidate-2026-08-20T20-03-00-133Z\local-app-data`,
  });
  assert.match(script, /ShellX Cut Qualification\\windows-installed-candidate-/);
  assert.match(script, /ReparsePoint/);
  assert.match(script, /Remove-Item -LiteralPath [$]path -Recurse -Force/);
  for (const target of [
    String.raw`C:\Users\User\AppData\Local`,
    String.raw`C:\Users\User\AppData\Local\ShellX Cut Qualification`,
    String.raw`C:\Users\User\AppData\Local\ShellX Cut Qualification\bad\local-app-data`,
    String.raw`C:\Temp\windows-installed-candidate-ok\local-app-data`,
  ]) {
    assert.throws(() => buildWindowsQualificationProfileRemovalScript({
      localAppData: String.raw`C:\Users\User\AppData\Local`,
      runtimeLocalAppData: target,
    }));
  }
});
