//! Native PipeWire system-audio capture. First nonempty `process` buffer is timestamped on `Instant`
//! before WAV I/O; process startup and buffered writes never set placement.
use pipewire as pw;
use pw::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pw::spa::param::ParamType;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Pod, Value};
use pw::{properties::properties, spa};
use record_core::{error_codes, RecordError, Result};
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::linux_system_audio_target::default_sink_target;
use crate::system_audio_timing::{SystemAudioCapture, SystemAudioTimingTracker};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const BITS_PER_SAMPLE: u16 = 16;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct State {
    format: AudioInfoRaw,
    timing: SystemAudioTimingTracker,
    writer: Option<hound::WavWriter<BufWriter<File>>>,
    error: Option<String>,
}

fn capture_error(context: &str, error: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, context, error.to_string()).with_action(
        "ensure PipeWire is running in the logged-in desktop session and a default audio sink is available",
    )
}

fn io_error(context: &str, error: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::IO, context, error.to_string())
}

fn packet_samples(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(usize::from(BITS_PER_SAMPLE / 8)) {
        return None;
    }
    let samples = bytes.len() / usize::from(BITS_PER_SAMPLE / 8);
    samples
        .is_multiple_of(usize::try_from(CHANNELS).ok()?)
        .then_some(samples)
}

fn record_packet(
    state: &mut State,
    bytes: &[u8],
    received_at: Duration,
) -> std::result::Result<(), String> {
    let samples = packet_samples(bytes).ok_or_else(|| {
        "PipeWire delivered a nonempty system-audio buffer with an invalid S16LE frame size"
            .to_string()
    })?;
    // Timestamp a real packet before decoding or disk I/O adds scheduling delay.
    state
        .timing
        .record_packet(samples, received_at)
        .map_err(|error| error.to_string())?;
    let writer = state
        .writer
        .as_mut()
        .ok_or_else(|| "system-audio WAV writer was unavailable".to_string())?;
    for sample in bytes.chunks_exact(2) {
        writer
            .write_sample(i16::from_le_bytes([sample[0], sample[1]]))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn set_error(state: &mut State, error: impl Into<String>) {
    if state.error.is_none() {
        state.error = Some(error.into());
    }
}

/// Capture the default PipeWire sink monitor to a 48 kHz stereo WAV.
///
pub fn capture_system_pipewire(
    path: &Path,
    duration_ms: Option<u64>,
    stop: Arc<AtomicBool>,
    capture_started: Instant,
) -> Result<SystemAudioCapture> {
    let parent = path.parent().ok_or_else(|| {
        io_error(
            "reserve system audio staging",
            "system audio path has no parent",
        )
    })?;
    let (part, file) = record_recovery::create_staging_file(parent, "system-wav")
        .map_err(|error| io_error("reserve system audio staging", error))?;
    let writer = hound::WavWriter::new(
        BufWriter::new(file),
        hound::WavSpec {
            channels: u16::try_from(CHANNELS).expect("fixed channel count fits u16"),
            sample_rate: SAMPLE_RATE,
            bits_per_sample: BITS_PER_SAMPLE,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|error| io_error("create system audio", error))?;

    let result = (|| -> Result<SystemAudioCapture> {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)
            .map_err(|error| capture_error("create PipeWire main loop", error))?;
        let context = pw::context::ContextRc::new(&mainloop, None)
            .map_err(|error| capture_error("create PipeWire context", error))?;
        let core = context
            .connect_rc(None)
            .map_err(|error| capture_error("connect PipeWire", error))?;
        let sink = default_sink_target(&core, &mainloop)
            .map_err(|error| capture_error("resolve PipeWire default audio sink", error))?;

        let state = Rc::new(RefCell::new(State {
            format: AudioInfoRaw::new(),
            timing: SystemAudioTimingTracker::default(),
            writer: Some(writer),
            error: None,
        }));
        let mut properties = properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::NODE_LATENCY => "1024/48000",
        };
        properties.insert("target.object", sink);
        let stream = pw::stream::StreamBox::new(&core, "shellx-cut-system-audio", properties)
            .map_err(|error| capture_error("create PipeWire system-audio stream", error))?;

        let state_format = state.clone();
        let state_process = state.clone();
        let error_loop = mainloop.downgrade();
        let listener = stream
            .add_local_listener_with_user_data(())
            .param_changed(move |_, _, id, param| {
                let Some(param) = param else { return };
                if id != ParamType::Format.as_raw() {
                    return;
                }
                let mut format = AudioInfoRaw::new();
                if format.parse(param).is_err() {
                    return;
                }
                let mut state = state_format.borrow_mut();
                if format.format() != AudioFormat::S16LE
                    || format.rate() != SAMPLE_RATE
                    || format.channels() != CHANNELS
                {
                    set_error(
                        &mut state,
                        format!(
                            "PipeWire negotiated unsupported system-audio format {:?}/{}/{}",
                            format.format(),
                            format.rate(),
                            format.channels()
                        ),
                    );
                    return;
                }
                state.format = format;
            })
            .process(move |stream, _| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Ok(mut state) = state_process.try_borrow_mut() else {
                    return;
                };
                if state.error.is_some() {
                    return;
                }
                if state.format.format() != AudioFormat::S16LE {
                    set_error(
                        &mut state,
                        "PipeWire delivered audio before format negotiation",
                    );
                    if let Some(mainloop) = error_loop.upgrade() {
                        mainloop.quit();
                    }
                    return;
                }
                let Some(data) = buffer.datas_mut().first_mut() else {
                    set_error(
                        &mut state,
                        "PipeWire delivered a system-audio buffer without data",
                    );
                    return;
                };
                let chunk = data.chunk();
                let Ok(offset) = usize::try_from(chunk.offset()) else {
                    set_error(&mut state, "PipeWire system-audio buffer offset overflowed");
                    return;
                };
                let Ok(size) = usize::try_from(chunk.size()) else {
                    set_error(&mut state, "PipeWire system-audio buffer size overflowed");
                    return;
                };
                if size == 0 {
                    return;
                }
                let Some(end) = offset.checked_add(size) else {
                    set_error(&mut state, "PipeWire system-audio buffer range overflowed");
                    return;
                };
                let Some(bytes) = data.data().and_then(|data| data.get(offset..end)) else {
                    set_error(&mut state, "PipeWire system-audio payload was not mappable");
                    return;
                };
                if let Err(error) = record_packet(&mut state, bytes, capture_started.elapsed()) {
                    set_error(&mut state, error);
                    if let Some(mainloop) = error_loop.upgrade() {
                        mainloop.quit();
                    }
                }
            })
            .register()
            .map_err(|error| capture_error("register PipeWire system-audio stream", error))?;

        let mut info = AudioInfoRaw::new();
        info.set_format(AudioFormat::S16LE);
        info.set_rate(SAMPLE_RATE);
        info.set_channels(CHANNELS);
        let object = spa::pod::Object {
            type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: ParamType::EnumFormat.as_raw(),
            properties: info.into(),
        };
        let values =
            PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(object))
                .map_err(|error| capture_error("serialize PipeWire system-audio format", error))?
                .0
                .into_inner();
        let mut params = [Pod::from_bytes(&values).ok_or_else(|| {
            capture_error("build PipeWire system-audio format", "invalid format pod")
        })?];
        stream
            .connect(
                spa::utils::Direction::Input,
                None,
                pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .map_err(|error| capture_error("connect PipeWire system-audio stream", error))?;

        let deadline = duration_ms
            .and_then(|duration| capture_started.checked_add(Duration::from_millis(duration)));
        let weak = mainloop.downgrade();
        let timer = mainloop.loop_().add_timer(move |_| {
            if stop.load(Ordering::Relaxed)
                || deadline.is_some_and(|deadline| Instant::now() >= deadline)
            {
                if let Some(mainloop) = weak.upgrade() {
                    mainloop.quit();
                }
            }
        });
        timer
            .update_timer(Some(POLL_INTERVAL), Some(POLL_INTERVAL))
            .into_result()
            .map_err(|error| capture_error("arm PipeWire system-audio timer", error))?;
        mainloop.run();

        drop(timer);
        drop(listener);
        stream
            .disconnect()
            .map_err(|error| capture_error("disconnect PipeWire system-audio stream", error))?;
        let mut state = state.borrow_mut();
        if let Some(error) = state.error.take() {
            return Err(capture_error("capture PipeWire system audio", error));
        }
        let timing = state.timing.first_packet_offset_ms();
        state
            .writer
            .take()
            .ok_or_else(|| io_error("finalize system audio", "WAV writer was unavailable"))?
            .finalize()
            .map_err(|error| io_error("finalize system audio", error))?;
        record_recovery::publish_new_synced(&part, path)
            .map_err(|error| io_error("publish system audio", error))?;
        Ok(SystemAudioCapture {
            path: path.to_string_lossy().into_owned(),
            first_packet_offset_ms: timing,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{packet_samples, record_packet, State};
    use crate::system_audio_timing::SystemAudioTimingTracker;
    use std::fs::File;
    use std::io::BufWriter;
    use std::time::Duration;

    fn state() -> State {
        let path = tempfile::NamedTempFile::new().unwrap();
        let writer = hound::WavWriter::new(
            BufWriter::new(File::create(path.path()).unwrap()),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        State {
            format: Default::default(),
            timing: SystemAudioTimingTracker::default(),
            writer: Some(writer),
            error: None,
        }
    }

    #[test]
    fn first_nonempty_pipewire_packet_keeps_a_delayed_capture_clock_offset() {
        let mut state = state();
        assert_eq!(packet_samples(&[]), None, "empty buffers are not packets");
        record_packet(&mut state, &[0, 0, 1, 0], Duration::from_millis(2_350)).unwrap();
        assert_eq!(state.timing.first_packet_offset_ms(), Some(2_350));
    }

    #[test]
    fn malformed_packet_cannot_create_timing_or_wav_samples() {
        let mut state = state();
        assert!(record_packet(&mut state, &[0], Duration::ZERO).is_err());
        assert_eq!(state.timing.first_packet_offset_ms(), None);
    }
}
