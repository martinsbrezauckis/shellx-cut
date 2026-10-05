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

fn track_video_timescale(file: &mut (impl Read + Seek), track: Mp4Box) -> io::Result<Option<u32>> {
    let mut mdia = None;
    let mut at = track.body;
    while at < track.end {
        let child = read_box(file, at, track.end)?;
        if child.kind == *b"mdia" && mdia.replace(child).is_some() {
            return Err(invalid("duplicate MP4 mdia"));
        }
        at = child.end;
    }
    let mdia = mdia.ok_or_else(|| invalid("missing MP4 mdia"))?;
    let mut hdlr = None;
    let mut mdhd = None;
    let mut at = mdia.body;
    while at < mdia.end {
        let child = read_box(file, at, mdia.end)?;
        match &child.kind {
            b"hdlr" if hdlr.replace(child).is_some() => {
                return Err(invalid("duplicate MP4 hdlr"));
            }
            b"mdhd" if mdhd.replace(child).is_some() => {
                return Err(invalid("duplicate MP4 mdhd"));
            }
            _ => {}
        }
        at = child.end;
    }
    let hdlr = hdlr.ok_or_else(|| invalid("missing MP4 hdlr"))?;
    if !handler_is_video(file, hdlr)? {
        return Ok(None);
    }
    let mdhd = mdhd.ok_or_else(|| invalid("missing MP4 video mdhd"))?;
    Ok(Some(mdhd_timescale(file, mdhd)?))
}

/// Return the sole video track's `mdhd` timescale from a retained file handle.
pub(super) fn video_timescale(file: &mut (impl Read + Seek), bytes: u64) -> io::Result<u32> {
    if file.seek(SeekFrom::End(0))? != bytes {
        return Err(invalid("MP4 file length changed"));
    }
    let mut at = 0;
    let mut found_moov = false;
    let mut video = None;
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
                    if let Some(scale) = track_video_timescale(file, child)? {
                        if video.replace(scale).is_some() {
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
