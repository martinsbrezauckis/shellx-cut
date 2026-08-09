//! Native fake generation CLI shared by the assets.generate lifecycle tests.
//!
//! The production generator is an executable, so these tests need one too.  A
//! small Rust binary gives every supported test platform the same stdin prompt
//! parsing, reference assertion, delay, copy, and cancellation behavior without
//! turning the Windows case into a separate batch-script approximation.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = "SHELLX_CUT_TEST_GENERATION_FIXTURE";
const VARIATION_FIXTURE: &str = "SHELLX_CUT_TEST_GENERATION_VARIATION_FIXTURE";
const VARIATION_TRIGGER: &str = "SHELLX_CUT_TEST_GENERATION_VARIATION_TRIGGER";
const REQUIRE_REFERENCE_TRIGGER: &str = "SHELLX_CUT_TEST_GENERATION_REQUIRE_REFERENCE_TRIGGER";
const INVOCATION_LOG: &str = "SHELLX_CUT_TEST_GENERATION_INVOCATION_LOG";
const DEFAULT_DELAY_MS: &str = "SHELLX_CUT_TEST_GENERATION_DEFAULT_DELAY_MS";
const EXTRA_DELAY_TRIGGER: &str = "SHELLX_CUT_TEST_GENERATION_EXTRA_DELAY_TRIGGER";
const EXTRA_DELAY_MS: &str = "SHELLX_CUT_TEST_GENERATION_EXTRA_DELAY_MS";

const STUB_SOURCE: &str = r#"
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::Duration;

const FIXTURE: &str = "SHELLX_CUT_TEST_GENERATION_FIXTURE";
const VARIATION_FIXTURE: &str = "SHELLX_CUT_TEST_GENERATION_VARIATION_FIXTURE";
const VARIATION_TRIGGER: &str = "SHELLX_CUT_TEST_GENERATION_VARIATION_TRIGGER";
const REQUIRE_REFERENCE_TRIGGER: &str = "SHELLX_CUT_TEST_GENERATION_REQUIRE_REFERENCE_TRIGGER";
const INVOCATION_LOG: &str = "SHELLX_CUT_TEST_GENERATION_INVOCATION_LOG";
const DEFAULT_DELAY_MS: &str = "SHELLX_CUT_TEST_GENERATION_DEFAULT_DELAY_MS";
const EXTRA_DELAY_TRIGGER: &str = "SHELLX_CUT_TEST_GENERATION_EXTRA_DELAY_TRIGGER";
const EXTRA_DELAY_MS: &str = "SHELLX_CUT_TEST_GENERATION_EXTRA_DELAY_MS";

fn fail(message: &str, code: i32) -> ! {
    eprintln!("{message}");
    process::exit(code);
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key).map(PathBuf::from)
}

fn sleep_from_env(key: &str) {
    let millis = env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if millis > 0 {
        thread::sleep(Duration::from_millis(millis));
    }
}

fn output_path(prompt: &str) -> Option<&str> {
    let mut after_output_instruction = false;
    for line in prompt.lines() {
        if after_output_instruction && !line.trim().is_empty() {
            return Some(line.trim());
        }
        after_output_instruction = line.contains("EXACTLY this path");
    }
    None
}

fn main() {
    let mut prompt = String::new();
    io::stdin()
        .read_to_string(&mut prompt)
        .unwrap_or_else(|error| fail(&format!("read generation prompt: {error}"), 2));

    let output = output_path(&prompt)
        .filter(|path| Path::new(path).is_absolute())
        .unwrap_or_else(|| fail("generation prompt did not contain an absolute output path", 4));

    if let Ok(trigger) = env::var(REQUIRE_REFERENCE_TRIGGER) {
        if !trigger.is_empty() && prompt.contains(&trigger) && !prompt.contains("Reference 1:") {
            fail("generation prompt omitted Reference 1", 3);
        }
    }

    if let Some(log) = env_path(INVOCATION_LOG) {
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap_or_else(|error| fail(&format!("open invocation log: {error}"), 5));
        writeln!(log, "run").unwrap_or_else(|error| fail(&format!("write invocation log: {error}"), 5));
    }

    sleep_from_env(DEFAULT_DELAY_MS);
    if let Ok(trigger) = env::var(EXTRA_DELAY_TRIGGER) {
        if !trigger.is_empty() && prompt.contains(&trigger) {
            sleep_from_env(EXTRA_DELAY_MS);
        }
    }

    let source = match env::var(VARIATION_TRIGGER) {
        Ok(trigger) if !trigger.is_empty() && prompt.contains(&trigger) => env_path(VARIATION_FIXTURE),
        _ => env_path(FIXTURE),
    }
    .unwrap_or_else(|| fail("fake generation CLI has no output fixture", 6));

    let output = PathBuf::from(output);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| fail(&format!("create output directory: {error}"), 7));
    }
    fs::copy(&source, &output).unwrap_or_else(|error| {
        fail(
            &format!("copy fake generation fixture {}: {error}", source.display()),
            8,
        )
    });
    let json_path = output
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    println!("{{\"ok\":true,\"path\":\"{json_path}\",\"summary\":\"fake generation\"}}");
}
"#;

pub(super) struct FakeGenerationCliConfig {
    fixture: Option<PathBuf>,
    variation: Option<(String, PathBuf)>,
    require_reference_trigger: Option<String>,
    invocation_log: Option<PathBuf>,
    default_delay_ms: u64,
    extra_delay: Option<(String, u64)>,
}

impl FakeGenerationCliConfig {
    pub(super) fn copying(fixture: PathBuf) -> Self {
        Self {
            fixture: Some(fixture),
            variation: None,
            require_reference_trigger: None,
            invocation_log: None,
            default_delay_ms: 0,
            extra_delay: None,
        }
    }

    pub(super) fn waiting() -> Self {
        Self {
            fixture: None,
            variation: None,
            require_reference_trigger: None,
            invocation_log: None,
            default_delay_ms: 0,
            extra_delay: None,
        }
    }

    pub(super) fn with_variation(mut self, trigger: &str, fixture: PathBuf) -> Self {
        self.variation = Some((trigger.into(), fixture));
        self
    }

    pub(super) fn require_reference_for(mut self, trigger: &str) -> Self {
        self.require_reference_trigger = Some(trigger.into());
        self
    }

    pub(super) fn log_invocations(mut self, path: PathBuf) -> Self {
        self.invocation_log = Some(path);
        self
    }

    pub(super) fn with_default_delay_ms(mut self, delay_ms: u64) -> Self {
        self.default_delay_ms = delay_ms;
        self
    }

    pub(super) fn with_extra_delay_if_prompt(mut self, trigger: &str, delay_ms: u64) -> Self {
        self.extra_delay = Some((trigger.into(), delay_ms));
        self
    }
}

pub(super) struct FakeGenerationCli {
    previous_env: Vec<(&'static str, Option<OsString>)>,
}

impl FakeGenerationCli {
    pub(super) fn install(dir: &Path, config: FakeGenerationCliConfig) -> Self {
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("create fake generation CLI directory");
        let source = bin.join("fake_generation_cli.rs");
        std::fs::write(&source, STUB_SOURCE).expect("write fake generation CLI source");
        let program = bin.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let compiled = Command::new(rustc)
            .args(["--edition=2021"])
            .arg(&source)
            .arg("-o")
            .arg(&program)
            .output()
            .expect("start rustc for fake generation CLI");
        assert!(
            compiled.status.success(),
            "compile fake generation CLI: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );

        let mut cli = Self {
            previous_env: Vec::new(),
        };
        cli.set_path(&bin);
        cli.set_optional_path(FIXTURE, config.fixture);
        if let Some((trigger, fixture)) = config.variation {
            cli.set(VARIATION_TRIGGER, trigger);
            cli.set_optional_path(VARIATION_FIXTURE, Some(fixture));
        } else {
            cli.set_optional_path(VARIATION_TRIGGER, None);
            cli.set_optional_path(VARIATION_FIXTURE, None);
        }
        cli.set_optional_string(REQUIRE_REFERENCE_TRIGGER, config.require_reference_trigger);
        cli.set_optional_path(INVOCATION_LOG, config.invocation_log);
        cli.set(DEFAULT_DELAY_MS, config.default_delay_ms.to_string());
        if let Some((trigger, delay_ms)) = config.extra_delay {
            cli.set(EXTRA_DELAY_TRIGGER, trigger);
            cli.set(EXTRA_DELAY_MS, delay_ms.to_string());
        } else {
            cli.set_optional_path(EXTRA_DELAY_TRIGGER, None);
            cli.set_optional_path(EXTRA_DELAY_MS, None);
        }
        cli
    }

    fn set_path(&mut self, bin: &Path) {
        let old_path = std::env::var_os("PATH");
        let mut paths = vec![bin.to_path_buf()];
        if let Some(path) = old_path.as_ref() {
            paths.extend(std::env::split_paths(path));
        }
        self.set_os(
            "PATH",
            std::env::join_paths(paths).expect("join fake CLI PATH"),
        );
    }

    fn set_optional_path(&mut self, key: &'static str, value: Option<PathBuf>) {
        match value {
            Some(path) => self.set_os(key, path.into_os_string()),
            None => self.remove(key),
        }
    }

    fn set_optional_string(&mut self, key: &'static str, value: Option<String>) {
        match value {
            Some(value) => self.set(key, value),
            None => self.remove(key),
        }
    }

    fn set(&mut self, key: &'static str, value: String) {
        self.set_os(key, value.into());
    }

    fn set_os(&mut self, key: &'static str, value: OsString) {
        self.remember(key);
        std::env::set_var(key, value);
    }

    fn remove(&mut self, key: &'static str) {
        self.remember(key);
        std::env::remove_var(key);
    }

    fn remember(&mut self, key: &'static str) {
        if !self.previous_env.iter().any(|(seen, _)| *seen == key) {
            self.previous_env.push((key, std::env::var_os(key)));
        }
    }
}

impl Drop for FakeGenerationCli {
    fn drop(&mut self) {
        for (key, previous) in self.previous_env.drain(..).rev() {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}
