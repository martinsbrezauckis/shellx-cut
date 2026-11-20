//! Can the first start right after a built-in download hit "Text file busy"?
//! The pinned route installs a fixture build again and again while other
//! threads start processes without pause, and starts each freshly installed
//! program at once with a plain `Command` (no retry), counting busy failures.
//! Run by name on Linux:
//! `cargo test -p server --bin cutd fetch::pinned::busy_tests -- --ignored --nocapture`.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

fn sha256_hex(bytes: &[u8]) -> &'static str {
    Box::leak(hex::encode(Sha256::digest(bytes)).into_boxed_str())
}

#[test]
#[ignore = "stress run: installs and starts a fixture build many times"]
fn freshly_installed_programs_start_while_other_threads_spawn() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let installs: usize = std::env::var("CUT_STRESS_INSTALLS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
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
            sha256: sha256_hex(body.as_bytes()),
        });
    }
    let archive = work.path().join("fixture.tar.xz");
    let status = std::process::Command::new("tar")
        .arg("-cJf")
        .arg(&archive)
        .arg("-C")
        .arg(work.path())
        .arg("pkg")
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&archive).unwrap();
    let release = PinnedRelease {
        version: "fixture",
        build: "fixture",
        base: "https://example.invalid",
        assets: Box::leak(Box::new([PinnedAsset {
            file: "fixture.tar.xz",
            kind: ArchiveKind::TarXz,
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            members: Box::leak(members.into_boxed_slice()),
        }])),
        check: ProgramCheck::None,
    };

    // A loopback server that answers every request with the archive.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let served = bytes.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                served.len()
            );
            let _ = stream.write_all(&served);
        }
    });

    let stop = Arc::new(AtomicBool::new(false));
    let spawners: Vec<_> = (0..4)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = std::process::Command::new("/bin/true").status();
                }
            })
        })
        .collect();
    let busy = AtomicU64::new(0);
    let mut other = Vec::new();
    for _ in 0..installs {
        let tools = tempfile::tempdir().unwrap();
        install_into("ffmpeg", &release, &base, tools.path(), &|_, _| {}).unwrap();
        for stem in ["ffmpeg", "ffprobe"] {
            let program = tools.path().join("ffmpeg/bin").join(stem);
            match std::process::Command::new(&program).output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => other.push(format!("{stem}: exit {:?}", out.status)),
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                    busy.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => other.push(format!("{stem}: {e}")),
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    for thread in spawners {
        thread.join().unwrap();
    }
    let busy = busy.load(Ordering::Relaxed);
    eprintln!(
        "INSTALL-STRESS installs={installs} first_starts={} text_file_busy={busy} other={other:?}",
        installs * 2
    );
    assert_eq!(busy, 0);
    assert!(other.is_empty(), "{other:?}");
}
