//! ScreenCaptureKit recording-output completion boundary for durable segments.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use screencapturekit::recording_output::{
    RecordingCallbacks, SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};

pub(crate) struct SegmentOutput {
    output: SCRecordingOutput,
    path: String,
    finished: Arc<AtomicBool>,
    failed: Arc<Mutex<Option<String>>>,
}

impl SegmentOutput {
    pub(crate) fn new(path: &Path) -> Result<Self, String> {
        let path_s = path.to_string_lossy().into_owned();
        let config = SCRecordingOutputConfiguration::new()
            .with_output_url(path)
            .with_video_codec(SCRecordingOutputCodec::H264)
            .with_output_file_type(SCRecordingOutputFileType::MP4);
        let finished = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(Mutex::new(None));
        let on_finish = finished.clone();
        let on_fail = failed.clone();
        let callbacks = RecordingCallbacks::new()
            .on_finish(move || on_finish.store(true, Ordering::Relaxed))
            .on_fail(move |error| {
                if let Ok(mut failure) = on_fail.lock() {
                    *failure = Some(error);
                }
            });
        let output = SCRecordingOutput::new_with_delegate(&config, callbacks).ok_or_else(|| {
            "SCRecordingOutput::new_with_delegate returned None (needs macOS 15+)".to_string()
        })?;
        Ok(Self {
            output,
            path: path_s,
            finished,
            failed,
        })
    }

    pub(crate) fn output(&self) -> &SCRecordingOutput {
        &self.output
    }

    /// `remove_recording_output` and `stop_capture` are asynchronous. Never call
    /// the manifest publisher until this completion barrier confirms a closed MP4.
    pub(crate) fn wait_complete(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(error) = self
                .failed
                .lock()
                .ok()
                .and_then(|mut failure| failure.take())
            {
                return Err(error);
            }
            let current = std::fs::metadata(&self.path)
                .map(|meta| meta.len())
                .unwrap_or(0);
            if self.finished.load(Ordering::Relaxed) && current > 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("ScreenCaptureKit did not finalize recording output within 20s".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
