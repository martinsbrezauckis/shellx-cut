//! Immutable GPU sample snapshots, including the Region prefix crop.

use windows::core::Interface;
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_BOX};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;
use windows_capture::frame::Frame;

pub(crate) fn snapshot(
    frame: &Frame<'_>,
    width: u32,
    height: u32,
) -> windows::core::Result<IDirect3DSurface> {
    let mut desc = *frame.desc();
    desc.Width = width;
    desc.Height = height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.CPUAccessFlags = 0;
    desc.MiscFlags = 0;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
    let mut texture = None;
    let source = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: width.min(frame.width()),
        bottom: height.min(frame.height()),
        back: 1,
    };
    // SAFETY: the admitted prefix is contained in the source. Each queued
    // sample owns a separate GPU resource, preventing the next WGC callback
    // or Region crop from modifying a surface still awaited by the encoder.
    unsafe {
        frame
            .device()
            .CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.ok_or_else(windows::core::Error::empty)?;
        // Preserve the previous encoder's padding when a selected window
        // shrinks below its initially admitted geometry. Pixels outside the
        // current frame are opaque black, never uninitialized GPU memory.
        if width > frame.width() || height > frame.height() {
            let mut view = None;
            frame
                .device()
                .CreateRenderTargetView(&texture, None, Some(&mut view))?;
            let view = view.ok_or_else(windows::core::Error::empty)?;
            frame
                .device_context()
                .ClearRenderTargetView(&view, &[0.0, 0.0, 0.0, 1.0]);
        }
        frame.device_context().CopySubresourceRegion(
            &texture,
            0,
            0,
            0,
            0,
            frame.as_raw_texture(),
            0,
            Some(&source),
        );
        // Submit the producer copy before handing the immutable surface to
        // MediaStreamSource's consumer context, as the previous encoder did.
        frame.device_context().Flush();
        let dxgi: IDXGISurface = texture.cast()?;
        CreateDirect3D11SurfaceFromDXGISurface(&dxgi)?.cast()
    }
}
