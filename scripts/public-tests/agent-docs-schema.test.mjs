import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { AGENT_DOCS_SCHEMA } from "../lib/agent-docs.mjs";

const files = {
  server: new URL("../../app/server/src/http.rs", import.meta.url),
  windowsInstall: new URL("../windows/install-cut-current.ps1", import.meta.url),
  debugApi: new URL("../../docs/public/DEBUG_API.md", import.meta.url),
};

test("installed agent-doc producers, consumers, and operator docs share one schema", async () => {
  const source = Object.fromEntries(await Promise.all(
    Object.entries(files).map(async ([name, url]) => [name, await readFile(url, "utf8")]),
  ));

  assert.equal(AGENT_DOCS_SCHEMA, "shellx-cut/agent-docs/2");
  for (const [name, text] of Object.entries(source)) {
    assert.match(text, new RegExp(AGENT_DOCS_SCHEMA.replaceAll("/", "\\/")), `${name} schema drift`);
    assert.doesNotMatch(text, /shellx-cut\/agent-docs\/1/, `${name} retains the retired schema`);
  }
});
