//! A microphone request must refuse before any screen/native capture work.

use super::*;
use crate::screen_record::microphone::readiness::{MicrophoneReadiness, TestReadiness};
use record_capture::{MicrophoneAccess, MicrophoneSwitch};

fn ready() -> MicrophoneReadiness {
    MicrophoneReadiness {
        compiled: true,
        access: MicrophoneAccess::Granted,
        windows_switch: None,
        no_device: false,
    }
}

#[tokio::test]
async fn requested_microphone_refuses_every_passive_problem_before_capture() {
    let _lock = super::super::capture_test_lock().lock().await;
    record_capture::forbid_native_capture_access();
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("microphone.cutproj");
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"microphone", "dir":project_dir}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let cases = [
        MicrophoneReadiness {
            windows_switch: Some(MicrophoneSwitch::DesktopApps),
            ..ready()
        },
        MicrophoneReadiness {
            compiled: false,
            ..ready()
        },
        MicrophoneReadiness {
            access: MicrophoneAccess::Denied,
            ..ready()
        },
        MicrophoneReadiness {
            access: MicrophoneAccess::Restricted,
            ..ready()
        },
        MicrophoneReadiness {
            no_device: true,
            ..ready()
        },
        MicrophoneReadiness {
            windows_switch: Some(MicrophoneSwitch::Device),
            ..ready()
        },
        MicrophoneReadiness {
            windows_switch: Some(MicrophoneSwitch::Apps),
            ..ready()
        },
    ];
    for readiness in cases {
        let expected = readiness.problem().unwrap();
        let _injected = TestReadiness::install(readiness);
        let screens_before = record_capture::native_screen_access_attempts();
        let result =
            screen_record_start(&state, json!({"audio":true,"system_audio":false,"fps":30})).await;
        stop_if_started(&result).await;
        let error = result.expect_err(expected.reason);
        assert_eq!(error.message, expected.error.message, "{}", expected.reason);
        assert_eq!(error.cause, expected.reason);
        assert_eq!(error.suggested_action, expected.error.suggested_action);
        assert_eq!(
            record_capture::native_screen_access_attempts(),
            screens_before,
            "no screen read before microphone refusal"
        );
        assert!(
            !project_dir.join("cache/screen_record").exists(),
            "no capture or recovery artifacts"
        );
    }
}

async fn stop_if_started(result: &Result<VerbResult, CutError>) {
    if let Ok(result) = result {
        let id = result.result.as_ref().unwrap()["capture_id"]
            .as_str()
            .unwrap();
        super::super::stop_capture(id);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while super::super::is_capture_active(id) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("source-test worker releases its capture");
    }
}

#[tokio::test]
async fn screen_only_start_keeps_its_existing_result_with_a_blocked_microphone() {
    let _lock = super::super::capture_test_lock().lock().await;
    record_capture::forbid_native_capture_access();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"screen-only", "dir":root.path().join("screen-only.cutproj")}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    for system_audio in [false, true] {
        let _ready = TestReadiness::install(ready());
        let before = screen_record_start(
            &state,
            json!({"audio":false,"system_audio":system_audio,"fps":30}),
        )
        .await;
        let _blocked = TestReadiness::install(MicrophoneReadiness {
            windows_switch: Some(MicrophoneSwitch::DesktopApps),
            ..ready()
        });
        let after = screen_record_start(
            &state,
            json!({"audio":false,"system_audio":system_audio,"fps":30}),
        )
        .await;
        // Native capture is forbidden in this source test. The same screen
        // refusal proves microphone readiness leaves screen-only Start unchanged.
        assert_eq!(before.unwrap_err(), after.unwrap_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn empty_native_microphone_inventory_refuses_before_capture_setup() {
    let root = tempfile::tempdir().unwrap();
    if std::env::var_os("CUT_EMPTY_MICROPHONE_CASE").is_none() {
        let config = root.path().join("empty-alsa.conf");
        std::fs::write(&config, "").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "screen_record::start_handler::microphone_tests::empty_native_microphone_inventory_refuses_before_capture_setup", "--nocapture"])
            .env("ALSA_CONFIG_PATH", config)
            .env("CUT_EMPTY_MICROPHONE_CASE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    record_capture::forbid_native_capture_access();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let project_dir = root.path().join("no-input.cutproj");
        let state = AppState::new();
        let created = crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name":"no-input", "dir":project_dir}),
            cut_core::Actor::system(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        let error = screen_record_start(&state, json!({"audio":true,"fps":30}))
            .await
            .unwrap_err();
        assert_eq!(error.cause, "no_microphone_device");
        assert_eq!(error.message, "No microphone was found");
        assert!(error.suggested_action.unwrap().contains("sound settings"));
        assert_eq!(record_capture::native_screen_access_attempts(), 0);
        assert!(!project_dir.join("cache/screen_record").exists());
    });
}
