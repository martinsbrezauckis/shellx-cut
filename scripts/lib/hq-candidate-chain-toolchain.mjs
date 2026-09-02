import { spawn, spawnSync } from "node:child_process";
import { closeSync, existsSync, openSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

import { oneLine } from "./hq-media-identity.mjs";
import { assertNoReparseAncestors, createSafeDirectories, safeRegularArtifact, writeSafeText } from "./hq-candidate-chain-security.mjs";
import { governedHqXwinCacheLayout } from "./hq-candidate-chain-xwin-cache.mjs";

const AMBIENT_TOOL_PREFIX = /^(CARGO_|RUST|RUSTUP_|GITHUB_TOKEN$|CC$|CXX$|LINK$|AR$|RANLIB$|LIB$|LIBPATH$|INCLUDE$|VCINSTALLDIR$|VSINSTALLDIR$)/i;
const AMBIENT_XWIN_PREFIX = /^XWIN_/i;

export function assertNoAmbientXwinOverrides(environment = process.env) {
  const names = Object.keys(environment).filter((name) => AMBIENT_XWIN_PREFIX.test(name) && environment[name]);
  if (names.length) throw new Error(`HQ candidate build refuses inherited cargo-xwin environment: ${names.sort().join(", ")}`);
  return true;
}

export function assertNoAmbientToolDrift(environment = process.env) {
  assertNoAmbientXwinOverrides(environment);
  const names = Object.keys(environment).filter((name) => AMBIENT_TOOL_PREFIX.test(name) && environment[name]);
  if (names.length) throw new Error(`HQ candidate build refuses inherited Cargo/Rust/toolchain environment: ${names.sort().join(", ")}`);
  return true;
}

function commandPath(command, locate = process.platform === "win32" ? "where.exe" : "which") {
  const resolved = spawnSync(locate, [command], { encoding: "utf8", windowsHide: true });
  if (resolved.status !== 0) throw new Error(`required HQ build tool '${command}' is not on PATH`);
  const executable = String(resolved.stdout || "").split(/\r?\n/).find(Boolean)?.trim();
  if (!executable) throw new Error(`required HQ build tool '${command}' did not resolve to an executable`);
  return executable;
}

function pinnedTool(command, args, dependencies = {}) {
  const executable = commandPath(command, dependencies.locate);
  const result = spawnSync(executable, args, { encoding: "utf8", windowsHide: true });
  if (result.status !== 0) throw new Error(`${command} ${args.join(" ")} failed: ${oneLine(result.stderr || result.stdout || result.error?.message)}`);
  const version = String(result.stdout || result.stderr || "").split(/\r?\n/).find(Boolean)?.trim();
  if (!version) throw new Error(`${command} did not report a version`);
  return { command, executable, version, artifact: safeRegularArtifact(executable, `${command} executable`) };
}

export function pinHqToolchain(dependencies = {}) {
  return {
    cargo: pinnedTool("cargo", ["--version"], dependencies),
    rustc: pinnedTool("rustc", ["--version"], dependencies),
    cargoXwin: pinnedTool("cargo-xwin", ["--version"], dependencies),
    linker: pinnedTool("lld-link.exe", ["--version"], dependencies),
  };
}

export function assertPinnedToolchain(toolchain) {
  for (const [name, tool] of Object.entries(toolchain || {})) {
    const current = safeRegularArtifact(tool?.executable, `${name} pinned executable`);
    if (current.sha256 !== tool?.artifact?.sha256 || current.bytes !== tool?.artifact?.bytes) {
      throw new Error(`HQ candidate build tool changed after pinning: ${name}`);
    }
  }
  return true;
}

export function governedHqCargoLayout(runDir) {
  const cargoHome = join(runDir, "cargo-home");
  const userProfile = join(runDir, "cargo-profile");
  return {
    cargoHome,
    userProfile,
    configPath: join(cargoHome, "config.toml"),
    // This contains only deterministic transport policy. Target/linker paths
    // remain explicit environment values recorded in the build receipt.
    config: "[net]\ngit-fetch-with-cli = false\n",
  };
}

export function prepareGovernedHqCargoHome(layout) {
  createSafeDirectories(layout.cargoHome, "HQ candidate Cargo home", { requireNewLeaf: true });
  createSafeDirectories(layout.userProfile, "HQ candidate Cargo profile", { requireNewLeaf: true });
  return writeSafeText(layout.configPath, layout.config, "HQ candidate Cargo config");
}

export function isolatedHqCargoEnvironment(layout) {
  return {
    USERPROFILE: layout.userProfile,
    HOME: layout.userProfile,
    HOMEDRIVE: "",
    HOMEPATH: "",
    CARGO_HOME: layout.cargoHome,
  };
}

// Cargo's rustup proxy needs its separately installed toolchain store. Derive
// that store from the pinned cargo location rather than accepting inherited
// CARGO/RUSTUP environment. Cargo configuration itself remains fresh and
// checkout-owned through CARGO_HOME and the empty USERPROFILE/HOME.
export function rustupHomeForPinnedCargo(cargoExecutable) {
  const cargoHome = dirname(dirname(resolve(cargoExecutable)));
  if (basename(cargoHome).toLowerCase() !== ".cargo") throw new Error("HQ candidate Cargo executable is not in a conventional .cargo/bin tool root");
  const rustupHome = join(dirname(cargoHome), ".rustup");
  if (!existsSync(rustupHome)) throw new Error("HQ candidate Cargo toolchain store is unavailable beside the pinned cargo executable");
  assertNoReparseAncestors(rustupHome, "HQ candidate Rustup toolchain store");
  return rustupHome;
}

export function hqCandidateBuildSpec(runDir, toolchain, baseEnvironment = process.env) {
  assertNoAmbientToolDrift(baseEnvironment);
  assertPinnedToolchain(toolchain);
  const targetDir = join(runDir, "cargo-target");
  const cargoLayout = governedHqCargoLayout(runDir);
  const xwinCache = governedHqXwinCacheLayout(runDir);
  const rustupHome = rustupHomeForPinnedCargo(toolchain.cargo.executable);
  const toolDirs = [...new Set(Object.values(toolchain).map((tool) => dirname(tool.executable)))];
  const systemRoot = baseEnvironment.SystemRoot || baseEnvironment.WINDIR;
  if (!systemRoot) throw new Error("HQ candidate build requires Windows SystemRoot without inheriting toolchain overrides");
  const environment = {
    SystemRoot: systemRoot,
    WINDIR: baseEnvironment.WINDIR || systemRoot,
    ComSpec: baseEnvironment.ComSpec || join(systemRoot, "System32", "cmd.exe"),
    ...isolatedHqCargoEnvironment(cargoLayout),
    RUSTUP_HOME: rustupHome,
    TEMP: join(runDir, "tmp"),
    TMP: join(runDir, "tmp"),
    PATH: [...toolDirs, join(systemRoot, "System32")].join(";"),
    CARGO_TARGET_DIR: targetDir,
    XWIN_CACHE_DIR: xwinCache.path,
    CARGO: toolchain.cargo.executable,
    RUSTC: toolchain.rustc.executable,
    CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER: toolchain.linker.executable,
    RUSTFLAGS: "-C target-feature=+crt-static",
    SHELLX_DISABLE_UPDATER_ARTIFACTS: "1",
    SHELLX_WINDOWS_SIGNING_REQUIRED: "0",
  };
  return {
    command: toolchain.cargoXwin.executable,
    args: ["build", "--manifest-path", "app/Cargo.toml", "--package", "server", "--bin", "cutd", "--target", "x86_64-pc-windows-msvc", "--release"],
    environment,
    cargoLayout,
    xwinCache,
    rustupHome,
    output: join(targetDir, "x86_64-pc-windows-msvc", "release", "cutd.exe"),
  };
}

export function runLogged(command, args, { cwd, env, logPath, spawnProcess = spawn } = {}) {
  return new Promise((resolveRun, reject) => {
    let log;
    try {
      log = openSync(logPath, "wx", 0o600);
    } catch (error) {
      reject(error);
      return;
    }
    const close = () => { try { closeSync(log); } catch {} };
    const child = spawnProcess(command, args, { cwd, env, windowsHide: true, stdio: ["ignore", log, log] });
    child.once("error", (error) => { close(); reject(new Error(`${command} could not start: ${error.message}`)); });
    child.once("exit", (code, signal) => {
      close();
      const artifact = safeRegularArtifact(logPath, "HQ candidate build log");
      if (code === 0) resolveRun({ command, args, cwd, environment: env, log: artifact });
      else reject(new Error(`${command} ${args.join(" ")} failed: code=${code} signal=${signal || "none"}; log: ${logPath}; log-sha256: ${artifact.sha256}`));
    });
  });
}
