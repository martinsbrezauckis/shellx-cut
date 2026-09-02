import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { AGENT_DOCS } from "../lib/agent-docs.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (path) => readFileSync(resolve(ROOT, path), "utf8");

test("local-machine trust docs stay tied to bind and HTTP guard behavior", () => {
  const security = read("SECURITY.md");
  const readme = read("README.md");
  const debugApi = read("docs/public/DEBUG_API.md");
  const startHere = read("START_HERE_FOR_AGENT.txt");
  const skill = read("skill/shellx-cut/SKILL.md");
  const reference = read("skill/shellx-cut/reference.md");
  const features = read("docs/public/FEATURES.md");
  const report = read("docs/public/shellx-cut-threat-model.md");
  const manual = read("docs/public/site/manual/cut/index.html");
  const http = read("app/server/src/http.rs");
  const tauri = read("app/desktop/src-tauri/tauri.conf.json");

  for (const [path, text] of [
    ["SECURITY.md", security],
    ["README.md", readme],
    ["DEBUG_API.md", debugApi],
    ["START_HERE_FOR_AGENT.txt", startHere],
    ["SKILL.md", skill],
    ["reference.md", reference],
    ["FEATURES.md", features],
    ["manual", manual],
  ]) {
    const normalized = text.replace(/\s+/g, " ");
    assert.match(
      normalized,
      /personal workstation \/ one trusted interactive environment/i,
      `${path} must name the supported local deployment`,
    );
    assert.match(normalized, /machine-wide|whole-machine|whole local machine/i, `${path} must reject same-user isolation`);
    assert.match(normalized, /remote use.{0,160}(requires|supported only through).{0,160}authenticat/is, `${path} must require authenticated remote transport`);
  }

  assert.match(security, /NOT A DEFECT/, "SECURITY.md must state the accepted deployment decision");
  assert.match(security, /Sec-Fetch-Site: cross-site/, "SECURITY.md must document the no-Origin browser guard");
  const normalizedDebug = debugApi.replace(/\s+/g, " ");
  assert.match(normalizedDebug, /native caller can omit `Origin` and can forge/i);
  assert.match(normalizedDebug, /do \*\*not\*\* authenticate native local callers/i);
  assert.match(normalizedDebug, /MCP.*inherits this machine-wide boundary/i);
  assert.match(report, /No verb, schema entry, or MCP tool changes/i);
  assert.match(report, /Sec-Fetch-Site: cross-site/, "threat model must document the no-Origin browser guard");
  assert.match(http, /pub const DEFAULT_ADDR: &str = "127\.0\.0\.1:6161"/);
  assert.match(http, /if authority_is_loopback\(addr\) \{\s*return Ok\(\(\)\);/);
  assert.match(http, /if let Some\(origin\) = headers/);
  assert.match(http, /get\("sec-fetch-site"\)/);
  assert.match(http, /if let Some\(host\) = headers/);
  assert.match(http, /async fn guard_rejects_cross_origin_and_no_origin_cross_site_requests/);
  assert.match(http, /status\(&u, None, None\)[\s\S]*assert_eq!\(\s*no_origin,\s*200/);
  assert.match(http, /Some\("http:\/\/127\.0\.0\.1"\), None\)[\s\S]*assert_eq!\(\s*loopback,\s*200/);
  assert.match(http, /Some\("http:\/\/evil\.com"\), None\)[\s\S]*assert_eq!\(\s*cross,\s*403/);
  assert.match(http, /status\(&u, None, Some\("cross-site"\)\)[\s\S]*assert_eq!\(\s*cross_site_no_origin,\s*403/);

  const localTrust = AGENT_DOCS.find((doc) => doc.id === "local-trust");
  assert.deepEqual(localTrust, {
    id: "local-trust",
    path: "docs/public/shellx-cut-threat-model.md",
    advertised: true,
  });
  assert.match(
    http,
    /\{"id": "local-trust", "path": "docs\/public\/shellx-cut-threat-model\.md", "url": "\/api\/agent-doc\/docs\/public\/shellx-cut-threat-model\.md"\}/,
  );
  assert.match(http, /path == "docs\/public\/shellx-cut-threat-model\.md"/);
  assert.match(
    tauri,
    /"\.\.\/\.\.\/\.\.\/docs\/public\/shellx-cut-threat-model\.md": "agent-docs\/docs\/public\/shellx-cut-threat-model\.md"/,
  );
});
