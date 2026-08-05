import assert from "node:assert/strict";
import test from "node:test";

import {
  buildWebviewProfileReleaseScript,
  normalizeWebviewProfileToken,
} from "../lib/windows-webview-profile.mjs";

test("WebView2 profile cleanup is bounded to one validated token", () => {
  const script = buildWebviewProfileReleaseScript("ShellXCutFinalAction-20260730");
  assert.match(script, /Name = 'msedgewebview2[.]exe'/);
  assert.match(script, /CommandLine -like/);
  assert.match(script, /ShellXCutFinalAction-20260730/);
  assert.match(script, /AddSeconds[(]10[)]/);
  assert.match(script, /Stop-Process -Id [$]_[.]ProcessId/);
  assert.doesNotMatch(script, /Get-Process[^]*Stop-Process -Force[^]*msedgewebview2/);
});

test("WebView2 profile cleanup rejects paths and broad tokens", () => {
  assert.equal(normalizeWebviewProfileToken("profile_1"), "profile_1");
  for (const value of ["", ".", "..", "profile/path", String.raw`C:\Temp`, "*"]) {
    assert.throws(() => normalizeWebviewProfileToken(value));
  }
});
