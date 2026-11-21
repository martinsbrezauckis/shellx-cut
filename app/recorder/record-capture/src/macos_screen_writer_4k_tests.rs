//! Full-size native screen-writer rotation while a software export runs.

use super::super::{clock_now_ns, NativeSegment, ScreenWriter};
use super::{message, sxc_screen_writer_fixture_frame, ERROR_CAPACITY};
use screencapturekit::cm::CMSampleBuffer;
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;
const SECONDS: u32 = 31;

struct Export(Child);
impl Drop for Export {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

enum RunDir {
    Temporary(tempfile::TempDir),
    Retained(PathBuf),
}
impl RunDir {
    fn new(fps: u32) -> Self {
        if let Some(parent) = std::env::var_os("SXC_WRITER_4K_EVIDENCE_DIR") {
            let parent = PathBuf::from(parent);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = parent.join(format!("writer-4k-{fps}-{}-{stamp}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self::Retained(path)
        } else {
            Self::Temporary(tempfile::tempdir().unwrap())
        }
    }
    fn path(&self) -> &Path {
        match self {
            Self::Temporary(dir) => dir.path(),
            Self::Retained(path) => path,
        }
    }
}

fn frame(pts: i64, fps: u32, number: u32) -> CMSampleBuffer {
    let mut error = [0; ERROR_CAPACITY];
    let raw = unsafe {
        sxc_screen_writer_fixture_frame(
            WIDTH,
            HEIGHT,
            pts,
            1_000_000_000 / i64::from(fps),
            number,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert!(!raw.is_null(), "{}", message(&error));
    unsafe { CMSampleBuffer::from_ptr(raw) }
}

fn tool(variable: &str, fallback: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| fallback.into())
}

fn packet_times(path: &Path, ffprobe: &str) -> Vec<f64> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "packet probe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["packets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|packet| packet["pts_time"].as_str().unwrap().parse().unwrap())
        .collect()
}

fn frame_hashes(path: &Path, ffmpeg: &str, tail: bool) -> Vec<String> {
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error"]);
    if tail {
        command.args(["-sseof", "-0.5"]);
    }
    command.arg("-i").arg(path);
    command.args([
        "-t",
        "0.5",
        "-an",
        "-vf",
        "scale=32:32",
        "-f",
        "framemd5",
        "-",
    ]);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "frame decode: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.rsplit(',').next().unwrap().trim().to_owned())
        .collect()
}

fn assert_changing_picture(first: &Path, next: &Path, ffmpeg: &str, fps: u32) {
    let mut hashes = frame_hashes(first, ffmpeg, true);
    let prior_count = hashes.len();
    hashes.extend(frame_hashes(next, ffmpeg, false));
    assert!(
        prior_count >= 5 && hashes.len() >= prior_count + 5,
        "{fps} fps: insufficient decoded pictures around the boundary"
    );
    assert!(
        hashes.windows(2).all(|pair| pair[0] != pair[1]),
        "{fps} fps: repeated picture around a segment boundary"
    );
}

fn start_export(root: &Path, ffmpeg: &str) -> Export {
    let log = File::create(root.join("export.stderr.log")).unwrap();
    let child = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=60",
            "-t",
            "35",
            "-threads",
            "2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(root.join("export.mp4"))
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    Export(child)
}

fn wait_export(export: &mut Export, root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(status) = export.0.try_wait().unwrap() {
            assert!(
                status.success(),
                "concurrent export failed: {}",
                std::fs::read_to_string(root.join("export.stderr.log")).unwrap_or_default()
            );
            assert!(root.join("export.mp4").is_file());
            return;
        }
        assert!(Instant::now() < deadline, "concurrent export timed out");
        thread::sleep(Duration::from_millis(100));
    }
}

fn run_at_fps(fps: u32) {
    let run = RunDir::new(fps);
    let paths = [
        run.path().join("first.mp4"),
        run.path().join("second.mp4"),
        run.path().join("third.mp4"),
    ];
    let ffmpeg = tool("SHELLX_RECORD_FFMPEG", "ffmpeg");
    let ffprobe = tool("SHELLX_RECORD_FFPROBE", "ffprobe");
    let mut export = start_export(run.path(), &ffmpeg);
    let writer = ScreenWriter::new(NativeSegment::new(&paths[0], WIDTH, HEIGHT, fps).unwrap());
    let base = clock_now_ns();
    let started = Instant::now();
    let mut finishers = Vec::new();
    for number in 0..SECONDS * fps {
        let deadline =
            started + Duration::from_nanos(u64::from(number) * 1_000_000_000 / u64::from(fps));
        if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            thread::sleep(remaining);
        }
        if number == 15 * fps || number == 30 * fps {
            assert!(
                export.0.try_wait().unwrap().is_none(),
                "export ended before the {number} frame boundary"
            );
            let index = (number / (15 * fps)) as usize;
            let next = NativeSegment::new(&paths[index], WIDTH, HEIGHT, fps).unwrap();
            let (old, _) = writer.switch(Some(next)).unwrap();
            let end_pts = base + i64::from(number) * 1_000_000_000 / i64::from(fps);
            finishers.push(thread::spawn(move || {
                thread::sleep(Duration::from_millis(600));
                old.finish(end_pts).unwrap();
                old.stats()
            }));
        }
        let pts = base + i64::from(number) * 1_000_000_000 / i64::from(fps);
        writer.append(&frame(pts, fps, number));
        writer.window().unwrap();
    }
    let (last, _) = writer.switch(None).unwrap();
    last.finish(base + i64::from(SECONDS) * 1_000_000_000)
        .unwrap();
    let mut counts: Vec<_> = finishers
        .into_iter()
        .map(|handle| handle.join().unwrap().2)
        .collect();
    counts.push(last.stats().2);
    assert_eq!(
        counts,
        [u64::from(15 * fps), u64::from(15 * fps), u64::from(fps)]
    );
    wait_export(&mut export, run.path());

    let mut all_times = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let mut times = packet_times(path, &ffprobe);
        assert_eq!(times.len(), counts[index] as usize);
        assert!(times.windows(2).all(|pair| pair[1] > pair[0]));
        let offset = if index == 0 { 0.0 } else { 15.0 * index as f64 };
        all_times.extend(times.drain(..).map(|time| time + offset));
    }
    assert_eq!(all_times.len(), (SECONDS * fps) as usize);
    let maximum_gap = all_times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0_f64, f64::max);
    assert!(
        maximum_gap <= 2.0 / f64::from(fps) + 0.000_001,
        "{fps} fps: maximum packet gap {maximum_gap:.6}s"
    );
    assert_changing_picture(&paths[0], &paths[1], &ffmpeg, fps);
    assert_changing_picture(&paths[1], &paths[2], &ffmpeg, fps);
    println!(
        "{fps} fps 4K: {} frames, maximum packet gap {maximum_gap:.6}s, both picture seams change, concurrent export complete; files at {}",
        all_times.len(),
        run.path().display(),
    );
}

#[test]
#[ignore = "31 seconds of real-time 4K capture plus concurrent video export at 30 and 60 fps"]
fn full_size_rotation_preserves_picture_under_export_load() {
    for fps in [30, 60] {
        run_at_fps(fps);
    }
}
