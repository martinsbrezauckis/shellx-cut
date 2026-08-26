#!/usr/bin/env node
/** Keep bounded verb sources identical to the runtime/public aggregate. */
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkVerbAggregate } from "../lib/verb-fragment-contract.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const result = checkVerbAggregate(ROOT);
const aggregate = JSON.parse(readFileSync(resolve(ROOT, "schema/verbs.json"), "utf8"));
const registry = readFileSync(resolve(ROOT, "app/server/src/registry.rs"), "utf8");

assert.equal(result.verbCount, aggregate.verbs.length, "every canonical verb reaches the aggregate");
assert.ok(result.fragmentCount > 1, "verb sources remain reviewably fragmented");
assert.deepEqual(
  Object.keys(aggregate),
  ["$comment", "schema", "mutation_controls", "envelope", "verbs", "events", "behavior_contract"],
  "aggregate root order remains public-contract compatible",
);
assert.match(
  registry,
  /include_str!\("\.\.\/\.\.\/\.\.\/schema\/verbs\.json"\)/,
  "runtime continues to embed the compatibility aggregate",
);

console.log(`PASS verb-fragments (${result.fragmentCount} fragments, ${result.verbCount} verbs)`);
