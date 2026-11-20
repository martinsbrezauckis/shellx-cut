//! zip_member.rs — accept a downloaded zip only when every entry is safe and
//! the programs Cut will take from it are there, exactly as pinned.
//!
//! The archive bytes were already checked against a pinned SHA-256, so this
//! is defence in depth for the day a pin changes: it reads the central
//! directory and refuses any entry with an absolute path, a `..` component, a
//! backslash or drive prefix, a link, encryption, an unknown compression
//! method or zip64 records, and it requires each pinned member once, as a
//! regular file of its pinned unpacked size, with a matching local header.
//! Other entries (documentation, ffplay) may exist; they are never unpacked.

use cut_media::tool_setup::PinnedMember;

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
const UNIX_HOST: u16 = 3;
const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A path inside an archive that stays inside the unpack folder.
fn safe_entry_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.contains(':')
        && name.split('/').all(|part| part != "..")
}

/// Check that `bytes` is a zip whose entries are all safe and which holds
/// each of `members` once as a regular file of its pinned size.
pub(crate) fn check_zip(bytes: &[u8], members: &[PinnedMember]) -> Result<(), String> {
    let truncated = || "the archive is truncated or not a zip".to_string();
    let lowest = bytes.len().saturating_sub(22 + u16::MAX as usize);
    let eocd = (lowest..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&at| u32_at(bytes, at) == Some(EOCD_SIG))
        .ok_or_else(truncated)?;
    let disk = u16_at(bytes, eocd + 4).ok_or_else(truncated)?;
    let cd_disk = u16_at(bytes, eocd + 6).ok_or_else(truncated)?;
    let on_disk = u16_at(bytes, eocd + 8).ok_or_else(truncated)?;
    let total = u16_at(bytes, eocd + 10).ok_or_else(truncated)?;
    let cd_size = u32_at(bytes, eocd + 12).ok_or_else(truncated)?;
    let cd_offset = u32_at(bytes, eocd + 16).ok_or_else(truncated)?;
    let comment = u16_at(bytes, eocd + 20).ok_or_else(truncated)? as usize;
    if disk != 0 || cd_disk != 0 || on_disk != total {
        return Err("the archive is split across disks".into());
    }
    if total == u16::MAX || cd_size == u32::MAX || cd_offset == u32::MAX {
        return Err("the archive uses zip64 records, which this download never has".into());
    }
    if eocd + 22 + comment != bytes.len() {
        return Err("the archive has data after its end record".into());
    }
    if cd_offset as usize + cd_size as usize != eocd {
        return Err("the archive directory is not where its end record says".into());
    }
    let mut found = vec![0usize; members.len()];
    let mut c = cd_offset as usize;
    for _ in 0..total {
        if u32_at(bytes, c) != Some(CENTRAL_SIG) {
            return Err(truncated());
        }
        let made_by = u16_at(bytes, c + 4).ok_or_else(truncated)?;
        let flags = u16_at(bytes, c + 8).ok_or_else(truncated)?;
        let method = u16_at(bytes, c + 10).ok_or_else(truncated)?;
        let unpacked = u32_at(bytes, c + 24).ok_or_else(truncated)? as u64;
        let name_len = u16_at(bytes, c + 28).ok_or_else(truncated)? as usize;
        let extra_len = u16_at(bytes, c + 30).ok_or_else(truncated)? as usize;
        let comment_len = u16_at(bytes, c + 32).ok_or_else(truncated)? as usize;
        let external = u32_at(bytes, c + 38).ok_or_else(truncated)?;
        let local = u32_at(bytes, c + 42).ok_or_else(truncated)? as usize;
        let raw = bytes.get(c + 46..c + 46 + name_len).ok_or_else(truncated)?;
        let name = std::str::from_utf8(raw).map_err(|_| "an archive entry name is not UTF-8")?;
        if !safe_entry_name(name) {
            return Err(format!(
                "the archive entry '{name}' would leave the unpack folder"
            ));
        }
        if flags & 0x1 != 0 {
            return Err(format!("the archive entry '{name}' is encrypted"));
        }
        if method != 0 && method != 8 {
            return Err(format!(
                "the archive entry '{name}' uses compression method {method}"
            ));
        }
        let is_dir = name.ends_with('/');
        let kind = (external >> 16) & S_IFMT;
        if made_by >> 8 == UNIX_HOST {
            if kind == S_IFLNK {
                return Err(format!("the archive entry '{name}' is a link"));
            }
            if kind != 0 && kind != S_IFREG && kind != S_IFDIR {
                return Err(format!(
                    "the archive entry '{name}' is not a file or folder"
                ));
            }
        }
        if let Some(index) = members.iter().position(|m| m.path == name) {
            let member = &members[index];
            if is_dir || kind == S_IFDIR || external & 0x10 != 0 {
                return Err(format!(
                    "the archive entry '{name}' is a folder, not a program"
                ));
            }
            if unpacked != member.size {
                return Err(format!(
                    "'{name}' unpacks to {unpacked} bytes; the pinned program is {}",
                    member.size
                ));
            }
            if u32_at(bytes, local) != Some(LOCAL_SIG) {
                return Err(truncated());
            }
            let local_len = u16_at(bytes, local + 26).ok_or_else(truncated)? as usize;
            let local_name = bytes
                .get(local + 30..local + 30 + local_len)
                .ok_or_else(truncated)?;
            if local_name != raw {
                return Err("the archive's local and central records disagree on a name".into());
            }
            found[index] += 1;
        }
        c += 46 + name_len + extra_len + comment_len;
    }
    if c != eocd {
        return Err("the archive directory does not end where its end record says".into());
    }
    for (member, count) in members.iter().zip(found) {
        if count != 1 {
            return Err(format!(
                "the archive holds '{}' {count} times; the pinned release has it once",
                member.path
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A stored (uncompressed) zip with the given (name, data, unix mode)
    /// entries. CRCs are left zero: the checker does not read them.
    pub(crate) fn zip(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, mode) in entries {
            let offset = out.len() as u32;
            out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            central.extend_from_slice(&((UNIX_HOST << 8) | 20).to_le_bytes());
            // version needed, flags, method, time, date, then a zero CRC.
            central.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&(mode << 16).to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    const FFMPEG: PinnedMember = PinnedMember {
        path: "pkg/bin/ffmpeg",
        install_as: "ffmpeg",
        size: 6,
        sha256: "",
    };

    #[test]
    fn the_pinned_programs_among_safe_entries_are_accepted() {
        let bytes = zip(&[
            ("pkg/", b"", 0o040_755),
            ("pkg/bin/ffmpeg", b"binary", 0o100_755),
            ("pkg/doc/readme.txt", b"text", 0o100_644),
        ]);
        check_zip(&bytes, &[FFMPEG]).unwrap();
    }

    #[test]
    fn a_missing_or_doubled_program_is_refused() {
        let bytes = zip(&[("pkg/bin/other", b"binary", 0o100_755)]);
        assert!(check_zip(&bytes, &[FFMPEG])
            .unwrap_err()
            .contains("0 times"));
        let bytes = zip(&[
            ("pkg/bin/ffmpeg", b"binary", 0o100_755),
            ("pkg/bin/ffmpeg", b"binary", 0o100_755),
        ]);
        assert!(check_zip(&bytes, &[FFMPEG])
            .unwrap_err()
            .contains("2 times"));
    }

    #[test]
    fn a_link_anywhere_is_refused() {
        let bytes = zip(&[
            ("pkg/bin/ffmpeg", b"binary", 0o100_755),
            ("pkg/bin/ffplay", b"/usr/bin/true", 0o120_777),
        ]);
        assert!(check_zip(&bytes, &[FFMPEG])
            .unwrap_err()
            .contains("is a link"));
        let bytes = zip(&[("pkg/bin/ffmpeg", b"/bin/sh", 0o120_777)]);
        assert!(check_zip(&bytes, &[FFMPEG])
            .unwrap_err()
            .contains("is a link"));
    }

    #[test]
    fn path_tricks_are_refused_even_for_entries_never_unpacked() {
        for name in [
            "../evil",
            "pkg/../../evil",
            "/etc/evil",
            "pkg\\evil",
            "C:evil",
        ] {
            let bytes = zip(&[
                ("pkg/bin/ffmpeg", b"binary", 0o100_755),
                (name, b"x", 0o100_644),
            ]);
            let error = check_zip(&bytes, &[FFMPEG]).unwrap_err();
            assert!(
                error.contains("would leave the unpack folder"),
                "{name}: {error}"
            );
        }
    }

    #[test]
    fn wrong_size_truncation_trailing_data_and_encryption_are_refused() {
        let bytes = zip(&[("pkg/bin/ffmpeg", b"binary!", 0o100_755)]);
        assert!(check_zip(&bytes, &[FFMPEG])
            .unwrap_err()
            .contains("pinned program is 6"));
        let good = zip(&[("pkg/bin/ffmpeg", b"binary", 0o100_755)]);
        assert!(check_zip(&good[..good.len() - 3], &[FFMPEG]).is_err());
        let mut trailing = good.clone();
        trailing.extend_from_slice(b"junk");
        assert!(check_zip(&trailing, &[FFMPEG]).is_err());
        assert!(check_zip(b"not a zip at all", &[FFMPEG]).is_err());
        let mut encrypted = good;
        let central = encrypted.len() - 22 - (46 + "pkg/bin/ffmpeg".len());
        encrypted[central + 8] |= 1;
        assert!(check_zip(&encrypted, &[FFMPEG])
            .unwrap_err()
            .contains("encrypted"));
    }
}
