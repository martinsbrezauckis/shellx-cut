import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("public and private test tooling has explicit physical boundaries", () => {
  for (const path of [
    "docs/public",
    "scripts/public-tests",
    "ui/public-tests",
  ]) {
    assert.equal(existsSync(resolve(root, path)), true, `missing classified path: ${path}`);
  }

  if (existsSync(resolve(root, "PUBLIC_EXPORT_MANIFEST.json"))) {
    for (const path of ["docs/private", "scripts/private", "scripts/private-tests", "ui/private-tests"]) {
      assert.equal(existsSync(resolve(root, path)), false, `private path reached the public export: ${path}`);
    }
  }

  for (const path of [
    "docs/public/FEATURE_CHANGE_WORKFLOW.md",
    "docs/public/TEST_RIGS.md",
  ]) {
    assert.equal(existsSync(resolve(root, path)), false, `internal workflow remains public: ${path}`);
  }
  for (const legacy of ["scripts/demo", "scripts/tests", "ui/tests"]) {
    assert.equal(existsSync(resolve(root, legacy)), false, `legacy mixed path remains: ${legacy}`);
  }
});
