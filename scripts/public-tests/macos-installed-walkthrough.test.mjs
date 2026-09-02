import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import { summarizeWindowEvidence } from "../macos-installed-walkthrough.mjs";

const walkthroughUrl = new URL("../macos-installed-walkthrough.mjs", import.meta.url);

test("uses screenshot evidence when a PNG was captured", () => {
  const result = summarizeWindowEvidence({
    screenshotOk: true,
    screenshotError: "",
    windows: [{
      kCGWindowOwnerName: "ShellX Cut",
      kCGWindowIsOnscreen: 1,
      kCGWindowBounds: { Width: 1440, Height: 901, X: 240, Y: 63 },
    }],
  });

  assert.equal(result.ok, true);
  assert.equal(result.mode, "screenshot");
});

test("uses CoreGraphics window metadata when remote screenshot capture is denied", () => {
  const result = summarizeWindowEvidence({
    screenshotOk: false,
    screenshotError: "could not create image from display",
    windows: [
      {
        kCGWindowOwnerName: "ShellX Cut",
        kCGWindowIsOnscreen: 1,
        kCGWindowBounds: {
          Width: 1440,
          Height: 901,
          X: 240,
          Y: 63,
        },
      },
    ],
  });

  assert.equal(result.ok, true);
  assert.equal(result.mode, "coregraphics");
  assert.match(result.detail, /1440x901/);
});

test("fails window evidence when neither screenshot nor on-screen window metadata exists", () => {
  const result = summarizeWindowEvidence({
    screenshotOk: false,
    screenshotError: "could not create image from display",
    windows: [],
  });

  assert.equal(result.ok, false);
  assert.equal(result.mode, "none");
});

test("does not accept an unrelated full-screen capture without a ShellX Cut window", () => {
  const result = summarizeWindowEvidence({ screenshotOk: true, windows: [] });
  assert.equal(result.ok, false);
  assert.equal(result.mode, "none");
});

test("shipping macOS walkthrough binds source, visuals, and pre/post native integrity", async () => {
  const source = await readFile(walkthroughUrl, "utf8");
  assert.match(source, /--source-commit/);
  assert.match(source, /--source-content-manifest/);
  assert.match(source, /--out/);
  assert.match(source, /collectInstalledRuntimeEvidence/);
  assert.match(source, /onSurfaceOpened/);
  assert.match(source, /screencapture[\s\S]+-l/);
  assert.match(source, /already reachable before the installed app launch/);
  assert.match(source, /waitForEngineClosed/);
  assert.match(source, /macIntegrityCommands[(]appPath, ["']pre["'][)]/);
  assert.match(source, /macIntegrityCommands[(]appPath, ["']post["'][)]/);
  assert.match(source, /buildNativeIntegrityEvidence/);
  assert.match(source, /buildInstalledWalkthroughReceipt/);
  assert.match(source, /--keep-open cannot produce the required post-use code-seal proof/);
});
