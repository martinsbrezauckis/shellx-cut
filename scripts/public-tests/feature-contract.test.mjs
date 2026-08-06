#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { readFileSync, readdirSync } from "node:fs";
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

function listFiles(root, prefix = "") {
  return readdirSync(resolve(root, prefix), { withFileTypes: true }).flatMap((entry) => {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    return entry.isDirectory() ? listFiles(root, relative) : [relative];
  });
}

const schema = JSON.parse(read("schema/verbs.json"));
const verbs = schema.verbs.map((verb) => verb.name);
const domains = [...new Set(schema.verbs.map((verb) => verb.domain))].sort();
const tauriConf = JSON.parse(read("app/desktop/src-tauri/tauri.conf.json"));
const expectedTesterVersion = "0.6.106";
const tauriFrontendDist = tauriConf.build?.frontendDist;
const fallbackIndex = read("app/desktop/fallback/index.html");
const serverMain = read("app/server/src/main.rs");
const motionEffectsPanel = read("ui/src/panels/Inspector/MotionEffectsSection.tsx");
const motionTrackingPanel = read("ui/src/panels/Inspector/MotionTrackingSection.tsx");
const motionLineageGate = read("scripts/release/motion-lineage-gate.mjs");
const motionLineageProducer = read("scripts/release/motion-lineage-producer.mts");

for (const required of [
  "shellx-motion/cut-lineage-gate-producer@1",
  "motion.map_import",
  "motion_apply_import",
  "originAttestation",
  "CUTD_PROXY_ADDR",
]) {
  assert.match(motionLineageGate, new RegExp(escapeRegExp(required)), `Motion lineage gate is missing ${required}`);
}
assert.match(motionLineageProducer, /createLocalMotionSdk/, "Motion lineage gate must use the real local SDK producer");
assert.match(motionLineageProducer, /cutHandoff:\s*\{\s*target:\s*"shellx-cut"/, "Motion lineage producer must request a real Cut handoff");
assert.doesNotMatch(
  `${motionLineageGate}\n${motionLineageProducer}`,
  /playwright|chromium\.launch|launchBrowser/,
  "Motion lineage release gate must remain browser-free",
);

for (const selector of ["data-cut-motion-effects", "data-cut-motion-effect-layer"]) {
  assert.match(motionEffectsPanel, new RegExp(selector), `Motion effects UI is missing ${selector}`);
}
for (const selector of [
  "data-cut-motion-tracking",
  "data-cut-motion-tracking-analysis",
  "data-cut-motion-tracking-asset",
  "data-cut-motion-tracking-layer",
  "data-cut-motion-tracking-analyze",
  "data-cut-motion-tracking-apply",
  "data-cut-motion-tracking-verify",
  "data-cut-motion-tracking-detach",
]) {
  assert.match(motionTrackingPanel, new RegExp(selector), `Motion tracking UI is missing ${selector}`);
}
for (const verb of ["inventory", "request", "inspect", "apply", "verify", "detach"]) {
  assert.match(
    motionTrackingPanel,
    new RegExp(`motion\\.link\\.tracking\\.${verb}`),
    `Motion tracking UI is missing the ${verb} connector call`,
  );
}

assert.equal(schema.schema, "shellx-cut/verbs/1");
assert.equal(new Set(verbs).size, verbs.length, "schema/verbs.json must not contain duplicate verb names");
assert.equal(
  tauriConf.version,
  expectedTesterVersion,
  "Tauri config must be bumped for the next package when source changes after an installed build",
);

assert.equal(
  tauriFrontendDist,
  "../fallback",
  "Tauri frontendDist must point at the desktop fallback bundle",
);
assert.match(
  fallbackIndex,
  /engine_status/,
  "desktop fallback page must remain the engine-status airlock",
);
assert.doesNotMatch(
  fallbackIndex,
  /<div id="root"><\/div>|\/assets\/index-/,
  "desktop fallback page must not be overwritten by the generated full UI bundle",
);
assert.equal(
  tauriConf.bundle?.resources?.["../../../ui/dist"],
  "ui-dist",
  "Tauri resources must bundle ui/dist for the engine-served product UI",
);
assert.equal(
  tauriConf.bundle?.resources?.["../../../docs/public/DEBUG_API.md"],
  "agent-docs/docs/public/DEBUG_API.md",
  "Tauri resources must bundle the Debug API operator reference",
);
assert.equal(
  tauriConf.bundle?.resources?.["../../../docs/public/JUDGE_REVIEW.md"],
  "agent-docs/docs/public/JUDGE_REVIEW.md",
  "Tauri resources must bundle the judge-review honesty contract",
);
assert.equal(
  tauriConf.bundle?.resources?.["../../perception/py/judge"],
  "perception/judge",
  "Tauri resources must bundle the verify.judge subscription-CLI ladder",
);
for (const relative of [
  "judge.py",
  "adapters/ladder_judge.py",
  "adapters/cli_judge.py",
  "adapters/codex_judge.py",
  "adapters/antigravity_judge.py",
  "adapters/grok_judge.py",
]) {
  assert.doesNotThrow(
    () => read(`app/perception/py/judge/${relative}`),
    `Bundled judge module is missing: ${relative}`,
  );
}
for (const path of AGENT_DOC_PATHS) {
  assert.doesNotThrow(() => read(path), `Canonical installed agent-doc manifest points to a missing source file: ${path}`);
}
assert.deepEqual(
  listFiles(resolve(ROOT, "skill/shellx-cut")).sort(),
  AGENT_DOC_PATHS.filter((path) => path.startsWith("skill/shellx-cut/")).map((path) => path.slice("skill/shellx-cut/".length)).sort(),
  "Canonical installed agent-doc manifest must include every Cut skill and craft file",
);
assert.equal(
  tauriConf.bundle?.resources?.["../../../skill/shellx-cut"],
  "agent-docs/skill/shellx-cut",
  "Tauri resources must bundle the complete Cut agent skill directory",
);
for (const script of ["scripts/build-linux.sh", "scripts/build-macos.sh", "scripts/build-windows.sh"]) {
  assert.match(
    read(script),
    /node scripts\/lib\/agent-docs\.mjs --paths/,
    `${script} must consume the canonical installed agent-doc manifest`,
  );
}
const linuxBuild = read("scripts/build-linux.sh");
assert.match(
  linuxBuild,
  /rpm -K --nosignature[\s\S]+rpm2cpio[\s\S]+\[ -s "\$rpm_cpio" \][\s\S]+cpio --quiet -idmu[\s\S]+\.rpm agent-docs\/\$rel differs from source/,
  "Linux release builds must verify RPM integrity and compare every bundled agent doc byte-for-byte",
);
const windowsInstallSmoke = read("scripts/windows/install-cut-current.ps1");
assert.match(
  windowsInstallSmoke,
  /scripts\\lib\\agent-docs\.mjs[\s\S]+& \$node \$agentDocManifest --paths/,
  "Windows installed smoke must derive its exact resource checks from the canonical agent-doc manifest",
);
const windowsLoopbackProof = read("scripts/windows/process-loopback-proof.ps1");
assert.match(
  windowsLoopbackProof,
  /OutputRoot already exists/,
  "Windows process-loopback proof must refuse to overwrite prior evidence",
);
assert.match(
  windowsLoopbackProof,
  /sampleRate48k[\s\S]+stereo[\s\S]+duration[\s\S]+audible/,
  "Windows process-loopback proof must verify the full WAV and audibility contract",
);
assert.match(
  windowsLoopbackProof,
  /\$maxDb -gt -60\.0 -and \$meanDb -gt -70\.0/,
  "Windows process-loopback proof must reject effectively silent capture",
);
assert.match(
  windowsLoopbackProof,
  /shellx-cut\/windows-process-loopback-proof@1/,
  "Windows process-loopback proof must write its stable receipt schema",
);
assert.doesNotMatch(
  windowsLoopbackProof,
  /C:\\\\Users\\\\|\/home\/|wsl\.localhost/i,
  "Windows process-loopback proof must not embed workstation-specific paths",
);
assert.deepEqual(
  tauriConf.bundle?.windows?.signCommand,
  { cmd: "bash", args: ["../scripts/windows-artifact-sign.sh", "%1"] },
  "Windows signCommand must use structured arguments because release installer paths contain spaces",
);
assert.match(
  serverMain,
  /std::env::set_var\("PYTHONDONTWRITEBYTECODE", "1"\);/,
  "cutd must prevent packaged Python sidecars from writing bytecode into the signed app bundle",
);
assert.ok(
  serverMain.indexOf('std::env::set_var("PYTHONDONTWRITEBYTECODE", "1");')
    < serverMain.indexOf("let cli = Cli::parse();"),
  "the Python bytecode guard must be set before cutd starts commands or workers",
);

for (const script of ["scripts/build-windows.sh", "scripts/build-macos.sh"]) {
  const source = read(script);
  assert.match(
    source,
    /fallback_dir="app\/desktop\/fallback"/,
    `${script} must know the configured Tauri fallback web asset directory`,
  );
  assert.match(
    source,
    /rm -rf "\$fallback_dir\/assets"/,
    `${script} must clear stale generated fallback assets before packaging`,
  );
  assert.match(
    source,
    /grep -q "engine_status" "\$fallback_dir\/index\.html"/,
    `${script} must verify the fallback airlock was not overwritten`,
  );
  assert.doesNotMatch(
    source,
    /cp -R ui\/dist\/\. "\$fallback_dir\/"/,
    `${script} must not overwrite the desktop fallback airlock with ui/dist`,
  );
}

const desktopShell = read("app/desktop/src-tauri/src/lib.rs");
assert.match(desktopShell, /fn stop_owned_engine\(app: &tauri::AppHandle\)/);
assert.match(desktopShell, /child\.kill\(\)/);
assert.match(desktopShell, /child\.wait\(\)/);
assert.match(
  desktopShell,
  /fn validated_engine_origin[\s\S]+host_str\(\) != Some\("127\.0\.0\.1"\)[\s\S]+CapabilityBuilder::new\("engine-remote-selected"\)[\s\S]+\.remote\(origin\)/,
  "desktop must grant native helpers to the exact selected loopback engine origin",
);
for (const permission of [
  "dialog:allow-open",
  "dialog:allow-save",
  "dialog:allow-message",
  "core:event:allow-listen",
  "core:event:allow-unlisten",
]) {
  assert.match(
    desktopShell,
    new RegExp(`\\.permission\\("${escapeRegExp(permission)}"\\)`),
    `dynamic engine capability must retain ${permission}`,
  );
}
assert.doesNotMatch(
  desktopShell,
  /\.permission\("dialog:allow-(?:ask|confirm)"\)/,
  "desktop must not grant removed dialog command aliases",
);
assert.match(
  desktopShell,
  /#\[cfg\(feature = "webdriver-test"\)\]\s+let capability = capability[\s\S]+\.permission\("core:event:allow-emit-to"\)[\s\S]+\.permission\("wdio:allow-log-frontend"\);/,
  "remote engine content may emit targeted Tauri events and forward WDIO logs only in webdriver-test builds",
);
assert.equal(
  (desktopShell.match(/\.permission\("core:event:allow-emit-to"\)/g) || []).length,
  1,
  "the test-only targeted event-emission permission must not be granted elsewhere",
);
assert.equal(
  (desktopShell.match(/\.permission\("wdio:allow-log-frontend"\)/g) || []).length,
  1,
  "the WDIO console bridge permission must remain test-only",
);
assert.doesNotThrow(
  () => read("app/desktop/src-tauri/capabilities/default.json"),
  "desktop local-origin capability must remain present",
);
assert.match(desktopShell, /WindowEvent::Destroyed/);
assert.match(desktopShell, /RunEvent::ExitRequested/);
assert.match(desktopShell, /RunEvent::Exit/);

for (const rel of [
  "app/desktop/src-tauri/src/tools.rs",
  "app/server/src/doctor.rs",
  "app/server/src/fetch.rs",
  "docs/public/BUILDING.md",
]) {
  const source = read(rel);
  assert.match(
    source,
    /brew install ffmpeg-full/,
    `${rel} must direct macOS users to the complete Homebrew FFmpeg build`,
  );
}

{
  const windowsBuild = read("scripts/build-windows.sh");
  const macosBuild = read("scripts/build-macos.sh");
  assert.match(
    windowsBuild,
    /cleaning previous ShellX Cut package files/,
    "Windows build must clear stale package files before producing an installer",
  );
  assert.match(
    windowsBuild,
    /ShellX Cut_\*\.exe/,
    "Windows build cleanup must target old ShellX Cut setup executables",
  );
  assert.match(
    windowsBuild,
    /SHELLX_DISABLE_UPDATER_ARTIFACTS[\s\S]+unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD[\s\S]+createUpdaterArtifacts":false/,
    "Windows candidate builds must be able to disable updater artifacts even when a local updater key exists",
  );
  assert.match(
    macosBuild,
    /cleaning previous ShellX Cut DMGs/,
    "macOS build must clear stale DMGs before producing a package",
  );
  assert.match(
    macosBuild,
    /ShellX Cut\.app\.tar\.gz/,
    "macOS build cleanup must target old updater app archives",
  );
}

const registry = read("app/server/src/registry.rs");
assert.match(
  registry,
  new RegExp(`assert_eq!\\(\\s*reg\\.verbs\\.len\\(\\),\\s*${verbs.length},`),
  "registry count tripwire must match schema/verbs.json",
);

const reference = read("skill/shellx-cut/reference.md");
const referencePrelude = reference.split("\n## Invocation\n", 1)[0];
assert.doesNotMatch(
  referencePrelude,
  /changelog started|legacy verb count|155\/155|114 verbs|203\/203|SUPERSEDES|older .*coverage/i,
  "reference.md prelude must be compact/current, not an old changelog with superseded coverage counts",
);
for (const verb of verbs) {
  assert.match(
    reference,
    new RegExp(`\\|\\s*\`${escapeRegExp(verb)}\``),
    `skill/shellx-cut/reference.md is missing verb row ${verb}`,
  );
}

const debugApi = read("docs/public/DEBUG_API.md");
for (const endpoint of [
  "/api/verb/{name}",
  "/api/state",
  "/api/verbs",
  "/api/events",
  "/api/frame?at_ms=",
  "/api/agent",
  "/api/agent-doc/*path",
]) {
  assert.match(
    debugApi,
    new RegExp(escapeRegExp(endpoint)),
    `docs/public/DEBUG_API.md must document ${endpoint}`,
  );
}
assert.match(debugApi, /loopback-only, no token/i, "docs/public/DEBUG_API.md must document the local trust boundary");
assert.match(debugApi, /Origin \+ Host guard/i, "docs/public/DEBUG_API.md must document browser and DNS-rebinding protection");
assert.match(
  read("app/server/src/http.rs"),
  /\{"id": "debug-api", "path": "docs\/public\/DEBUG_API\.md", "url": "\/api\/agent-doc\/docs\/public\/DEBUG_API\.md"\}/,
  "/api/agent must advertise the bundled Debug API operator reference",
);
assert.match(
  read("app/server/src/http.rs"),
  /\{"id": "craft-index", "path": "skill\/shellx-cut\/craft\/INDEX\.md", "url": "\/api\/agent-doc\/skill\/shellx-cut\/craft\/INDEX\.md"\}/,
  "/api/agent must advertise the complete bundled craft guide index",
);

const featureInventory = read("docs/public/FEATURES.md");
const featureWorkflow = read("docs/public/FEATURE_CHANGE_WORKFLOW.md");
for (const required of [
  "schema/verbs.json",
  "app/server/src/registry.rs",
  "skill/shellx-cut/reference.md",
  "SKILL.md",
  "README.md",
  "scripts/coverage-audit.sh",
  "scripts/schema-validation-parity.mjs",
  "scripts/verbargs-sync.sh",
  "ui/public-tests/full-coverage-verify.mjs",
]) {
  assert.match(
    featureWorkflow,
    new RegExp(escapeRegExp(required)),
    `docs/public/FEATURE_CHANGE_WORKFLOW.md must name ${required} in the feature-sync workflow`,
  );
}
assert.match(featureInventory, /feature[\s*]+view/i, "docs/public/FEATURES.md must identify itself as the public feature view");
assert.match(featureWorkflow, /debug-API covered/i, "docs/public/FEATURE_CHANGE_WORKFLOW.md must define debug API coverage");
assert.doesNotMatch(
  featureInventory,
  /claude in v1|codex\/grok follow-up|codex\/grok chat not wired/i,
  "docs/public/FEATURES.md must not describe Codex/Grok chat as unwired",
);
assert.match(
  featureInventory,
  /AGENT CHAT[\s\S]*claude[\s\S]*codex[\s\S]*grok[\s\S]*wired/i,
  "docs/public/FEATURES.md must describe all wired Agent Chat CLIs",
);

for (const recentFeature of [
  "audio.dub",
  "media.diarize",
  "system.set_stt_model",
  "generate.from_prompt",
  "generate.storyboard",
  "debug.screenshot",
]) {
  assert.match(
    featureInventory,
    new RegExp(escapeRegExp(recentFeature)),
    `docs/public/FEATURES.md must include recent feature ${recentFeature}`,
  );
}

const skill = read("skill/shellx-cut/SKILL.md");
assert.match(skill, /reference\.md.*full/i, "SKILL.md must point agents at the full reference");
assert.match(
  skill,
  new RegExp(`Engine v${escapeRegExp(tauriConf.version)}\\b`),
  "SKILL.md engine version must match app/desktop/src-tauri/tauri.conf.json",
);
assert.match(
  skill,
  new RegExp(`${verbs.length} verbs across ${domains.length} domains`),
  "SKILL.md contract count must match schema/verbs.json",
);
assert.match(
  skill,
  new RegExp(`${verbs.length}/${verbs.length} REST \\+ ${verbs.length}/${verbs.length} MCP`),
  "SKILL.md coverage count must match schema/verbs.json",
);
assert.match(
  skill,
  /cutd mcp[\s\S]*live discovery[\s\S]*6161/i,
  "SKILL.md must tell installed-app agents that MCP uses live discovery before falling back to 6161",
);
assert.doesNotMatch(skill, /claude only in v1|codex\/grok are detected but NOT yet wired/i);
for (const recentFeature of ["audio.dub", "media.diarize", "generate.storyboard"]) {
  assert.match(skill, new RegExp(escapeRegExp(recentFeature)), `SKILL.md must mention ${recentFeature}`);
}
assert.match(skill, /claude[\s\S]*codex[\s\S]*grok/i, "SKILL.md must name all wired chat CLIs");

assert.doesNotMatch(reference, /claude wired in v1|codex\/grok = follow-up/i);
assert.match(reference, /codex exec[\s\S]*Grok project/i, "reference.md must describe Codex and Grok agent.chat wiring");
assert.match(
  reference,
  /live address discovery[\s\S]*falling back to 127\.0\.0\.1:6161/i,
  "reference.md must document fallback-port discovery for MCP/CLI agents",
);
const readme = read("README.md");
assert.match(readme, /schema\/verbs\.json.*contract/i, "README.md must name schema/verbs.json as the contract");
assert.match(
  readme,
  new RegExp(`${verbs.length} verbs across ${domains.length}`),
  "README.md contract count must match schema/verbs.json",
);
assert.match(
  readme,
  new RegExp(`${verbs.length}/${verbs.length}`),
  "README.md coverage count must match schema/verbs.json",
);
assert.match(readme, /scripts\/coverage-audit\.sh/, "README.md must tell contributors how to run REST+MCP coverage");
assert.match(
  readme,
  /scripts\/schema-validation-parity\.mjs/,
  "README.md must tell contributors how to run transport schema parity",
);
assert.match(readme, /skill\/shellx-cut\/reference\.md[\s\S]*full/i, "README.md must point at the full verb reference");
assert.match(readme, /docs\/public\/FEATURES\.md/, "README.md must point fresh users at the public feature inventory");
for (const systemVerb of [
  "system.doctor",
  "system.fetch_tool",
  "system.setup_perception",
  "system.setup_matte",
  "system.set_ffmpeg",
  "system.set_stt_model",
]) {
  assert.match(readme, new RegExp(escapeRegExp(systemVerb)), `README.md must document ${systemVerb}`);
}
const uiReadme = read("ui/public-tests/README.md");
assert.match(
  uiReadme,
  /When you add a surface[\s\S]+add a\s+check here in the same change/i,
  "ui/public-tests/README.md must require new surfaces to add harness coverage in the same change",
);

assert.match(
  reference,
  new RegExp(`${verbs.length} verbs across ${domains.length} domains`),
  "reference.md contract count must match schema/verbs.json",
);

const fullCoverage = read("ui/public-tests/full-coverage-verify.mjs");
const claudeFixture = read("scripts/release/fixtures/claude");
const agentEditFixture = read("scripts/release/fixtures/agent-edit-fixture.mjs");
assert.match(
  fullCoverage,
  /async function secMatte[\s\S]+freshProject\(page, ['"]matte['"], FACE\)/,
  "matte coverage must use the bounded detector-proven face role, not the long speech/menu role",
);
assert.match(fullCoverage, /const COVERED_VERBS\s*=\s*\[/, "full coverage harness must keep an explicit COVERED_VERBS set");
assert.match(fullCoverage, /const KNOWN_NON_UI_VERBS\s*=\s*\{/, "full coverage harness must keep an explicit KNOWN_NON_UI_VERBS set");
assert.match(
  fullCoverage,
  /A NEW engine verb must be ADDED to COVERED_VERBS[\s\S]+Nothing is silently skipped/,
  "full coverage harness must fail loudly when a new verb lacks a coverage decision",
);
assert.match(fullCoverage, /generate\.storyboard/, "full coverage harness must include the Generate storyboard surface");
assert.match(fullCoverage, /audio\.dub/, "full coverage harness must include the dubbing surface");
assert.match(fullCoverage, /media\.diarize/, "full coverage harness must include the diarization surface");
assert.match(
  agentEditFixture,
  /['"]x-cut-actor['"]:\s*process\.env\.CUTD_PROXY_ACTOR/,
  "agent fixture edits must use the per-turn proxy actor so review can claim them",
);
assert.match(
  fullCoverage,
  /const MENU_FIXTURE\s*=\s*SPEECH/,
  "menu fixture imports must use an engine-resolved media role on cross-host runs",
);
assert.doesNotMatch(
  fullCoverage,
  /talking_head\.perception\.json/,
  "full coverage must not depend on ignored machine-local perception receipts",
);
assert.match(
  fullCoverage,
  /engine ffmpeg lacks release filters[\s\S]+libvidstab[\s\S]+zscale/,
  "installed coverage must preflight the engine's full ffmpeg capabilities",
);

const coverageGate = read("scripts/release/full-coverage-gate.mjs");
assert.match(coverageGate, /CUT_DIARIZE_ENDPOINT/, "release full-coverage gate must preflight diarization service wiring");
assert.match(coverageGate, /CUT_DUB_ENDPOINT/, "release full-coverage gate must preflight dubbing service wiring");
assert.match(coverageGate, /Canary timestamp alignment/i, "release full-coverage gate must preflight Canary timestamp support");
assert.match(coverageGate, /role:\s*'FACE'[\s\S]+face_hq\.mp4/, "release full-coverage gate must require the real face role");

for (const rel of [
  "app/perception/py/matte_runner.py",
  "app/perception/py/matanyone_runner.py",
]) {
  const source = read(rel);
  assert.match(source, /"-fps_mode",\s*"cfr"/, `${rel} must support current bundled ffmpeg builds`);
  assert.doesNotMatch(source, /"-vsync"/, `${rel} must not use the removed ffmpeg -vsync option`);
}

for (const rel of [
  "docs/public/FEATURES.md",
  "README.md",
  "skill/shellx-cut/SKILL.md",
  "skill/shellx-cut/reference.md",
]) {
  assert.doesNotMatch(
    read(rel),
    /Canary lacks native word|BUILD DEFERRED to the optimization phase|Whisper retired from primary only when Canary lands/i,
    `${rel} must not describe Canary STT timestamps as deferred or missing`,
  );
}

const publicFeatures = read("docs/public/FEATURES.md");
assert.match(publicFeatures, /Whisper large-v3 compatibility\s+fallback/i, "docs/public/FEATURES.md must use the user-facing Whisper fallback label");
assert.doesNotMatch(publicFeatures, /WhisperX fallback/i, "docs/public/FEATURES.md must not lead with the implementation name WhisperX");

await import("./docs-release-contract.test.mjs");
await import("./ui-action-manifest.test.mjs");
await import("../module-size-gate.mjs");

console.log(`PASS feature-contract (${verbs.length} verbs, ${domains.length} domains)`);
