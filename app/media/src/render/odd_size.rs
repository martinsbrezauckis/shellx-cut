//! An older project with an odd frame size.
//!
//! Projects made before 0.6.116 could take an odd size from their first clip
//! (641x361). The render graph composes in 4:2:0 and every video encoder needs
//! even sides, so a video at that size cannot be made. Cut does not round it
//! quietly: the server's verb gate refuses every video export at the project
//! size up front with `odd_project_size` and the exact `project.format` call,
//! and this check repeats it at the media layer for any path (a job, an
//! internal call) that reaches the encoder without the gate, so no raw ffmpeg
//! error ever reaches the person or the agent.
//!
//! Stills are different: no video encoder is involved, so they are made, and
//! composed 4:2:0 at the even size just below (`output_geometry`); a still of a
//! 641x361 project is a 640x360 image, and `export.frame` says so.

use super::{RenderOptions, Resolution};

/// Refuse a video at the project size when that size is odd. Explicit or
/// source-derived output sizes are made even by `output_geometry` itself.
pub fn require_even_output(
    project: &cut_core::Project,
    opts: &RenderOptions,
) -> Result<(), cut_core::CutError> {
    if opts.resolution != Resolution::Project {
        return Ok(());
    }
    cut_core::frame_size::require_even_project(project.settings.width, project.settings.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(w: u32, h: u32) -> cut_core::Project {
        cut_core::Project::new(
            "odd",
            cut_core::ProjectSettings {
                width: w,
                height: h,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        )
    }

    #[test]
    fn an_odd_project_video_is_refused_and_keeps_its_own_size() {
        let p = project(641, 361);
        let edl = cut_core::edl_from_project(&p);
        let opts = RenderOptions::default();
        // Composition (stills) is even; a VIDEO at the project size refuses.
        assert_eq!(opts.output_geometry(&p, &edl), (640, 360));
        let e = require_even_output(&p, &opts).unwrap_err();
        assert_eq!(e.code, cut_core::error_codes::ODD_PROJECT_SIZE);
        // An explicit size is the export's own and is even.
        let explicit = RenderOptions {
            resolution: Resolution::Explicit {
                width: 1280,
                height: 720,
            },
            ..RenderOptions::default()
        };
        assert!(require_even_output(&p, &explicit).is_ok());
    }

    #[test]
    fn an_even_project_is_unchanged() {
        let p = project(1920, 1080);
        let edl = cut_core::edl_from_project(&p);
        let opts = RenderOptions::default();
        assert_eq!(opts.output_geometry(&p, &edl), (1920, 1080));
        assert!(require_even_output(&p, &opts).is_ok());
    }
}
