#!/usr/bin/env node
import { strict as assert } from "node:assert";

import { base64ToBuffer, safeJsonParse, stripPngDataUrl } from "../lib/safe-data.mjs";

const png1x1 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";

const bytes = base64ToBuffer(png1x1);
assert.ok(Buffer.isBuffer(bytes), "decoder returns a Buffer");
assert.ok(bytes.length > 32, "decoder returns PNG bytes");
assert.equal(bytes[0], 0x89, "PNG signature byte is preserved");

assert.equal(stripPngDataUrl(`data:image/png;base64,${png1x1}`), png1x1);
assert.throws(() => base64ToBuffer("abc#$"), /Invalid base64/);
assert.throws(() => base64ToBuffer("abcd"), /Decoded data is not a PNG/);
const tinyJpeg = Buffer.from([0xff, 0xd8, 0xff, 0xd9]).toString("base64");
assert.deepEqual([...base64ToBuffer(tinyJpeg, { expectPng: false })], [0xff, 0xd8, 0xff, 0xd9]);
assert.deepEqual(safeJsonParse('{"ok":true,"items":[1,2]}'), { ok: true, items: [1, 2] });
assert.throws(() => safeJsonParse('{"__proto__":{"polluted":true}}'), /Unsafe JSON key/);
assert.throws(() => safeJsonParse('{"constructor":{"prototype":{"polluted":true}}}'), /Unsafe JSON key/);
assert.equal(safeJsonParse("not json", { fallback: null }), null);

console.log("PASS safe-data");
