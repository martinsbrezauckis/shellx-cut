//! Recovery must not resolve playlist references before checking checkpoint hashes.

use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::{CaptureStart, CheckpointFacts, ManifestOwner, MediaFacts, OwnerState};

struct LoopbackCounter {
    url: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LoopbackCounter {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/segment.ts", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let count = requests.clone();
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        count.fetch_add(1, Ordering::SeqCst);
                        socket
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("loopback counter: {error}"),
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn playlist(&self) -> String {
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:1,\n{}\n#EXT-X-ENDLIST\n", self.url)
    }

    fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for LoopbackCounter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn facts() -> MediaFacts {
    MediaFacts {
        duration_ms: 100,
        decoded_video_frames: 1,
        has_audio: false,
        width: None,
        height: None,
        codec_name: None,
        avg_frame_rate: None,
        r_frame_rate: None,
    }
}

fn publish(owner: &mut ManifestOwner, contents: &[u8]) {
    let staging = owner.begin_segment(0, 0).unwrap();
    std::fs::write(&staging, contents).unwrap();
    owner
        .publish(
            0,
            &staging,
            CheckpointFacts {
                start_ms: 0,
                end_ms: 100,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            facts(),
        )
        .unwrap();
}

#[test]
fn real_probe_blocks_disguised_hls_before_hash_or_recovery_mutation() {
    let counter = LoopbackCounter::new();
    let root = tempfile::tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    publish(&mut owner, b"original checkpoint with different hash");
    drop(owner);
    let checkpoint = root.path().join("checkpoints/segment-000000.mp4");
    let hostile = counter.playlist();
    std::fs::write(&checkpoint, &hostile).unwrap();
    let manifest = std::fs::read(root.path().join(crate::MANIFEST_FILE)).unwrap();
    assert!(
        crate::recover_interrupted(root.path(), "ffmpeg", "ffprobe", OwnerState::Dead).is_err()
    );
    assert_eq!(
        counter.count(),
        0,
        "probe must not fetch before noticing a hash mismatch"
    );
    assert_eq!(std::fs::read(&checkpoint).unwrap(), hostile.as_bytes());
    assert_eq!(
        std::fs::read(root.path().join(crate::MANIFEST_FILE)).unwrap(),
        manifest
    );
    assert!(!root.path().join("quarantine").exists());
    assert!(!root.path().join("recovered.mp4").exists());
    assert!(crate::read_manifest(root.path()).unwrap().receipt.is_none());

    #[cfg(unix)]
    {
        // Some current tools decline HLS sniffing under .mp4. Force that
        // demuxer in the owned fixture to exercise the actual format guard,
        // without relying on a particular tool release's sniffing heuristic.
        let probe = script(
            &root.path().join("hls-probe"),
            "#!/bin/sh\nexec ffprobe -f hls \"$@\"\n",
        );
        assert!(
            crate::recover_interrupted(root.path(), "ffmpeg", &probe, OwnerState::Dead).is_err()
        );
        assert_eq!(counter.count(), 0);
        assert_eq!(std::fs::read(&checkpoint).unwrap(), hostile.as_bytes());
        assert_eq!(
            std::fs::read(root.path().join(crate::MANIFEST_FILE)).unwrap(),
            manifest
        );
    }

    // Positive control: the same bytes reach our controlled listener when
    // ffprobe is explicitly allowed HLS/HTTP. No external destination is used.
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,http,tcp",
            "-f",
            "hls",
            "-count_frames",
        ])
        .arg(&checkpoint)
        .output()
        .expect("real ffprobe is required");
    assert!(!output.status.success());
    assert!(
        counter.count() > 0,
        "fixture must exercise a real playlist fetch without the guard: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn script(path: &Path, contents: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn probe_stub(root: &Path, audio: bool) -> String {
    let streams = if audio {
        ",{\"codec_type\":\"audio\"}"
    } else {
        ""
    };
    script(&root.join("probe-stub"), &format!(
        "#!/bin/sh\nprintf '%s' '{{\"format\":{{\"duration\":\"0.100\"}},\"streams\":[{{\"codec_type\":\"video\",\"nb_read_frames\":\"1\"}}{streams}]}}'\n"))
}

#[cfg(unix)]
#[test]
fn real_video_and_audio_decoders_block_disguised_hls_independently() {
    for audio in [false, true] {
        let counter = LoopbackCounter::new();
        let root = tempfile::tempdir().unwrap();
        let checkpoint = root.path().join("checkpoint.mp4");
        std::fs::write(&checkpoint, counter.playlist()).unwrap();
        let probe = probe_stub(root.path(), audio);
        let decoder = if audio {
            // Admit the earlier video decode so the real audio command runs.
            script(&root.path().join("decoder"),
                "#!/bin/sh\nfor arg do [ \"$arg\" = 0:v:0 ] && exit 0; done\nexec ffmpeg -f hls \"$@\"\n")
        } else {
            "ffmpeg".into()
        };
        assert!(
            crate::media::verify_checkpoint_media(&decoder, &probe, &checkpoint)
                .unwrap()
                .is_none()
        );
        assert_eq!(counter.count(), 0, "audio={audio}");
    }
}

#[cfg(unix)]
#[test]
fn real_stitch_input_rejects_playlist_even_if_an_earlier_verifier_admitted_it() {
    let counter = LoopbackCounter::new();
    let root = tempfile::tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    publish(&mut owner, counter.playlist().as_bytes()); // journal hash matches
    let checkpoints = owner.manifest().checkpoints.clone();
    drop(owner);
    let probe = probe_stub(root.path(), false);
    // Bypass only the earlier decoder in the fixture, exercising the real
    // production transcode with its own point-of-use input restrictions.
    let decoder = script(
        &root.path().join("decoder"),
        "#!/bin/sh\nfor arg do [ \"$arg\" = 0:v:0 ] && exit 0; done\nexec ffmpeg -f hls \"$@\"\n",
    );
    assert!(
        crate::stitch_complete(root.path(), &checkpoints, &decoder, &probe, "source.mp4").is_err()
    );
    assert_eq!(counter.count(), 0);
    assert!(!root.path().join("source.mp4").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("checkpoints/segment-000000.mp4")).unwrap(),
        counter.playlist()
    );
}

#[test]
fn genuine_mp4_keeps_video_frames_and_audio_decode_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("with-audio.mp4");
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "0.2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ])
        .arg(&path)
        .output()
        .expect("real ffmpeg is required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let media = crate::verify_media("ffmpeg", "ffprobe", &path).unwrap();
    assert_eq!(media.decoded_video_frames, 5);
    assert!(media.has_audio);
    assert_eq!(media.width, Some(32));
    assert!(media.duration_ms.abs_diff(200) <= 20);
}

#[cfg(unix)]
#[test]
fn owned_concat_cannot_expand_its_nested_input_format_beyond_mov() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("nested.mkv");
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=10",
            "-t",
            "0.1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "matroska",
        ])
        .arg(&nested)
        .output()
        .expect("real ffmpeg is required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    publish(&mut owner, b"fixture checkpoint");
    let checkpoints = owner.manifest().checkpoints.clone();
    drop(owner);
    let probe = probe_stub(root.path(), false);
    // Supply a non-MOV child from the fixture's transcode stage, allowing the
    // real concat command to exercise its nested-demuxer policy. The private
    // list itself is still constructed only by production StitchWorkspace.
    let decoder = script(&root.path().join("decoder"),
        "#!/bin/sh\nlast=\nis_concat=\nfor arg do\n [ \"$arg\" = 0:v:0 ] && exit 0\n [ \"$arg\" = concat ] && is_concat=1\n last=$arg\ndone\nif [ \"$is_concat\" = 1 ]; then exec ffmpeg \"$@\"; fi\ncp \"$(dirname \"$0\")/nested.mkv\" \"$last\"\n");
    assert!(
        crate::stitch_complete(root.path(), &checkpoints, &decoder, &probe, "source.mp4").is_err()
    );
    assert!(!root.path().join("source.mp4").exists());
    assert_eq!(
        std::fs::read(root.path().join("checkpoints/segment-000000.mp4")).unwrap(),
        b"fixture checkpoint"
    );
}
