#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (path) => readFileSync(resolve(root, path), "utf8");
const documents = [
  ["docs/public/FEATURES.md", read("docs/public/FEATURES.md")],
  ["skill/shellx-cut/SKILL.md", read("skill/shellx-cut/SKILL.md")],
  ["skill/shellx-cut/reference.md", read("skill/shellx-cut/reference.md")],
];
const verbs = [
  "inspect.media",
  "inspect.range",
  "jobs.retry",
  "media.intelligence_rebuild",
  "media.intelligence_search",
  "media.intelligence_status",
  "media.relink_apply",
  "media.relink_preview",
  "project.cache_preview",
  "project.cache_purge",
  "project.package_create",
  "project.package_plan",
];

test("v0.6.111 additions stay visible in features, skill, and reference", () => {
  for (const verb of verbs) {
    for (const [path, source] of documents) {
      assert.equal(source.includes(verb), true, `${path} is missing v0.6.111 feature ${verb}`);
    }
  }
});
