//! Private proof that a macOS cutd child belongs to the Tauri controller.
//!
//! ScreenCaptureKit exclusion must name only windows owned by the shell that
//! spawned this engine. The values below are child-only environment transport:
//! they are scrubbed before each spawn and never leave the native process
//! boundary as a status value or receipt field.

use std::process::Command;

pub const ENV_CONTROLLER_OWNER: &str = "SHELLX_CUT_MACOS_CONTROLLER_OWNER";
pub const ENV_CONTROLLER_OWNER_PID: &str = "SHELLX_CUT_MACOS_CONTROLLER_OWNER_PID";
pub const CONTROLLER_OWNER: &str = "tauri-shell-v1";

/// Give only a freshly spawned UI child a parent-PID proof. An adopted engine
/// never visits this path, and inherited values are explicitly removed first.
pub fn apply_to_spawned_child(command: &mut Command, ui_present: bool) {
    command.env_remove(ENV_CONTROLLER_OWNER);
    command.env_remove(ENV_CONTROLLER_OWNER_PID);
    if ui_present {
        command.env(ENV_CONTROLLER_OWNER, CONTROLLER_OWNER);
        command.env(ENV_CONTROLLER_OWNER_PID, std::process::id().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn env_value(command: &Command, name: &str) -> Option<&OsStr> {
        command
            .get_envs()
            .find_map(|(key, value)| (key == OsStr::new(name)).then_some(value).flatten())
    }

    #[test]
    fn owner_proof_names_the_shell_but_not_a_native_handle() {
        assert_eq!(CONTROLLER_OWNER, "tauri-shell-v1");
        assert!(ENV_CONTROLLER_OWNER.contains("OWNER"));
        assert!(ENV_CONTROLLER_OWNER_PID.contains("PID"));
    }

    #[test]
    fn spawned_ui_child_gets_fresh_owner_proof_after_inherited_values_are_scrubbed() {
        let mut command = Command::new("cutd-test");
        command.env(ENV_CONTROLLER_OWNER, "spoofed-owner");
        command.env(ENV_CONTROLLER_OWNER_PID, "999");
        apply_to_spawned_child(&mut command, true);
        let process_id = std::process::id().to_string();
        assert_eq!(
            env_value(&command, ENV_CONTROLLER_OWNER),
            Some(OsStr::new(CONTROLLER_OWNER))
        );
        assert_eq!(
            env_value(&command, ENV_CONTROLLER_OWNER_PID),
            Some(OsStr::new(&process_id))
        );
    }

    #[test]
    fn headless_child_cannot_inherit_or_claim_a_controller_owner() {
        let mut command = Command::new("cutd-test");
        command.env(ENV_CONTROLLER_OWNER, CONTROLLER_OWNER);
        command.env(ENV_CONTROLLER_OWNER_PID, "123");
        apply_to_spawned_child(&mut command, false);
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| key == OsStr::new(ENV_CONTROLLER_OWNER))
                .and_then(|(_, value)| value),
            None
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| key == OsStr::new(ENV_CONTROLLER_OWNER_PID))
                .and_then(|(_, value)| value),
            None
        );
    }
}
