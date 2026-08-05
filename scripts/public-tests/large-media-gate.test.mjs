import assert from "node:assert/strict";

import {
  buildGatePlan,
  classifyFrameHashes,
  parseLargeMediaGateArgs,
  resolveReceiptDir,
  summarizeJobs,
  toHashRows,
} from "../lib/large-media-gate.mjs";

assert.deepEqual(parseLargeMediaGateArgs(["--media", "a.mp4", "--media", "b.mp4"]), {
  addr: "127.0.0.1:6219",
  media: ["a.mp4", "b.mp4"],
  out: undefined,
  timeoutMs: 900_000,
  frameHeight: 540,
  rangeMs: [0, 1000],
  projectName: undefined,
});

assert.deepEqual(
  parseLargeMediaGateArgs([
    "--addr",
    "http://127.0.0.1:6161",
    "--media",
    "base.mp4",
    "--out",
    "/tmp/receipt",
    "--timeout-ms",
    "120000",
    "--frame-height",
    "360",
    "--range-ms",
    "500:2500",
    "--project-name",
    "gate",
  ]),
  {
    addr: "http://127.0.0.1:6161",
    media: ["base.mp4"],
    out: "/tmp/receipt",
    timeoutMs: 120_000,
    frameHeight: 360,
    rangeMs: [500, 2500],
    projectName: "gate",
  },
);

assert.throws(
  () => parseLargeMediaGateArgs(["--media", "a.mp4", "--range-ms", "2500:500"]),
  /range-ms/,
);

assert.match(
  resolveReceiptDir({ platform: "linux", date: new Date("2026-07-08T10:11:12Z") }),
  /\.shellx-scratch\/shellx-cut\/qualification-2026-07-08\/linux$/,
);

assert.deepEqual(buildGatePlan(["base.mp4"]), {
  base: "base.mp4",
  overlays: [
    { path: "base.mp4", reusedBase: true, atMs: 0, srcRangeMs: [1000, 5000] },
    { path: "base.mp4", reusedBase: true, atMs: 1500, srcRangeMs: [3000, 7000] },
  ],
});

assert.deepEqual(buildGatePlan(["base.mp4", "b.mp4", "c.mp4"]), {
  base: "base.mp4",
  overlays: [
    { path: "b.mp4", reusedBase: false, atMs: 0, srcRangeMs: undefined },
    { path: "c.mp4", reusedBase: false, atMs: 1500, srcRangeMs: undefined },
  ],
});

assert.deepEqual(
  summarizeJobs([
    { job_id: "job_001", kind: "import", state: "done" },
    { job_id: "job_002", kind: "proxy", state: "failed" },
  ]),
  {
    done: 1,
    failed: 1,
    active: 0,
    byKind: { import: { done: 1 }, proxy: { failed: 1 } },
  },
);

const frameRows = toHashRows([
  { label: "early", atMs: 500, compose: false, sha256: "a" },
  { label: "early", atMs: 500, compose: false, sha256: "a" },
  { label: "overlap", atMs: 1500, compose: false, sha256: "b" },
  { label: "overlap", atMs: 1500, compose: true, sha256: "c" },
  { label: "late", atMs: 9000, compose: true, sha256: "d" },
  { label: "late", atMs: 9000, compose: true, sha256: "d" },
]);
assert.deepEqual(classifyFrameHashes(frameRows), {
  repeatedStable: true,
  composedDiffersOnOverlap: true,
  unstableRepeats: [],
});

assert.deepEqual(
  classifyFrameHashes(
    toHashRows([
      { label: "early", atMs: 500, compose: false, sha256: "a" },
      { label: "early", atMs: 500, compose: false, sha256: "z" },
    ]),
  ),
  {
    repeatedStable: false,
    composedDiffersOnOverlap: false,
    unstableRepeats: ["early raw 500ms"],
  },
);

console.log("PASS large-media-gate");
