//! The remembered export folder is per-user state: it lives in the state home
//! (`SHELLX_CUT_HOME`) like the recent-projects list, so an engine started with
//! an isolated home never reads or writes the person's real setting, and the
//! choice survives an engine restart in that same home.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Engine {
    child: Child,
    addr: String,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(state: &Path, home: &Path, data: &Path) -> Engine {
    let addr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .to_string();
    let child = Command::new(env!("CARGO_BIN_EXE_cutd"))
        .args(["serve", "--headless", "--addr", &addr])
        .env("SHELLX_CUT_HOME", state)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_DATA_HOME", data)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start cutd");
    let engine = Engine { child, addr };
    let deadline = Instant::now() + Duration::from_secs(30);
    while TcpStream::connect(&engine.addr).is_err() {
        assert!(Instant::now() < deadline, "engine did not listen");
        std::thread::sleep(Duration::from_millis(50));
    }
    engine
}

fn verb(engine: &Engine, name: &str, body: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(&engine.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    write!(
        stream,
        "POST /api/verb/{name} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        engine.addr,
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    serde_json::from_str(response.split_once("\r\n\r\n").expect("http body").1).expect("json")
}

fn files_named(root: &Path, name: &str, found: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            files_named(&path, name, found);
        } else if path.file_name().is_some_and(|n| n == name) {
            found.push(path);
        }
    }
}

#[test]
fn an_isolated_engine_keeps_the_export_folder_in_its_own_state_home() {
    let root = tempfile::tempdir().unwrap();
    let (state, home, data) = (
        root.path().join("state"),
        root.path().join("home"),
        root.path().join("data"),
    );
    for dir in [&state, &home, &data] {
        fs::create_dir_all(dir).unwrap();
    }
    let chosen = root.path().join("deliveries");
    fs::create_dir(&chosen).unwrap();
    fs::set_permissions(&chosen, fs::Permissions::from_mode(0o700)).unwrap();
    let chosen = chosen.canonicalize().unwrap();

    {
        let engine = start(&state, &home, &data);
        let set = verb(
            &engine,
            "project.set_output_dir",
            &format!(r#"{{"dir":{}}}"#, serde_json::json!(chosen)),
        );
        assert_eq!(set["ok"], true, "{set}");
    }
    // The file is in the state home, and nowhere else the engine could have put
    // per-user data (the home folder and the data folder stay free of it).
    let saved = state.join("export-folder.json");
    assert!(
        saved.is_file(),
        "the choice is remembered in SHELLX_CUT_HOME"
    );
    let mut elsewhere = Vec::new();
    files_named(&home, "export-folder.json", &mut elsewhere);
    files_named(&data, "export-folder.json", &mut elsewhere);
    assert!(
        elsewhere.is_empty(),
        "the real per-user places stay untouched: {elsewhere:?}"
    );

    // A new engine in the same home still has it; one in another home does not.
    let again = start(&state, &home, &data);
    let read = verb(&again, "project.get_output_dir", "{}");
    assert_eq!(read["result"]["dir"], serde_json::json!(chosen), "{read}");
    let other = root.path().join("other");
    fs::create_dir_all(&other).unwrap();
    let stranger = start(&other.join("state"), &other, &other.join("data"));
    let none = verb(&stranger, "project.get_output_dir", "{}");
    assert_eq!(none["result"]["dir"], serde_json::Value::Null, "{none}");
}

cut_core::isolate_test_process!();
