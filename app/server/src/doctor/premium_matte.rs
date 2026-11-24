//! Premium matte readiness shares the engine's admitted model-group resolver.

use super::{probe_command, Card, CardStatus, ProbeOutcome};
use cut_core::{CutError, MatteModel};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub(super) enum MattePremiumScan {
    Ready(Card),
    Deferred {
        card: Card,
        python: PathBuf,
        native_context: bool,
    },
}

impl MattePremiumScan {
    pub(super) fn into_card(self) -> Card {
        match self {
            Self::Ready(card) | Self::Deferred { card, .. } => card,
        }
    }
}

pub(super) fn probe_cuda_availability(python: &Path, native_context: bool) -> Option<bool> {
    matte_premium_cuda_availability(python, native_context)
}

pub(super) fn finalize_matte_premium_card(mut card: Card, cuda_avail: Option<bool>) -> Card {
    let (status, hint) = matte_premium_status(true, cuda_avail);
    card.status = status;
    card.hint = hint;
    if let Some(details) = card.details.as_object_mut() {
        details.insert("cuda_available".into(), json!(cuda_avail));
        details.remove("checking");
    }
    card
}

/// PREMIUM background-removal capability: MatAnyone2 (`edit.matte{model:"matanyone"}`)
/// — target-assigned matting (pick WHICH subject) with cleaner edges + temporal
/// stability than RVM. Opt-in, NVIDIA-realistic, NON-COMMERCIAL (NTU S-Lab License
/// 1.0). Its OWN isolated torch venv + a 141 MB checkpoint + 646 MB SAM2 weights, installed by
/// `system.setup_matte{model:"matanyone", accept_noncommercial:true}`. A supplied
/// native runtime instead binds the sealed RVM, MatAnyone2, and SAM2 group. OPTIONAL —
/// the default RVM tier + core editing work without it.
#[allow(dead_code)]
pub(super) fn matte_premium_card() -> Card {
    scan_matte_premium(false).into_card()
}

pub(super) fn scan_matte_premium(defer_cuda: bool) -> MattePremiumScan {
    let native_context = match cut_perception::native_runtime_context() {
        Ok(context) => context,
        Err(error) => {
            return MattePremiumScan::Ready(prepared_matte_premium_card_failure(&error.message));
        }
    };
    if native_context.is_some() {
        return prepared_matte_premium_card_from_resolution_with_mode(
            crate::matte::native_runtime::prepared_matte_runtime(&MatteModel::Matanyone),
            defer_cuda,
        );
    }

    let rt = crate::matte::runtime_matanyone();
    let installed = rt.is_some();
    // Report the RESOLVED checkpoint when the runtime is present (honours the env
    // override + the browse setting), else the default fetch target.
    let model = rt
        .as_ref()
        .map(|r| r.model.clone())
        .or_else(crate::matte::read_matanyone_model_setting)
        .or_else(crate::matte::matanyone_default_model);
    let model_present = model.as_ref().map(|p| p.exists()).unwrap_or(false);
    if defer_cuda {
        if let Some(runtime) = rt.as_ref() {
            let card = Card {
                id: "matte_premium".into(),
                kind: "matte".into(),
                status: CardStatus::Unknown,
                source: None,
                version: None,
                hint: Some("Checking CUDA GPU support for MatAnyone2...".to_string()),
                details: json!({
                    "model": "matanyone2",
                    "installed": true,
                    "checking": true,
                    "setup_supported": Value::Null,
                    "checkpoint_present": model_present,
                    "checkpoint_path": model.map(|p| p.display().to_string()),
                    "cuda_available": Value::Null,
                    "license": "NTU S-Lab License 1.0 (non-commercial)",
                    "unlocks": "premium target-assigned matte (edit.matte{model:matanyone}) — cleaner edges, pick the subject",
                }),
            };
            return MattePremiumScan::Deferred {
                card,
                python: runtime.python.clone(),
                native_context: false,
            };
        }
    }
    // CUDA probe only when installed (a bounded torch import — premium users only,
    // so the cost is never paid on a default install). We read the FULL probe
    // outcome (not version_line's Option) so a TIMED-OUT probe is distinguishable
    // from a confirmed CPU-only box: `Ran` ⇒ Some(has-cuda), but a `Timeout`/
    // `NotFound` ⇒ None (unverified). A timed-out CUDA probe must not read as
    // Ok — a CPU-only box would then advertise premium matte as ready and run
    // unusably slow.
    let cuda_avail = rt
        .as_ref()
        .and_then(|runtime| matte_premium_cuda_availability(&runtime.python, false));
    // This is NEW-install admission only. An already installed older runtime
    // keeps its own readiness and is not disabled by the candidate's policy.
    let generation = (!installed)
        .then(cut_media::hwencode::cuda_compute_capability)
        .flatten();
    let setup_refusal = generation
        .and_then(|cap| crate::perception_setup::require_premium_gpu_generation(Some(cap)).err());
    let (status, hint) = if let Some(error) = &setup_refusal {
        (CardStatus::Degraded, Some(error.message.clone()))
    } else {
        matte_premium_status(installed, cuda_avail)
    };
    MattePremiumScan::Ready(Card {
        id: "matte_premium".into(),
        kind: "matte".into(),
        status,
        source: None,
        version: None,
        hint,
        details: json!({
            "model": "matanyone2",
            "installed": installed,
            // null = unqueried/unknown; never a confident rejection.
            "setup_supported": generation.map(|_| setup_refusal.is_none()),
            "checkpoint_present": model_present,
            "checkpoint_path": model.map(|p| p.display().to_string()),
            // null when UNVERIFIED (the GPU probe timed out) — never a confident
            // false that would read as "definitely CPU-only".
            "cuda_available": cuda_avail,
            "license": "NTU S-Lab License 1.0 (non-commercial)",
            "unlocks": "premium target-assigned matte (edit.matte{model:matanyone}) — cleaner edges, pick the subject",
        }),
    })
}

/// Interpret the authoritative native-context resolver once it has selected the
/// requested premium runtime.  Keeping this branch independent from process
/// environment lookup lets tests exercise the real resolver with an explicit
/// `RuntimeContext`, without changing the global context locator.
#[allow(dead_code)]
fn prepared_matte_premium_card_from_resolution(
    resolution: Result<Option<crate::matte::native_runtime::PreparedMatteRuntime>, CutError>,
) -> Card {
    prepared_matte_premium_card_from_resolution_with_mode(resolution, false).into_card()
}

fn prepared_matte_premium_card_from_resolution_with_mode(
    resolution: Result<Option<crate::matte::native_runtime::PreparedMatteRuntime>, CutError>,
    defer_cuda: bool,
) -> MattePremiumScan {
    match resolution {
        Ok(Some(crate::matte::native_runtime::PreparedMatteRuntime::Matanyone(runtime))) => {
            if defer_cuda {
                let mut card = prepared_matte_premium_card_with_cuda(&runtime, None);
                card.status = CardStatus::Unknown;
                card.hint = Some("Checking CUDA GPU support for MatAnyone2...".to_string());
                if let Some(details) = card.details.as_object_mut() {
                    details.insert("checking".into(), json!(true));
                }
                MattePremiumScan::Deferred {
                    card,
                    python: runtime.python.clone(),
                    native_context: true,
                }
            } else {
                MattePremiumScan::Ready(prepared_matte_premium_card(&runtime))
            }
        }
        Ok(Some(crate::matte::native_runtime::PreparedMatteRuntime::Rvm(_))) => {
            MattePremiumScan::Ready(prepared_matte_premium_card_failure(
                "the prepared native-runtime context selected an RVM runtime for MatAnyone2",
            ))
        }
        Ok(None) => MattePremiumScan::Ready(prepared_matte_premium_card_failure(
            "the prepared native-runtime context disappeared while Doctor was checking MatAnyone2",
        )),
        Err(error) => MattePremiumScan::Ready(prepared_matte_premium_card_failure(&error.message)),
    }
}

fn prepared_matte_premium_card(
    runtime: &crate::matte::premium_runtime::PreparedMatanyoneRuntime,
) -> Card {
    prepared_matte_premium_card_with_cuda(
        runtime,
        matte_premium_cuda_availability(&runtime.python, true),
    )
}

fn prepared_matte_premium_card_with_cuda(
    runtime: &crate::matte::premium_runtime::PreparedMatanyoneRuntime,
    cuda_avail: Option<bool>,
) -> Card {
    let (status, hint) = matte_premium_status(true, cuda_avail);
    Card {
        id: "matte_premium".into(),
        kind: "matte".into(),
        status,
        source: None,
        version: None,
        hint,
        details: json!({
            "model": "matanyone2",
            "installed": true,
            "checkpoint_present": true,
            "checkpoint_path": runtime.matanyone_model.display().to_string(),
            "cuda_available": cuda_avail,
            "prepared_runtime": {
                "contract": &runtime.binding.contract,
                "manifest_sha256": &runtime.binding.manifest_sha256,
                "receipt_sha256": &runtime.binding.receipt_sha256,
                "rvm_model_id": &runtime.binding.rvm_model_id,
                "rvm_model_sha256": &runtime.binding.rvm_model_sha256,
                "matanyone_model_id": &runtime.binding.matanyone_model_id,
                "matanyone_model_sha256": &runtime.binding.matanyone_model_sha256,
                "sam2_model_id": &runtime.binding.sam2_model_id,
                "sam2_model_sha256": &runtime.binding.sam2_model_sha256,
            },
            "license": "NTU S-Lab License 1.0 (non-commercial)",
            "unlocks": "premium target-assigned matte (edit.matte{model:matanyone}) — cleaner edges, pick the subject",
        }),
    }
}

fn prepared_matte_premium_card_failure(reason: &str) -> Card {
    Card {
        id: "matte_premium".into(),
        kind: "matte".into(),
        status: CardStatus::Degraded,
        source: None,
        version: None,
        hint: Some(format!(
            "Prepared native runtime cannot supply the pinned RVM, MatAnyone2, and SAM2 model group: {reason}. Doctor did not fall back to user settings, app-data, a download, or HTTP."
        )),
        details: json!({
            "model": "matanyone2",
            "installed": false,
            "checkpoint_present": false,
            "checkpoint_path": Value::Null,
            "cuda_available": Value::Null,
            "prepared_runtime": { "rejected": true, "reason": reason },
            "license": "NTU S-Lab License 1.0 (non-commercial)",
            "unlocks": "premium target-assigned matte (edit.matte{model:matanyone}) — cleaner edges, pick the subject",
        }),
    }
}

fn matte_premium_cuda_probe_command(
    python: &Path,
    native_context: bool,
) -> Result<Command, String> {
    // Keep the standard Windows batch-shim handling used by Doctor probes. A
    // prepared context normally declares a native interpreter, but an invalid
    // declaration must retain the ordinary spawn classification rather than
    // bypassing that command construction policy.
    let mut command = crate::gen::agent_std_command(python, &[])?;
    cut_perception::apply_python_command_policy(&mut command, native_context);
    command.args([
        "-c",
        "import torch; print('cuda', torch.cuda.is_available())",
    ]);
    Ok(command)
}

fn matte_premium_cuda_availability(python: &Path, native_context: bool) -> Option<bool> {
    let mut command = matte_premium_cuda_probe_command(python, native_context).ok()?;
    match probe_command(
        &mut command,
        Duration::from_secs(25),
        "check premium matte CUDA availability",
    ) {
        ProbeOutcome::Ran(stdout) => Some(stdout.contains("True")),
        ProbeOutcome::Timeout | ProbeOutcome::NotFound => None,
    }
}

fn matte_premium_status(installed: bool, cuda_avail: Option<bool>) -> (CardStatus, Option<String>) {
    if !installed {
        (
            CardStatus::Missing,
            Some(
                "Premium background removal (MatAnyone2 — cleaner edges, pick which subject) is not \
                 installed. Run system.setup_matte{model:\"matanyone\", accept_noncommercial:true} — it's \
                 NVIDIA-realistic and NON-COMMERCIAL (NTU S-Lab License 1.0). Optional: the default RVM \
                 tier works without it."
                    .to_string(),
            ),
        )
    } else if cuda_avail == Some(false) {
        (
            CardStatus::Degraded,
            Some(
                "MatAnyone2 is installed but torch reports no CUDA device — it would run on CPU, which is \
                 unusably slow for video. Use an NVIDIA GPU, or stick with the default RVM tier."
                    .to_string(),
            ),
        )
    } else if cuda_avail.is_none() {
        // The torch CUDA probe TIMED OUT (or its interpreter wouldn't run) — we
        // cannot confirm a usable GPU. Do not read that as Ok: a CPU-only box
        // would then advertise premium matte as ready and run unusably slow.
        (
            CardStatus::Unknown,
            Some(
                "MatAnyone2 is installed, but the GPU check timed out, so its CUDA device couldn't be \
                 confirmed this scan. It needs an NVIDIA GPU to run usably (CPU is far too slow for video). \
                 Re-scan to verify; if the check keeps timing out, the torch import may be wedged."
                    .to_string(),
            ),
        )
    } else {
        (CardStatus::Ok, None)
    }
}

#[cfg(test)]
#[path = "premium_matte_tests.rs"]
mod tests;
