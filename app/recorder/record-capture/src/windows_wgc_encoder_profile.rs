//! WGC Media Foundation input and output descriptor cadence.

use windows::Media::Core::VideoStreamDescriptor;
use windows::Media::MediaProperties::{
    MediaEncodingProfile, MediaEncodingSubtypes, VideoEncodingProperties, VideoEncodingQuality,
};

fn set_cadence(properties: &VideoEncodingProperties, fps: u32) -> windows::core::Result<()> {
    properties.FrameRate()?.SetNumerator(fps)?;
    properties.FrameRate()?.SetDenominator(1)
}

pub(crate) fn encoding_properties(
    width: u32,
    height: u32,
    fps: u32,
) -> windows::core::Result<(VideoStreamDescriptor, MediaEncodingProfile)> {
    let input = VideoEncodingProperties::CreateUncompressed(
        &MediaEncodingSubtypes::Bgra8()?,
        width,
        height,
    )?;
    // Both sides must describe the same requested timebase. Setting only
    // the output rate lets the implicit input rate drive transcoding.
    set_cadence(&input, fps)?;
    let profile = MediaEncodingProfile::CreateMp4(VideoEncodingQuality::HD1080p)?;
    profile.SetAudio(None)?;
    let output = profile.Video()?;
    output.SetSubtype(&MediaEncodingSubtypes::Hevc()?)?;
    output.SetWidth(width)?;
    output.SetHeight(height)?;
    output.SetBitrate(15_000_000)?;
    set_cadence(&output, fps)?;
    Ok((VideoStreamDescriptor::Create(&input)?, profile))
}
