use super::*;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn frame(pts: i64, fps: u32, number: u32) -> CMSampleBuffer {
    let mut error = [0; ERROR_CAPACITY];
    let raw = unsafe {
        sxc_screen_writer_fixture_frame(
            256,
            144,
            pts,
            1_000_000_000 / i64::from(fps),
            number,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert!(!raw.is_null(), "{}", message(&error));
    // The fixture returns a +1 sample reference, consumed by the Rust wrapper.
    unsafe { CMSampleBuffer::from_ptr(raw) }
}

fn packet_times(path: &Path) -> Vec<f64> {
    let tool = std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
    let out = Command::new(tool)
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
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    data["packets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["pts_time"].as_str().unwrap().parse().unwrap())
        .collect()
}

#[test]
fn macos_screen_writer_rotation_keeps_every_sample_while_old_completion_is_delayed() {
    for fps in [30, 60] {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.mp4");
        let next_path = dir.path().join("next.mp4");
        let writer = ScreenWriter::new(NativeSegment::new(&first_path, 256, 144, fps).unwrap());
        let base = clock_now_ns();
        let timestamp = |n: u32| base + i64::from(n) * 1_000_000_000 / i64::from(fps);
        for n in 0..15 * fps {
            writer.append(&frame(timestamp(n), fps, n));
        }
        let next = NativeSegment::new(&next_path, 256, 144, fps).unwrap();
        let (old, _) = writer.switch(Some(next)).unwrap();
        // Slow finalization/publication must not make the next writer wait.
        let finish_old = thread::spawn(move || {
            thread::sleep(Duration::from_millis(600));
            old.finish(base + 15_000_000_000).unwrap();
            old.stats()
        });
        for n in 15 * fps..31 * fps {
            writer.append(&frame(timestamp(n), fps, n));
        }
        let (next, _) = writer.switch(None).unwrap();
        next.finish(base + 31_000_000_000).unwrap();
        let prior_stats = finish_old.join().unwrap();
        let next_stats = next.stats();
        assert_eq!(prior_stats.2, u64::from(15 * fps));
        assert_eq!(next_stats.2, u64::from(16 * fps));
        assert_eq!(next_stats.0, timestamp(15 * fps));
        let mut times = packet_times(&first_path);
        assert_eq!(times.len(), (15 * fps) as usize);
        let next_times = packet_times(&next_path);
        assert_eq!(next_times.len(), (16 * fps) as usize);
        times.extend(next_times.into_iter().map(|t| t + 15.0));
        let maximum_gap = times
            .windows(2)
            .map(|p| p[1] - p[0])
            .fold(0.0_f64, f64::max);
        assert!(
            maximum_gap <= 2.0 / f64::from(fps) + 0.000001,
            "{fps} fps: {maximum_gap}s gap"
        );
        println!("{fps} fps: {} packets, max gap {maximum_gap:.6}s, boundary PTS preserved, delayed finish 600ms", times.len());
    }
}

#[test]
fn macos_screen_writer_rejects_duplicate_timestamps_and_empty_movies() {
    let dir = tempfile::tempdir().unwrap();
    let native = NativeSegment::new(&dir.path().join("movie.mp4"), 256, 144, 30).unwrap();
    let sample = frame(1_000_000_000, 30, 0);
    native.append(&sample).unwrap();
    assert!(native.append(&sample).unwrap_err().contains("backwards"));
    native.finish(1_033_333_333).unwrap();
    assert_eq!(native.stats().2, 1);
    // A completed close can be retried by the pause close state machine.
    native.finish(1_033_333_333).unwrap();
    let empty = NativeSegment::new(&dir.path().join("empty.mp4"), 256, 144, 30).unwrap();
    assert!(empty.finish(0).unwrap_err().contains("no picture"));
}

#[test]
fn macos_screen_writer_releases_queued_samples_after_early_owner_drop() {
    let dir = tempfile::tempdir().unwrap();
    let native = NativeSegment::new(&dir.path().join("aborted.mp4"), 256, 144, 30).unwrap();
    for n in 0..30 {
        native
            .append(&frame(1_000_000_000 + i64::from(n) * 33_333_333, 30, n))
            .unwrap();
    }
    drop(native);
    // Queue blocks retain their owner and samples after the admission handle
    // is gone; a second writer can start and close independently.
    let next = NativeSegment::new(&dir.path().join("next.mp4"), 256, 144, 30).unwrap();
    next.append(&frame(2_000_000_000, 30, 1)).unwrap();
    next.finish(2_033_333_333).unwrap();
    assert_eq!(next.stats().2, 1);
}

extern "C" {
    fn sxc_screen_writer_fixture_frame(
        width: u32,
        height: u32,
        pts_ns: i64,
        duration_ns: i64,
        number: u32,
        error: *mut c_char,
        capacity: usize,
    ) -> *mut c_void;
}

#[path = "macos_screen_writer_recovery_tests.rs"]
mod recovery_tests;

struct ExportLoad(std::process::Child);
impl Drop for ExportLoad {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn macos_screen_writer_keeps_rotation_samples_during_export() {
    let tool = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let dir = tempfile::tempdir().unwrap();
    let mut load = ExportLoad(
        Command::new(tool)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1920x1080:rate=60",
                "-t",
                "4",
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-threads",
                "2",
                "-y",
            ])
            .arg(dir.path().join("export.mp4"))
            .spawn()
            .unwrap(),
    );
    macos_screen_writer_rotation_keeps_every_sample_while_old_completion_is_delayed();
    assert!(load.0.wait().unwrap().success());
    println!(
        "concurrent 1080p60 H.264 export completed; both rotation rates preserved every packet"
    );
}

#[test]
fn macos_screen_writer_stop_drains_the_serial_delivery_queue() {
    let dir = tempfile::tempdir().unwrap();
    let writer = ScreenWriter::new(
        NativeSegment::new(&dir.path().join("drained.mp4"), 256, 144, 30).unwrap(),
    );
    let pending = writer.clone();
    let sample = frame(1_000_000_000, 30, 1);
    let boxed = Box::new((pending, sample));
    unsafe extern "C" fn deliver(context: *mut c_void) {
        let (writer, sample) =
            *unsafe { Box::from_raw(context.cast::<(ScreenWriter, CMSampleBuffer)>()) };
        thread::sleep(Duration::from_millis(40));
        writer.append(&sample);
    }
    unsafe {
        dispatch_async_f(
            writer.delivery_queue().as_ptr(),
            Box::into_raw(boxed).cast(),
            deliver,
        );
    }
    writer.drain().unwrap();
    let (old, _) = writer.switch(None).unwrap();
    old.finish(1_033_333_333).unwrap();
    assert_eq!(old.stats().2, 1);
}

extern "C" {
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void),
    );
}

#[path = "macos_screen_writer_4k_tests.rs"]
mod full_size_tests;

#[path = "macos_screen_writer_av_tests.rs"]
mod av_tests;

#[test]
fn macos_screen_writer_drains_pre_stop_pictures_and_excludes_later_pictures() {
    let dir = tempfile::tempdir().unwrap();
    let writer = ScreenWriter::new(
        NativeSegment::new(&dir.path().join("bounded.mp4"), 256, 144, 30).unwrap(),
    );
    let before = clock_now_ns() - 20_000_000;
    writer.request_stop();
    writer.append(&frame(before, 30, 1));
    writer.append(&frame(clock_now_ns() + 20_000_000, 30, 2));
    let (native, boundary) = writer.switch(None).unwrap();
    native.finish(boundary.pts_ns).unwrap();
    assert_eq!(native.stats().2, 1);
    assert_eq!(native.stats().0, before);
}
