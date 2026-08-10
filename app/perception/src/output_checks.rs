//! Output-only checks shared by render.final and verify.rerun.
//!
//! These checks consume facts measured from the immutable rendered bytes. They
//! intentionally exclude timeline and source facts, so a later recheck cannot
//! present a changed project as evidence for an older render.

use crate::checks::{
    black_or_frozen_frames, black_or_frozen_frames_screen_demo, lufs, silence_at_edges,
    uniform_border, waive, FootageProfile, RenderFacts,
};
use cut_core::CheckResult;

/// The checks whose evidence comes solely from the rendered output.
///
/// `caption_presence`, `cut_on_word`, and `duration_matches_edl` need the
/// original timeline snapshot, so `verify.rerun` must never re-evaluate them.
pub struct OutputChecks {
    pub lufs: CheckResult,
    pub black_or_frozen_frames: CheckResult,
    pub uniform_border: CheckResult,
    pub silence_at_edges: CheckResult,
}

impl OutputChecks {
    pub fn into_vec(self) -> Vec<CheckResult> {
        vec![
            self.lufs,
            self.black_or_frozen_frames,
            self.uniform_border,
            self.silence_at_edges,
        ]
    }
}

/// Interpret measured output facts under the render's selected footage
/// profile. The profile changes interpretation only; measurements stay intact.
pub fn output_checks_with_profile(facts: &RenderFacts, profile: FootageProfile) -> OutputChecks {
    match profile {
        FootageProfile::TalkingHead => OutputChecks {
            lufs: lufs(facts, -16.0, 2.0),
            black_or_frozen_frames: black_or_frozen_frames(facts),
            uniform_border: uniform_border(facts),
            silence_at_edges: silence_at_edges(facts, 500),
        },
        FootageProfile::SilentScreenDemo => OutputChecks {
            lufs: waive(
                lufs(facts, -16.0, 2.0),
                profile,
                "silent-by-design footage — the spoken-content loudness target does not apply; measured values remain in evidence",
            ),
            black_or_frozen_frames: black_or_frozen_frames_screen_demo(facts),
            // A baked-in margin is a render defect under either profile.
            uniform_border: uniform_border(facts),
            silence_at_edges: waive(
                silence_at_edges(facts, 500),
                profile,
                "fully silent footage — every edge is silent by design, the padding budget is meaningless",
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_profile_waives_only_its_output_interpretations() {
        let facts = RenderFacts {
            duration_ms: 1_000,
            loudness: None,
            output_report: None,
        };

        let checks = output_checks_with_profile(&facts, FootageProfile::SilentScreenDemo);
        assert!(checks.lufs.pass);
        assert_eq!(
            checks.lufs.details["waived_by_profile"],
            "silent_screen_demo"
        );
        assert!(checks.silence_at_edges.pass);
        assert!(
            !checks.uniform_border.pass,
            "uniform borders are never waived"
        );
    }
}
