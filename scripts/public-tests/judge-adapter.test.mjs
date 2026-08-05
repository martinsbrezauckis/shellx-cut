#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const JUDGE = join(ROOT, "app", "perception", "py", "judge");
const LADDER = join(JUDGE, "adapters", "ladder_judge.py");
const VALIDATION = join(ROOT, "scripts", "public-tests", "judge-adapter-validation.py");
const REQUIRED = [
  "judge.py",
  "adapters/ladder_judge.py",
  "adapters/cli_judge.py",
  "adapters/codex_judge.py",
  "adapters/antigravity_judge.py",
  "adapters/grok_judge.py",
];

for (const relative of REQUIRED) {
  assert.equal(
    existsSync(join(JUDGE, relative)),
    true,
    `bundled judge module is missing: ${relative}`,
  );
}

const tauri = JSON.parse(
  readFileSync(join(ROOT, "app", "desktop", "src-tauri", "tauri.conf.json"), "utf8"),
);
assert.equal(
  tauri.bundle?.resources?.["../../perception/py/judge"],
  "perception/judge",
  "the installed app must carry the complete judge ladder in its perception payload",
);

const pythonProbe = spawnSync(
  process.env.CUTD_ADAPTER_PYTHON || (process.platform === "win32" ? "python" : "python3"),
  ["-c", "import sys; print(sys.executable)"],
  { encoding: "utf8" },
);
if (pythonProbe.status !== 0 || !pythonProbe.stdout.trim()) {
  console.log("SKIP judge-adapter.test.mjs: Python is not available");
  process.exit(0);
}
const python = pythonProbe.stdout.trim();

const validation = spawnSync(python, [VALIDATION], {
  cwd: ROOT,
  encoding: "utf8",
  env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
});
assert.equal(
  validation.status,
  0,
  `bundled adapter validation failed:\n${validation.stdout}\n${validation.stderr}`,
);

const emptyPath = join(ROOT, ".scratch", "judge-test-empty-path");
const detect = spawnSync(python, [LADDER, "detect"], {
  cwd: ROOT,
  encoding: "utf8",
  env: {
    ...process.env,
    PATH: emptyPath,
    PYTHONDONTWRITEBYTECODE: "1",
  },
});
assert.equal(detect.status, 0, detect.stderr);
const report = JSON.parse(detect.stdout);
assert.deepEqual(report.order, ["claude", "codex", "antigravity", "grok"]);
assert.equal(report.auto_selected, null);
assert.equal(report.rungs.every((rung) => rung.found === false), true);

const badProvider = spawnSync(
  python,
  [LADDER, "review", "--provider", "not-a-provider"],
  {
    cwd: ROOT,
    encoding: "utf8",
    env: { ...process.env, PATH: emptyPath, PYTHONDONTWRITEBYTECODE: "1" },
  },
);
assert.equal(badProvider.status, 2);
assert.match(badProvider.stderr, /unknown --provider/);

// Full no-quota wiring proof on POSIX: real ffmpeg sampling + a fake Codex
// executable that speaks the current non-interactive CLI output contract.
if (process.platform !== "win32") {
  const toolProbe = spawnSync(
    python,
    ["-c", "import json, shutil; print(json.dumps({'ffmpeg': shutil.which('ffmpeg'), 'ffprobe': shutil.which('ffprobe')}))"],
    { encoding: "utf8" },
  );
  const tools = toolProbe.status === 0 ? JSON.parse(toolProbe.stdout) : {};
  if (tools.ffmpeg && tools.ffprobe) {
    const work = mkdtempSync(join(tmpdir(), "cut-judge-adapter-"));
    try {
      const render = join(work, "render.mp4");
      const perception = join(work, "render.perception.json");
      const bundle = join(work, "bundle");
      const out = join(bundle, "envelope.json");
      const fakeCodex = join(work, "codex");
      const fakeClaude = join(work, "claude");
      const generated = spawnSync(
        tools.ffmpeg,
        [
          "-hide_banner", "-loglevel", "error", "-y",
          "-f", "lavfi", "-i", "color=c=blue:s=160x90:d=1",
          "-pix_fmt", "yuv420p", render,
        ],
        { encoding: "utf8" },
      );
      assert.equal(generated.status, 0, generated.stderr);
      writeFileSync(perception, JSON.stringify({
        schema: "shellx-cut/perception/1",
        asset_id: "render",
        scenes: [],
        silences: [],
      }));
      writeFileSync(
        fakeCodex,
        `#!${process.execPath}\n`
          + `const fs = require("node:fs");\n`
          + `const args = process.argv.slice(2);\n`
          + `if (args.includes("--version")) { console.log("codex-cli fixture"); process.exit(0); }\n`
          + `const out = args[args.indexOf("-o") + 1];\n`
          + `fs.writeFileSync(out, JSON.stringify({verdict:"pass",issues:[],cannot_assess:[],confidence:0.93,summary:"fixture reviewed sampled frames"}));\n`
          + `console.log(JSON.stringify({type:"thread.started",thread_id:"fixture"}));\n`
          + `console.log(JSON.stringify({type:"turn.completed",usage:{input_tokens:1,output_tokens:1}}));\n`,
      );
      chmodSync(fakeCodex, 0o755);
      writeFileSync(
        fakeClaude,
        `#!${process.execPath}\n`
          + `const args = process.argv.slice(2);\n`
          + `if (args.includes("--version")) { console.log("claude fixture"); process.exit(0); }\n`
          + `process.stderr.write("fixture Read unavailable\\n");\n`
          + `process.exit(1);\n`,
      );
      chmodSync(fakeClaude, 0o755);
      const reviewed = spawnSync(
        python,
        [
          LADDER, "review", "--provider", "codex",
          "--render", render,
          "--perception", perception,
          "--intent", "verify fixture continuity",
          "--bundle-dir", bundle,
          "--keep-bundle",
          "--out", out,
          "--timeout", "20",
          "--max-frames", "2",
          "--width", "96",
        ],
        {
          cwd: ROOT,
          encoding: "utf8",
          env: {
            ...process.env,
            PATH: work,
            PYTHONDONTWRITEBYTECODE: "1",
            SHELLX_CUT_FFMPEG: tools.ffmpeg,
            SHELLX_CUT_FFPROBE: tools.ffprobe,
          },
        },
      );
      assert.equal(reviewed.status, 0, `${reviewed.stdout}\n${reviewed.stderr}`);
      const envelope = JSON.parse(readFileSync(out, "utf8"));
      assert.equal(envelope.schema, "shellx-cut/judge-review/1");
      assert.equal(envelope.status, "completed");
      assert.equal(envelope.backend.provider, "codex");
      assert.equal(envelope.review.verdict, "pass");
      assert.equal(envelope.ladder.selected, "codex");
      assert.equal(envelope.backend.frames_sent > 0, true);

      // Auto must not stop at an installed-but-unusable first rung. Claude's
      // Read preflight is an attempted infrastructure failure, so the ladder
      // records it and continues to the working Codex fixture without quota.
      const autoBundle = join(work, "auto-bundle");
      const autoOut = join(autoBundle, "envelope.json");
      const autoReviewed = spawnSync(
        python,
        [
          LADDER, "review", "--provider", "auto",
          "--render", render,
          "--perception", perception,
          "--intent", "verify automatic infrastructure fallback",
          "--bundle-dir", autoBundle,
          "--keep-bundle",
          "--out", autoOut,
          "--timeout", "20",
          "--max-frames", "2",
          "--width", "96",
        ],
        {
          cwd: ROOT,
          encoding: "utf8",
          env: {
            ...process.env,
            PATH: work,
            PYTHONDONTWRITEBYTECODE: "1",
            SHELLX_CUT_FFMPEG: tools.ffmpeg,
            SHELLX_CUT_FFPROBE: tools.ffprobe,
          },
        },
      );
      assert.equal(autoReviewed.status, 0, `${autoReviewed.stdout}\n${autoReviewed.stderr}`);
      const autoEnvelope = JSON.parse(readFileSync(autoOut, "utf8"));
      assert.equal(autoEnvelope.status, "completed");
      assert.equal(autoEnvelope.backend.provider, "codex");
      assert.equal(autoEnvelope.ladder.selected, "codex");
      assert.equal(autoEnvelope.ladder.attempted.length, 1);
      assert.equal(autoEnvelope.ladder.attempted[0].provider, "claude");
      assert.equal(autoEnvelope.ladder.attempted[0].status, "error");
      assert.equal(autoEnvelope.ladder.attempted[0].error_class, "infrastructure");
      assert.deepEqual(
        readdirSync(work).filter((name) => name.startsWith("cli_judge_retry_")),
        [],
        "runner-owned Claude retry bundles must self-clean",
      );
    } finally {
      rmSync(work, { recursive: true, force: true });
    }
  }
}

console.log("judge-adapter.test.mjs PASS (bundle, validation, ladder, no-quota review)");
