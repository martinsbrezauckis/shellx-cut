//! Windows display-affinity admission for the spawned Cut controller window.

use crate::CaptureControllerPlacement;

const ENV_CONTROLLER_PLACEMENT: &str = "SHELLX_CUT_CONTROLLER_PLACEMENT";
const ENV_CONTROLLER_PLACEMENT_OWNER: &str = "SHELLX_CUT_CONTROLLER_PLACEMENT_OWNER";
const PLACEMENT_OWNER: &str = "tauri-shell-v1";

/// Consume only the redacted conclusion from the Tauri process that owns the
/// top-level window. This process never receives a native handle and never calls the
/// Windows affinity API against another process's window.
pub(super) fn admit_controller_placement(
    placement: Option<&CaptureControllerPlacement>,
    selected_window: bool,
) {
    let Some(placement) = placement else {
        return;
    };
    // A true window target is already bounded to its selected native window.
    // Applying a display-controller policy here would change that source
    // contract, so its inapplicability is explicitly reported instead.
    if selected_window {
        placement.unavailable(
            "Controller exclusion applies to display capture; selected-window capture preserves its exact source semantics.",
        );
        return;
    }
    match (
        std::env::var(ENV_CONTROLLER_PLACEMENT_OWNER).ok().as_deref(),
        std::env::var(ENV_CONTROLLER_PLACEMENT).ok().as_deref(),
    ) {
        (Some(PLACEMENT_OWNER), Some("excluded")) => {
            placement.excluded("The Windows shell confirmed controller exclusion by native readback.");
        }
        (Some(PLACEMENT_OWNER), Some("refused")) => {
            placement.refused("Windows refused or did not confirm controller exclusion in shell readback.");
        }
        _ => placement.unavailable(
            "This Windows capture was not spawned by a shell with an observed controller-placement result.",
        ),
    }
}
