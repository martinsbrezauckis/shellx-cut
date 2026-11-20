//! No Python to run: the person and the agent get one plain reason and the
//! real setup action, never a spawn error or the Microsoft Store shortcut text.

use super::lock_native_runtime_env;
use super::{
    run_instruments, sidecar_runtime, transcribe, InstrumentSet, ENV_PYTHON, ENV_SIDECAR_DIR,
};
use cut_core::CutError;
use std::ffi::OsString;
use std::path::Path;

const MESSAGE: &str = "Captions and transcription are not installed";

/// Sets the env the sidecar ladder reads and restores it on drop.
struct Env(Vec<(&'static str, Option<OsString>)>);

impl Env {
    fn set(vars: &[(&'static str, Option<&Path>)]) -> Self {
        let saved = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .chain([(
                cut_native_runtime_context::CONTEXT_ENV,
                std::env::var_os(cut_native_runtime_context::CONTEXT_ENV),
            )])
            .collect();
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        for (k, v) in vars {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        Self(saved)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (k, v) in self.0.drain(..) {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

fn assert_plain_refusal(error: &CutError) {
    assert_eq!(error.message, MESSAGE, "{error}");
    assert_eq!(error.code, "sidecar");
    let all = format!(
        "{} {}",
        error.cause,
        error.suggested_action.clone().unwrap_or_default()
    );
    assert!(!all.contains("Python was not found"), "{all}");
    assert!(!all.contains("exit 9009"), "{all}");
    let action = error.suggested_action.as_deref().expect("an action");
    assert!(action.contains("Install captions"), "{action}");
    assert!(action.contains("system.setup_perception"), "{action}");
}

#[test]
fn a_python_that_is_not_there_is_refused_with_the_setup_action() {
    let _guard = lock_native_runtime_env();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-python-here");
    let _env = Env::set(&[
        (ENV_PYTHON, Some(&missing)),
        (ENV_SIDECAR_DIR, Some(dir.path())),
    ]);
    let error = sidecar_runtime().expect_err("a missing interpreter must be refused");
    assert_plain_refusal(&error);
    // media.transcribe and the enrich job reach the same refusal.
    std::fs::write(dir.path().join("instruments.py"), "").unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let error = transcribe(
        &dir.path().join("clip.mp4"),
        &receipts,
        "a1",
        "sha256:cafe",
        None,
    )
    .expect_err("transcribe needs a Python");
    assert_plain_refusal(&error);
}

#[test]
fn the_store_shortcut_run_is_named_as_missing_captions() {
    let _guard = lock_native_runtime_env();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("instruments.py"), "").unwrap();
    // A command that behaves like the Windows Store shortcut: prints its hint
    // and exits 9009.
    #[cfg(windows)]
    let stub = {
        let p = dir.path().join("python-stub.cmd");
        std::fs::write(
            &p,
            "@echo off\r\necho Python was not found; run without arguments to install from the Microsoft Store 1>&2\r\nexit /b 9009\r\n",
        )
        .unwrap();
        p
    };
    #[cfg(not(windows))]
    let stub = {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.path().join("python-stub.sh");
        std::fs::write(
            &p,
            "#!/bin/sh\necho 'Python was not found; run without arguments to install from the Microsoft Store' >&2\nexit 9009\n",
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };
    let _env = Env::set(&[
        (ENV_PYTHON, Some(&stub)),
        (ENV_SIDECAR_DIR, Some(dir.path())),
    ]);
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let error = run_instruments(
        &dir.path().join("clip.mp4"),
        &receipts,
        "a1",
        "sha256:cafe",
        InstrumentSet::RenderChecks,
        None,
    )
    .expect_err("the shortcut is not a Python");
    assert_plain_refusal(&error);
}

/// No suggested action in the engine may send a person of the installed app to a
/// source checkout: a path inside the repository, a venv they must build by
/// hand, or a developer command. The installed routes are Install captions and
/// system.setup_perception.
#[test]
fn no_suggested_action_names_a_source_tree_route() {
    const BANNED: [&str; 10] = [
        "app/perception",
        ".venv",
        "cargo ",
        "npm ",
        "checkout",
        "python3 -m",
        "pip install",
        "py -3",
        "(dev)",
        "Set up perception",
    ];
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name != "target" && name != "tests" && name != "node_modules" {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    walk(app, &mut files);
    let mut offences = Vec::new();
    let mut scanned = 0;
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with("_tests.rs") || name == "tests.rs" || name == "python_missing_tests.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let text = text.split("#[cfg(test)]").next().unwrap();
        for (at, _) in text.match_indices("with_suggested_action(") {
            scanned += 1;
            // The call ends at its matching parenthesis (string-aware).
            let tail = &text[at + "with_suggested_action(".len()..];
            let (mut depth, mut in_str, mut prev, mut end) = (1usize, false, ' ', tail.len());
            for (n, c) in tail.char_indices() {
                if in_str {
                    if c == '"' && prev != '\\' {
                        in_str = false;
                    }
                } else if c == '"' {
                    in_str = true;
                } else if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        end = n;
                        break;
                    }
                }
                prev = c;
            }
            let call = &tail[..end];
            for word in BANNED {
                if call.contains(word) {
                    offences.push(format!(
                        "{}: {word}: {}",
                        file.display(),
                        call.replace('\n', " ")
                    ));
                }
            }
        }
    }
    assert!(
        scanned > 100,
        "the scan must see the engine's actions ({scanned})"
    );
    assert!(offences.is_empty(), "{}", offences.join("\n"));
}
