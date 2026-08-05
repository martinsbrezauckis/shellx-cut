#!/usr/bin/env node
import {
  largeMediaGateUsage,
  parseLargeMediaGateArgs,
  runLargeMediaGate,
} from "./lib/large-media-gate.mjs";

async function main() {
  const options = parseLargeMediaGateArgs(process.argv.slice(2));
  if (options.help) {
    console.log(largeMediaGateUsage());
    return;
  }
  const receipt = await runLargeMediaGate(options);
  console.log(`${receipt.pass ? "PASS" : "FAIL"} large-media-gate`);
  console.log(`receipt: ${receipt.receiptDir}/large-media-gate-receipt.json`);
  if (!receipt.pass) process.exitCode = 1;
}

main().catch((error) => {
  console.error(`FAIL large-media-gate: ${error?.message || error}`);
  process.exit(1);
});
