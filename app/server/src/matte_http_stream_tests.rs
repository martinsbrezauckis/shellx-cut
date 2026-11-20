//! The HTTP matte bake streams the clip from its file with its length, instead
//! of reading the whole clip into memory first.

use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

const STATS: &str = r#"{"frames":2,"fps":24.0,"width":16,"height":16,"coverage_mean":0.5,"cov_min":0.4,"cov_max":0.6,"temporal_flicker":0.1}"#;

/// A stand-in sidecar: reports the request's Content-Length and how many body
/// bytes arrived, then answers with a tiny alpha.
fn stand_in_sidecar() -> (String, std::thread::JoinHandle<(u64, u64)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut declared = 0_u64;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                declared = value.trim().parse().unwrap();
            }
        }
        let received =
            std::io::copy(&mut reader.by_ref().take(declared), &mut std::io::sink()).unwrap();
        let mut stream = stream;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nX-Matte-Stats: {STATS}\r\nContent-Length: 5\r\nConnection: close\r\n\r\nalpha"
        )
        .unwrap();
        stream.flush().unwrap();
        (declared, received)
    });
    (address, handle)
}

#[test]
fn the_clip_is_sent_from_its_file_with_its_length() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mov");
    let len = 8 * 1024 * 1024 + 3;
    let mut file = std::fs::File::create(&clip).unwrap();
    file.write_all(&vec![7_u8; len]).unwrap();
    drop(file);
    let alpha = dir.path().join("alpha.mkv");
    let matte: ClipMatte =
        serde_json::from_str(r#"{"mode":"remove","model":"rvm","quality":"good"}"#).unwrap();
    let (address, sidecar) = stand_in_sidecar();

    let stats = bake_http_at(&address, &clip, &alpha, &matte).unwrap();

    let (declared, received) = sidecar.join().unwrap();
    assert_eq!(declared, len as u64, "the request states the clip's length");
    assert_eq!(received, len as u64, "the whole clip arrives");
    assert_eq!(stats.frames, 2);
    assert_eq!(std::fs::read(&alpha).unwrap(), b"alpha");
}
