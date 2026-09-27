//! Keep Recorder tool paths aligned with Cut's current tool choice.
//!
//! The first Record Doctor may run before a consented FFmpeg install. An auto
//! assignment of bare `ffmpeg` must not stick after that install, while an
//! operator-supplied `SHELLX_RECORD_*` path must remain authoritative.

use std::ffi::OsString;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct AutoAssignment {
    last: Option<OsString>,
}

impl AutoAssignment {
    fn next(&mut self, current: Option<OsString>, resolved: OsString) -> Option<OsString> {
        if current.is_some() && current != self.last {
            self.last = None;
            return None;
        }
        self.last = Some(resolved.clone());
        (current.as_ref() != Some(&resolved)).then_some(resolved)
    }
}

pub(super) fn align() {
    static AUTO: OnceLock<Mutex<(AutoAssignment, AutoAssignment)>> = OnceLock::new();
    let mut assignments = AUTO
        .get_or_init(|| Mutex::new((AutoAssignment::default(), AutoAssignment::default())))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let ffmpeg = std::env::var_os("SHELLX_RECORD_FFMPEG");
    if let Some(next) = assignments.0.next(ffmpeg, cut_media::toolpath::ffmpeg()) {
        std::env::set_var("SHELLX_RECORD_FFMPEG", next);
    }
    let ffprobe = std::env::var_os("SHELLX_RECORD_FFPROBE");
    if let Some(next) = assignments.1.next(ffprobe, cut_media::toolpath::ffprobe()) {
        std::env::set_var("SHELLX_RECORD_FFPROBE", next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_doctor_before_fetch_refreshes_only_its_own_auto_assignment() {
        let mut ffmpeg = AutoAssignment::default();
        let missing = OsString::from("ffmpeg");
        let installed = OsString::from(r"C:\Cut Tools\ffmpeg.exe");
        assert_eq!(ffmpeg.next(None, missing.clone()), Some(missing.clone()));
        assert_eq!(
            ffmpeg.next(Some(missing), installed.clone()),
            Some(installed.clone())
        );
        assert_eq!(
            ffmpeg.next(Some(installed.clone()), installed.clone()),
            None
        );

        let operator = OsString::from(r"C:\Custom\ffmpeg.exe");
        assert_eq!(ffmpeg.next(Some(operator.clone()), installed.clone()), None);
        assert_eq!(ffmpeg.next(Some(operator), installed.clone()), None);
        assert_eq!(ffmpeg.next(None, installed.clone()), Some(installed));
    }

    #[test]
    fn initial_explicit_record_tool_override_is_preserved() {
        let mut ffprobe = AutoAssignment::default();
        assert_eq!(
            ffprobe.next(
                Some(OsString::from("user-ffprobe")),
                OsString::from("discovered")
            ),
            None
        );
        assert_eq!(
            ffprobe.next(
                Some(OsString::from("user-ffprobe")),
                OsString::from("new-discovery")
            ),
            None
        );
    }
}
