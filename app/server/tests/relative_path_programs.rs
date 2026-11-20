//! A program in the engine's working folder never runs because of a relative
//! or empty PATH entry, or a relative tool setting.
//!
//! An agent may start `cutd` inside a received project with `PATH=.:/usr/bin`.
//! The project carries executable `ffmpeg` / `ffprobe` stand-ins that record
//! that they ran. Doctor's version probes and a media import (which passes the
//! media-tool gate) must start only programs found in absolute folders, never
//! the planted ones.

#![cfg(unix)]

cut_core::isolate_test_process!();

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Received {
    _temp: tempfile::TempDir,
    root: PathBuf,
    /// The received project folder `cutd` is started in.
    cwd: PathBuf,
}

fn received() -> Received {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let cwd = root.join("received");
    std::fs::create_dir_all(&cwd).unwrap();
    for name in ["ffmpeg", "ffprobe"] {
        let marker = root.join(format!("planted-{name}.ran"));
        let script = format!("#!/bin/sh\necho ran > '{}'\nexit 0\n", marker.display());
        let planted = cwd.join(name);
        std::fs::write(&planted, script).unwrap();
        std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(cwd.join("clip.mp4"), b"not really a movie").unwrap();
    Received {
        _temp: temp,
        root,
        cwd,
    }
}

fn planted_ran(received: &Received) -> Vec<String> {
    ["ffmpeg", "ffprobe"]
        .into_iter()
        .filter(|name| received.root.join(format!("planted-{name}.ran")).exists())
        .map(str::to_string)
        .collect()
}

fn verb(
    received: &Received,
    name: &str,
    args: &str,
    project: Option<&Path>,
    extra: &[(&str, &str)],
) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cutd"));
    command.args(["verb", name, args, "--standalone"]);
    if let Some(project) = project {
        command.arg("--project").arg(project);
    }
    command
        .current_dir(&received.cwd)
        // A relative and an empty entry first, then the system folders.
        .env("PATH", ".::/usr/bin:/bin")
        .env("SHELLX_CUT_HOME", received.root.join("state"))
        .env("HOME", received.root.join("home"))
        .env("XDG_DATA_HOME", received.root.join("data"))
        .env_remove("SHELLX_CUT_FFMPEG")
        .env_remove("SHELLX_CUT_FFPROBE")
        .env_remove("SHELLX_CUT_FFMPEG_DIR")
        .env_remove("SHELLX_CUT_FFMPEG_AUTO")
        .stdin(Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    let output = command.output().expect("run cutd verb");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "cutd printed no JSON: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn media_cards(doctor: &serde_json::Value) -> Vec<(String, String)> {
    doctor["result"]["cards"]
        .as_array()
        .expect("doctor cards")
        .iter()
        .filter(|card| card["id"] == "ffmpeg" || card["id"] == "ffprobe")
        .map(|card| {
            (
                card["id"].as_str().unwrap().to_string(),
                card["details"]["resolved"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn doctor_never_starts_a_media_tool_from_a_relative_path_entry() {
    let received = received();
    let doctor = verb(&received, "system.doctor", r#"{"refresh":true}"#, None, &[]);
    assert_eq!(
        planted_ran(&received),
        Vec::<String>::new(),
        "a planted tool ran"
    );
    for (id, resolved) in media_cards(&doctor) {
        let path = Path::new(&resolved);
        assert!(
            path.is_absolute(),
            "{id} resolves to a bare name: {resolved:?}"
        );
        assert!(!path.starts_with(&received.cwd), "{id}: {resolved}");
    }
}

#[test]
fn a_media_import_starts_the_tool_the_gate_admitted() {
    let received = received();
    let created = verb(
        &received,
        "project.create",
        &serde_json::json!({ "name": "film", "dir": received.root.join("projects").join("film.cutproj") }).to_string(),
        None,
        &[],
    );
    let project = PathBuf::from(
        created["result"]["path"]
            .as_str()
            .unwrap_or_else(|| panic!("project.create: {created}")),
    );
    let _ = verb(
        &received,
        "media.import",
        &serde_json::json!({ "path": received.cwd.join("clip.mp4") }).to_string(),
        Some(&project),
        &[],
    );
    assert_eq!(
        planted_ran(&received),
        Vec::<String>::new(),
        "a planted tool ran"
    );
}

#[test]
fn relative_tool_settings_are_never_used() {
    let received = received();
    let doctor = verb(
        &received,
        "system.doctor",
        r#"{"refresh":true}"#,
        None,
        &[
            ("SHELLX_CUT_FFMPEG", "./ffmpeg"),
            ("SHELLX_CUT_FFPROBE", "ffprobe"),
        ],
    );
    assert_eq!(
        planted_ran(&received),
        Vec::<String>::new(),
        "a planted tool ran"
    );
    for (id, resolved) in media_cards(&doctor) {
        assert!(Path::new(&resolved).is_absolute(), "{id}: {resolved:?}");
    }
    let doctor = verb(
        &received,
        "system.doctor",
        r#"{"refresh":true}"#,
        None,
        &[("SHELLX_CUT_FFMPEG_DIR", ".")],
    );
    assert_eq!(
        planted_ran(&received),
        Vec::<String>::new(),
        "a planted tool ran"
    );
    for (id, resolved) in media_cards(&doctor) {
        assert!(
            !Path::new(&resolved).starts_with(&received.cwd),
            "{id}: {resolved}"
        );
    }
}
