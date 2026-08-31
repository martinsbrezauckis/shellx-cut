//! Admission for the private Screen-only scene receipt owned by live capture.

use std::fs::File;
use std::io::Read;
use std::path::Path;

#[cfg(unix)]
use cut_core::error_codes;
use cut_core::CutError;
use record_capture::PrivateSceneProjection;
use record_core::{RecordError, RecordingProject};
use record_recovery::{is_plain_regular_file, CaptureRoot};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::capture_session_control::CaptureSessionControl;

const SCENE_RECEIPT_FILE: &str = ".private-screen-scene.receipt.json";
const SCENE_RECEIPT_SCHEMA: &str = "shellx-cut/private-screen-scene-receipt@1";

/// The only durable handoff from a private Screen-only scene journal to the
/// normal recorder outputs. The source and project remain the existing editable
/// screen output; this receipt adds no selectable scene or presentation mode.
#[derive(Serialize)]
struct PrivateScreenSceneReceipt {
    schema: &'static str,
    screen_output: &'static str,
    editable_project: &'static str,
    source_sha256: String,
    project_sha256: String,
    scene: PrivateSceneProjection,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn new_control(
    duration_ms: Option<u64>,
    audio: bool,
    system_audio: bool,
    passive_input_capture_active: bool,
    project_dir: &Path,
    capture_id: &str,
) -> Result<CaptureSessionControl, CutError> {
    #[cfg(unix)]
    {
        CaptureSessionControl::new_live(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
            project_dir,
            capture_id,
        )
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                "could not create private Screen-only scene receipt",
                error.to_string(),
            )
        })
    }

    #[cfg(not(unix))]
    {
        // The existing journal deliberately refuses platforms that cannot sync
        // its parent directory. Do not make the ordinary recorder claim this
        // private scene subset, or fail an otherwise-supported native capture.
        let _ = (project_dir, capture_id);
        Ok(CaptureSessionControl::new(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
        ))
    }
}

/// Bind the terminal private scene state to the rooted capture-owned screen
/// source and editable `project.json`. `CaptureRoot` revalidates the local
/// capture path before each leaf use; this receipt does not claim a concurrent
/// same-user source/project pathname-swap barrier. A pre-existing matching
/// receipt is an idempotent reopen; a differing leaf fails closed and is never
/// replaced.
pub(super) fn publish_completed_projection(
    control: &CaptureSessionControl,
    project_dir: &Path,
    capture_id: &str,
    out_dir: &Path,
    project_path: &Path,
    project: &RecordingProject,
) -> record_core::Result<()> {
    let Some(scene) = control.completed_scene_projection().map_err(scene_error)? else {
        return Ok(());
    };
    let root = CaptureRoot::for_project(project_dir).map_err(root_error)?;
    let capture_dir = root
        .existing_capture_dir(capture_id)
        .map_err(root_error)?
        .ok_or_else(|| invalid("private scene receipt has no capture directory"))?;
    if capture_dir != out_dir {
        return Err(invalid(
            "private scene receipt output directory does not match its capture owner",
        ));
    }
    let expected_project = root
        .capture_file(capture_id, "project.json")
        .map_err(root_error)?;
    if expected_project != project_path
        || !is_plain_regular_file(project_path).map_err(root_error)?
    {
        return Err(invalid(
            "private scene receipt project is not the capture-owned local project.json",
        ));
    }
    let source = root
        .capture_file(capture_id, "source.mp4")
        .map_err(root_error)?;
    if !matches_owned_source(&project.source_video, &source)
        || !is_plain_regular_file(&source).map_err(root_error)?
    {
        return Err(invalid(
            "private scene receipt screen output is not the capture-owned local source.mp4",
        ));
    }

    let receipt = PrivateScreenSceneReceipt {
        schema: SCENE_RECEIPT_SCHEMA,
        screen_output: "source.mp4",
        editable_project: "project.json",
        source_sha256: sha256(&source)?,
        project_sha256: sha256(project_path)?,
        scene,
    };
    let bytes = serde_json::to_vec(&receipt)
        .map_err(|error| invalid(format!("serialize private scene receipt: {error}")))?;
    let receipt_path = root
        .capture_file(capture_id, SCENE_RECEIPT_FILE)
        .map_err(root_error)?;
    match std::fs::symlink_metadata(&receipt_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => root
            .publish_new_capture_file(capture_id, SCENE_RECEIPT_FILE, &bytes)
            .map(|_| ())
            .map_err(root_error),
        Err(error) => Err(io_error(error)),
        Ok(_) if !is_plain_regular_file(&receipt_path).map_err(root_error)? => Err(invalid(
            "private scene receipt is linked or not a local regular file",
        )),
        Ok(_) => {
            let existing = std::fs::read(&receipt_path).map_err(io_error)?;
            if existing == bytes {
                Ok(())
            } else {
                Err(invalid(
                    "existing private scene receipt differs from the capture outputs",
                ))
            }
        }
    }
}

fn matches_owned_source(source_video: &str, expected_source: &Path) -> bool {
    Path::new(source_video) == expected_source || Path::new(source_video) == Path::new("source.mp4")
}

fn sha256(path: &Path) -> record_core::Result<String> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn root_error(error: record_recovery::ManifestError) -> RecordError {
    invalid(format!("resolve private scene receipt owner: {error}"))
}

fn scene_error(error: record_capture::PrivateSceneCoordinatorError) -> RecordError {
    invalid(format!("seal private Screen-only scene receipt: {error}"))
}

fn io_error(error: std::io::Error) -> RecordError {
    invalid(format!("read private scene receipt output: {error}"))
}

fn invalid(detail: impl Into<String>) -> RecordError {
    RecordError::new(
        record_core::error_codes::IO,
        "private Screen-only scene receipt could not be published",
        detail.into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_core::{fixtures, Settings};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    fn setup(capture_id: &str) -> (tempfile::TempDir, CaptureRoot, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("scene-receipt.cutproj");
        fs::create_dir(&project_dir).unwrap();
        let root = CaptureRoot::for_project(&project_dir).unwrap();
        let capture_dir = root.create_capture_dir(capture_id).unwrap();
        (temp, root, project_dir, capture_dir)
    }

    #[cfg(unix)]
    #[test]
    fn terminal_scene_receipt_binds_one_editable_screen_output_and_reopens_idempotently() {
        let (_temp, _root, project_dir, capture_dir) = setup("scene-receipt-ok");
        let source = capture_dir.join("source.mp4");
        let project_path = capture_dir.join("project.json");
        fs::write(&source, b"screen output").unwrap();
        let project = RecordingProject::new(
            source.display().to_string(),
            Settings::default(),
            fixtures::generate("click-walkthrough").unwrap(),
        );
        fs::write(&project_path, serde_json::to_vec_pretty(&project).unwrap()).unwrap();

        let control = CaptureSessionControl::new_live(
            None,
            false,
            false,
            false,
            &project_dir,
            "scene-receipt-ok",
        )
        .unwrap();
        let origin = Instant::now();
        control.backend_started_at(origin);
        control
            .terminalize_at_for_test(origin + Duration::from_millis(250))
            .unwrap();

        publish_completed_projection(
            &control,
            &project_dir,
            "scene-receipt-ok",
            &capture_dir,
            &project_path,
            &project,
        )
        .unwrap();
        // A finalization retry must prove the same output rather than replacing
        // its receipt or generating a second scene projection.
        publish_completed_projection(
            &control,
            &project_dir,
            "scene-receipt-ok",
            &capture_dir,
            &project_path,
            &project,
        )
        .unwrap();

        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(capture_dir.join(SCENE_RECEIPT_FILE)).unwrap())
                .unwrap();
        assert_eq!(receipt["schema"], SCENE_RECEIPT_SCHEMA);
        assert_eq!(receipt["screen_output"], "source.mp4");
        assert_eq!(receipt["editable_project"], "project.json");
        assert_eq!(receipt["scene"]["logical_media_time_ms"], 250);
        assert_eq!(receipt["scene"]["scene"]["composition"], "ScreenOnly");
        assert_eq!(
            receipt["scene"]["scene"]["timer"]["Elapsed"]["phase"],
            "Ended"
        );
        assert_eq!(
            receipt["scene"]["scene"]["timer"]["Elapsed"]["elapsed_ms"],
            250
        );
        assert_eq!(
            receipt["scene"]["journal_sha256"].as_str().map(str::len),
            Some(64)
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminal_scene_receipt_refuses_a_screen_output_outside_its_capture() {
        let (_temp, _root, project_dir, capture_dir) = setup("scene-receipt-outside");
        let source = capture_dir.join("source.mp4");
        let project_path = capture_dir.join("project.json");
        let outside = project_dir.join("outside.mp4");
        fs::write(&source, b"owned screen output").unwrap();
        fs::write(&outside, b"outside").unwrap();
        let project = RecordingProject::new(
            outside.display().to_string(),
            Settings::default(),
            fixtures::generate("click-walkthrough").unwrap(),
        );
        fs::write(&project_path, serde_json::to_vec_pretty(&project).unwrap()).unwrap();

        let control = CaptureSessionControl::new_live(
            None,
            false,
            false,
            false,
            &project_dir,
            "scene-receipt-outside",
        )
        .unwrap();
        let origin = Instant::now();
        control.backend_started_at(origin);
        control.terminalize_at_for_test(origin).unwrap();

        let error = publish_completed_projection(
            &control,
            &project_dir,
            "scene-receipt-outside",
            &capture_dir,
            &project_path,
            &project,
        )
        .unwrap_err();
        assert!(error.cause.contains("capture-owned local source.mp4"));
        assert!(!capture_dir.join(SCENE_RECEIPT_FILE).exists());
    }
}
