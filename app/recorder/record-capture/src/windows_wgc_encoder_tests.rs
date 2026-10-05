use super::*;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;

fn solid_surface(width: u32, height: u32) -> Result<IDirect3DSurface, Error> {
    let (device, context) = windows_capture::d3d11::create_d3d_device()?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        ..Default::default()
    };
    let mut texture = None;
    unsafe {
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.ok_or_else(windows::core::Error::empty)?;
        let mut view = None;
        device.CreateRenderTargetView(&texture, None, Some(&mut view))?;
        context.ClearRenderTargetView(
            &view.ok_or_else(windows::core::Error::empty)?,
            &[0.0, 0.2, 0.4, 1.0],
        );
        context.Flush();
        let dxgi: IDXGISurface = texture.cast()?;
        Ok(CreateDirect3D11SurfaceFromDXGISurface(&dxgi)?.cast()?)
    }
}

#[test]
fn uncompressed_descriptor_and_hevc_profile_use_requested_rate() -> Result<(), Error> {
    crate::windows_runtime::pin_process_mta()?;
    for fps in [24, 25, 30, 60] {
        let (descriptor, profile) = encoding_properties(1920, 1080, fps)?;
        for properties in [descriptor.EncodingProperties()?, profile.Video()?] {
            assert_eq!(properties.FrameRate()?.Numerator()?, fps);
            assert_eq!(properties.FrameRate()?.Denominator()?, 1);
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires Windows D3D/MF and SHELLX_RECORD_FFMPEG/FFPROBE; run in native gate"]
fn one_frame_static_stop_produces_playable_duration() -> Result<(), Error> {
    use std::time::Duration;

    crate::windows_runtime::pin_process_mta()?;
    let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG")?;
    let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE")?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("static.mp4");
    let mut encoder = WgcVideoEncoder::new(640, 480, 30, &path)?;
    let stop_at = Instant::now();
    // Inject the actual two observation instants into the native media
    // path, while avoiding a 15-second wall-clock wait in the test.
    encoder.send_snapshot(
        solid_surface(640, 480)?,
        5_112_895_216_850,
        stop_at - Duration::from_secs(15),
    )?;
    assert_eq!(encoder.finish_after_control_stop(stop_at)?, Some(15_000));
    let media = record_recovery::verify_media(&ffmpeg, &ffprobe, &path)?;
    assert!(media.duration_ms >= 14_500 && media.duration_ms <= 15_500);
    assert!(media.decoded_video_frames >= 1);
    Ok(())
}

#[test]
#[ignore = "requires Windows D3D/MF and SHELLX_RECORD_FFMPEG/FFPROBE; run in native gate"]
fn sparse_native_frames_hold_only_until_owned_stop() -> Result<(), Error> {
    use std::time::Duration;

    crate::windows_runtime::pin_process_mta()?;
    let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG")?;
    let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE")?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("sparse.mp4");
    let surface = solid_surface(640, 480)?;
    let mut encoder = WgcVideoEncoder::new(640, 480, 30, &path)?;
    let stop_at = Instant::now();
    let first_at = stop_at - Duration::from_secs(3);
    encoder.send_snapshot(surface.clone(), 1_000_000, first_at)?;
    encoder.send_snapshot(surface, 11_000_000, first_at + Duration::from_secs(1))?;
    assert_eq!(encoder.finish_after_control_stop(stop_at)?, Some(2_000));
    let media = record_recovery::verify_media(&ffmpeg, &ffprobe, &path)?;
    assert!(media.duration_ms >= 2_500 && media.duration_ms <= 3_500);
    assert!(media.decoded_video_frames >= 2);
    Ok(())
}

#[test]
#[ignore = "requires Windows D3D/MF and SHELLX_RECORD_FFMPEG/FFPROBE; run in native gate"]
fn unowned_close_cannot_claim_a_static_hold() -> Result<(), Error> {
    use std::time::Duration;

    crate::windows_runtime::pin_process_mta()?;
    let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG")?;
    let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE")?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("unowned.mp4");
    let mut encoder = WgcVideoEncoder::new(640, 480, 30, &path)?;
    encoder.send_snapshot(
        solid_surface(640, 480)?,
        1_000_000,
        Instant::now() - Duration::from_secs(15),
    )?;
    // on_closed and failed native Stop take this ordinary close path.
    // Even though the observation is old, it must not append a 15s sample.
    let _ = encoder.finish();
    if let Ok(media) = record_recovery::verify_media(&ffmpeg, &ffprobe, &path) {
        assert!(media.duration_ms < 1_000);
    }
    Ok(())
}

#[test]
#[ignore = "requires Windows D3D/MF and SHELLX_RECORD_FFMPEG/FFPROBE; run in native gate"]
fn normal_cadence_stop_needs_no_extra_queued_sample() -> Result<(), Error> {
    use std::time::Duration;

    crate::windows_runtime::pin_process_mta()?;
    let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG")?;
    let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE")?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("normal-cadence.mp4");
    let mut encoder = WgcVideoEncoder::new(640, 480, 30, &path)?;
    let stop_at = Instant::now();
    let surface = solid_surface(640, 480)?;
    encoder.send_snapshot(
        surface.clone(),
        1_000_000,
        stop_at - Duration::from_micros(34_333),
    )?;
    encoder.send_snapshot(surface, 1_333_333, stop_at - Duration::from_millis(1))?;
    assert_eq!(encoder.finish_after_control_stop(stop_at)?, None);
    let media = record_recovery::verify_media(&ffmpeg, &ffprobe, &path)?;
    assert!(media.duration_ms > 0 && media.duration_ms < 1_000);
    assert!(media.decoded_video_frames >= 2);
    Ok(())
}
