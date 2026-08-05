#!/usr/bin/env node
import { strict as assert } from "node:assert";

import { buildFullCoverageReceipt } from "../lib/full-coverage-receipt.mjs";

const rows = [
  {
    surface: "generate",
    name: "Generate preview",
    rowKind: "ui_action",
    present: "pass",
    render: "pass",
    click: "pass",
    result: "pass",
    evidence: "preview PNG wrote to disk",
    shot: "/tmp/generate.png",
  },
  {
    surface: "record",
    name: "Screen record start",
    rowKind: "support",
    present: "pass",
    render: "pass",
    click: "na",
    result: "na",
    evidence: "recorder rig gate owns effect-proof",
    shot: "/tmp/record.png",
  },
  {
    surface: "agent",
    name: "Grok optional prompt",
    rowKind: "support",
    present: "pass",
    render: "pass",
    click: "na",
    result: "na",
    evidence: "optional multi-agent backend not authed on this rig",
    shot: "",
  },
  {
    surface: "comments",
    name: "Export review package",
    rowKind: "support",
    present: "pass",
    render: "pass",
    click: "na",
    result: "na",
    evidence: "render-bound package effect-proof is delegated to verify-review-handoff.mjs",
    shot: "/tmp/review-export.png",
  },
];

const receipt = buildFullCoverageReceipt(rows, {
  full: true,
  coverage: { covered: 211, excluded: 0, total: 211, ok: true },
  media: { scene: "scene.mp4", speech: "speech.mp4", speakers: "speakers.mp4" },
  screenshotsDir: "/tmp/fcv",
  generatedAt: "2026-06-30T01:30:00.000Z",
});

assert.equal(receipt.schema, "shellx-cut/full-coverage-results@1");
assert.equal(receipt.ok, true);
assert.equal(receipt.full, true);
assert.deepEqual(receipt.summary.controls, {
  total: 4,
  uiActions: 1,
  supportRows: 3,
  fullyVerified: 1,
  delegated: 2,
  dependencySkips: 0,
  optionalAgentSkips: 1,
  guards: 0,
  couldNotVerify: 0,
  strictUnverified: 0,
  failures: 0,
});
assert.match(receipt.actionManifest.sha256, /^[a-f0-9]{64}$/);
assert.equal(receipt.actionManifest.total, 1);
assert.deepEqual(receipt.actionManifest.observed, ["generate::Generate preview"]);
assert.equal(receipt.actionManifest.occurrences, 1);
assert.deepEqual(receipt.actionManifest.repeated, []);
assert.equal(receipt.sourceActionManifest.matchesExpected, false);
assert.equal(receipt.summary.dimensions.result.pass, 1);
assert.equal(receipt.summary.dimensions.result.na, 3);
assert.equal(receipt.results[0].ok, true);
assert.equal(receipt.results[1].classification, "delegated");
assert.equal(receipt.results[3].classification, "delegated");

const weak = buildFullCoverageReceipt(
  [
    {
      surface: "library",
      name: "Add to project",
      rowKind: "ui_action",
      present: "pass",
      render: "pass",
      click: "pass",
      result: "na",
      evidence: "button clicked but no result assertion",
    },
  ],
  { full: true },
);

assert.equal(weak.ok, false);
assert.equal(weak.summary.controls.couldNotVerify, 1);
assert.equal(weak.results[0].classification, "could_not_verify");

const strict = buildFullCoverageReceipt(rows, {
  full: true,
  strictAllActions: true,
  surface: "windows-installed",
  runtime: { installedApp: true, driver: "webview2-cdp" },
});

assert.equal(strict.ok, false);
assert.equal(strict.strictAllActions, true);
assert.equal(strict.surface, "windows-installed");
assert.equal(strict.summary.controls.fullyVerified, 1);
assert.equal(strict.summary.controls.strictUnverified, 0);
assert.equal(strict.results[1].classification, "delegated");
assert.equal(strict.actionManifest.total, 1);

const strictGreen = buildFullCoverageReceipt(
  [
    {
      actionId: "settings::open",
      rowKind: "ui_action",
      surface: "settings",
      name: "Open Settings",
      present: "pass",
      render: "pass",
      click: "pass",
      result: "pass",
      evidence: "dialog opened and focus moved inside",
    },
  ],
  {
    full: true,
    strictAllActions: true,
    sourceActionIds: ["settings-open"],
    expectedSourceActionIds: ["settings-open"],
    runtimeSourceActionIds: ["settings-open"],
    expectedRuntimeSourceActionIds: ["settings-open"],
  },
);
assert.equal(strictGreen.ok, true);
assert.equal(strictGreen.summary.controls.strictUnverified, 0);
assert.equal(strictGreen.sourceActionManifest.matchesExpected, true);
assert.equal(strictGreen.sourceActionManifest.sha256, strictGreen.sourceActionManifest.expectedSha256);
assert.equal(strictGreen.runtimeSourceActionManifest.matchesExpected, true);
assert.deepEqual(strictGreen.runtimeSourceActionManifest.observed, ["settings-open"]);

const duplicate = buildFullCoverageReceipt(
  [strictGreen.results[0], strictGreen.results[0]],
  {
    full: true,
    strictAllActions: true,
    sourceActionIds: ["settings-open"],
    expectedSourceActionIds: ["settings-open"],
  },
);
assert.equal(duplicate.ok, true);
assert.equal(duplicate.actionManifest.occurrences, 2);
assert.deepEqual(duplicate.actionManifest.repeated, [{ id: "settings::open", count: 2 }]);

const drift = buildFullCoverageReceipt(
  [strictGreen.results[0]],
  {
    full: true,
    strictAllActions: true,
    sourceActionIds: ["settings-open"],
    expectedSourceActionIds: ["settings-open", "library-open"],
  },
);
assert.equal(drift.ok, false);
assert.deepEqual(drift.sourceActionManifest.missing, ["library-open"]);

const runtimeDrift = buildFullCoverageReceipt(
  [strictGreen.results[0]],
  {
    full: true,
    strictAllActions: true,
    sourceActionIds: ["settings-open", "library-open"],
    expectedSourceActionIds: ["settings-open", "library-open"],
    runtimeSourceActionIds: ["settings-open"],
    expectedRuntimeSourceActionIds: ["settings-open", "library-open"],
  },
);
assert.equal(runtimeDrift.ok, false);
assert.deepEqual(runtimeDrift.runtimeSourceActionManifest.observed, ["settings-open"]);
assert.deepEqual(runtimeDrift.runtimeSourceActionManifest.missing, ["library-open"]);

console.log("PASS full-coverage-receipt");
