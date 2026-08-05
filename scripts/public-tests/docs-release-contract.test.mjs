#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { AGENT_DOC_PATHS } from "../lib/agent-docs.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

function read(rel) {
  return readFileSync(resolve(ROOT, rel), "utf8");
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const craftGuideCount = AGENT_DOC_PATHS.filter(
  (path) => path.startsWith("skill/shellx-cut/craft/") && !path.endsWith("/INDEX.md"),
).length;

const readme = read("README.md");
assert.match(
  readme,
  new RegExp(`craft/ layer \\(${craftGuideCount} editing-craft guides`),
  "README.md craft-guide count must match the canonical installed agent-doc manifest",
);
assert.doesNotMatch(
  readme,
  /cross-product module-extraction plan/,
  "README.md repo map must not advertise a missing private planning document",
);

const skill = read("skill/shellx-cut/SKILL.md");
assert.match(skill, /all 7 receipt checks PASS/, "SKILL.md completion example must count every listed receipt check");
assert.doesNotMatch(skill, /all 6 receipt checks PASS/, "SKILL.md must not retain the stale six-check completion claim");
for (const [client, setup] of [
  ["Claude Code", "claude mcp add --scope user shellx-cut"],
  ["Codex", "codex mcp add shellx-cut"],
  ["Grok Build", "grok mcp add --scope user shellx-cut"],
]) {
  assert.match(
    skill,
    new RegExp(escapeRegExp(setup)),
    `SKILL.md must give ${client} users a concrete MCP setup command`,
  );
}
assert.match(
  skill,
  /Codex stores[\s\S]{0,100}[.]codex\/config[.]toml/,
  "SKILL.md must explain Codex MCP configuration scope",
);
for (const required of [
  ".gemini/config/mcp_config.json",
  ".agents/mcp_config.json",
  ".gemini/antigravity-cli/settings.json",
  "mcp(shellx-cut/system_mcp_test)",
  "--dangerously-skip-permissions",
]) {
  assert.match(
    skill,
    new RegExp(escapeRegExp(required)),
    `SKILL.md must include Antigravity MCP guidance for ${required}`,
  );
}
assert.match(
  skill,
  /For every client, call `system[.]mcp_test \{\}`/,
  "SKILL.md must make Cut's same-engine self-test universal",
);

const startHere = read("START_HERE_FOR_AGENT.txt");
for (const required of [
  "engine.addr",
  "system.mcp_test",
  "project.list",
  "ui.* verbs require a connected app UI",
  "Antigravity CLI",
]) {
  assert.match(
    startHere,
    new RegExp(escapeRegExp(required)),
    `START_HERE_FOR_AGENT.txt must include ${required}`,
  );
}

const debugApi = read("docs/public/DEBUG_API.md");
for (const required of [
  "claude mcp add --scope user shellx-cut",
  "codex mcp add shellx-cut",
  "codex mcp get shellx-cut --json",
  "grok mcp add --scope user shellx-cut",
  "grok mcp doctor shellx-cut",
  ".gemini/config/mcp_config.json",
  ".agents/mcp_config.json",
  ".gemini/antigravity-cli/settings.json",
  "mcp(shellx-cut/system_mcp_test)",
  "agy --print",
  "--dangerously-skip-permissions",
  "all four clients",
  "Configuration presence alone is not a live handshake",
  "all 260 tools",
  "same running Cut engine",
]) {
  assert.match(
    debugApi,
    new RegExp(escapeRegExp(required)),
    `docs/public/DEBUG_API.md must include ${required}`,
  );
}

for (const sourcePath of ["app/server/src/dispatch/ui_system.rs", "app/server/src/ui_bridge.rs"]) {
  const source = read(sourcePath);
  assert.match(source, /system[.]doctor/, `${sourcePath} no-UI recovery must route through live address discovery`);
  assert.doesNotMatch(
    source,
    /open http:\/\/127[.]0[.]0[.]1:6161/,
    `${sourcePath} no-UI recovery must not hardcode the default port`,
  );
}

const uiReadme = read("ui/public-tests/README.md");
assert.doesNotMatch(
  uiReadme,
  /Music bed[\s\S]{0,120}legacy `?\.mb-/i,
  "ui/public-tests/README.md must not retain the completed Music Bed legacy-markup follow-up",
);

const buildingGuide = read("docs/public/BUILDING.md");
assert.match(
  buildingGuide,
  /verify-accessibility-surfaces/,
  "docs/public/BUILDING.md must include the live accessibility surface gate",
);

const testRigsGuide = read("docs/public/TEST_RIGS.md");
for (const required of [
  "--final-all-actions",
  "FCV_REQUIRE_FULL=1",
  "FCV_INSTALLED_APP=1",
  "FCV_ACTION_MANIFEST",
  "FCV_SECTION",
  "FCV_ONLY",
  "FCV_NO_AGENT",
  "macOS",
  "Windows",
  "native Linux",
  "PRESENT",
  "RENDER",
  "CLICK",
  "RESULT",
]) {
  assert.match(
    testRigsGuide,
    new RegExp(escapeRegExp(required)),
    `docs/public/TEST_RIGS.md final installed gate must name ${required}`,
  );
}
assert.match(
  testRigsGuide,
  /candidate[\s\S]+cannot satisfy installed release proof/i,
  "docs/public/TEST_RIGS.md must reject candidate receipts as installed proof",
);
assert.match(
  testRigsGuide,
  /linux-wdio-full-coverage[.]mjs[\s\S]+--installed-final/,
  "docs/public/TEST_RIGS.md must name the exact native Linux shipping-package adapter",
);
assert.match(
  testRigsGuide,
  /windows-installed-full-coverage[.]mjs[\s\S]+--unattended-native-ui[\s\S]+--clean-after/,
  "docs/public/TEST_RIGS.md must include the Windows focus-safety opt-in",
);
assert.match(
  testRigsGuide,
  /macOS remains release-red/i,
  "docs/public/TEST_RIGS.md must keep macOS red until an exact installed WKWebView adapter exists",
);

const serverHttp = read("app/server/src/http.rs");
for (const path of AGENT_DOC_PATHS.filter(
  (value) => !value.startsWith("skill/shellx-cut/"),
)) {
  assert.match(
    serverHttp,
    new RegExp(`path == "${escapeRegExp(path)}"`),
    `installed agent-doc HTTP allowlist must serve canonical manifest entry ${path}`,
  );
}
