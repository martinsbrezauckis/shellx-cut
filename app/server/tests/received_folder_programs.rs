//! A folder from someone else must never supply a program the engine runs.
//!
//! An agent CLI whose working folder is a received project, or a person in a
//! terminal there, can run `cutd verb …` or `cutd mcp --standalone`; the verbs
//! then run in a process whose current folder is that project. The engine
//! used to look for the Generate planner adapters in the current folder and
//! the folders above it, and for a ShellX Motion checkout the same way. These
//! tests plant both in a received folder and in a project folder above the
//! current folder and prove that nothing planted is used.

cut_core::isolate_test_process!();

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn python() -> PathBuf {
    if let Some(explicit) = std::env::var_os("CUTD_ADAPTER_PYTHON").filter(|p| !p.is_empty()) {
        return PathBuf::from(explicit);
    }
    let path = std::env::var_os("PATH").expect("PATH");
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| {
            ["python3", "python", "python3.exe", "python.exe"].map(|name| dir.join(name))
        })
        .find(|candidate| {
            candidate.is_file() && !candidate.to_string_lossy().contains("WindowsApps")
        })
        .expect("a Python interpreter on PATH or in CUTD_ADAPTER_PYTHON")
}

/// A stand-in adapter that records that it ran, then answers not_run.
fn adapter_script(marker: &Path) -> String {
    format!(
        "import json\nopen({marker:?}, 'w').write('ran')\n\
print(json.dumps({{'schema': 'shellx-cut/generate-plan/1', 'status': 'not_run', \
'reason': 'stand-in', 'plan': None, 'warnings': []}}))\n",
        marker = marker.display().to_string()
    )
}

struct Received {
    _root: tempfile::TempDir,
    root: PathBuf,
    /// The folder `cutd` is started in.
    cwd: PathBuf,
    sidecar: PathBuf,
    state: PathBuf,
}

/// `received/` holds a planted `app/perception/py/<adapter>` and a planted
/// ShellX Motion checkout; `cwd` is that folder or a project folder below it.
fn received(start_inside_project: bool) -> Received {
    let temp = tempfile::tempdir().expect("temp root");
    let root = temp.path().to_path_buf();
    let received = root.join("received");
    let planted = received.join("app/perception/py");
    std::fs::create_dir_all(&planted).unwrap();
    for adapter in [
        "generate_prompt_adapter.py",
        "generate_storyboard_adapter.py",
    ] {
        let marker = root.join(format!("planted-{adapter}.ran"));
        std::fs::write(planted.join(adapter), adapter_script(&marker)).unwrap();
    }
    std::fs::create_dir_all(received.join("ShellX Motion/packages/cli")).unwrap();
    std::fs::write(received.join("ShellX Motion/package.json"), b"{}").unwrap();
    let project = received.join("film.cutproj");
    std::fs::create_dir_all(project.join(".scratch")).unwrap();
    std::fs::write(project.join("ops.jsonl"), b"").unwrap();
    let cwd = if start_inside_project {
        project.join(".scratch")
    } else {
        received.clone()
    };
    // The application's own perception payload, with its own stand-ins.
    let sidecar = root.join("installed/perception");
    std::fs::create_dir_all(&sidecar).unwrap();
    std::fs::write(sidecar.join("instruments.py"), b"# stand-in\n").unwrap();
    for adapter in [
        "generate_prompt_adapter.py",
        "generate_storyboard_adapter.py",
    ] {
        let marker = root.join(format!("bundled-{adapter}.ran"));
        std::fs::write(sidecar.join(adapter), adapter_script(&marker)).unwrap();
    }
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    Received {
        _root: temp,
        root,
        cwd,
        sidecar,
        state,
    }
}

fn run_verb(received: &Received, verb: &str, args: &str) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cutd"));
    command.env("SHELLX_CUT_SIDECAR_DIR", &received.sidecar);
    run_verb_with(received, command, verb, args)
}

/// Run `command` (a cutd) as a verb in the received folder. The sidecar
/// Python is named explicitly: a build folder that also holds the desktop
/// shell's build has tauri-build's copy of the `perception/` payload beside
/// cutd, which makes the engine treat itself as installed, and an installed
/// engine uses only its provisioned Python.
fn run_verb_with(
    received: &Received,
    mut command: Command,
    verb: &str,
    args: &str,
) -> serde_json::Value {
    let python = python();
    let python_dir = python.parent().unwrap().to_path_buf();
    let output = command
        .args(["verb", verb, args, "--standalone"])
        .current_dir(&received.cwd)
        .env("PATH", &python_dir)
        .env("SHELLX_CUT_HOME", &received.state)
        .env("XDG_DATA_HOME", received.root.join("data"))
        .env("LOCALAPPDATA", received.root.join("data"))
        .env("APPDATA", received.root.join("data"))
        .env("SHELLX_CUT_PYTHON", &python)
        .env("CUTD_ADAPTER_PYTHON", &python)
        .env_remove("CUTD_GENERATE_PROMPT_ADAPTER")
        .env_remove("CUTD_GENERATE_STORYBOARD_ADAPTER")
        .env_remove("SHELLX_MOTION_ROOT")
        .env_remove("SHELLX_MOTION_BIN")
        .env_remove("SHELLX_MOTION_CLI")
        .env_remove("RELEASE_RUNNER_PROVIDER_CONTEXT")
        .env_remove("RELEASE_RUNNER_NATIVE_RUNTIME_CONTEXT")
        .stdin(Stdio::null())
        .output()
        .expect("run cutd verb");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "cutd printed no JSON: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn ran(received: &Received, which: &str, adapter: &str) -> bool {
    received
        .root
        .join(format!("{which}-{adapter}.ran"))
        .exists()
}

#[test]
fn a_planted_generate_adapter_in_the_current_or_a_project_folder_is_not_run() {
    for start_inside_project in [false, true] {
        for (verb, args, adapter) in [
            (
                "generate.from_prompt",
                r#"{"prompt":"a title card","policy":"plan","timeout_ms":20000}"#,
                "generate_prompt_adapter.py",
            ),
            (
                "generate.storyboard",
                r#"{"input":"a short story","timeout_ms":20000}"#,
                "generate_storyboard_adapter.py",
            ),
        ] {
            let received = received(start_inside_project);
            let answer = run_verb(&received, verb, args);
            assert!(
                !ran(&received, "planted", adapter),
                "{verb} ran the {adapter} planted in the received folder (cwd {})",
                received.cwd.display()
            );
            assert!(
                ran(&received, "bundled", adapter),
                "{verb} must still run the application's own {adapter}: {answer}"
            );
        }
    }
}

#[test]
fn a_planted_motion_checkout_above_the_current_folder_is_not_used() {
    for start_inside_project in [false, true] {
        let received = received(start_inside_project);
        let listing = run_verb(&received, "generate.list", r#"{"kind":"motion"}"#);
        let templates = listing["result"]["templates"]
            .as_array()
            .expect("motion templates listed");
        assert!(!templates.is_empty(), "{listing}");
        assert!(
            templates
                .iter()
                .all(|template| template["available"] == serde_json::json!(false)),
            "a planted ShellX Motion checkout must not make Motion templates available: {listing}"
        );
    }
}

/// The engine finds the application's own adapter from its own location in
/// every package layout, with no variable naming it, while the received
/// folder it is started in still supplies nothing: the macOS bundle
/// (`Contents/MacOS/cutd` + `Contents/Resources/perception`), the Windows
/// install folder (`cutd.exe` + `perception`), and the Linux package
/// (`/usr/bin/cutd` + `/usr/lib/ShellX Cut/perception`). The engine is
/// linked (or copied) into each layout.
#[test]
fn the_applications_own_adapter_is_found_in_every_package_layout() {
    let exe_name = if cfg!(windows) { "cutd.exe" } else { "cutd" };
    let layouts: [(&str, &str); 3] = [
        (
            "ShellX Cut.app/Contents/MacOS",
            "ShellX Cut.app/Contents/Resources/perception",
        ),
        ("ShellX Cut", "ShellX Cut/perception"),
        ("usr/bin", "usr/lib/ShellX Cut/perception"),
    ];
    for (bin, payload) in layouts {
        let received = received(true);
        // Beside the engine binary so a hard link works (same volume).
        let install = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let bin_dir = install.path().join(bin);
        std::fs::create_dir_all(&bin_dir).unwrap();
        let engine = bin_dir.join(exe_name);
        if std::fs::hard_link(env!("CARGO_BIN_EXE_cutd"), &engine).is_err() {
            std::fs::copy(env!("CARGO_BIN_EXE_cutd"), &engine).unwrap();
        }
        let payload_dir = install.path().join(payload);
        std::fs::create_dir_all(payload_dir.parent().unwrap()).unwrap();
        // The stand-in payload of `received()` plays the packaged one.
        copy_dir(&received.sidecar, &payload_dir);
        let mut command = Command::new(&engine);
        command.env_remove("SHELLX_CUT_SIDECAR_DIR");
        let answer = run_verb_with(
            &received,
            command,
            "generate.from_prompt",
            r#"{"prompt":"a title card","policy":"plan","timeout_ms":20000}"#,
        );
        assert!(
            !ran(&received, "planted", "generate_prompt_adapter.py"),
            "{bin}: the planted adapter ran"
        );
        assert!(
            ran(&received, "bundled", "generate_prompt_adapter.py"),
            "{bin}: the packaged adapter beside {payload} was not run: {answer}"
        );
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}
