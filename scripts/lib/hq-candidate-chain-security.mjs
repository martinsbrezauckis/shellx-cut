import { closeSync, constants as fsConstants, copyFileSync, existsSync, fstatSync, lstatSync, mkdirSync, openSync, realpathSync, writeSync } from "node:fs";
import { dirname, parse, relative, resolve } from "node:path";

import { artifactInfo } from "./ignored-test-rig.mjs";

function samePath(left, right) {
  const a = resolve(left);
  const b = resolve(right);
  return process.platform === "win32" ? a.toLowerCase() === b.toLowerCase() : a === b;
}

function ancestors(path) {
  const result = [];
  let current = resolve(path);
  while (true) {
    result.push(current);
    const parent = dirname(current);
    if (parent === current) return result.reverse();
    current = parent;
  }
}

function assertSafeExisting(path, label, { lstat = lstatSync, realpath = realpathSync.native } = {}) {
  const stat = lstat(path);
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    throw new Error(`${label} contains a symlink, junction, reparse point, or non-directory ancestor: ${path}`);
  }
  const canonical = realpath(path);
  if (!samePath(canonical, path)) {
    throw new Error(`${label} contains a symlink, junction, or reparse point ancestor: ${path}`);
  }
}

function regularArtifact(path, label) {
  if (!path || !existsSync(path)) throw new Error(`${label} is missing: ${path || "(empty path)"}`);
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size <= 0) throw new Error(`${label} must be a non-empty regular, non-symlink file: ${path}`);
  const info = artifactInfo(path);
  if (!info.sha256) throw new Error(`${label} could not be SHA-256 hashed: ${path}`);
  return info;
}

// lstat catches symbolic links and Windows junctions. realpath equality is a
// second check for reparse points that Node presents as directories.
export function assertNoReparseAncestors(path, label, dependencies = {}) {
  for (const ancestor of ancestors(path)) {
    if (existsSync(ancestor)) assertSafeExisting(ancestor, label, dependencies);
  }
  return resolve(path);
}

export function createSafeDirectories(path, label, { mkdir = mkdirSync, requireNewLeaf = false, ...dependencies } = {}) {
  const absolute = resolve(path);
  if (requireNewLeaf && existsSync(absolute)) throw new Error(`${label} creation raced with an existing path: ${absolute}`);
  const missing = [];
  let current = absolute;
  while (!existsSync(current)) {
    missing.push(current);
    const parent = dirname(current);
    if (parent === current) throw new Error(`${label} has no existing filesystem ancestor: ${absolute}`);
    current = parent;
  }
  assertNoReparseAncestors(current, label, dependencies);
  for (const directory of missing.reverse()) {
    try {
      mkdir(directory, { recursive: false, mode: 0o700 });
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      if (requireNewLeaf && directory === absolute) throw new Error(`${label} creation raced with an existing path: ${absolute}`);
    }
    assertSafeExisting(directory, label, dependencies);
  }
  assertNoReparseAncestors(absolute, label, dependencies);
  return absolute;
}

export function safeRegularArtifact(path, label, dependencies = {}) {
  const absolute = resolve(path);
  assertNoReparseAncestors(dirname(absolute), label, dependencies);
  const artifact = regularArtifact(absolute, label);
  // regularArtifact performs lstat on the leaf and refuses symlinks; ancestors
  // above are the only paths that must be directories here.
  assertNoReparseAncestors(dirname(absolute), label, dependencies);
  return artifact;
}

export function copySafeRegularFile(source, destination, label, dependencies = {}) {
  const input = safeRegularArtifact(source, label, dependencies);
  createSafeDirectories(dirname(destination), `${label} destination`, dependencies);
  assertNoReparseAncestors(destination, `${label} destination`, dependencies);
  copyFileSync(input.path, destination, fsConstants.COPYFILE_EXCL);
  const output = safeRegularArtifact(destination, `${label} staged copy`, dependencies);
  if (output.sha256 !== input.sha256 || output.bytes !== input.bytes) {
    throw new Error(`${label} staged copy changed while being created`);
  }
  return output;
}

export function writeSafeText(path, text, label, dependencies = {}) {
  const destination = resolve(path);
  createSafeDirectories(dirname(destination), `${label} destination`, dependencies);
  assertNoReparseAncestors(dirname(destination), `${label} destination`, dependencies);
  let descriptor;
  try {
    descriptor = openSync(destination, "wx", 0o600);
    const opened = fstatSync(descriptor);
    if (!opened.isFile() || opened.nlink !== 1) throw new Error(`${label} creation did not yield one regular file`);
    writeSync(descriptor, text, undefined, "utf8");
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  return safeRegularArtifact(destination, label, dependencies);
}

export function writeSafeJson(path, value, label, dependencies = {}) {
  return writeSafeText(path, `${JSON.stringify(value, null, 2)}\n`, label, dependencies);
}

export function assertPathInside(path, root, label) {
  const relation = relative(resolve(root), resolve(path));
  if (!relation || relation.startsWith("..") || parse(relation).root) {
    throw new Error(`${label} must remain below ${resolve(root)}`);
  }
  return resolve(path);
}
