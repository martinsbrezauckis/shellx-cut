import { createHash } from "node:crypto";
import { existsSync, lstatSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";

import { assertNoReparseAncestors, assertPathInside, createSafeDirectories, safeRegularArtifact, writeSafeJson } from "./hq-candidate-chain-security.mjs";
import { removeOwnedTree } from "./runtime-sealed-b1-b2-files.mjs";

export const HQ_XWIN_CACHE_SCHEMA = "shellx-cut/hq-candidate-xwin-cache@1";
export const HQ_XWIN_CACHE_INVENTORY_SCHEMA = "shellx-cut/hq-candidate-xwin-cache-inventory@1";
export const HQ_DERIVED_BUILD_DIRECTORY_NAMES = Object.freeze({ cargoTarget: "cargo-target", cargoHome: "cargo-home" });

function samePath(left, right) {
  const normalizedLeft = resolve(left);
  const normalizedRight = resolve(right);
  return process.platform === "win32" ? normalizedLeft.toLowerCase() === normalizedRight.toLowerCase() : normalizedLeft === normalizedRight;
}

function cacheFileEntries(cacheDir) {
  const root = resolve(cacheDir);
  const files = [];
  const visit = (directory) => {
    assertNoReparseAncestors(directory, "HQ candidate cargo-xwin cache");
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name))) {
      const path = join(directory, entry.name);
      const stat = lstatSync(path);
      if (stat.isSymbolicLink()) throw new Error(`HQ candidate cargo-xwin cache contains a symlink, junction, or reparse point: ${path}`);
      if (stat.isDirectory()) {
        assertNoReparseAncestors(path, "HQ candidate cargo-xwin cache");
        visit(path);
      } else if (stat.isFile()) {
        const artifact = safeRegularArtifact(path, "HQ candidate cargo-xwin cache file");
        files.push({ path: relative(root, artifact.path).replaceAll("\\", "/"), bytes: artifact.bytes, sha256: artifact.sha256 });
      } else throw new Error(`HQ candidate cargo-xwin cache contains an unsupported filesystem entry: ${path}`);
    }
    assertNoReparseAncestors(directory, "HQ candidate cargo-xwin cache");
  };
  visit(root);
  return files;
}

function cacheContentSha256(files) {
  return createHash("sha256").update(JSON.stringify(files)).digest("hex");
}

function assertAbsentLeaf(path, label) {
  try {
    lstatSync(path);
  } catch (error) {
    if (error?.code === "ENOENT") return true;
    throw error;
  }
  throw new Error(`${label} remained after cleanup: ${path}`);
}

function assertInventoryContract(inventory, layout, runDir) {
  const expected = assertGovernedHqXwinCacheLayout(layout, runDir);
  if (inventory?.schema !== HQ_XWIN_CACHE_INVENTORY_SCHEMA || inventory?.cache?.schema !== HQ_XWIN_CACHE_SCHEMA || !samePath(inventory.cache.path || "", expected.path) || !Array.isArray(inventory.files) || inventory.cache.files !== inventory.files.length || !Number.isSafeInteger(inventory.cache.bytes) || !/^[a-f0-9]{64}$/.test(inventory.cache.contentSha256 || "")) {
    throw new Error("HQ candidate cargo-xwin cache inventory has an invalid contract");
  }
  let previous = "", bytes = 0;
  for (const file of inventory.files) {
    if (typeof file?.path !== "string" || !file.path || file.path.startsWith("/") || file.path.split("/").includes("..") || file.path <= previous || !Number.isSafeInteger(file.bytes) || file.bytes <= 0 || !/^[a-f0-9]{64}$/.test(file.sha256 || "")) {
      throw new Error("HQ candidate cargo-xwin cache inventory has an invalid file entry");
    }
    previous = file.path;
    bytes += file.bytes;
  }
  if (bytes !== inventory.cache.bytes || cacheContentSha256(inventory.files) !== inventory.cache.contentSha256) throw new Error("HQ candidate cargo-xwin cache inventory hash or byte count is invalid");
  return expected;
}

export function governedHqXwinCacheLayout(runDir) {
  const root = resolve(runDir);
  return { schema: HQ_XWIN_CACHE_SCHEMA, path: join(root, "xwin-cache"), inventoryPath: join(root, "xwin-cache-inventory.json") };
}

export function assertGovernedHqXwinCacheLayout(layout, runDir) {
  const expected = governedHqXwinCacheLayout(runDir);
  if (layout?.schema !== HQ_XWIN_CACHE_SCHEMA || !samePath(layout.path || "", expected.path) || !samePath(layout.inventoryPath || "", expected.inventoryPath)) {
    throw new Error("HQ candidate cargo-xwin cache must use the exact run-owned cache layout");
  }
  assertPathInside(layout.path, runDir, "HQ candidate cargo-xwin cache");
  assertPathInside(layout.inventoryPath, runDir, "HQ candidate cargo-xwin cache inventory");
  return expected;
}

export function prepareGovernedHqXwinCache(layout, runDir) {
  const expected = assertGovernedHqXwinCacheLayout(layout, runDir);
  createSafeDirectories(expected.path, "HQ candidate cargo-xwin cache", { requireNewLeaf: true });
  assertNoReparseAncestors(expected.path, "HQ candidate cargo-xwin cache");
  return expected;
}

export function snapshotHqXwinCache(layout, runDir) {
  const expected = assertGovernedHqXwinCacheLayout(layout, runDir);
  if (!existsSync(expected.path)) throw new Error(`HQ candidate cargo-xwin cache is missing: ${expected.path}`);
  const files = cacheFileEntries(expected.path);
  const bytes = files.reduce((sum, file) => sum + file.bytes, 0);
  return { schema: HQ_XWIN_CACHE_INVENTORY_SCHEMA, cache: { schema: HQ_XWIN_CACHE_SCHEMA, path: expected.path, files: files.length, bytes, contentSha256: cacheContentSha256(files) }, files };
}

export function persistHqXwinCacheInventory(layout, runDir) {
  const expected = assertGovernedHqXwinCacheLayout(layout, runDir);
  const inventory = snapshotHqXwinCache(expected, runDir);
  const artifact = writeSafeJson(expected.inventoryPath, inventory, "HQ candidate cargo-xwin cache inventory");
  return { inventory, artifact };
}

export function assertHqXwinCacheInventory(inventory, layout, runDir) {
  const expected = assertInventoryContract(inventory, layout, runDir);
  const current = snapshotHqXwinCache(expected, runDir);
  if (JSON.stringify(current) !== JSON.stringify(inventory)) throw new Error("HQ candidate cargo-xwin cache no longer matches its persisted inventory");
  return current;
}

export function assertPersistedHqXwinCacheInventory(inventory, layout, runDir) {
  assertInventoryContract(inventory, layout, runDir);
  return inventory;
}

export function cleanupGovernedHqXwinCache(layout, runDir, { removeOwned = removeOwnedTree } = {}) {
  const expected = assertGovernedHqXwinCacheLayout(layout, runDir);
  const removed = removeOwned(runDir, expected.path, "HQ candidate cargo-xwin cache");
  assertAbsentLeaf(expected.path, "HQ candidate cargo-xwin cache");
  assertNoReparseAncestors(runDir, "HQ candidate evidence after cargo-xwin cache cleanup");
  return { attempted: true, status: "removed", path: expected.path, relativePath: removed.path };
}

export function assertGovernedHqXwinCacheRemoved(cleanup, layout, runDir) {
  const expected = assertGovernedHqXwinCacheLayout(layout, runDir);
  const expectedRelativePath = relative(resolve(runDir), expected.path).replaceAll("\\", "/");
  if (cleanup?.attempted !== true || cleanup?.status !== "removed" || !samePath(cleanup.path || "", expected.path) || cleanup.relativePath !== expectedRelativePath || cleanup.error) {
    throw new Error("HQ candidate cargo-xwin cache cleanup is incomplete");
  }
  try { assertAbsentLeaf(expected.path, "HQ candidate cargo-xwin cache"); } catch { throw new Error("HQ candidate cargo-xwin cache cleanup is incomplete"); }
  assertNoReparseAncestors(runDir, "HQ candidate evidence after cargo-xwin cache cleanup");
  return true;
}

export function governedHqDerivedBuildDirectory(kind, runDir) {
  const name = HQ_DERIVED_BUILD_DIRECTORY_NAMES[kind];
  if (!name) throw new Error(`unknown HQ candidate derived build directory: ${kind}`);
  return join(resolve(runDir), name);
}

export function assertGovernedHqDerivedBuildDirectory(kind, path, runDir) {
  const expected = governedHqDerivedBuildDirectory(kind, runDir);
  if (!samePath(path || "", expected)) throw new Error(`HQ candidate ${kind} must use the exact run-owned derived build directory`);
  assertPathInside(expected, runDir, `HQ candidate ${kind}`);
  return expected;
}

export function cleanupGovernedHqDerivedBuildDirectory(kind, path, runDir, { removeOwned = removeOwnedTree } = {}) {
  const expected = assertGovernedHqDerivedBuildDirectory(kind, path, runDir);
  const removed = removeOwned(runDir, expected, `HQ candidate ${kind}`);
  assertAbsentLeaf(expected, `HQ candidate ${kind}`);
  assertNoReparseAncestors(runDir, `HQ candidate evidence after ${kind} cleanup`);
  return { attempted: true, status: "removed", path: expected, relativePath: removed.path };
}

export function assertGovernedHqDerivedBuildDirectoryRemoved(kind, cleanup, path, runDir) {
  const expected = assertGovernedHqDerivedBuildDirectory(kind, path, runDir);
  const expectedRelativePath = relative(resolve(runDir), expected).replaceAll("\\", "/");
  if (cleanup?.attempted !== true || cleanup?.status !== "removed" || !samePath(cleanup.path || "", expected) || cleanup.relativePath !== expectedRelativePath || cleanup.error) {
    throw new Error(`HQ candidate ${kind} cleanup is incomplete`);
  }
  try { assertAbsentLeaf(expected, `HQ candidate ${kind}`); } catch { throw new Error(`HQ candidate ${kind} cleanup is incomplete`); }
  assertNoReparseAncestors(runDir, `HQ candidate evidence after ${kind} cleanup`);
  return true;
}
