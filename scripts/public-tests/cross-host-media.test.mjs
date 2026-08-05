import assert from "node:assert/strict";

import {
  basenameHostPath,
  dirnameHostPath,
  joinHostPath,
  resolveDriverPath,
} from "../lib/cross-host-media.mjs";

assert.equal(basenameHostPath("C:\\Users\\Example\\Downloads\\talkinghead_hq.mp4"), "talkinghead_hq.mp4");
assert.equal(dirnameHostPath("C:\\Users\\Example\\Downloads\\talkinghead_hq.mp4"), "C:\\Users\\Example\\Downloads");
assert.equal(dirnameHostPath("C:\\clip.mp4"), "C:\\");
assert.equal(basenameHostPath("/Users/example/Downloads/talkinghead_hq.mp4"), "talkinghead_hq.mp4");
assert.equal(dirnameHostPath("/Users/example/Downloads/talkinghead_hq.mp4"), "/Users/example/Downloads");
assert.equal(joinHostPath("C:\\Users\\Example\\Downloads", "fixture.mp4"), "C:\\Users\\Example\\Downloads\\fixture.mp4");

assert.equal(
  resolveDriverPath("C:\\Users\\Example\\AppData\\Local\\ShellX Cut\\perception\\.venv\\Scripts\\python.exe", {
    platform: "linux",
    isWsl: true,
  }),
  "/mnt/c/Users/Example/AppData/Local/ShellX Cut/perception/.venv/Scripts/python.exe",
);

assert.equal(
  resolveDriverPath("D:\\tools\\python.exe", { platform: "linux", isWsl: true }),
  "/mnt/d/tools/python.exe",
);

assert.equal(
  resolveDriverPath("\\\\?\\C:\\Users\\Example\\Documents\\ShellX Cut Projects\\fcv_menus.cutproj", {
    platform: "linux",
    isWsl: true,
  }),
  "/mnt/c/Users/Example/Documents/ShellX Cut Projects/fcv_menus.cutproj",
);

assert.equal(
  resolveDriverPath("C:\\tools\\python.exe", { platform: "linux", isWsl: false }),
  "C:\\tools\\python.exe",
);

assert.equal(
  resolveDriverPath("C:\\tools\\python.exe", { platform: "darwin", isWsl: true }),
  "C:\\tools\\python.exe",
);

assert.equal(
  resolveDriverPath("/usr/bin/python3", { platform: "linux", isWsl: true }),
  "/usr/bin/python3",
);

console.log("PASS cross-host-media");
