//! Recorder Doctor for a status display: the same cards as [`crate::doctor`],
//! but the screen card never starts a capture or shows a system prompt.

use crate::doctor::{doctor_with_screen_card, screen_backend, Card};
use crate::doctor_probe::{self, ScreenCheck};

/// Status-display cards plus what is known about screen capture (see [`ScreenCheck`]).
#[derive(Debug, Clone)]
pub struct DoctorStatus {
    pub cards: Vec<Card>,
    pub screen_check: ScreenCheck,
}

/// Build [`DoctorStatus`] without starting any capture.
pub fn doctor_status() -> DoctorStatus {
    let (status, detail) = screen_backend();
    let (screen_card, screen_check) = if status != "ok" {
        (
            Card::new("screen_capture", "capture", status, detail),
            ScreenCheck::NotApplicable,
        )
    } else {
        let (observation, check) = doctor_probe::screen_status();
        let (status, detail) = observation.card(detail);
        (
            Card::new("screen_capture", "capture", status, detail),
            check,
        )
    };
    DoctorStatus {
        cards: doctor_with_screen_card(screen_card),
        screen_check,
    }
}
