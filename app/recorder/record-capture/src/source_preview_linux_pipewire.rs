//! Portal-granted PipeWire raw-frame pump for the memory-only preview owner.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::param::ParamType;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Pod, Value};
use pw::{properties::properties, spa};

use crate::source_preview::SOURCE_PREVIEW_MIN_FRAME_INTERVAL_MS;
use crate::source_preview_bitmap::SourcePreviewPixelFormat;
use crate::source_preview_native::{NativeSourcePreviewPixels, SourcePreviewMailbox};
use crate::wayland_pw::valid_chunk_data;

pub(crate) struct PipewirePreviewRequest {
    pub pw_fd: OwnedFd,
    pub node: u32,
    pub stop: Arc<AtomicBool>,
    pub mailbox: SourcePreviewMailbox,
}

struct PreviewState {
    start: Instant,
    width: u32,
    height: u32,
    format: Option<SourcePreviewPixelFormat>,
    last_frame_at_ms: Option<u64>,
    mailbox: SourcePreviewMailbox,
}

/// Runs on its own blocking thread: PipeWire's loop, stream, and listener stay
/// thread-local; only the bounded mailbox crosses back to the recorder owner.
pub(crate) fn preview(request: PipewirePreviewRequest) -> Result<(), String> {
    pw::init();
    let mainloop =
        pw::main_loop::MainLoopRc::new(None).map_err(|error| format!("pw mainloop: {error}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|error| format!("pw context: {error}"))?;
    let core = context
        .connect_fd_rc(request.pw_fd, None)
        .map_err(|error| format!("pw connect_fd: {error}"))?;
    let state = Rc::new(RefCell::new(PreviewState {
        start: Instant::now(),
        width: 0,
        height: 0,
        format: None,
        last_frame_at_ms: None,
        mailbox: request.mailbox,
    }));
    let stream = pw::stream::StreamBox::new(
        &core,
        "shellx-source-preview",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|error| format!("pw stream: {error}"))?;

    let parameter_state = state.clone();
    let process_state = state.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .param_changed(move |_, _, id, param| {
            let Some(param) = param else { return };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let mut info = VideoInfoRaw::default();
            if info.parse(param).is_err() {
                return;
            }
            let Some(format) = pixel_format(info.format()) else {
                parameter_state.borrow().mailbox.mark_unavailable();
                return;
            };
            let width = info.size().width;
            let height = info.size().height;
            if width == 0 || height == 0 {
                parameter_state.borrow().mailbox.mark_unavailable();
                return;
            }
            let mut state = parameter_state.borrow_mut();
            state.width = width;
            state.height = height;
            state.format = Some(format);
        })
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Ok(mut state) = process_state.try_borrow_mut() else {
                return;
            };
            let Some(format) = state.format else { return };
            let captured_at_ms =
                u64::try_from(state.start.elapsed().as_millis()).unwrap_or(u64::MAX);
            if state.last_frame_at_ms.is_some_and(|previous| {
                captured_at_ms.saturating_sub(previous) < SOURCE_PREVIEW_MIN_FRAME_INTERVAL_MS
            }) {
                return;
            }
            let (width, height) = (state.width, state.height);
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            if data.as_raw().chunk.is_null() {
                return;
            }
            let chunk = data.chunk();
            let (Some(size), Some(offset), Some(stride), Some(mapped)) = (
                usize::try_from(chunk.size()).ok(),
                usize::try_from(chunk.offset()).ok(),
                usize::try_from(chunk.stride().max(0)).ok(),
                data.data(),
            ) else {
                return;
            };
            let Some(valid) = valid_chunk_data(mapped, offset, size) else {
                return;
            };
            if state
                .mailbox
                .publish_native_pixels(NativeSourcePreviewPixels {
                    captured_at_ms,
                    width,
                    height,
                    format,
                    stride,
                    pixels: valid,
                    offset,
                })
                .is_ok()
            {
                state.last_frame_at_ms = Some(captured_at_ms);
            }
        })
        .register()
        .map_err(|error| format!("pw register: {error}"))?;

    // `Pod` borrows the serialized storage, which stays live for this synchronous
    // `connect` call. PipeWire copies the parameter during connection setup.
    let parameter_bytes = preview_format_param()?;
    let parameter =
        Pod::from_bytes(&parameter_bytes).ok_or_else(|| "build preview format pod".to_string())?;
    let mut params = [parameter];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(request.node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|error| format!("pw connect: {error}"))?;

    let stop = request.stop;
    let weak = mainloop.downgrade();
    let timer = mainloop.loop_().add_timer(move |_| {
        if stop.load(Ordering::Relaxed) {
            if let Some(mainloop) = weak.upgrade() {
                mainloop.quit();
            }
        }
    });
    let _ = timer.update_timer(
        Some(Duration::from_millis(25)),
        Some(Duration::from_millis(25)),
    );
    mainloop.run();
    Ok(())
}

fn pixel_format(format: VideoFormat) -> Option<SourcePreviewPixelFormat> {
    match format {
        VideoFormat::BGRx => Some(SourcePreviewPixelFormat::Bgrx),
        VideoFormat::BGRA => Some(SourcePreviewPixelFormat::Bgra),
        VideoFormat::RGBx => Some(SourcePreviewPixelFormat::Rgbx),
        VideoFormat::RGBA => Some(SourcePreviewPixelFormat::Rgba),
        VideoFormat::RGB => Some(SourcePreviewPixelFormat::Rgb),
        VideoFormat::BGR => Some(SourcePreviewPixelFormat::Bgr),
        _ => None,
    }
}

fn preview_format_param() -> Result<Vec<u8>, String> {
    let object = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaType,
            Id,
            pw::spa::param::format::MediaType::Video
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaSubtype,
            Id,
            pw::spa::param::format::MediaSubtype::Raw
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::RGBx,
            VideoFormat::BGRA,
            VideoFormat::RGBA,
            VideoFormat::RGB,
            VideoFormat::BGR,
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            pw::spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            pw::spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            pw::spa::utils::Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            pw::spa::utils::Fraction { num: 10, denom: 1 },
            pw::spa::utils::Fraction { num: 0, denom: 1 },
            pw::spa::utils::Fraction { num: 10, denom: 1 }
        ),
    );
    let bytes = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(object))
        .map_err(|error| format!("serialize preview format: {error}"))?
        .0
        .into_inner();
    Ok(bytes)
}
