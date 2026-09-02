// This deliberately has no callable standalone HQ workload. The workload is
// Module-private to the installed/component HQ orchestrators so a prior receipt
// can never authorize another process, daemon, or render.
export async function runHqMediaGate() {
  throw new Error("HQ media workload is owned by hq-candidate-chain or windows-installed-hq and cannot be run from a standalone receipt or public CLI");
}
