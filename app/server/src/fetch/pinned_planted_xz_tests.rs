//! The pinned `.tar.xz` unpack never starts a decompressor from the engine's
//! working folder. `tar -J` starts `xz` by name; with `PATH=.:/usr/bin` and a
//! planted `./xz` in the working folder (a received project), tar must still
//! use the absolute `xz` the preflight found.
//!
//! The working folder is per process, so the parent re-runs this test binary
//! as a child in a folder with a planted `xz`; the child installs a fixture
//! build through the real route from a loopback server.

use super::*;
use std::os::unix::fs::PermissionsExt;

const CHILD: &str = "fetch::pinned::planted_xz_tests::child_installs_beside_a_planted_xz";
const CHILD_ENV: &str = "SHELLX_CUT_TEST_PLANTED_XZ_CHILD";

#[test]
fn a_planted_xz_in_the_working_folder_never_runs() {
    let root = tempfile::tempdir().unwrap();
    let received = root.path().join("received");
    std::fs::create_dir(&received).unwrap();
    let marker = root.path().join("planted-xz.ran");
    std::fs::write(
        received.join("xz"),
        format!("#!/bin/sh\necho ran > '{}'\nexit 9\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(received.join("xz"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
        .current_dir(&received)
        .env("PATH", ".:/usr/bin:/bin")
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let report = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!marker.exists(), "the planted xz ran:\n{report}");
    assert!(output.status.success(), "the install failed:\n{report}");
    assert!(report.contains("1 passed"), "{report}");
}

/// The child half: does its work only when the test above starts it (with
/// the marker variable, in the planted folder); otherwise it returns at once.
#[test]
fn child_installs_beside_a_planted_xz() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    use std::io::Write;
    let sha = |bytes: &[u8]| -> &'static str {
        Box::leak(hex::encode(Sha256::digest(bytes)).into_boxed_str())
    };
    let work = tempfile::tempdir().unwrap();
    let bin = work.path().join("pkg/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let mut members = Vec::new();
    for stem in ["ffmpeg", "ffprobe"] {
        let body = format!("#!/bin/sh\necho {stem} version fixture\n");
        std::fs::write(bin.join(stem), &body).unwrap();
        std::fs::set_permissions(bin.join(stem), std::fs::Permissions::from_mode(0o755)).unwrap();
        members.push(PinnedMember {
            path: Box::leak(format!("pkg/bin/{stem}").into_boxed_str()),
            install_as: stem,
            size: body.len() as u64,
            sha256: sha(body.as_bytes()),
        });
    }
    // The fixture is packed with the system xz (an absolute-only PATH).
    let archive = work.path().join("fixture.tar.xz");
    let packed = std::process::Command::new(system_tar())
        .arg("-cJf")
        .arg(&archive)
        .arg("-C")
        .arg(work.path())
        .arg("pkg")
        .env("PATH", "/usr/bin:/bin")
        .status()
        .unwrap();
    assert!(packed.success());
    let bytes = std::fs::read(&archive).unwrap();
    let release = PinnedRelease {
        version: "fixture",
        build: "fixture",
        base: "https://example.invalid",
        assets: Box::leak(Box::new([PinnedAsset {
            file: "fixture.tar.xz",
            kind: ArchiveKind::TarXz,
            size: bytes.len() as u64,
            sha256: sha(&bytes),
            members: Box::leak(members.into_boxed_slice()),
        }])),
        check: ProgramCheck::None,
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            let _ = stream.write_all(&bytes);
        }
    });
    let tools = tempfile::tempdir().unwrap();
    install_into("ffmpeg", &release, &base, tools.path(), &|_, _| {})
        .expect("the fixture installs with the system xz");
    assert!(tools.path().join("ffmpeg/bin/ffmpeg").is_file());
}
