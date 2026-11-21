use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

fn serve(body: Vec<u8>, length: bool, slow: bool) -> (String, std::thread::JoinHandle<usize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let received = stream.read(&mut request).unwrap();
        assert!(request[..received].starts_with(b"GET /model"));
        write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n").unwrap();
        if length {
            write!(stream, "Content-Length: {}\r\n", body.len()).unwrap();
        }
        stream.write_all(b"\r\n").unwrap();
        let mut sent = 0;
        for chunk in body.chunks(if slow { 1 } else { body.len().max(1) }) {
            if stream.write_all(chunk).is_err() {
                break;
            }
            sent += chunk.len();
            if slow {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        sent
    });
    (url, handle)
}

#[test]
fn oversized_model_stops_stream_and_leaves_no_files() {
    let (url, sender) = serve(vec![b'x'; 1024], false, true);
    let dir = tempfile::tempdir().unwrap();
    let err = install_pinned_model(&url, dir.path(), "test.onnx", "unused", 16).unwrap_err();
    assert!(err.message.contains("size limit"), "{err:?}");
    assert!(
        sender.join().unwrap() < 1024,
        "client read the full response"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn wrong_model_hash_leaves_no_files() {
    let (url, sender) = serve(b"model".to_vec(), true, false);
    let dir = tempfile::tempdir().unwrap();
    let err = install_pinned_model(&url, dir.path(), "test.pth", "wrong", 16).unwrap_err();
    assert!(err.message.contains("checksum mismatch"), "{err:?}");
    sender.join().unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn pinned_model_installs_from_local_fixture() {
    let body = b"model".to_vec();
    let hash = format!("{:x}", Sha256::digest(&body));
    let (url, sender) = serve(body.clone(), true, false);
    let dir = tempfile::tempdir().unwrap();
    let dest = install_pinned_model(&url, dir.path(), "test.pth", &hash, 16).unwrap();
    sender.join().unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
