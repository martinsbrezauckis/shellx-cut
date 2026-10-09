//! Window-only full-content GPU fitting; queued output textures stay immutable.
use crate::window_frame_fit::WindowFrameFit;
use windows::core::{s, Interface};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;
use windows_capture::frame::Frame;

type Error = Box<dyn std::error::Error + Send + Sync>;
const SHADER: &[u8] = br#"
struct Vertex { float4 position : SV_Position; float2 uv : TEXCOORD0; };
Vertex vertex(uint index : SV_VertexID) {
    Vertex result;
    float2 uv = float2((index << 1) & 2, index & 2);
    result.position = float4(uv.x * 2 - 1, 1 - uv.y * 2, 0, 1);
    result.uv = uv;
    return result;
}
Texture2D<float4> source : register(t0);
SamplerState linearClamp : register(s0);
float4 pixel(Vertex input) : SV_Target { return source.Sample(linearClamp, input.uv); }
"#;

struct InputTexture {
    size: (u32, u32),
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
}

pub(crate) struct WindowFrameFitter {
    device: ID3D11Device,
    deferred: ID3D11DeviceContext,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    rasterizer: ID3D11RasterizerState,
    input: Option<InputTexture>,
}

impl WindowFrameFitter {
    pub(crate) fn new(device: &ID3D11Device) -> Result<Self, Error> {
        // SV_VertexID shader model4 requires feature level10. Identity frames
        // use the existing copy path without constructing this fitter.
        if unsafe { device.GetFeatureLevel() }.0 < D3D_FEATURE_LEVEL_10_0.0 {
            return Err("full-window fitting requires Direct3D feature level10 or newer".into());
        }
        let vertex_code = compile(s!("vertex"), s!("vs_4_0"))?;
        let pixel_code = compile(s!("pixel"), s!("ps_4_0"))?;
        let mut vertex = None;
        let mut pixel = None;
        let mut sampler = None;
        let mut rasterizer = None;
        let mut deferred = None;
        // SAFETY: shader blobs remain owned during shader creation; all output
        // pointers name initialized Options, and descriptors contain no pointers.
        unsafe {
            device.CreateVertexShader(blob_bytes(&vertex_code), None, Some(&mut vertex))?;
            device.CreatePixelShader(blob_bytes(&pixel_code), None, Some(&mut pixel))?;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxAnisotropy: 1,
                    ComparisonFunc: D3D11_COMPARISON_NEVER,
                    MinLOD: 0.0,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    DepthClipEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut rasterizer),
            )?;
            device.CreateDeferredContext(0, Some(&mut deferred))?;
        }
        Ok(Self {
            device: device.clone(),
            deferred: deferred.ok_or("missing window fitting context")?,
            vertex: vertex.ok_or("missing window fitting vertex shader")?,
            pixel: pixel.ok_or("missing window fitting pixel shader")?,
            sampler: sampler.ok_or("missing window fitting sampler")?,
            rasterizer: rasterizer.ok_or("missing window fitting rasterizer")?,
            input: None,
        })
    }

    pub(crate) fn snapshot(
        &mut self,
        frame: &Frame<'_>,
        output: (u32, u32),
    ) -> Result<(IDirect3DSurface, WindowFrameFit), Error> {
        if self.device.as_raw() != frame.device().as_raw() {
            return Err("window fitting device changed during capture".into());
        }
        self.fit_texture(
            frame.device_context(),
            frame.as_raw_texture(),
            (frame.width(), frame.height()),
            output,
        )
    }

    fn fit_texture(
        &mut self,
        immediate: &ID3D11DeviceContext,
        source: &ID3D11Texture2D,
        size: (u32, u32),
        output: (u32, u32),
    ) -> Result<(IDirect3DSurface, WindowFrameFit), Error> {
        let fit = WindowFrameFit::new(size, output).ok_or("invalid full-window fit dimensions")?;
        let level = unsafe { self.device.GetFeatureLevel() };
        let limit = if level.0 >= D3D_FEATURE_LEVEL_11_0.0 {
            D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION
        } else {
            8192
        };
        if [size.0, size.1, output.0, output.1]
            .iter()
            .any(|n| *n > limit)
        {
            return Err("window fit exceeds the actual Direct3D device texture limit".into());
        }
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            source.GetDesc(&mut desc);
        }
        if desc.Format != DXGI_FORMAT_R8G8B8A8_UNORM
            || desc.SampleDesc.Count != 1
            || desc.ArraySize != 1
            || size.0 > desc.Width
            || size.1 > desc.Height
        {
            return Err("window fit requires an admitted full RGBA8 single-sample texture".into());
        }
        desc.MipLevels = 1;
        desc.ArraySize = 1;
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.CPUAccessFlags = 0;
        desc.MiscFlags = 0;
        if self.input.as_ref().is_none_or(|input| input.size != size) {
            desc.Width = size.0;
            desc.Height = size.1;
            desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
            let mut texture = None;
            let mut view = None;
            unsafe {
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut texture))?;
            }
            let texture = texture.ok_or("missing window fitting input texture")?;
            unsafe {
                self.device
                    .CreateShaderResourceView(&texture, None, Some(&mut view))?;
            }
            self.input = Some(InputTexture {
                size,
                texture,
                view: view.ok_or("missing window fitting input view")?,
            });
        }
        desc.Width = output.0;
        desc.Height = output.1;
        desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
        let mut texture = None;
        let mut view = None;
        unsafe {
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
        }
        let texture = texture.ok_or("missing immutable window fitting output")?;
        unsafe {
            self.device
                .CreateRenderTargetView(&texture, None, Some(&mut view))?;
        }
        let view = view.ok_or("missing window fitting output view")?;
        let input = self
            .input
            .as_ref()
            .ok_or("window fitting input not initialized")?;
        let (left, top, width, height) = fit.destination();
        let source_box = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: size.0,
            bottom: size.1,
            back: 1,
        };
        // SAFETY: source bounds were validated against the actual texture;
        // resources belong to this device. Each output is owned by one sample.
        // Private deferred state prevents shader bindings leaking into preview.
        unsafe {
            self.deferred.CopySubresourceRegion(
                &input.texture,
                0,
                0,
                0,
                0,
                source,
                0,
                Some(&source_box),
            );
            self.deferred
                .ClearRenderTargetView(&view, &[0.0, 0.0, 0.0, 1.0]);
            self.deferred.OMSetRenderTargets(Some(&[Some(view)]), None);
            self.deferred.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: left as f32,
                TopLeftY: top as f32,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            self.deferred.RSSetState(&self.rasterizer);
            self.deferred
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.deferred.VSSetShader(&self.vertex, None);
            self.deferred.PSSetShader(&self.pixel, None);
            self.deferred
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.deferred
                .PSSetShaderResources(0, Some(&[Some(input.view.clone())]));
            self.deferred.Draw(3, 0);
            let mut commands = None;
            self.deferred
                .FinishCommandList(false, Some(&mut commands))?;
            let commands = commands.ok_or("missing window fitting commands")?;
            immediate.ExecuteCommandList(&commands, true);
            immediate.Flush();
            let dxgi: IDXGISurface = texture.cast()?;
            Ok((CreateDirect3D11SurfaceFromDXGISurface(&dxgi)?.cast()?, fit))
        }
    }
}

fn compile(entry: windows::core::PCSTR, target: windows::core::PCSTR) -> Result<ID3DBlob, Error> {
    let mut blob = None;
    let mut errors = None;
    // Fixed local shader text: no includes, file reads or external compiler process.
    let compiled = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            s!("CutWindowFit"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut blob,
            Some(&mut errors),
        )
    };
    if let Err(error) = compiled {
        let detail = errors
            .as_ref()
            .map(|blob| {
                String::from_utf8_lossy(unsafe { blob_bytes(blob) })
                    .trim_end_matches('\0')
                    .to_string()
            })
            .unwrap_or_default();
        return Err(format!("compile full-window fit shader: {error}: {detail}").into());
    }
    blob.ok_or_else(|| "window fitting shader compiler returned no bytecode".into())
}

unsafe fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    // SAFETY: the owned blob retains this byte buffer for the returned borrow.
    std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;

    fn corner_texture(device: &ID3D11Device, size: (u32, u32)) -> Result<ID3D11Texture2D, Error> {
        let mut pixels = vec![0u8; (size.0 * size.1 * 4) as usize];
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        let origins = [
            (0, 0),
            (size.0 - 16, 0),
            (0, size.1 - 16),
            (size.0 - 16, size.1 - 16),
        ];
        for (origin, color) in origins.into_iter().zip([
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
        ]) {
            for y in origin.1..origin.1 + 16 {
                for x in origin.0..origin.0 + 16 {
                    let i = ((y * size.0 + x) * 4) as usize;
                    pixels[i..i + 4].copy_from_slice(&color);
                }
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr().cast(),
            SysMemPitch: size.0 * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        unsafe {
            device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
        }
        texture.ok_or_else(|| "native test input texture missing".into())
    }

    fn read_pixels(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        surface: &IDirect3DSurface,
    ) -> Result<Vec<u8>, Error> {
        let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut desc);
        }
        desc.BindFlags = 0;
        desc.MiscFlags = 0;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        unsafe {
            device.CreateTexture2D(&desc, None, Some(&mut staging))?;
        }
        let staging = staging.ok_or("native test staging texture missing")?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            context.CopyResource(&staging, &texture);
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }
        let mut pixels = vec![0u8; (desc.Width * desc.Height * 4) as usize];
        for y in 0..desc.Height as usize {
            // Native readback exists only in tests; honor the driver's row pitch.
            unsafe {
                pixels[y * desc.Width as usize * 4..(y + 1) * desc.Width as usize * 4]
                    .copy_from_slice(std::slice::from_raw_parts(
                        mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                        desc.Width as usize * 4,
                    ));
            }
        }
        unsafe {
            context.Unmap(&staging, 0);
        }
        Ok(pixels)
    }

    #[test]
    fn gpu_grow_shrink_retains_corners_bars_and_previous_sample() -> Result<(), Error> {
        crate::windows_runtime::pin_process_mta()?;
        let (device, context) = windows_capture::d3d11::create_d3d_device()?;
        let mut fitter = WindowFrameFitter::new(&device)?;
        let mut previous: Option<(IDirect3DSurface, Vec<u8>)> = None;
        for size in [(900, 650), (640, 400), (601, 901)] {
            let input = corner_texture(&device, size)?;
            let (surface, fit) = fitter.fit_texture(&context, &input, size, (800, 600))?;
            let pixels = read_pixels(&device, &context, &surface)?;
            for (origin, color) in [
                (0, 0),
                (size.0 - 16, 0),
                (0, size.1 - 16),
                (size.0 - 16, size.1 - 16),
            ]
            .into_iter()
            .zip([
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
            ]) {
                let point = fit
                    .map_point(f64::from(origin.0 + 8), f64::from(origin.1 + 8))
                    .unwrap();
                let i = ((point.1 as u32 * 800 + point.0 as u32) * 4) as usize;
                assert_eq!(&pixels[i..i + 4], &color);
            }
            assert_eq!(&pixels[0..4], &[0, 0, 0, 255]);
            assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
            if let Some((surface, expected)) = previous {
                assert_eq!(
                    read_pixels(&device, &context, &surface)?,
                    expected,
                    "later source/resize must not mutate an already accepted sample"
                );
            }
            previous = Some((surface, pixels));
        }
        Ok(())
    }

    #[test]
    fn gpu_fit_restores_shared_immediate_viewport() -> Result<(), Error> {
        crate::windows_runtime::pin_process_mta()?;
        let (device, context) = windows_capture::d3d11::create_d3d_device()?;
        let mut fitter = WindowFrameFitter::new(&device)?;
        let original = D3D11_VIEWPORT {
            TopLeftX: 7.0,
            TopLeftY: 9.0,
            Width: 123.0,
            Height: 234.0,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        unsafe {
            context.RSSetViewports(Some(&[original]));
        }
        let input = corner_texture(&device, (900, 650))?;
        let _ = fitter.fit_texture(&context, &input, (900, 650), (800, 600))?;
        let mut count = 1;
        let mut actual = D3D11_VIEWPORT::default();
        unsafe {
            context.RSGetViewports(&mut count, Some(&mut actual));
        }
        assert_eq!(count, 1);
        assert_eq!(
            (
                actual.TopLeftX,
                actual.TopLeftY,
                actual.Width,
                actual.Height
            ),
            (7.0, 9.0, 123.0, 234.0)
        );
        Ok(())
    }

    #[test]
    fn gpu_fit_refuses_source_dimensions_beyond_texture() -> Result<(), Error> {
        crate::windows_runtime::pin_process_mta()?;
        let (device, context) = windows_capture::d3d11::create_d3d_device()?;
        let mut fitter = WindowFrameFitter::new(&device)?;
        let input = corner_texture(&device, (800, 600))?;
        assert!(fitter
            .fit_texture(&context, &input, (900, 650), (800, 600))
            .is_err());
        Ok(())
    }
}
