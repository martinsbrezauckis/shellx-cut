//! Read the video time scale from the already-open, closed MP4 file.
//!
//! Only box headers and the small `hdlr`/`mdhd` prefixes are read. In
//! particular, `mdat` is skipped with a seek, so its size does not affect
//! memory use.

use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Copy)]
struct Mp4Box {
    kind: [u8; 4],
    body: u64,
    end: u64,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read_box(file: &mut (impl Read + Seek), at: u64, parent_end: u64) -> io::Result<Mp4Box> {
    let remaining = parent_end
        .checked_sub(at)
        .ok_or_else(|| invalid("MP4 box starts beyond parent"))?;
    if remaining < 8 {
        return Err(invalid("truncated MP4 box header"));
    }
    file.seek(SeekFrom::Start(at))?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header)?;
    let short_size = u32::from_be_bytes(header[..4].try_into().unwrap());
    let mut kind = [0u8; 4];
    kind.copy_from_slice(&header[4..]);
    let (size, header_size) = if short_size == 1 {
        if remaining < 16 {
            return Err(invalid("truncated extended MP4 box header"));
        }
        let mut extended = [0u8; 8];
        file.read_exact(&mut extended)?;
        (u64::from_be_bytes(extended), 16u64)
    } else if short_size == 0 {
        (remaining, 8)
    } else {
        (u64::from(short_size), 8)
    };
    if size < header_size || size > remaining {
        return Err(invalid("invalid MP4 box bounds"));
    }
    let body = at
        .checked_add(header_size)
        .ok_or_else(|| invalid("MP4 box offset overflow"))?;
    let end = at
        .checked_add(size)
        .ok_or_else(|| invalid("MP4 box offset overflow"))?;
    Ok(Mp4Box { kind, body, end })
}

fn read_prefix<const N: usize>(file: &mut (impl Read + Seek), box_: Mp4Box) -> io::Result<[u8; N]> {
    if box_.end - box_.body < N as u64 {
        return Err(invalid("truncated MP4 metadata box"));
    }
    file.seek(SeekFrom::Start(box_.body))?;
    let mut data = [0u8; N];
    file.read_exact(&mut data)?;
    Ok(data)
}

fn handler_is_video(file: &mut (impl Read + Seek), box_: Mp4Box) -> io::Result<bool> {
    // FullBox, pre_defined, handler_type, reserved[3]; name may be empty.
    let prefix = read_prefix::<24>(file, box_)?;
    if prefix[0] != 0 {
        return Err(invalid("unknown MP4 hdlr version"));
    }
    Ok(&prefix[8..12] == b"vide")
}

fn mdhd_timescale(file: &mut (impl Read + Seek), box_: Mp4Box) -> io::Result<u32> {
    let version = read_prefix::<1>(file, box_)?[0];
    let (prefix_len, offset) = match version {
        0 => (24, 12),
        1 => (36, 20),
        _ => return Err(invalid("unknown MP4 mdhd version")),
    };
    let mut prefix = [0u8; 36];
    if box_.end - box_.body < prefix_len {
        return Err(invalid("truncated MP4 mdhd"));
    }
    file.seek(SeekFrom::Start(box_.body))?;
    file.read_exact(&mut prefix[..prefix_len as usize])?;
    let timescale = u32::from_be_bytes(prefix[offset..offset + 4].try_into().unwrap());
    if timescale == 0 {
        return Err(invalid("zero MP4 video timescale"));
    }
    Ok(timescale)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct VideoTrackTiming {
    pub(super) timescale: u32,
    pub(super) has_ctts: bool,
    pub(super) has_elst: bool,
    pub(super) stts_samples: u64,
    pub(super) stts_total_ticks: u64,
    pub(super) last_stts_delta: u64,
}

fn u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().unwrap())
}

fn fullbox_count(file: &mut (impl Read + Seek), box_: Mp4Box) -> io::Result<u32> {
    Ok(u32_be(&read_prefix::<8>(file, box_)?[4..8]))
}

fn stts_summary(file: &mut (impl Read + Seek), box_: Mp4Box) -> io::Result<(u64, u64, u64)> {
    let entry_count = u64::from(fullbox_count(file, box_)?);
    if entry_count.checked_mul(8) != Some(box_.end - box_.body - 8) {
        return Err(invalid("invalid MP4 stts table length"));
    }
    let (mut samples, mut total_ticks, mut last_delta) = (0_u64, 0_u64, 0_u64);
    let mut entry = [0_u8; 8];
    for _ in 0..entry_count {
        file.read_exact(&mut entry)?;
        let (count, delta) = (
            u64::from(u32_be(&entry[..4])),
            u64::from(u32_be(&entry[4..8])),
        );
        let run = count
            .checked_mul(delta)
            .ok_or_else(|| invalid("MP4 stts duration overflow"))?;
        samples = samples
            .checked_add(count)
            .ok_or_else(|| invalid("MP4 stts count overflow"))?;
        total_ticks = total_ticks
            .checked_add(run)
            .ok_or_else(|| invalid("MP4 stts ticks overflow"))?;
        if count > 0 {
            last_delta = delta;
        }
    }
    Ok((samples, total_ticks, last_delta))
}

fn child_box(
    file: &mut (impl Read + Seek),
    parent: Mp4Box,
    target: &[u8; 4],
    dup: &'static str,
) -> io::Result<Option<Mp4Box>> {
    let (mut found, mut at) = (None, parent.body);
    while at < parent.end {
        let child = read_box(file, at, parent.end)?;
        if child.kind == *target && found.replace(child).is_some() {
            return Err(invalid(dup));
        }
        at = child.end;
    }
    Ok(found)
}

fn track_video_header(
    file: &mut (impl Read + Seek),
    track: Mp4Box,
) -> io::Result<Option<(u32, Mp4Box)>> {
    let mdia = child_box(file, track, b"mdia", "duplicate MP4 mdia")?
        .ok_or_else(|| invalid("missing MP4 mdia"))?;
    let hdlr = child_box(file, mdia, b"hdlr", "duplicate MP4 hdlr")?
        .ok_or_else(|| invalid("missing MP4 hdlr"))?;
    if !handler_is_video(file, hdlr)? {
        return Ok(None);
    }
    let mdhd = child_box(file, mdia, b"mdhd", "duplicate MP4 mdhd")?
        .ok_or_else(|| invalid("missing MP4 video mdhd"))?;
    Ok(Some((mdhd_timescale(file, mdhd)?, mdia)))
}

fn track_video_timing(
    file: &mut (impl Read + Seek),
    track: Mp4Box,
) -> io::Result<Option<VideoTrackTiming>> {
    let Some((timescale, mdia)) = track_video_header(file, track)? else {
        return Ok(None);
    };
    let has_elst = match child_box(file, track, b"edts", "duplicate MP4 edts")? {
        Some(edts) => match child_box(file, edts, b"elst", "duplicate MP4 elst")? {
            Some(elst) => fullbox_count(file, elst)? > 0,
            None => false,
        },
        None => false,
    };
    let minf = child_box(file, mdia, b"minf", "duplicate MP4 minf")?
        .ok_or_else(|| invalid("missing MP4 minf"))?;
    let stbl = child_box(file, minf, b"stbl", "duplicate MP4 stbl")?
        .ok_or_else(|| invalid("missing MP4 stbl"))?;
    let stts = child_box(file, stbl, b"stts", "duplicate MP4 stts")?
        .ok_or_else(|| invalid("missing MP4 stts"))?;
    let (stts_samples, stts_total_ticks, last_stts_delta) = stts_summary(file, stts)?;
    let has_ctts = match child_box(file, stbl, b"ctts", "duplicate MP4 ctts")? {
        Some(ctts) => fullbox_count(file, ctts)? > 0,
        None => false,
    };
    Ok(Some(VideoTrackTiming {
        timescale,
        has_ctts,
        has_elst,
        stts_samples,
        stts_total_ticks,
        last_stts_delta,
    }))
}

fn scan_moov_video<R: Read + Seek, T>(
    file: &mut R,
    bytes: u64,
    mut visit: impl FnMut(&mut R, Mp4Box) -> io::Result<Option<T>>,
) -> io::Result<T> {
    if file.seek(SeekFrom::End(0))? != bytes {
        return Err(invalid("MP4 file length changed"));
    }
    let (mut at, mut found_moov, mut video) = (0, false, None);
    while at < bytes {
        let top = read_box(file, at, bytes)?;
        if top.kind == *b"moov" {
            if found_moov {
                return Err(invalid("duplicate MP4 moov"));
            }
            found_moov = true;
            let mut child_at = top.body;
            while child_at < top.end {
                let child = read_box(file, child_at, top.end)?;
                if child.kind == *b"trak" {
                    if let Some(found) = visit(file, child)? {
                        if video.replace(found).is_some() {
                            return Err(invalid("multiple MP4 video tracks"));
                        }
                    }
                }
                child_at = child.end;
            }
        }
        at = top.end;
    }
    video.ok_or_else(|| invalid("missing MP4 video track"))
}

/// Return the sole video track's `mdhd` timescale from a retained file handle.
pub(super) fn video_timescale(file: &mut (impl Read + Seek), bytes: u64) -> io::Result<u32> {
    scan_moov_video(file, bytes, |reader, trak| {
        Ok(track_video_header(reader, trak)?.map(|(s, _)| s))
    })
}

/// Return the sole video track's container `mdhd`/`stts`/`ctts`/`elst` timing summary.
pub(super) fn video_track_timing(
    file: &mut (impl Read + Seek),
    bytes: u64,
) -> io::Result<VideoTrackTiming> {
    scan_moov_video(file, bytes, track_video_timing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::from(u32::try_from(body.len() + 8).unwrap().to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn metadata(version: u8, scale: u32) -> Vec<u8> {
        let mut body = vec![0; if version == 1 { 36 } else { 24 }];
        body[0] = version;
        let at = if version == 1 { 20 } else { 12 };
        body[at..at + 4].copy_from_slice(&scale.to_be_bytes());
        atom(b"mdhd", &body)
    }

    fn track(version: u8, scale: u32, handler: &[u8; 4]) -> Vec<u8> {
        let mut hdlr = vec![0; 24];
        hdlr[8..12].copy_from_slice(handler);
        let mut mdia = metadata(version, scale);
        mdia.extend(atom(b"hdlr", &hdlr));
        atom(b"trak", &atom(b"mdia", &mdia))
    }

    fn parse(data: Vec<u8>) -> io::Result<u32> {
        let len = data.len() as u64;
        video_timescale(&mut Cursor::new(data), len)
    }

    #[test]
    fn reads_both_mdhd_versions_and_skips_large_mdat() {
        for version in [0, 1] {
            let mut file = atom(b"mdat", &vec![7; 128 * 1024]);
            file.extend(atom(b"moov", &track(version, 90_000, b"vide")));
            assert_eq!(parse(file).unwrap(), 90_000);
        }
    }

    #[test]
    fn accepts_extended_and_parent_end_boxes() {
        let track = track(0, 1_000, b"vide");
        let mut file = Vec::from(1u32.to_be_bytes());
        file.extend_from_slice(b"moov");
        file.extend_from_slice(&(track.len() as u64 + 16).to_be_bytes());
        file.extend(track);
        assert_eq!(parse(file.clone()).unwrap(), 1_000);
        file[..4].copy_from_slice(&0u32.to_be_bytes());
        file.drain(8..16);
        assert_eq!(parse(file).unwrap(), 1_000);
    }

    #[test]
    fn rejects_missing_duplicate_zero_unknown_and_truncated_metadata() {
        assert!(parse(atom(b"moov", &[])).is_err());
        let one = track(0, 1_000, b"vide");
        let mut two = one.clone();
        two.extend(one);
        assert!(parse(atom(b"moov", &two)).is_err());
        assert!(parse(atom(b"moov", &track(0, 0, b"vide"))).is_err());
        assert!(parse(atom(b"moov", &track(2, 1_000, b"vide"))).is_err());
        let mut hdlr = vec![0; 24];
        hdlr[8..12].copy_from_slice(b"vide");
        let missing_mdhd = atom(b"mdia", &atom(b"hdlr", &hdlr));
        assert!(parse(atom(b"moov", &atom(b"trak", &missing_mdhd))).is_err());
        let mut short_mdhd = atom(b"mdhd", &[0; 4]);
        short_mdhd.extend(atom(b"hdlr", &hdlr));
        assert!(parse(atom(b"moov", &atom(b"trak", &atom(b"mdia", &short_mdhd)))).is_err());
        let mut duplicate_mdhd = metadata(0, 1_000);
        duplicate_mdhd.extend(metadata(0, 1_000));
        duplicate_mdhd.extend(atom(b"hdlr", &hdlr));
        assert!(parse(atom(
            b"moov",
            &atom(b"trak", &atom(b"mdia", &duplicate_mdhd))
        ))
        .is_err());
    }

    #[test]
    fn rejects_invalid_box_bounds_and_truncation() {
        assert!(parse(vec![0; 7]).is_err());
        let mut file = atom(b"moov", &track(0, 1_000, b"vide"));
        file[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(file).is_err());
        assert!(parse(vec![0, 0, 0, 1, b'm', b'o', b'o', b'v']).is_err());
    }
}
