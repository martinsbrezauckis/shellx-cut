//! color.rs — `Rgba` styling color (8-bit per channel).
//!
//! Used by render styling (background, frame shadow, caption box). Serialized as
//! `{r,g,b,a}`. Kept tiny + Copy so it threads through the plan cheaply.

use serde::{Deserialize, Serialize};

/// 8-bit straight-alpha RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Opaque RGB (alpha = 255).
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const TRANSPARENT: Rgba = Rgba::new(0, 0, 0, 0);
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);

    /// Premultiplied-alpha tuple in 0..1 floats (handy for compositors).
    pub fn to_f32_premul(self) -> (f32, f32, f32, f32) {
        let a = self.a as f32 / 255.0;
        (
            (self.r as f32 / 255.0) * a,
            (self.g as f32 / 255.0) * a,
            (self.b as f32 / 255.0) * a,
            a,
        )
    }
}
