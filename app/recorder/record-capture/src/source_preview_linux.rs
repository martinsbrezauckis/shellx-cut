//! Ephemeral Portal + PipeWire source-preview owner for Linux.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use enumflags2::BitFlags;
use record_core::{error_codes, RecordError, Result};

use crate::linux_portal;
use crate::linux_runtime::{cap_err, shared_runtime};
use crate::source_preview::{SourcePreviewRequest, SourcePreviewSource};
use crate::source_preview_linux_pipewire::{preview, PipewirePreviewRequest};
use crate::source_preview_native::{NativeSourcePreviewSession, SourcePreviewMailbox};
use crate::CaptureRegion;

/// Starts one portal-picked preview. There is intentionally no restore token or
/// persisted source identity: each start remains user-consented and memory-only.
pub(crate) fn start(
    request: SourcePreviewRequest,
    generation: u64,
    region: Option<CaptureRegion>,
) -> Result<NativeSourcePreviewSession> {
    validate_portal_selection(&request, region)?;
    let mailbox = SourcePreviewMailbox::default();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_mailbox = mailbox.clone();
    let worker = std::thread::Builder::new()
        .name("shellx-source-preview-portal".into())
        .spawn(move || {
            if run_portal_preview(worker_mailbox.clone(), worker_stop.clone()).is_err()
                && !worker_stop.load(Ordering::Relaxed)
            {
                worker_mailbox.mark_unavailable();
            }
        })
        .map_err(|error| {
            RecordError::new(
                error_codes::CAPTURE,
                "start portal preview worker",
                error.to_string(),
            )
        })?;

    Ok(NativeSourcePreviewSession::new(
        generation,
        mailbox,
        move || {
            stop.store(true, Ordering::Relaxed);
            worker.join().map_err(|_| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "portal preview worker stopped unexpectedly",
                    "join failed",
                )
            })
        },
    ))
}

fn run_portal_preview(mailbox: SourcePreviewMailbox, stop: Arc<AtomicBool>) -> Result<()> {
    let runtime = shared_runtime()?;
    runtime.block_on(async move {
        let deadline = linux_portal::pre_first_frame_deadline();
        let proxy = linux_portal::await_pre_first_frame(
            "connect ScreenCast portal for source preview",
            stop.as_ref(),
            deadline,
            async {
                Screencast::new()
                    .await
                    .map_err(|error| cap_err("connect preview ScreenCast portal", error))
            },
        )
        .await?;
        let session = linux_portal::await_pre_first_frame(
            "create source-preview portal session",
            stop.as_ref(),
            deadline,
            async {
                proxy
                    .create_session(Default::default())
                    .await
                    .map_err(|error| cap_err("create preview portal session", error))
            },
        )
        .await?;

        let result: Result<()> = async {
            // Do not set PersistMode or a restore token: the picker grants this
            // one preview session only and native identifiers never leave it.
            let options = SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(BitFlags::from(SourceType::Monitor))
                .set_multiple(false);
            linux_portal::await_pre_first_frame(
                "select source-preview portal source",
                stop.as_ref(),
                deadline,
                async {
                    proxy
                        .select_sources(&session, options)
                        .await
                        .map_err(|error| cap_err("select preview portal source", error))
                },
            )
            .await?;
            let streams = linux_portal::await_pre_first_frame(
                "start source-preview portal cast",
                stop.as_ref(),
                deadline,
                async {
                    let parent_window = linux_portal::portal_parent_window();
                    proxy
                        .start(&session, parent_window.as_ref(), Default::default())
                        .await
                        .map_err(|error| cap_err("start preview portal cast", error))
                },
            )
            .await?
            .response()
            .map_err(|error| cap_err("preview portal cast response", error))?;
            let stream = streams
                .streams()
                .first()
                .ok_or_else(|| cap_err("preview portal granted no streams", "empty stream list"))?;
            let pw_fd = linux_portal::await_pre_first_frame(
                "open source-preview PipeWire remote",
                stop.as_ref(),
                deadline,
                async {
                    proxy
                        .open_pipe_wire_remote(&session, Default::default())
                        .await
                        .map_err(|error| cap_err("open preview PipeWire remote", error))
                },
            )
            .await?;
            let node = stream.pipe_wire_node_id();
            let pipewire_stop = stop.clone();
            let pipewire_mailbox = mailbox.clone();
            tokio::task::spawn_blocking(move || {
                preview(PipewirePreviewRequest {
                    pw_fd,
                    node,
                    stop: pipewire_stop,
                    mailbox: pipewire_mailbox,
                })
            })
            .await
            .map_err(|error| cap_err("source-preview PipeWire worker", error))?
            .map_err(|error| cap_err("source-preview PipeWire stream", error))
        }
        .await;

        // `Session` does not close on Drop. This runs after every successful
        // create-session path, including Stop while picker/PipeWire is active.
        linux_portal::close_session(&session).await;
        result
    })
}

fn unsupported_source() -> RecordError {
    RecordError::new(
        error_codes::UNIMPLEMENTED,
        "Linux source preview accepts only the portal selection",
        "select the portal source; exact in-app monitor or window identifiers are unavailable on this backend",
    )
}

fn validate_portal_selection(
    request: &SourcePreviewRequest,
    region: Option<CaptureRegion>,
) -> Result<()> {
    if matches!(&request.source, SourcePreviewSource::Portal) && region.is_none() {
        Ok(())
    } else {
        Err(unsupported_source())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_user_consented_portal_selector_can_reach_linux_preview() {
        let portal = SourcePreviewRequest {
            source: SourcePreviewSource::Portal,
            camera_id: None,
        };
        assert!(validate_portal_selection(&portal, None).is_ok());
        let monitor = SourcePreviewRequest {
            source: SourcePreviewSource::Monitor {
                monitor_id: "opaque-monitor".into(),
            },
            camera_id: None,
        };
        assert!(validate_portal_selection(&monitor, None).is_err());
        let private_region = CaptureRegion::new(0, 0, 2, 2, 4, 4)
            .expect("fixture must be a valid private sub-region");
        assert!(validate_portal_selection(&portal, Some(private_region)).is_err());
    }
}
