import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

test("media parser regexes retain multi-character numeric fields", () => {
  const source = read("app/perception/py/instruments.py");
  for (const pattern of [
    String.raw`r"silence_start:\s*(-?[\d.]+)"`,
    String.raw`r"silence_end:\s*([\d.]+)"`,
    String.raw`r"crop=(\d+):(\d+):(\d+):(\d+)"`,
    String.raw`r"showinfo.*?pts_time:([-+\d.eE]+)"`,
    String.raw`r"black_start:([\d.]+)\s+black_end:([\d.]+)"`,
    String.raw`r"freeze_start:\s*([\d.]+)"`,
    String.raw`r"freeze_end:\s*([\d.]+)"`,
    String.raw`r"t:\s*([\d.]+).*?M:\s*(-?[\d.]+|nan)"`,
    String.raw`r"I:\s*(-?[\d.]+)\s*LUFS"`,
  ]) {
    assert.ok(source.includes(pattern), `missing parser quantifier contract: ${pattern}`);
  }
});

test("judge and UI helper regexes retain repeated-token matching", () => {
  const judge = read("app/perception/py/judge/adapters/cli_judge.py");
  for (const pattern of [
    String.raw`r"\bi\s+(?:can\s+)?hear(?:d)?\b"`,
    String.raw`r"-?\d+(?:\.\d+)?\s?(?:LUFS|dBTP|dB\b)"`,
    String.raw`r"\b(all\s+\d+\s+frame|all\s+(?:the\s+)?frames|every\s+frame"`,
    String.raw`r"|zero\s+frames?\b|no\s+frames?\s+(?:were\s+)?readable"`,
  ]) {
    assert.ok(judge.includes(pattern), `missing judge quantifier contract: ${pattern}`);
  }

  assert.ok(
    read("ui/src/panels/Review/mock.ts").includes(String.raw`/\/api\/verb\/([a-z_.]+)/`),
    "Review mock must parse complete dotted verb names",
  );
  assert.ok(
    read("ui/public-tests/surface-sweep.mjs").includes(String.raw`/mean_volume:\s*(-?[\d.]+) dB/`),
    "audio sweep must parse complete multi-digit dB values",
  );
});
