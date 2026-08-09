//! Capture-audio preparation for the planned MP4 export.
//!
//! Recorder sources publish mic and system WAVs beside the capture video. Export
//! must consume those fenced artifacts through the normal compositor, not bypass
//! it with the raw stream-copy mux. A delayed system packet is represented as
//! real silence in a temporary WAV so the rendered audio timeline stays aligned.

use std::path::{Path, PathBuf};

use cut_core::{error_codes, CutError};

const RETRY_EXPORT_ACTION: &str =
    "remove the unsafe or incomplete capture audio artifact and retry screen_record.export";

#[derive(Clone, Debug)]
pub(crate) struct CaptureExportAudio {
    mic: Option<PathBuf>,
    system: Option<PathBuf>,
    system_offset_ms: u64,
}

pub(crate) struct PreparedExportAudio {
    direct: Option<PathBuf>,
    // Keeping the tempfile alive makes the prepared mix available to the
    // compositor for the complete duration of the render job.
    _staged_mix: Option<tempfile::NamedTempFile>,
}

impl PreparedExportAudio {
    pub(crate) fn path(&self) -> Option<&Path> {
        self.direct.as_deref()
    }
}

/// Resolve recorder siblings only when the source is directly inside the
/// project-local capture root. Other project media keeps the renderer's normal
/// source-audio fallback and never inherits a nearby arbitrary WAV.
pub(crate) fn for_source(
    project_dir: &Path,
    source: &Path,
) -> Result<CaptureExportAudio, CutError> {
    let Some(capture_dir) = capture_dir_for_source(project_dir, source)? else {
        return Ok(CaptureExportAudio {
            mic: None,
            system: None,
            system_offset_ms: 0,
        });
    };
    let mic = super::optional_plain_file_in_dir(
        &capture_dir,
        "mic.wav",
        "recording microphone audio",
        RETRY_EXPORT_ACTION,
    )?;
    let mut system = super::optional_plain_file_in_dir(
        &capture_dir,
        "system.wav",
        "recording system audio",
        RETRY_EXPORT_ACTION,
    )?;
    let timing = system
        .as_ref()
        .map(|_| super::system_audio::read_timing(&capture_dir))
        .transpose()?
        .flatten();
    if timing
        .as_ref()
        .is_some_and(|timing| timing.first_packet_offset_ms.is_none())
    {
        system = None;
    }
    Ok(CaptureExportAudio {
        mic,
        system,
        system_offset_ms: timing
            .and_then(|timing| timing.first_packet_offset_ms)
            .unwrap_or(0),
    })
}

impl CaptureExportAudio {
    /// Return a renderer-ready audio input. A one-source capture passes its
    /// validated WAV directly. Two sources (or a delayed system-only source) are
    /// prepared as PCM before the normal planned render; this is deliberately not
    /// the raw mux path, so the EditPlan still controls the output video.
    pub(crate) fn prepare(
        &self,
        output_dir: &Path,
        control: &record_render::ffmpeg::ProcessControl,
    ) -> Result<PreparedExportAudio, CutError> {
        match (&self.mic, &self.system) {
            (None, None) => Ok(PreparedExportAudio {
                direct: None,
                _staged_mix: None,
            }),
            (Some(mic), None) => Ok(PreparedExportAudio {
                direct: Some(mic.clone()),
                _staged_mix: None,
            }),
            (None, Some(system)) if self.system_offset_ms == 0 => Ok(PreparedExportAudio {
                direct: Some(system.clone()),
                _staged_mix: None,
            }),
            _ => self.prepare_mix(output_dir, control),
        }
    }

    fn prepare_mix(
        &self,
        output_dir: &Path,
        control: &record_render::ffmpeg::ProcessControl,
    ) -> Result<PreparedExportAudio, CutError> {
        let staged_mix = tempfile::Builder::new()
            .prefix(".cut-recorder-export-audio-")
            .suffix(".wav")
            .tempfile_in(output_dir)
            .map_err(|error| {
                CutError::new(
                    error_codes::IO,
                    format!("could not create secure recording-audio staging: {error}"),
                    "preparing recorder audio for export failed",
                )
            })?;
        let path = staged_mix.path().to_path_buf();
        super::align_ffmpeg_env();
        let mut command = std::process::Command::new(cut_media::toolpath::ffmpeg());
        command.args(["-v", "error", "-y"]);
        let filter = match (&self.mic, &self.system) {
            (None, Some(system)) => {
                command.arg("-i").arg(system);
                format!(
                    "[0:a]aresample=48000,adelay={}:all=1[a]",
                    self.system_offset_ms
                )
            }
            (Some(mic), Some(system)) => {
                command.arg("-i").arg(mic).arg("-i").arg(system);
                format!(
                    "[0:a]aresample=48000[m];[1:a]aresample=48000,adelay={}:all=1[s];[m][s]amix=inputs=2:duration=longest:normalize=0[a]",
                    self.system_offset_ms
                )
            }
            _ => unreachable!("single direct audio source does not need a mix"),
        };
        command
            .args([
                "-filter_complex",
                &filter,
                "-map",
                "[a]",
                "-c:a",
                "pcm_s16le",
                "-ar",
                "48000",
                "-ac",
                "2",
            ])
            .arg(&path);
        let output = record_render::ffmpeg::command_output_with_control(
            &mut command,
            control,
            "recording export audio mix ffmpeg spawn",
        )
        .map_err(super::record_err)?;
        if !output.status.success() {
            let tail = String::from_utf8_lossy(&output.stderr);
            let last = tail.lines().last().unwrap_or("ffmpeg failed");
            return Err(CutError::new(
                error_codes::FFMPEG,
                format!("recording export audio mix failed: {last}"),
                "ffmpeg could not align the validated microphone and system-audio capture sources",
            ));
        }
        Ok(PreparedExportAudio {
            direct: Some(path),
            _staged_mix: Some(staged_mix),
        })
    }
}

fn capture_dir_for_source(project_dir: &Path, source: &Path) -> Result<Option<PathBuf>, CutError> {
    let Some(capture_dir) = source.parent() else {
        return Ok(None);
    };
    let cache_dir = super::screen_record_cache_dir(project_dir)?;
    let cache_dir = std::fs::canonicalize(&cache_dir).map_err(|error| {
        CutError::new(
            error_codes::IO,
            format!("could not resolve the screen-record capture root: {error}"),
            RETRY_EXPORT_ACTION,
        )
    })?;
    Ok((capture_dir.parent() == Some(cache_dir.as_path())).then(|| capture_dir.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::for_source;

    #[test]
    fn only_direct_capture_sources_resolve_sibling_audio() {
        let project = tempfile::tempdir().unwrap();
        let capture = project.path().join("cache/screen_record/cap-test");
        std::fs::create_dir_all(&capture).unwrap();
        let source = capture.join("source.mp4");
        std::fs::write(&source, b"video").unwrap();
        std::fs::write(capture.join("mic.wav"), b"mic").unwrap();

        let resolved = for_source(project.path(), &source.canonicalize().unwrap()).unwrap();
        assert!(resolved.mic.is_some());

        let nested = capture.join("nested/source.mp4");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, b"video").unwrap();
        let resolved = for_source(project.path(), &nested.canonicalize().unwrap()).unwrap();
        assert!(resolved.mic.is_none());
    }

    #[test]
    fn null_timed_system_audio_is_omitted_and_bad_timing_fails_closed() {
        let project = tempfile::tempdir().unwrap();
        let capture = project.path().join("cache/screen_record/cap-system");
        std::fs::create_dir_all(&capture).unwrap();
        let source = capture.join("source.mp4");
        std::fs::write(&source, b"video").unwrap();
        std::fs::write(capture.join("system.wav"), b"system").unwrap();
        let timing = capture.join("system-audio.json");
        std::fs::write(
            &timing,
            br#"{"schema":"shellx-cut/system-audio-timing/1","first_packet_offset_ms":null}"#,
        )
        .unwrap();

        let resolved = for_source(project.path(), &source.canonicalize().unwrap()).unwrap();
        assert!(
            resolved.system.is_none(),
            "null-timed system WAV must be skipped"
        );

        std::fs::write(&timing, b"not json").unwrap();
        assert!(
            for_source(project.path(), &source.canonicalize().unwrap()).is_err(),
            "malformed current-format timing must not fall back to zero"
        );
    }
}
