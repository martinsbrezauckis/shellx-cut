#!/usr/bin/env node
/** Rebuild the runtime/public verb aggregate from bounded canonical fragments. */
import { writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  AGGREGATE_PATH,
  checkVerbAggregate,
  loadVerbFragmentContract,
  renderVerbAggregate,
} from "./lib/verb-fragment-contract.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));
const allowed = new Set(["--check"]);
for (const arg of args) {
  if (!allowed.has(arg)) throw new Error(`unknown argument: ${arg}`);
}

if (args.has("--check")) {
  const result = checkVerbAggregate(ROOT);
  console.log(`PASS verb fragments (${result.fragmentCount} fragments, ${result.verbCount} verbs)`);
} else {
  const contract = loadVerbFragmentContract(ROOT);
  writeFileSync(resolve(ROOT, AGGREGATE_PATH), renderVerbAggregate(contract));
  console.log(`Generated ${AGGREGATE_PATH} from ${contract.manifest.fragments.length} fragments.`);
}
