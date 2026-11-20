//! Retry, size-limit and fail-closed checksum tests for the registry route
//! (uv: the publisher's per-asset `.sha256` read before each archive).

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
    assert_eq!(
        gone.status,
        Some(404),
        "the status lets a pinned download say the file is gone"
    );
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
fn checksum_manifest_without_length_is_bounded_after_decoding() {
    let (url, server) = served_body(vec![b'a'; 17], None);
    let error = http_get_string_with_limit(&url, 16).unwrap_err();
    server.join().unwrap();
    assert!(error.error.message.contains("checksum manifest exceeds"));
    assert!(!error.transient);
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

/// The uv asset for Windows and its per-asset checksum file name.
fn uv() -> (ToolSpec, String, String) {
    let spec = tool_spec("uv", "windows", "x86_64").unwrap();
    let asset = spec.asset.to_string();
    let sum = format!("{asset}.sha256");
    (spec, asset, sum)
}

#[test]
fn checksum_404_recovers_with_fresh_checksum_and_verified_archive() {
    let (spec, asset, sum) = uv();
    let archive = b"current archive".to_vec();
    let digest = hex::encode(Sha256::digest(&archive));
    let (base, server) = served_attempts(vec![
        (sum.clone(), 404, vec![]),
        (sum, 200, format!("{digest}  {asset}\n").into_bytes()),
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
fn archive_503_restarts_at_the_current_checksum() {
    let (spec, asset, sum) = uv();
    let archive = b"fresh archive".to_vec();
    let digest = hex::encode(Sha256::digest(&archive));
    let checksum = format!("{digest}  {asset}\n").into_bytes();
    let (base, server) = served_attempts(vec![
        (sum.clone(), 200, checksum.clone()),
        (asset.clone(), 503, vec![]),
        (sum, 200, checksum),
        (asset.clone(), 200, archive.clone()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(&asset);
    let (_, got) = fetch_verified_archive(
        &spec,
        &format!("{base}/{asset}"),
        &path,
        &|_, _| {},
        &[Duration::ZERO],
        |_| {},
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(got, digest);
    assert_eq!(std::fs::read(path).unwrap(), archive);
}

#[test]
fn persistent_404_preserves_the_last_status_and_never_downloads() {
    let (spec, asset, sum) = uv();
    let (base, server) = served_attempts(vec![
        (sum.clone(), 404, vec![]),
        (sum.clone(), 404, vec![]),
        (sum.clone(), 404, vec![]),
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
    assert!(error.cause.contains(&sum));
    assert!(!path.exists());
}

#[test]
fn publisher_checksum_mismatch_fails_closed_without_retry() {
    let (spec, asset, sum) = uv();
    let (base, server) = served_attempts(vec![
        (
            sum,
            200,
            format!("{}  {asset}\n", "f".repeat(64)).into_bytes(),
        ),
        (asset.clone(), 200, b"different archive".to_vec()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(&asset);
    let error = fetch_verified_archive(
        &spec,
        &format!("{base}/{asset}"),
        &path,
        &|_, _| {},
        &[Duration::ZERO],
        |_| panic!("checksum mismatch must not retry"),
    )
    .unwrap_err();
    server.join().unwrap();
    assert!(error.message.contains("sha256 mismatch"));
    assert_ne!(
        hex::encode(Sha256::digest(std::fs::read(path).unwrap())),
        "f".repeat(64)
    );
}
