//! Pinned uv archive, retry, and size-limit tests for the built-in registry.

use super::*;

#[test]
fn retry_classification_excludes_permanent_http_errors() {
    for error in [
        ureq::Error::StatusCode(404),
        ureq::Error::StatusCode(429),
        ureq::Error::StatusCode(503),
        ureq::Error::ConnectionFailed,
    ] {
        assert!(
            FetchAttemptError::network("download", "https://example.test/archive", error).transient
        );
    }
    let gone = FetchAttemptError::network(
        "download",
        "https://example.test/a",
        ureq::Error::StatusCode(404),
    );
    assert_eq!(gone.status, Some(404));
    for error in [ureq::Error::StatusCode(400), ureq::Error::StatusCode(403)] {
        assert!(
            !FetchAttemptError::network("download", "https://example.test/archive", error)
                .transient
        );
    }
}

fn served_attempts(
    responses: Vec<(String, u16, Vec<u8>)>,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for (path, status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let size = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).starts_with(&format!("GET /{path} ")));
            write!(
                stream,
                "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (base, server)
}

fn served_body(
    body: Vec<u8>,
    declared_length: Option<u64>,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/resource", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut reader = std::io::BufReader::new(&mut stream).take(8 * 1024);
        while !request.ends_with(b"\r\n\r\n") {
            assert!(
                std::io::BufRead::read_until(&mut reader, b'\n', &mut request).unwrap() > 0,
                "fixture request must end within 8 KiB"
            );
        }
        assert!(request.starts_with(b"GET /resource "));
        drop(reader);
        write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n").unwrap();
        if let Some(length) = declared_length {
            write!(stream, "Content-Length: {length}\r\n").unwrap();
        }
        stream.write_all(b"\r\n").unwrap();
        stream.write_all(&body).unwrap();
    });
    (url, server)
}

#[test]
fn archive_without_length_stops_before_writing_over_limit() {
    let (url, server) = served_body(vec![b'a'; 17], None);
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("archive");
    let error = download_hashing_with_limit(&url, &dest, &|_, _| {}, 16).unwrap_err();
    server.join().unwrap();
    assert!(error.error.message.contains("tool archive exceeds"));
    assert!(std::fs::metadata(dest).unwrap().len() <= 16);
    assert!(!error.transient);
}

#[test]
fn false_oversized_content_length_is_rejected_before_staging() {
    let (url, server) = served_body(b"small".to_vec(), Some(17));
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("archive");
    let error = download_hashing_with_limit(&url, &dest, &|_, _| {}, 16).unwrap_err();
    server.join().unwrap();
    assert!(error.error.message.contains("tool archive exceeds"));
    assert!(!dest.exists());
}

fn uv() -> ToolSpec {
    tool_spec("uv", "windows", "x86_64").unwrap()
}

#[test]
fn source_pins_every_downloadable_uv_platform() {
    let platforms = [
        ("windows", "x86_64"),
        ("windows", "aarch64"),
        ("linux", "x86_64"),
        ("linux", "aarch64"),
        ("macos", "aarch64"),
        ("macos", "x86_64"),
    ];
    for (os, arch) in platforms {
        let spec = tool_spec("uv", os, arch).expect("supported platform needs a source pin");
        assert_eq!(spec.base, UV_BASE);
        assert!(spec.base.ends_with("/releases/download/0.13.9"));
        assert_eq!(spec.sha256.len(), 64, "{os}/{arch}");
        assert!(
            spec.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "{os}/{arch}"
        );
    }
    assert!(tool_spec("uv", "freebsd", "x86_64").is_none());
    assert!(tool_spec("uv", "linux", "riscv64").is_none());
}

#[test]
fn archive_503_retries_and_accepts_matching_source_digest() {
    let mut spec = uv();
    let asset = spec.asset.to_string();
    let archive = b"known archive".to_vec();
    let digest = hex::encode(Sha256::digest(&archive));
    spec.sha256 = Box::leak(digest.clone().into_boxed_str());
    let (base, server) = served_attempts(vec![
        (asset.clone(), 503, vec![]),
        (asset.clone(), 200, archive.clone()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(&asset);
    let (size, got) = fetch_verified_archive(
        &spec,
        &format!("{base}/{asset}"),
        &path,
        &|_, _| {},
        &[Duration::ZERO],
        |_| {},
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(size, archive.len() as u64);
    assert_eq!(got, digest);
    assert_eq!(std::fs::read(path).unwrap(), archive);
}

#[test]
fn persistent_404_preserves_status_and_never_installs() {
    let spec = uv();
    let asset = spec.asset.to_string();
    let (base, server) = served_attempts(vec![
        (asset.clone(), 404, vec![]),
        (asset.clone(), 404, vec![]),
        (asset.clone(), 404, vec![]),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(&asset);
    let error = fetch_verified_archive(
        &spec,
        &format!("{base}/{asset}"),
        &path,
        &|_, _| {},
        &[Duration::ZERO, Duration::ZERO],
        |_| {},
    )
    .unwrap_err();
    server.join().unwrap();
    assert!(error.cause.contains("http status: 404"));
    assert!(!path.exists());
}

#[test]
fn same_host_checksum_cannot_authorize_changed_archive() {
    let spec = uv();
    let asset = spec.asset.to_string();
    let changed = b"changed release archive".to_vec();
    let matching_sibling = hex::encode(Sha256::digest(&changed));
    let (base, server) = served_attempts(vec![(asset.clone(), 200, changed)]);
    let temp = tempfile::tempdir().unwrap();
    let error = fetch_verified_archive(
        &spec,
        &format!("{base}/{asset}"),
        &temp.path().join(&asset),
        &|_, _| {},
        &[],
        |_| {},
    )
    .unwrap_err();
    server.join().unwrap();
    assert_ne!(matching_sibling, spec.sha256);
    assert_eq!(
        error.message,
        "The download did not match what Cut expects. Nothing was installed. Try again later."
    );
    assert!(error.cause.contains(spec.sha256));
}

#[test]
fn platform_without_a_pin_is_refused_before_download() {
    assert!(tool_spec("uv", "freebsd", "x86_64").is_none());
    let error = no_builtin_download("uv", "freebsd", "x86_64");
    assert_eq!(error.code, error_codes::UNSUPPORTED_PLATFORM);
    assert!(error.message.contains("freebsd/x86_64"));
    assert!(error.cause.contains("verified publisher build"));
    assert!(error.suggested_action.unwrap().contains("Install Cut"));
}
