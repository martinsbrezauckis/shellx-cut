//! GPU-only Region crop for Windows Graphics Capture frames.

use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_BOX};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows_capture::frame::Frame;

use crate::region_geometry::NativePixelCrop;

pub(crate) struct GpuCropSurface {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

/// Copy one exact sub-rectangle through a cached GPU-only texture and place it
/// at the source texture's origin. `VideoEncoder` is configured to the crop
/// dimensions, so its own Direct3D copy then reads only this prefix. No staging
/// texture, CPU mapping, or full-monitor encode is involved.
pub(crate) fn crop_frame_to_origin(
    frame: &Frame<'_>,
    crop: NativePixelCrop,
    scratch: &mut Option<GpuCropSurface>,
) -> std::result::Result<(), windows::core::Error> {
    let (parent_width, parent_height) = crop.parent_size();
    if frame.width() != parent_width || frame.height() != parent_height {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(0x80070057_u32 as i32),
            "the WGC frame no longer matches the selected Region parent",
        ));
    }
    let (left, top, width, height) = crop.origin_and_size();
    let format = frame.desc().Format;
    let recreate = scratch.as_ref().is_none_or(|surface| {
        surface.width != width || surface.height != height || surface.format != format
    });
    if recreate {
        let mut desc = *frame.desc();
        desc.Width = width;
        desc.Height = height;
        desc.MipLevels = 1;
        desc.ArraySize = 1;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = 0;
        desc.MiscFlags = 0;
        let mut texture = None;
        // SAFETY: `desc` is derived from the live WGC texture and narrowed to
        // one default GPU resource with no CPU access or caller-owned data.
        unsafe {
            frame
                .device()
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
        }
        *scratch = Some(GpuCropSurface {
            texture: texture.ok_or_else(|| {
                windows::core::Error::new(
                    windows::core::HRESULT(0x80004005_u32 as i32),
                    "Direct3D did not return a Region crop texture",
                )
            })?,
            width,
            height,
            format,
        });
    }
    let surface = scratch
        .as_ref()
        .expect("the GPU crop surface was created before use");
    let source_box = D3D11_BOX {
        left,
        top,
        front: 0,
        right: left + width,
        bottom: top + height,
        back: 1,
    };
    // SAFETY: the validated half-open source box is contained in the exact
    // parent frame. The temporary texture has precisely the crop dimensions;
    // copying it back to (0,0) cannot overlap the source resource.
    unsafe {
        frame.device_context().CopySubresourceRegion(
            &surface.texture,
            0,
            0,
            0,
            0,
            frame.as_raw_texture(),
            0,
            Some(&source_box),
        );
        frame.device_context().CopySubresourceRegion(
            frame.as_raw_texture(),
            0,
            0,
            0,
            0,
            &surface.texture,
            0,
            None,
        );
    }
    Ok(())
}
