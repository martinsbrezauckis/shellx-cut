import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmod, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const linuxUrl = new URL("../linux-wdio-full-coverage.mjs", import.meta.url);
const linuxCleanupUrl = new URL("../lib/linux-native-run-cleanup.sh", import.meta.url);
const windowsUrl = new URL("../windows-installed-full-coverage.mjs", import.meta.url);
const linuxPackageHelperUrl = new URL("../lib/retain-linux-shipping-packages.sh", import.meta.url);
const macUrl = new URL("../macos-wdio-track-controls.mjs", import.meta.url);
const sshStdinEnvUrl = new URL("../lib/ssh-stdin-env.mjs", import.meta.url);
const fullAgentFixtureEnvUrl = new URL("../lib/native-full-agent-fixture-env.mjs", import.meta.url);
const windowsNativeActionUrl = new URL("../release/native-os-action-windows.ps1", import.meta.url);
const desktopCargoUrl = new URL("../../app/desktop/src-tauri/Cargo.toml", import.meta.url);
const linuxBuildUrl = new URL("../build-linux.sh", import.meta.url);
const macBuildUrl = new URL("../build-macos.sh", import.meta.url);
const windowsBuildUrl = new URL("../build-windows.sh", import.meta.url);
const nativeIntegrityUrl = new URL("../lib/native-artifact-integrity.mjs", import.meta.url);
const fullCoverageUrl = new URL("../../ui/public-tests/full-coverage-verify.mjs", import.meta.url);
const nativePageAdapterUrl = new URL("../../ui/public-tests/lib/webdriverIoPage.mjs", import.meta.url);
const isolatedNativeAppUrl = new URL("../lib/run-isolated-native-app.sh", import.meta.url);

test("shipping builds select the ShellX Cut app instead of a helper binary", async () => {
  const cargo = await readFile(desktopCargoUrl, "utf8");
  const linux = await readFile(linuxBuildUrl, "utf8");
  const mac = await readFile(macBuildUrl, "utf8");
  const windows = await readFile(windowsBuildUrl, "utf8");
  assert.match(cargo, /default-run = "shellx-cut"/);
  assert.match(linux, /Built application at: [.]\*\/shellx-cut\$/);
  assert.match(mac, /Built application at: [.]\*\/shellx-cut\$/);
  assert.match(windows, /Built application at: [.]\*\/shellx-cut\[.]exe\$/);
  for (const source of [linux, mac]) {
    assert.match(source, /verify-updater-signature/);
    assert.match(source, /selected the updater verifier helper as a shipping executable/);
  }
});

test("Windows Authenticode integrity does not rely on empty PowerShell positional args", async () => {
  const source = await readFile(nativeIntegrityUrl, "utf8");
  assert.match(source, /powerShellSingleQuoted/);
  assert.match(source, /replaceAll\("'", "''"\)/);
  assert.doesNotMatch(source, /\$args\[0\]|\$args\[1\]/);
  assert.match(source, /'-NoProfile', '-NonInteractive', '-Command', script/);
});

test("native candidate runners isolate user state and never claim installed proof", async () => {
  for (const [surface, source] of [
    ["linux", await readFile(linuxUrl, "utf8")],
    ["macOS", await readFile(macUrl, "utf8")],
  ]) {
    assert.match(source, /SHELLX_CUT_HOME=/, `${surface} isolates internal user data`);
    assert.match(source, /SHELLX_CUT_PROJECTS_DIR=/, `${surface} isolates managed projects`);
    assert.match(source, /SHELLX_CUT_WDIO_APP_CWD=/, `${surface} isolates native app relative output`);
    assert.match(source, /SHELLX_CUT_WDIO_REAL_APP=/, `${surface} preserves the exact tested app binary`);
    assert.match(source, /run-isolated-native-app[.]sh/, `${surface} launches through the shared cwd fence`);
    assert.match(source, /INSTALLED_APP=0/, `${surface} keeps candidate receipts honest`);
    assert.match(source, /FCV_SOURCE_CONTENT_MANIFEST_SHA256=/, `${surface} binds synchronized source content`);
    assert.match(source, /--strict-candidate-actions/, `${surface} names strict candidate proof honestly`);
    assert.match(source, /FCV_REQUIRE_FULL=/, `${surface} can enforce the complete runtime`);
    assert.match(source, /FCV_FINAL_ALL_ACTIONS=/, `${surface} can enforce every registered UI action`);
    assert.match(source, /FCV_NATIVE_ACTION_CONTROLLER=/, `${surface} pairs WebView actions with the host OS dialog controller`);
    assert.match(source, /NODE_OPTIONS=--max-old-space-size=8192/, `${surface} gives the single whole-app gate enough heap`);
    assert.match(source, /--features webdriver-test/, `${surface} opts into test instrumentation`);
    assert.doesNotMatch(source, /pkill -x/, `${surface} does not broadly terminate shared processes`);
  }
  const macSource = await readFile(macUrl, "utf8");
  const linuxSource = await readFile(linuxUrl, "utf8");
  assert.match(linuxSource, /--real-screen-record/);
  assert.match(linuxSource, /installedFinal \|\| realScreenRecord/);
  assert.match(macSource, /--host <ssh-host>/);
  assert.match(macSource, /SHELLX_CUT_MAC_HOST/);
  assert.match(macSource, /--real-screen-record/);
  assert.match(macSource, /FCV_REAL_SCREEN_RECORD=/);
  assert.match(macSource, /FCV_NATIVE_EXPECTED_PROCESS=shellx-cut/);
  assert.match(macSource, /FCV_TARGET_SURFACE=macos-installed/);
  assert.doesNotMatch(macSource, /FCV_TARGET_SURFACE_VALUE/);
  assert.match(macSource, /SSH_KEEPALIVE_ARGS/);
  assert.match(
    macSource,
    /[/]usr[/]bin[/]caffeinate -dimsu -t 14400 npm --prefix ui run/,
    "the Mac runner keeps the unlocked console active for the bounded qualification",
  );
  assert.match(
    macSource,
    /CGSSessionScreenIsLocked[\s\S]+unlock it before native UI qualification/,
    "the Mac runner fails closed before building when native UI is behind the login lock",
  );
});

test("native app launcher fences relative output outside governed source", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "shellx-cut-native-app-cwd-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const sourceDir = join(root, "source", "ui");
  const appCwd = join(root, "run", "app-cwd");
  const fakeApp = join(root, "fake-app.sh");
  await mkdir(sourceDir, { recursive: true });
  await mkdir(appCwd, { recursive: true });
  await writeFile(fakeApp, "#!/bin/sh\nprintf '%s\\n' \"$PWD\" > observed-cwd.txt\nprintf '%s\\n' \"$*\" > observed-args.txt\n");
  await chmod(fakeApp, 0o700);

  const launched = spawnSync(
    fileURLToPath(isolatedNativeAppUrl),
    ["--probe", "value with spaces"],
    {
      cwd: sourceDir,
      encoding: "utf8",
      env: {
        ...process.env,
        SHELLX_CUT_WDIO_REAL_APP: fakeApp,
        SHELLX_CUT_WDIO_APP_CWD: appCwd,
      },
    },
  );
  assert.equal(launched.status, 0, launched.stderr);
  assert.equal((await readFile(join(appCwd, "observed-cwd.txt"), "utf8")).trim(), appCwd);
  assert.equal((await readFile(join(appCwd, "observed-args.txt"), "utf8")).trim(), "--probe value with spaces");
  assert.equal((await readdir(sourceDir)).length, 0, "the source cwd remains untouched");
});

test("native candidate runners require an explicitly configured SSH host", async () => {
  for (const [surface, source] of [
    ["linux", await readFile(linuxUrl, "utf8")],
    ["macOS", await readFile(macUrl, "utf8")],
  ]) {
    assert.match(source, /--host <ssh-host>/, `${surface} documents a generic host argument`);
    assert.match(source, /if \(!host\) throw new Error/, `${surface} fails closed without a host`);
    assert.match(
      source,
      /const host = arg\('--host', process\.env\.SHELLX_CUT_(?:LINUX|MAC)_HOST \|\| ''\)/,
      `${surface} has no built-in host alias`,
    );
  }
});

test("native page instrumentation does not clone binary media responses", async () => {
  const source = await readFile(nativePageAdapterUrl, "utf8");
  assert.equal(source.includes("responsePath.startsWith('/api/verb/')"), true);
  assert.match(source, /then\(async \(response\) =>/);
  assert.match(source, /const text = await response[.]clone\(\)[.]text\(\)/);
  assert.match(source, /if [(]json === undefined[)] entry[.]text = text/);
  assert.doesNotMatch(
    source,
    /state[.]events[.]push[(][{][\s\S]{0,220}\btext,\s*json,/,
    "a verb response must not retain duplicate text and parsed JSON bodies",
  );
  assert.match(
    source,
    /SHELLX_CUT_WDIO_PROVIDER[^\n]+external[\s\S]+__wdioConsoleCleanup/,
    "external shipping-app sessions remove the test service console wrapper instead of requiring a test-only app ACL",
  );
  assert.match(
    source,
    /addEventListener\('mousedown', recordMouseDownAction, true\)/,
    "native action evidence observes explicit controls whose real behavior completes on mousedown",
  );
});

test("native candidate runners accept distinct high-quality media roles", async () => {
  for (const [surface, source] of [
    ["linux", await readFile(linuxUrl, "utf8")],
    ["macOS", await readFile(macUrl, "utf8")],
  ]) {
    for (const role of ["scene", "speech", "face", "speakers", "second"]) {
      assert.match(source, new RegExp(`--${role}-clip`), `${surface} exposes the ${role} fixture`);
    }
    assert.match(source, /RELEASE_CLIP_FACE=/);
    assert.match(source, /RELEASE_CLIP_SPEAKERS=/);
    assert.match(source, /RELEASE_CLIP2=/);
    assert.doesNotMatch(source, /\bremoteClip\b|\bremoteLibraryClip\b/, `${surface} has no stale legacy path variables`);
  }
});

test("full native runners stage deterministic external seams without weakening UI action proof", async () => {
  const fixtureEnv = await readFile(fullAgentFixtureEnvUrl, "utf8");
  for (const [surface, source] of [
    ["linux", await readFile(linuxUrl, "utf8")],
    ["macOS", await readFile(macUrl, "utf8")],
  ]) {
    assert.match(source, /FCV_AGENT_FIXTURES_VALUE/, `${surface} scopes fixture activation to full runs`);
    assert.match(source, /FULL_AGENT_FIXTURE_SHELL/, `${surface} installs the shared external-seam environment`);
    // Either quoting form proves the wiring: Linux passes the value inline on
    // the remote command ("$VAR"), macOS writes it into a hosted command file
    // through an UNQUOTED heredoc, which expands at write time and leaves the
    // literal value wrapped in single quotes ('1'). Pinning only the double
    // quoted form would fail a correct runner (verified: unquoted heredoc +
    // single quotes yields export FCV_AGENT_FIXTURES='1', runtime value 1).
    assert.match(source, /FCV_AGENT_FIXTURES=(["'])\$FCV_AGENT_FIXTURES_VALUE\1/, `${surface} tells result assertions when the external seam is deterministic`);
  }
  assert.match(fixtureEnv, /scripts\/release\/fixtures:\$PATH/, "the shared environment launches staged CLI fixtures");
  assert.match(fixtureEnv, /CUTD_DRAFT_ADAPTER=/, "the shared environment stages deterministic comment drafting");
  assert.match(fixtureEnv, /CUTD_GENERATE_PROMPT_ADAPTER=/, "the shared environment stages deterministic Generate prompt planning");
  assert.match(fixtureEnv, /CUTD_GENERATE_STORYBOARD_ADAPTER=/, "the shared environment stages deterministic Generate storyboard planning");
  assert.match(fixtureEnv, /CUTD_JUDGE_ADAPTER=/, "the shared environment stages deterministic judge output");
});

test("native candidate runners expose the no-project drop-to-create suite", async () => {
  for (const [surface, source] of [
    ["linux", await readFile(linuxUrl, "utf8")],
    ["macOS", await readFile(macUrl, "utf8")],
  ]) {
    assert.match(source, /drop-to-create/, `${surface} exposes the focused onboarding suite`);
    assert.match(source, /SHELLX_CUT_WDIO_IMAGE=/, `${surface} passes a real still image`);
    assert.match(source, /SHELLX_CUT_WDIO_DROP_CASE=/, `${surface} can isolate each fresh-app drop case`);
    assert.match(source, /wdio:native-drop-to-create/, `${surface} selects the shared native spec`);
  }
});

test("Linux native runner uses a bounded virtual display and current source", async () => {
  const source = await readFile(linuxUrl, "utf8");
  const cleanup = await readFile(linuxCleanupUrl, "utf8");
  assert.match(source, /SSH_KEEPALIVE_ARGS/);
  assert.match(source, /xvfb-run -a/);
  assert.match(source, /1600x900x24/);
  assert.match(
    source,
    /runtime_dir="\$WDIO_OUT_RESOLVED\/runtime"[\s\S]+mkdir -p "\$runtime_dir"[\s\S]+chmod 700 "\$runtime_dir"/,
    "the Linux run owns a private mode-0700 runtime directory for portal state",
  );
  assert.match(
    source,
    /XDG_RUNTIME_DIR="\$runtime_dir" setsid xvfb-run -a -s '[^']+' dbus-run-session -- env/,
    "the private runtime, virtual display, and isolated D-Bus session share one portal environment",
  );
  assert.match(
    cleanup,
    /\/proc\/self\/mountinfo[\s\S]+FAIL: cleanup blocked by mounted portal runtime[\s\S]+rm -rf "\$runtime_dir"/,
    "the private portal runtime is removed only when no live mount can hang cleanup",
  );
  assert.match(cleanup, /cleanup_status[\s\S]+exit 86/, "a blocked runtime cleanup fails the host gate");
  assert.match(source, /[.] scripts\/lib\/linux-native-run-cleanup[.]sh/);
  assert.match(source, /FCV_TARGET_SURFACE=linux-control/);
  assert.match(source, /rsync/);
});

test("Linux installed-final mode drives the extracted shipping package externally", async () => {
  const source = await readFile(linuxUrl, "utf8");
  assert.match(source, /--installed-final/);
  assert.match(source, /TAURI_FEATURES="" scripts\/build-linux[.]sh release/);
  assert.match(source, /dpkg-deb -x/);
  assert.match(source, /artifact_root="\$WDIO_OUT_RESOLVED\/artifacts"/);
  assert.match(
    source,
    /retained_deb="\$\(bash scripts\/lib\/retain-linux-shipping-packages[.]sh "\$bundle_root" "\$artifact_root"\)"/,
  );
  assert.doesNotMatch(source, /\\\$\{#(?:debs|rpms)\[@\]\}/);
  assert.match(source, /sha256sum/);
  assert.match(source, /WDIO_PROVIDER=external/);
  assert.match(source, /INSTALLED_APP=1/);
  assert.match(source, /NATIVE_PROVIDER=external/);
  assert.match(source, /FCV_INSTALLED_APP="\$INSTALLED_APP"/);
  assert.match(source, /source-content-manifest[.]mjs/);
  assert.match(source, /FCV_NATIVE_PROVIDER="\$NATIVE_PROVIDER"/);
  assert.match(source, /FCV_REAL_SCREEN_RECORD="\$FCV_REAL_SCREEN_RECORD_VALUE"/);
  assert.match(source, /FCV_REAL_SCREEN_RECORD_VALUE=\$\{installedFinal \|\| realScreenRecord [?:] '1' [?:] '0'\}/);
  assert.match(source, /FCV_ACTION_MANIFEST="\$REMOTE_DIR_RESOLVED\/ui\/public-tests\/full-ui-action-manifest[.]json"/);
  assert.match(source, /SOURCE_COMMIT=/);
  assert.match(source, /FCV_SOURCE_GIT_COMMIT="\$SOURCE_COMMIT"/);
  assert.match(source, /FCV_INSTALLED_RUNTIME_RECEIPT="\$FCV_INSTALLED_RUNTIME_RECEIPT_VALUE"/);
  assert.match(source, /linux-installed-walkthrough-receipt[.]mjs --start/);
  assert.match(source, /linux-installed-walkthrough-receipt[.]mjs --finish/);
  assert.doesNotMatch(
    source,
    /--installed-final[\s\S]{0,400}--features webdriver-test/,
    "installed-final description must not claim the shipping package uses test instrumentation",
  );
});

test("Linux installed-final package retention accepts exactly one fresh deb and rpm", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "shellx-cut-linux-packages-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const bundle = join(root, "bundle");
  const artifacts = join(root, "artifacts");
  await mkdir(join(bundle, "deb"), { recursive: true });
  await mkdir(join(bundle, "rpm"), { recursive: true });
  await writeFile(join(bundle, "deb", "ShellX Cut_0.6.105_amd64.deb"), "deb");
  await writeFile(join(bundle, "rpm", "ShellX Cut-0.6.105-1.x86_64.rpm"), "rpm");

  const retained = spawnSync(
    "bash",
    [fileURLToPath(linuxPackageHelperUrl), bundle, artifacts],
    { encoding: "utf8" },
  );
  assert.equal(retained.status, 0, retained.stderr);
  assert.equal(
    retained.stdout.trim(),
    join(artifacts, "ShellX Cut_0.6.105_amd64.deb"),
  );
  assert.deepEqual(
    (await readdir(artifacts)).sort(),
    ["ShellX Cut-0.6.105-1.x86_64.rpm", "ShellX Cut_0.6.105_amd64.deb"],
  );

  await writeFile(join(bundle, "deb", "unexpected-second.deb"), "deb");
  const ambiguous = spawnSync(
    "bash",
    [fileURLToPath(linuxPackageHelperUrl), bundle, join(root, "ambiguous")],
    { encoding: "utf8" },
  );
  assert.notEqual(ambiguous.status, 0);
  assert.match(ambiguous.stderr, /expected exactly one fresh shipping [.]deb/);
});

test("native runners can clean rebuildable output without deleting retained evidence", async () => {
  for (const [surface, source] of [
    ["linux", `${await readFile(linuxUrl, "utf8")}\n${await readFile(linuxCleanupUrl, "utf8")}`],
    ["macOS", await readFile(macUrl, "utf8")],
  ]) {
    assert.match(source, /--clean-after/, `${surface} exposes explicit post-test cleanup`);
    assert.match(source, /rm -rf app\/target app\/desktop\/src-tauri\/target ui\/node_modules ui\/dist/);
    assert.match(source, /"\$WDIO_OUT_RESOLVED\/app-home"/, `${surface} removes the isolated app profile`);
    assert.match(source, /"\$WDIO_OUT_RESOLVED\/projects"/, `${surface} removes throwaway test projects`);
    assert.doesNotMatch(source, /rm -rf "\$WDIO_OUT_RESOLVED"/, `${surface} preserves evidence`);
  }
});

test("Windows skip-build restores the cleaned native UI harness before launch", async () => {
  const source = await readFile(windowsUrl, "utf8");
  assert.match(source, /ui\/node_modules\/playwright\/package[.]json/);
  assert.match(source, /await run\('npm', \['--prefix', 'ui', 'ci', '--no-audit', '--no-fund'\]\)/);
});

test("Linux native runner owns the whole WebDriver process group and retains its exit", async () => {
  const source = `${await readFile(linuxUrl, "utf8")}\n${await readFile(linuxCleanupUrl, "utf8")}`;
  assert.match(source, /setsid xvfb-run/);
  assert.match(source, /gate_pid=\$!/);
  assert.match(source, /kill -TERM -- "-\$gate_pid"/);
  assert.match(source, /kill -KILL -- "-\$gate_pid"/);
  assert.match(source, /"\$WDIO_OUT_RESOLVED\/[.]wdio-exit-code"/);
});

test("native export waits drain queued WebDriver request and response events", async () => {
  const source = await readFile(fullCoverageUrl, "utf8");
  const eventDrains = source.match(/await page[.]flushEvents[?][.]\(\)/g) || [];
  assert.ok(eventDrains.length >= 2, "default and Save As export waits must drain native bridge events");
});

test("traced native runs retain exact action ids and timed-out verb UI state", async () => {
  const adapter = await readFile(new URL("../../ui/public-tests/lib/webdriverIoPage.mjs", import.meta.url), "utf8");
  const source = await readFile(fullCoverageUrl, "utf8");
  assert.match(adapter, /Array[.]isArray[(]entry[.]actions[)]/);
  assert.match(adapter, /entry[.]actions[.]join\(','\)/);
  assert.match(source, /\[fcv-response-timeout\] verb=\$\{name\}/);
  assert.match(source, /firstOutput: value\('\[data-cut-render-queue-output="0"\]'\)/);
  assert.match(source, /applyDisabled: document[.]querySelector\('\[data-cut-kinetic-apply\]'\)/);
  assert.match(source, /result: document[.]querySelector\('\[data-cut-studio-result\]'\)/);
});

test("remote native runners stream optional Anthropic credentials without argv or disk persistence", async () => {
  for (const [surface, source] of [
    ["linux", await readFile(linuxUrl, "utf8")],
    ["macOS", await readFile(macUrl, "utf8")],
  ]) {
    assert.doesNotMatch(source, /pass-secret|pass-ref/, `${surface} has no private password-store workflow`);
    assert.match(source, /readEnvFirstLine\('ANTHROPIC_API_KEY'\)/, `${surface} reads the standard environment boundary`);
    assert.match(source, /buildSshEnvPayload/, `${surface} builds a stdin-only SSH payload`);
  }
  const helper = await readFile(sshStdinEnvUrl, "utf8");
  assert.match(helper, /ServerAliveInterval=15/);
  assert.match(helper, /ServerAliveCountMax=24/);
  assert.match(helper, /TCPKeepAlive=yes/);
  assert.match(helper, /IFS= read -r \$\{name\}/);
  assert.match(helper, /input: `\$\{value\}\\n\$\{script\}`/);
  assert.doesNotMatch(helper, /\$\{name\}=\$\{value\}/);
});

test("Windows native picker replaces the existing filename before selecting a path", async () => {
  const source = await readFile(windowsNativeActionUrl, "utf8");
  assert.ok(
    source.includes(String.raw`StartsWith("\\?\UNC\",`) &&
      source.includes("Substring(8)") &&
      source.includes(String.raw`StartsWith("\\?\",`) &&
      source.includes("Substring(4)"),
    "the controller must convert Windows verbatim paths to file-picker paths",
  );
  assert.match(
    source,
    /SendWait\("%n"\)[\s\S]+SendWait\("\^a"\)[\s\S]+SendWait\(\$pickerPath\)/,
    "the controller must select-all instead of appending to the picker filename",
  );
});
