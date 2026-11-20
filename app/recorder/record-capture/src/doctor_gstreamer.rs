//! Passive Linux screen-encoder tool probe, isolated from camera plugin init.

use std::path::Path;
use std::process::Command;

use super::Card;
use crate::doctor_process;

/// Elements the Linux screen recorder needs beyond the PipeWire source, with
/// the name of the GStreamer plugin that provides each.
const RECORD_ELEMENTS: [(&str, &str); 2] = [("x264enc", "x264"), ("mp4mux", "isomp4")];

fn encoder_package_hint(element: &str) -> &'static str {
    match element {
        "x264enc" => {
            "gstreamer1.0-plugins-ugly (Debian/Ubuntu) or gstreamer1-plugins-ugly from RPM Fusion (Fedora)"
        }
        _ => "gstreamer1.0-plugins-good (Debian/Ubuntu) or gstreamer1-plugins-good (Fedora)",
    }
}

pub(super) fn card() -> Card {
    // A probe may refresh a registry. Keep its filtered registry private so
    // explicit Start sees the normal, unfiltered plugin inventory.
    let private_registry = tempfile::tempdir();
    let probe_result = private_registry.as_ref().ok().map(|directory| {
        let registry = directory.path().join("doctor-gstreamer-registry.bin");
        let gst = std::env::var("SHELLX_RECORD_GST")
            .ok()
            .filter(|gst| std::path::Path::new(gst).is_absolute())
            .unwrap_or_else(|| "gst-launch-1.0".into());
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
        // The screen recorder encodes with x264enc and muxes with mp4mux (see
        // linux_gst_capture::command_args). x264enc is not in every
        // distribution's default repositories, so name it when it is missing.
        let missing_encoder = has_gst
            .then(|| {
                RECORD_ELEMENTS.iter().find_map(|(element, plugin)| {
                    let mut command = Command::new("gst-inspect-1.0");
                    command.arg(element);
                    restrict_plugins_to(&mut command, &registry, &[plugin]);
                    let present = doctor_process::output(&mut command, "probe GStreamer encoder")
                        .is_some_and(|o| o.status.success());
                    (!present).then_some(*element)
                })
            })
            .flatten();
        (has_gst, has_pw, missing_encoder)
    });
    let (status, detail) = match probe_result {
        None => (
            "degraded",
            "a private GStreamer Doctor registry could not be created".to_string(),
        ),
        Some((true, true, None)) => (
            "ok",
            "gst-launch-1.0 + pipewiresrc + x264enc + mp4mux present".to_string(),
        ),
        Some((true, true, Some(element))) => (
            "degraded",
            format!(
                "GStreamer element {element} is missing — install {}",
                encoder_package_hint(element)
            ),
        ),
        Some((true, false, _)) => (
            "degraded",
            "gst present but pipewiresrc missing — install gstreamer1.0-pipewire".to_string(),
        ),
        Some((false, _, _)) => (
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
    restrict_plugins_to(command, registry, &["pipewire"]);
}

fn restrict_plugins_to(command: &mut Command, registry: &Path, plugins: &[&str]) {
    command.env("GST_PLUGIN_LOADING_WHITELIST", plugins.join(":"));
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

    #[test]
    fn missing_recording_element_is_named_with_its_package() {
        assert!(encoder_package_hint("x264enc").contains("gstreamer1.0-plugins-ugly"));
        assert!(encoder_package_hint("x264enc").contains("RPM Fusion"));
        assert!(encoder_package_hint("mp4mux").contains("plugins-good"));
        // Every element the recorder pipeline uses must be probed.
        let args = crate::linux_gst_capture::command_args(1, 3, "/tmp/segment.mp4");
        for (element, _) in RECORD_ELEMENTS {
            assert!(
                args.iter().any(|arg| arg == element),
                "{element} not in the recorder pipeline"
            );
        }
    }
}
