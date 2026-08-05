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
  const fetchSource = read("app/server/src/fetch.rs");
  for (const source of [desktopTools, fetchSource]) {
    assert.match(source, /ffmpeg-master-latest-win64-gpl\.zip/);
  }
  assert.doesNotMatch(desktopTools, /ffmpeg-master-latest-win64-lgpl-shared\.zip/);
  assert.match(desktopTools, /separate GPL-licensed runtime, not part of Cut/);
  assert.match(notice, /ShellX Cut installers do not contain FFmpeg or FFprobe/);
});
