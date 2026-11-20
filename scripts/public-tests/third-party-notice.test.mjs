#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

function read(rel, encoding = "utf8") {
  return readFileSync(resolve(ROOT, rel), encoding);
}

function sha256(rel) {
  return createHash("sha256").update(read(rel, null)).digest("hex");
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

test("bundled model assets retain complete packaged attribution", () => {
  const notice = read("NOTICE");
  const tauriConf = JSON.parse(read("app/desktop/src-tauri/tauri.conf.json"));
  assert.equal(
    tauriConf.bundle?.resources?.["../../../NOTICE"],
    "NOTICE",
    "Tauri resources must bundle NOTICE beside the model assets",
  );

  const modelNotices = [
    {
      path: "app/perception/py/blaze_face_short_range.tflite",
      sha: "b4578f35940bf5a1a655214a1cce5cab13eba73c1297cd78e1a04c2380b0152f",
      license: /MediaPipe BlazeFace[\s\S]+Apache License 2\.0/,
      source: /storage\.googleapis\.com\/mediapipe-models\/face_detector\/blaze_face_short_range/,
    },
    {
      path: "app/perception/py/face_detection_yunet_2023mar.onnx",
      sha: "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
      license: /OpenCV Zoo YuNet[\s\S]+License: MIT/,
      source: /github\.com\/opencv\/opencv_zoo\/tree\/main\/models\/face_detection_yunet/,
    },
  ];
  for (const model of modelNotices) {
    assert.equal(sha256(model.path), model.sha, `${model.path} must remain the attributed upstream artifact`);
    assert.match(notice, new RegExp(escapeRegExp(model.path)), `NOTICE must name ${model.path}`);
    assert.match(notice, new RegExp(model.sha), `NOTICE must pin the SHA-256 for ${model.path}`);
    assert.match(notice, model.license, `NOTICE must state the license for ${model.path}`);
    assert.match(notice, model.source, `NOTICE must state the upstream source for ${model.path}`);
  }
});

test("FFmpeg guidance matches the consented separate runtime", () => {
  const notice = read("NOTICE");
  const desktopTools = read("app/desktop/src-tauri/src/tools.rs");
  const pins = read("app/media/src/tool_setup/download.rs");
  // The built-in download installs the GPL static builds, pinned by file.
  assert.match(pins, /ffmpeg-n9\.0\.2-39-g7adfbe40dc-win64-gpl-9\.0\.zip/);
  assert.match(pins, /ffmpeg-n9\.0\.2-39-g7adfbe40dc-linux64-gpl-9\.0\.tar\.xz/);
  assert.match(desktopTools, /win64-gpl zip/);
  assert.doesNotMatch(desktopTools + pins, /lgpl/);
  assert.match(desktopTools, /separate GPL-licensed runtime, not part of Cut/);
  assert.match(notice, /ShellX Cut installers do not contain FFmpeg or FFprobe/);
});

// The generated THIRD-PARTY-NOTICES.txt lists the libraries compiled into the programs and bundled into the UI.
// This test is the cheap gate (pure Node, no cargo): it fails when a lockfile changed and the file was not
// regenerated, when the file stops being bundled, or when it names a package the lockfiles no longer hold.
const LOCKFILES = ["app/Cargo.lock", "app/desktop/src-tauri/Cargo.lock", "ui/package-lock.json"];

// The npm lockfile is digested over its production packages only (the public export rewrites the file to the public
// dependency subset; the production closure is the same).
function lockDigest(rel) {
  const text = read(rel).replace(/\r\n?/g, "\n");
  if (rel !== "ui/package-lock.json") return createHash("sha256").update(text).digest("hex");
  const lines = Object.entries(JSON.parse(text).packages ?? {})
    .filter(([path, entry]) => path && !entry.dev && !entry.link)
    .map(([path, entry]) => `${path} ${entry.version} ${entry.integrity ?? ""}`)
    .sort();
  return createHash("sha256").update(`${lines.join("\n")}\n`).digest("hex");
}

test("third-party notices are bundled beside NOTICE in every installer", () => {
  const tauriConf = JSON.parse(read("app/desktop/src-tauri/tauri.conf.json"));
  assert.equal(
    tauriConf.bundle?.resources?.["../../../THIRD-PARTY-NOTICES.txt"],
    "THIRD-PARTY-NOTICES.txt",
    "Tauri resources (shared by the Windows, macOS and Linux bundles) must carry THIRD-PARTY-NOTICES.txt",
  );
  assert.equal(tauriConf.bundle?.resources?.["../../../NOTICE"], "NOTICE");
  assert.ok(read("THIRD-PARTY-NOTICES.txt").length > 100_000, "the notices file must carry the licence texts");
});

test("third-party notices are current for the lockfiles", () => {
  const notices = read("THIRD-PARTY-NOTICES.txt");
  for (const rel of LOCKFILES) {
    assert.ok(
      notices.includes(`  ${rel}${rel === "ui/package-lock.json" ? " (production packages)" : ""}  ${lockDigest(rel)}\n`),
      `${rel} changed since THIRD-PARTY-NOTICES.txt was generated: regenerate it with the maintainers' release tooling`,
    );
  }
});

test("third-party notices carry licence texts and name the direct dependencies", () => {
  const notices = read("THIRD-PARTY-NOTICES.txt");
  assert.match(notices, /Permission is hereby granted, free of charge/);
  assert.match(notices, /Apache License\s+Version 2\.0, January 2004/);
  assert.match(notices, /captions Python packages/);
  const uiPackage = JSON.parse(read("ui/package.json"));
  for (const name of Object.keys(uiPackage.dependencies ?? {})) {
    assert.match(notices, new RegExp(`^${escapeRegExp(name)} \\d`, "m"), `UI dependency ${name} must be listed`);
  }
  for (const crate of ["tauri", "tauri-plugin-single-instance", "tauri-plugin-dialog", "axum", "tokio", "serde"]) {
    assert.match(notices, new RegExp(`^${escapeRegExp(crate)} \\d.*\\[rust`, "m"), `crate ${crate} must be listed`);
  }
  assert.doesNotMatch(notices, /^(shellx-cut|cut-core|cut-media|server) \d.*\[rust/m, "first-party crates are not third-party");
});

test("every Rust package in the notices is in a lockfile with the same version", () => {
  const notices = read("THIRD-PARTY-NOTICES.txt");
  const locked = new Set();
  for (const rel of ["app/Cargo.lock", "app/desktop/src-tauri/Cargo.lock"]) {
    for (const block of read(rel).split("[[package]]").slice(1)) {
      const name = block.match(/^name = "([^"]+)"/m)?.[1];
      const version = block.match(/^version = "([^"]+)"/m)?.[1];
      if (name && version) locked.add(`${name} ${version}`);
    }
  }
  const listed = [...notices.matchAll(/^(\S+) (\S+)  \[rust, /gm)].map((m) => `${m[1]} ${m[2]}`);
  assert.ok(listed.length > 300, "the notices must list the shipped Rust crates");
  assert.deepEqual(listed.filter((entry) => !locked.has(entry)), [], "notices list a crate version that no lockfile holds");
});
