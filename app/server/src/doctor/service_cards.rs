//! Optional remote-service doctor cards.
//!
//! Dub and diarize are optional model services. Unlike ffmpeg/perception, they
//! are not installed by a ShellX Cut setup verb, and they are normally absent on
//! a plain editing box. "Not reachable" is therefore neutral `Unknown`, never a
//! red `Missing` for an essential dependency. A reachable service without its
//! local connector is not usable by Cut and must not read as ready either.

use super::{Card, CardStatus};
use serde_json::json;
use std::time::Duration;

/// Lightweight reachability probe for an optional dub/diarize service. These
/// services have a real `/health` contract, so a bare TCP accept is not enough:
/// stale proxies can accept and then reset, which previously made dead model
/// cards look OK.
fn service_reachable(endpoint: &str) -> bool {
    let base = endpoint.trim_end_matches('/');
    let health = format!("{base}/health");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .http_status_as_error(false)
        .build()
        .into();
    agent
        .get(health.as_str())
        .call()
        .map(|r| r.status().as_u16())
        .map(|status| (200..300).contains(&status))
        .unwrap_or(false)
}

/// Shared builder for the two optional-service cards. Both the remote health
/// response and local connector are required for `Ok`. There is no setup verb
/// for these cards; the hint names whichever requirement is missing.
pub(super) fn service_card(
    id: &str,
    endpoint: String,
    secret_set: bool,
    human: &str,
    verb: &str,
    endpoint_env: &str,
    model: &str,
    runner_available: bool,
) -> Card {
    let reachable = service_reachable(&endpoint);
    let (status, hint) = if reachable && runner_available {
        (CardStatus::Ok, None)
    } else if reachable {
        (
            CardStatus::Unknown,
            Some(format!(
                "Optional {human} service is reachable, but the local perception connector for {verb} is missing. Install or repair the ShellX Cut perception sidecar, then Re-scan."
            )),
        )
    } else {
        (
            CardStatus::Unknown,
            Some(format!(
                "Optional {human} service — not reachable. Core editing and rendering work \
                 without it; {verb} only needs it when you use that feature. Start the service \
                 package, or set {endpoint_env} if it uses a custom address, then Re-scan."
            )),
        )
    };
    Card {
        id: id.to_string(),
        kind: "service".into(),
        status,
        source: None,
        version: None,
        hint,
        details: json!({
            "endpoint": endpoint,
            "model": model,
            "runner_available": runner_available,
            "secret_set": secret_set,
            "reachable": reachable,
            "optional": true,
            "powers": verb,
            "endpoint_env": endpoint_env,
        }),
    }
}

/// Dub (OmniVoice TTS) service card — powers `audio.dub`.
pub(super) fn dub_card() -> Card {
    service_card(
        "dub",
        crate::dub::endpoint(),
        crate::dub::secret().is_some(),
        "dubbing (OmniVoice TTS)",
        "audio.dub",
        "CUT_DUB_ENDPOINT",
        "OmniVoice TTS",
        crate::dub::runtime().ok().flatten().is_some(),
    )
}

/// Diarize (Sortformer) service card — powers `media.diarize`.
pub(super) fn diarize_card() -> Card {
    service_card(
        "diarize",
        crate::diarize::endpoint(),
        crate::diarize::secret().is_some(),
        "speaker diarization",
        "media.diarize",
        "CUT_DIARIZE_ENDPOINT",
        "Sortformer v2",
        crate::diarize::runtime().ok().flatten().is_some(),
    )
}
