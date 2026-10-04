//! Passive Linux screen-encoder tool probe, isolated from camera plugin init.

use std::path::Path;
use std::process::Command;

use super::Card;
use crate::doctor_process;

pub(super) fn card() -> Card {
    // A probe may refresh a registry. Keep its filtered registry private so
    // explicit Start sees the normal, unfiltered plugin inventory.
    let private_registry = tempfile::tempdir();
    let probe_result = private_registry.as_ref().ok().map(|directory| {
        let registry = directory.path().join("doctor-gstreamer-registry.bin");
        let gst = std::env::var("SHELLX_RECORD_GST").unwrap_or_else(|_| "gst-launch-1.0".into());
        let mut gst_command = Command::new(&gst);
        gst_command.arg("--version");
        restrict_plugins(&mut gst_command, &registry);
        let has_gst = doctor_process::output(&mut gst_command, "probe GStreamer")
            .is_some_and(|o| o.status.success());
        let mut pipewire_command = Command::new("gst-inspect-1.0");
        pipewire_command.arg("pipewiresrc");
        restrict_plugins(&mut pipewire_command, &registry);
        let has_pw = doctor_process::output(&mut pipewire_command, "probe GStreamer PipeWire")
            .is_some_and(|o| o.status.success());
        (has_gst, has_pw)
    });
    let (status, detail) = match probe_result {
        None => (
            "degraded",
            "a private GStreamer Doctor registry could not be created".to_string(),
        ),
        Some((true, true)) => ("ok", "gst-launch-1.0 + pipewiresrc present".to_string()),
        Some((true, false)) => (
            "degraded",
            "gst present but pipewiresrc missing — install gstreamer1.0-pipewire".to_string(),
        ),
        Some((false, _)) => (
            "missing",
            "gst-launch-1.0 not found — install gstreamer1.0-tools + gstreamer1.0-pipewire"
                .to_string(),
        ),
    };
    Card::new("gstreamer", "tool", status, detail)
}

/// GStreamer checks this whitelist before V4L2 plugin_init, which may probe
/// camera nodes. It applies only to these Doctor subprocesses, not capture.
fn restrict_plugins(command: &mut Command, registry: &Path) {
    command.env("GST_PLUGIN_LOADING_WHITELIST", "pipewire");
    command.env("GST_REGISTRY", registry);
    command.env("GST_REGISTRY_1_0", registry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gstreamer_doctor_commands_override_inherited_plugin_whitelist() {
        let mut command = Command::new("gst-inspect-1.0");
        let registry = Path::new("/private/doctor-registry.bin");
        restrict_plugins(&mut command, registry);
        for (key, expected) in [
            ("GST_PLUGIN_LOADING_WHITELIST", "pipewire"),
            ("GST_REGISTRY", "/private/doctor-registry.bin"),
            ("GST_REGISTRY_1_0", "/private/doctor-registry.bin"),
        ] {
            let actual = command
                .get_envs()
                .find(|(name, _)| *name == key)
                .and_then(|(_, value)| value);
            assert_eq!(actual, Some(std::ffi::OsStr::new(expected)));
        }
    }
}
