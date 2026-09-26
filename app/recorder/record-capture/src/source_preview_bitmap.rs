//! Bounded in-memory BGRA to BMP conversion for native preview callbacks.

use record_core::{error_codes, RecordError, Result};

use crate::source_preview::MAX_SOURCE_PREVIEW_FRAME_BYTES;
use crate::CaptureRegion;

const BMP_HEADER_BYTES: usize = 54;

/// Raw CPU layouts accepted from native capture callbacks. The output is always
/// a 32-bit BMP, so no original platform buffer leaves the recorder owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourcePreviewPixelFormat {
    #[cfg(any(test, all(target_os = "linux", feature = "capture-linux")))]
    Bgrx,
    Bgra,
    #[cfg(any(test, all(target_os = "linux", feature = "capture-linux")))]
    Rgbx,
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    Rgba,
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    Rgb,
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    Bgr,
}

/// Encode one real native BGRA/BGRx frame as a top-down 32-bit BMP.
///
/// The returned bytes have no path or file side effect. Sampling is nearest-neighbour
/// only to make an over-large native frame fit the lifecycle's fixed memory ceiling.
pub(crate) fn bgra_bmp(
    width: u32,
    height: u32,
    stride: usize,
    pixels: &[u8],
    region: Option<CaptureRegion>,
) -> Result<Vec<u8>> {
    native_bmp(
        width,
        height,
        SourcePreviewPixelFormat::Bgra,
        stride,
        pixels,
        0,
        region,
    )
}

/// Encode a complete CPU-readable native frame without allocating a full-size
/// conversion buffer. Sampling and conversion happen directly into the capped BMP.
pub(crate) fn native_bmp(
    width: u32,
    height: u32,
    format: SourcePreviewPixelFormat,
    stride: usize,
    pixels: &[u8],
    offset: usize,
    region: Option<CaptureRegion>,
) -> Result<Vec<u8>> {
    native_bmp_with_pixel_limit(
        width,
        height,
        format,
        stride,
        pixels,
        offset,
        region,
        ((MAX_SOURCE_PREVIEW_FRAME_BYTES - BMP_HEADER_BYTES) / 4) as u64,
    )
}

/// Active recording previews use a smaller transport ceiling than the
/// pre-start source picker, which can afford larger one-off frames.
pub(crate) fn active_native_bmp(
    width: u32,
    height: u32,
    format: SourcePreviewPixelFormat,
    stride: usize,
    pixels: &[u8],
    offset: usize,
) -> Result<Vec<u8>> {
    native_bmp_with_pixel_limit(
        width,
        height,
        format,
        stride,
        pixels,
        offset,
        None,
        640 * 360,
    )
}

fn native_bmp_with_pixel_limit(
    width: u32,
    height: u32,
    format: SourcePreviewPixelFormat,
    stride: usize,
    pixels: &[u8],
    offset: usize,
    region: Option<CaptureRegion>,
    max_pixels: u64,
) -> Result<Vec<u8>> {
    let width_usize = usize::try_from(width).map_err(|_| invalid("preview width overflows"))?;
    let height_usize = usize::try_from(height).map_err(|_| invalid("preview height overflows"))?;
    let row = width_usize
        .checked_mul(format.bytes_per_pixel())
        .filter(|row| *row > 0)
        .ok_or_else(|| invalid("preview dimensions are empty or overflow"))?;
    let stride = if stride == 0 { row } else { stride };
    if stride < row
        || height_usize
            .checked_sub(1)
            .and_then(|last| last.checked_mul(stride))
            .and_then(|last| last.checked_add(row))
            .and_then(|last| last.checked_add(offset))
            .is_none_or(|needed| needed > pixels.len())
    {
        return Err(invalid(
            "preview native pixel buffer is not a complete BGRA frame",
        ));
    }
    let (left, top, source_width, source_height) = region_parts(region, width, height)?;
    let (out_w, out_h) = bounded_dimensions(source_width, source_height, max_pixels)?;
    let output_bytes = usize::try_from(out_w)
        .ok()
        .and_then(|w| usize::try_from(out_h).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| bytes.checked_add(BMP_HEADER_BYTES))
        .filter(|bytes| *bytes <= MAX_SOURCE_PREVIEW_FRAME_BYTES)
        .ok_or_else(|| invalid("preview output exceeds its memory ceiling"))?;
    let mut bmp = Vec::with_capacity(output_bytes);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(u32::try_from(output_bytes).unwrap_or(u32::MAX)).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&(BMP_HEADER_BYTES as u32).to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(out_w as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(out_h as i32)).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(
        &(u32::try_from(output_bytes - BMP_HEADER_BYTES).unwrap_or(u32::MAX)).to_le_bytes(),
    );
    bmp.extend_from_slice(&[0; 16]);
    for y in 0..out_h {
        let source_y = usize::try_from(
            u64::from(top) + (u64::from(y) * u64::from(source_height)) / u64::from(out_h),
        )
        .map_err(|_| invalid("preview source row overflows"))?;
        for x in 0..out_w {
            let source_x = usize::try_from(
                u64::from(left) + (u64::from(x) * u64::from(source_width)) / u64::from(out_w),
            )
            .map_err(|_| invalid("preview source column overflows"))?;
            let start = source_y
                .checked_mul(stride)
                .and_then(|row_start| {
                    source_x
                        .checked_mul(format.bytes_per_pixel())
                        .and_then(|column| row_start.checked_add(column))
                })
                .and_then(|start| start.checked_add(offset))
                .ok_or_else(|| invalid("preview source offset overflows"))?;
            let pixel = pixels
                .get(start..start + format.bytes_per_pixel())
                .ok_or_else(|| invalid("preview source pixels are incomplete"))?;
            bmp.extend_from_slice(&format.to_bgra(pixel));
        }
    }
    Ok(bmp)
}

impl SourcePreviewPixelFormat {
    const fn bytes_per_pixel(self) -> usize {
        match self {
            #[cfg(all(target_os = "linux", feature = "capture-linux"))]
            Self::Rgb | Self::Bgr => 3,
            Self::Bgra => 4,
            #[cfg(any(test, all(target_os = "linux", feature = "capture-linux")))]
            Self::Bgrx | Self::Rgbx => 4,
            #[cfg(all(target_os = "linux", feature = "capture-linux"))]
            Self::Rgba => 4,
        }
    }

    fn to_bgra(self, source: &[u8]) -> [u8; 4] {
        match self {
            #[cfg(any(test, all(target_os = "linux", feature = "capture-linux")))]
            Self::Bgrx => [source[0], source[1], source[2], 255],
            Self::Bgra => [source[0], source[1], source[2], source[3]],
            #[cfg(any(test, all(target_os = "linux", feature = "capture-linux")))]
            Self::Rgbx => [source[2], source[1], source[0], 255],
            #[cfg(all(target_os = "linux", feature = "capture-linux"))]
            Self::Rgba => [source[2], source[1], source[0], source[3]],
            #[cfg(all(target_os = "linux", feature = "capture-linux"))]
            Self::Rgb => [source[2], source[1], source[0], 255],
            #[cfg(all(target_os = "linux", feature = "capture-linux"))]
            Self::Bgr => [source[0], source[1], source[2], 255],
        }
    }
}

fn region_parts(
    region: Option<CaptureRegion>,
    width: u32,
    height: u32,
) -> Result<(u32, u32, u32, u32)> {
    let Some(region) = region else {
        return Ok((0, 0, width, height));
    };
    let (left, top, region_width, region_height, parent_width, parent_height) =
        region.native_parts();
    (parent_width == width && parent_height == height)
        .then_some((left, top, region_width, region_height))
        .ok_or_else(|| invalid("preview region does not match the current display frame"))
}

fn bounded_dimensions(width: u32, height: u32, max_pixels: u64) -> Result<(u32, u32)> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| invalid("preview dimensions overflow"))?;
    if pixels <= max_pixels {
        return Ok((width, height));
    }
    let scale = ((max_pixels as f64) / (pixels as f64)).sqrt();
    let out_w = ((width as f64 * scale).floor() as u32).max(1);
    let out_h = ((height as f64 * scale).floor() as u32).max(1);
    (u64::from(out_w) * u64::from(out_h) <= max_pixels)
        .then_some((out_w, out_h))
        .ok_or_else(|| invalid("preview dimensions cannot fit the memory ceiling"))
}

fn invalid(message: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        message,
        "invalid native preview pixels",
    )
}
