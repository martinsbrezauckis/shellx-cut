//! One full-window transform shared by accepted GPU pixels and native clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowFrameFit {
    source: (u32, u32),
    output: (u32, u32),
    destination: (u32, u32, u32, u32),
}

impl WindowFrameFit {
    // D3D11's largest 2D texture bound. The native fitter also checks the
    // actual device feature level before allocating transformed resources.
    pub(crate) const MAX_DIMENSION: u32 = 16_384;

    pub(crate) fn new(source: (u32, u32), output: (u32, u32)) -> Option<Self> {
        if [source.0, source.1, output.0, output.1]
            .iter()
            .any(|&n| n == 0 || n > Self::MAX_DIMENSION)
        {
            return None;
        }
        let (sw, sh) = (u64::from(source.0), u64::from(source.1));
        let (ow, oh) = (u64::from(output.0), u64::from(output.1));
        let (width, height) = if sw * oh >= sh * ow {
            (output.0, ((ow * sh + sw / 2) / sw).clamp(1, oh) as u32)
        } else {
            (((oh * sw + sh / 2) / sh).clamp(1, ow) as u32, output.1)
        };
        Some(Self {
            source,
            output,
            destination: (
                (output.0 - width) / 2,
                (output.1 - height) / 2,
                width,
                height,
            ),
        })
    }

    pub(crate) fn source_size(self) -> (u32, u32) {
        self.source
    }
    pub(crate) fn output_size(self) -> (u32, u32) {
        self.output
    }
    pub(crate) fn destination(self) -> (u32, u32, u32, u32) {
        self.destination
    }
    pub(crate) fn is_identity(self) -> bool {
        self.source == self.output
    }

    pub(crate) fn map_point(self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..f64::from(self.source.0)).contains(&x)
            || !(0.0..f64::from(self.source.1)).contains(&y)
        {
            return None;
        }
        let (left, top, width, height) = self.destination;
        let mapped = (
            f64::from(left) + x * f64::from(width) / f64::from(self.source.0),
            f64::from(top) + y * f64::from(height) / f64::from(self.source.1),
        );
        ((f64::from(left)..f64::from(left + width)).contains(&mapped.0)
            && (f64::from(top)..f64::from(top + height)).contains(&mapped.1))
        .then_some(mapped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_and_grown_window_keep_all_edges() {
        let unchanged = WindowFrameFit::new((800, 600), (800, 600)).unwrap();
        assert!(unchanged.is_identity());
        assert_eq!(unchanged.map_point(799.0, 599.0), Some((799.0, 599.0)));
        let fit = WindowFrameFit::new((900, 650), (800, 600)).unwrap();
        assert_eq!(fit.destination(), (0, 11, 800, 578));
        let edge = fit.map_point(899.0, 649.0).unwrap();
        assert!(edge.0 > 799.0 && edge.0 < 800.0 && edge.1 > 588.0 && edge.1 < 589.0);
        assert_eq!(fit.map_point(900.0, 649.0), None);
    }
    #[test]
    fn shrink_and_tall_odd_sources_share_integer_fit() {
        assert_eq!(
            WindowFrameFit::new((640, 400), (800, 600))
                .unwrap()
                .destination(),
            (0, 50, 800, 500)
        );
        let tall = WindowFrameFit::new((601, 901), (800, 600)).unwrap();
        assert_eq!(tall.destination(), (200, 0, 400, 600));
        assert_eq!(tall.map_point(0.0, 0.0), Some((200.0, 0.0)));
        assert_eq!(tall.map_point(601.0, 0.0), None);
    }
    #[test]
    fn bad_dimensions_and_nonfinite_points_are_refused() {
        for dimensions in [(0, 600), (16385, 600), (u32::MAX, 600)] {
            assert!(WindowFrameFit::new(dimensions, (800, 600)).is_none());
            assert!(WindowFrameFit::new((800, 600), dimensions).is_none());
        }
        let fit = WindowFrameFit::new((800, 600), (800, 600)).unwrap();
        assert_eq!(fit.map_point(f64::NAN, 5.0), None);
        assert_eq!(fit.map_point(5.0, f64::INFINITY), None);
        assert_eq!(fit.map_point(-1.0, 5.0), None);
    }
    #[test]
    fn cpu_reference_fit_retains_four_corner_pixels_and_opaque_bars() {
        // Reference raster exercises the shared integer transform. Native GPU
        // sampling/format/state still needs its separate Windows proof.
        for size in [(900, 650), (640, 400), (601, 901)] {
            let fit = WindowFrameFit::new(size, (800, 600)).unwrap();
            let mut source = tiny_skia::Pixmap::new(size.0, size.1).unwrap();
            source.fill(tiny_skia::Color::BLACK);
            let colors = [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
            ];
            let origins = [
                (0, 0),
                (size.0 - 16, 0),
                (0, size.1 - 16),
                (size.0 - 16, size.1 - 16),
            ];
            for (origin, color) in origins.iter().zip(colors) {
                for y in origin.1..origin.1 + 16 {
                    for x in origin.0..origin.0 + 16 {
                        let index = ((y * size.0 + x) * 4) as usize;
                        source.data_mut()[index..index + 4].copy_from_slice(&color);
                    }
                }
            }
            let (left, top, width, height) = fit.destination();
            let mut output = tiny_skia::Pixmap::new(800, 600).unwrap();
            output.fill(tiny_skia::Color::BLACK);
            output.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &tiny_skia::PixmapPaint {
                    quality: tiny_skia::FilterQuality::Bilinear,
                    ..Default::default()
                },
                tiny_skia::Transform::from_row(
                    width as f32 / size.0 as f32,
                    0.0,
                    0.0,
                    height as f32 / size.1 as f32,
                    left as f32,
                    top as f32,
                ),
                None,
            );
            for (origin, color) in origins.iter().zip(colors) {
                let point = fit
                    .map_point(f64::from(origin.0 + 8), f64::from(origin.1 + 8))
                    .unwrap();
                let index = ((point.1 as u32 * 800 + point.0 as u32) * 4) as usize;
                assert_eq!(&output.data()[index..index + 4], &color);
            }
            assert!(output.data().as_chunks::<4>().0.iter().all(|p| p[3] == 255));
            assert_eq!(&output.data()[0..4], &[0, 0, 0, 255]);
        }
    }
}
