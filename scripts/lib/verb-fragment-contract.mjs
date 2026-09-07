import { strict as assert } from "node:assert";
import { readFileSync, readdirSync } from "node:fs";
import { isAbsolute, relative, resolve, sep } from "node:path";

export const BASE_PATH = "schema/verbs/base.json";
export const MANIFEST_PATH = "schema/verbs/manifest.json";
export const AGGREGATE_PATH = "schema/verbs.json";
export const FRAGMENT_SCHEMA = "shellx-cut/verb-fragment/1";
export const MAX_FRAGMENT_LINES = 600;

const readJson = (root, path) => {
  try {
    return JSON.parse(readFileSync(resolve(root, path), "utf8"));
  } catch (error) {
    throw new Error(`${path}: ${error.message}`);
  }
};

const lineCount = (source) => source.split("\n").length - Number(source.endsWith("\n"));
const renderJson = (value) => `${JSON.stringify(value, null, 2)}\n`;

const listJsonFiles = (directory, root = directory) => {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) files.push(...listJsonFiles(path, root));
    if (entry.isFile() && entry.name.endsWith(".json")) {
      files.push(relative(root, path).split(sep).join("/"));
    }
  }
  return files.sort();
};

const validateFragmentPath = (path) => {
  assert.equal(typeof path, "string", "fragment path must be a string");
  assert.match(path, /^fragments\/[0-9]{3}-[a-z0-9_]+\.json$/, `${path}: deterministic fragment path`);
  assert.equal(isAbsolute(path), false, `${path}: fragment path must be relative`);
  assert.equal(path.includes(".."), false, `${path}: fragment path cannot escape its source directory`);
};

export const loadVerbFragmentContract = (root) => {
  const base = readJson(root, BASE_PATH);
  const manifest = readJson(root, MANIFEST_PATH);
  assert.equal(base.schema, "shellx-cut/verb-base/1", `${BASE_PATH}: schema`);
  assert.equal(manifest.schema, "shellx-cut/verb-fragment-manifest/1", `${MANIFEST_PATH}: schema`);
  assert.equal(typeof base.verbs_after, "string", `${BASE_PATH}: verbs_after`);
  assert.ok(base.root && !Array.isArray(base.root), `${BASE_PATH}: root object`);
  assert.equal(Object.hasOwn(base.root, "verbs"), false, `${BASE_PATH}: root cannot embed verbs`);
  assert.ok(Object.hasOwn(base.root, base.verbs_after), `${BASE_PATH}: verbs_after key exists`);
  assert.ok(Array.isArray(manifest.fragments) && manifest.fragments.length > 0, `${MANIFEST_PATH}: fragments`);

  const declared = new Set();
  const names = new Set();
  const verbs = [];
  for (const path of manifest.fragments) {
    validateFragmentPath(path);
    assert.equal(declared.has(path), false, `${MANIFEST_PATH}: duplicate ${path}`);
    declared.add(path);
    const relativePath = `schema/verbs/${path}`;
    const source = readFileSync(resolve(root, relativePath), "utf8");
    assert.ok(lineCount(source) <= MAX_FRAGMENT_LINES, `${relativePath}: exceeds ${MAX_FRAGMENT_LINES} lines`);
    const fragment = readJson(root, relativePath);
    assert.equal(fragment.schema, FRAGMENT_SCHEMA, `${relativePath}: schema`);
    assert.match(fragment.domain, /^[a-z0-9_]+$/, `${relativePath}: domain`);
    assert.ok(Array.isArray(fragment.verbs) && fragment.verbs.length > 0, `${relativePath}: verbs`);
    for (const verb of fragment.verbs) {
      assert.equal(verb.domain, fragment.domain, `${relativePath}: ${verb.name} domain`);
      assert.match(verb.name, /^[a-z0-9_.]+$/, `${relativePath}: verb name`);
      assert.equal(names.has(verb.name), false, `${relativePath}: duplicate verb ${verb.name}`);
      names.add(verb.name);
      verbs.push(verb);
    }
  }

  const sourceDirectory = resolve(root, "schema/verbs");
  const present = listJsonFiles(resolve(sourceDirectory, "fragments"))
    .map((path) => `fragments/${path}`);
  assert.deepEqual(present, [...declared].sort(), `${MANIFEST_PATH}: no missing or orphan fragments`);
  return { base, manifest, verbs };
};

export const renderVerbAggregate = ({ base, verbs }) => {
  const aggregate = {};
  for (const [key, value] of Object.entries(base.root)) {
    aggregate[key] = value;
    if (key === base.verbs_after) aggregate.verbs = verbs;
  }
  assert.ok(Array.isArray(aggregate.verbs), `${BASE_PATH}: failed to place verbs`);
  return renderJson(aggregate);
};

export const checkVerbAggregate = (root) => {
  const contract = loadVerbFragmentContract(root);
  const expected = renderVerbAggregate(contract);
  const actual = readFileSync(resolve(root, AGGREGATE_PATH), "utf8");
  assert.equal(actual, expected, `${AGGREGATE_PATH}: regenerate with node scripts/generate-verbs.mjs`);
  return { fragmentCount: contract.manifest.fragments.length, verbCount: contract.verbs.length };
};
