//! Make the sound of an interrupted recording playable again.
//!
//! The microphone and system-audio writers stream PCM into hidden
//! `.mic-wav-*.part` / `.system-wav-*.part` files and write the final RIFF
//! sizes only when the recording finishes. After a crash or a forced quit the
//! samples are all there but the header still says the data is empty. A copy
//! with the sizes taken from the file length plays normally. The original
//! `.part` file is never changed.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use record_recovery::{is_plain_dir, is_plain_regular_file};

/// The newest plain `.{prefix}-*.part` file directly inside `dir`.
pub(crate) fn find_part(dir: &Path, prefix: &str) -> Option<PathBuf> {
    if !is_plain_dir(dir).unwrap_or(false) {
        return None;
    }
    let wanted = format!(".{prefix}-");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&wanted) && name.ends_with(".part"))
        })
        .map(|entry| entry.path())
        .filter(|path| is_plain_regular_file(path).unwrap_or(false))
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        })
}

/// Location of the PCM data in a RIFF/WAVE header: offset of the `data`
/// chunk's size field, offset of the first sample, and the frame size.
struct WavLayout {
    data_size_offset: u64,
    data_offset: u64,
    block_align: u64,
}

fn layout(header: &[u8]) -> Option<WavLayout> {
    if header.len() < 12 || &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    let mut position = 12usize;
    let mut block_align = None;
    while position + 8 <= header.len() {
        let id = &header[position..position + 4];
        let size = u32::from_le_bytes(header[position + 4..position + 8].try_into().ok()?) as usize;
        if id == b"fmt " && position + 8 + 14 <= header.len() {
            let align = u16::from_le_bytes(
                header[position + 8 + 12..position + 8 + 14]
                    .try_into()
                    .ok()?,
            );
            block_align = Some(u64::from(align.max(1)));
        }
        if id == b"data" {
            return Some(WavLayout {
                data_size_offset: (position + 4) as u64,
                data_offset: (position + 8) as u64,
                block_align: block_align?,
            });
        }
        position = position.checked_add(8)?.checked_add(size + (size & 1))?;
    }
    None
}

/// What became of one unfinished sound file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Repair {
    /// A playable copy was saved at the destination; whole frames kept.
    Saved(u64),
    /// Not a WAV this recorder wrote, or no samples in it.
    Empty,
    /// The part, or the destination name, is a link or not an ordinary file.
    /// Nothing was read through it or written through it.
    Refused(String),
}

/// Copy `part` to `dest` with RIFF and data sizes set from the real length.
///
/// The part is opened only if it is an ordinary single-name file and without
/// following a link (an interrupted capture can arrive inside a received
/// project, and a planted link must never pull an outside file in). The copy
/// is written inside a private folder this process creates and then renamed
/// to `dest`; `dest` itself is never opened.
pub(crate) fn repair_wav_copy(part: &Path, dest: &Path) -> std::io::Result<Repair> {
    let shown = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let Ok(mut source) = cut_core::store::open_plain_file(part) else {
        return Ok(Repair::Refused(format!(
            "{} is a link or not an ordinary file; it was not used",
            shown(part)
        )));
    };
    let length = source.metadata()?.len();
    let mut header = vec![0u8; length.min(4096) as usize];
    source.read_exact(&mut header)?;
    let Some(layout) = layout(&header) else {
        return Ok(Repair::Empty);
    };
    let data_bytes = length.saturating_sub(layout.data_offset);
    let data_bytes =
        (data_bytes / layout.block_align * layout.block_align).min(u64::from(u32::MAX) - 64);
    if data_bytes == 0 {
        return Ok(Repair::Empty);
    }
    let folder = dest.parent().unwrap_or(Path::new("."));
    let run = super::private_output::RunDir::create(folder)?;
    let staging = run.file("repaired.wav");
    {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        source.seek(SeekFrom::Start(0))?;
        std::io::copy(
            &mut (&mut source).take(layout.data_offset + data_bytes),
            &mut out,
        )?;
        let riff_size = (layout.data_offset + data_bytes - 8) as u32;
        out.seek(SeekFrom::Start(4))?;
        out.write_all(&riff_size.to_le_bytes())?;
        out.seek(SeekFrom::Start(layout.data_size_offset))?;
        out.write_all(&(data_bytes as u32).to_le_bytes())?;
        out.sync_all()?;
    }
    match super::private_output::publish(&staging, dest) {
        Ok(()) => Ok(Repair::Saved(data_bytes / layout.block_align)),
        Err(refused) => Ok(Repair::Refused(refused.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header exactly as a streaming writer leaves it before finalization:
    /// RIFF size and data size are still zero.
    fn unfinished_wav(samples: &[i16], channels: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&(48_000u32 * 2 * u32::from(channels)).to_le_bytes());
        bytes.extend_from_slice(&(2 * channels).to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn interrupted_wav_becomes_a_readable_copy_and_the_part_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join(".mic-wav-abc.part");
        let samples: Vec<i16> = (0..4_801).map(|i| (i % 100) as i16).collect();
        let original = unfinished_wav(&samples, 2);
        std::fs::write(&part, &original).unwrap();
        assert_eq!(find_part(dir.path(), "mic-wav"), Some(part.clone()));
        let dest = dir.path().join("mic.recovered.wav");
        // 4_801 samples over 2 channels: one dangling sample is dropped.
        assert_eq!(repair_wav_copy(&part, &dest).unwrap(), Repair::Saved(2_400));
        assert_eq!(std::fs::read(&part).unwrap(), original);
        let mut reader = hound::WavReader::open(&dest).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.duration(), 2_400);
        let read: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(read.as_slice(), &samples[..4_800]);
    }

    #[test]
    fn empty_or_foreign_files_are_not_offered_as_sound() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join(".system-wav-x.part");
        std::fs::write(&empty, unfinished_wav(&[], 2)).unwrap();
        assert_eq!(
            repair_wav_copy(&empty, &dir.path().join("a.wav")).unwrap(),
            Repair::Empty
        );
        let foreign = dir.path().join(".system-wav-y.part");
        std::fs::write(&foreign, b"not a wav file at all").unwrap();
        assert_eq!(
            repair_wav_copy(&foreign, &dir.path().join("b.wav")).unwrap(),
            Repair::Empty
        );
        assert!(!dir.path().join("a.wav").exists());
    }

    fn outside_sentinel() -> (tempfile::TempDir, PathBuf, Vec<u8>) {
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("private.wav");
        let bytes = unfinished_wav(&[1, 2, 3, 4, 5, 6, 7, 8], 2);
        std::fs::write(&sentinel, &bytes).unwrap();
        (outside, sentinel, bytes)
    }

    #[cfg(unix)]
    #[test]
    fn a_part_that_is_a_link_is_not_read_into_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let (_outside, sentinel, bytes) = outside_sentinel();
        let part = dir.path().join(".mic-wav-link.part");
        std::os::unix::fs::symlink(&sentinel, &part).unwrap();
        let dest = dir.path().join("mic.recovered.wav");
        let outcome = repair_wav_copy(&part, &dest).unwrap();
        assert!(
            matches!(&outcome, Repair::Refused(reason) if reason.contains("not used")),
            "{outcome:?}"
        );
        assert!(!dest.exists());
        let hard = dir.path().join(".mic-wav-hard.part");
        std::fs::hard_link(&sentinel, &hard).unwrap();
        assert!(matches!(
            repair_wav_copy(&hard, &dest).unwrap(),
            Repair::Refused(_)
        ));
        assert!(!dest.exists());
        assert_eq!(std::fs::read(&sentinel).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_planted_at_the_copy_or_staging_name_is_never_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let (_outside, sentinel, bytes) = outside_sentinel();
        let part = dir.path().join(".mic-wav-abc.part");
        std::fs::write(&part, unfinished_wav(&[9; 960], 2)).unwrap();
        // The earlier staging name and the final name both point outside.
        let dest = dir.path().join("mic.recovered.wav");
        std::os::unix::fs::symlink(&sentinel, dir.path().join("mic.recovered.wav.repairing"))
            .unwrap();
        std::os::unix::fs::symlink(&sentinel, &dest).unwrap();
        let outcome = repair_wav_copy(&part, &dest).unwrap();
        assert!(
            matches!(&outcome, Repair::Refused(reason) if reason.contains("left untouched")),
            "{outcome:?}"
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), bytes);
        // No private folder is left behind.
        assert!(std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .all(|entry| !entry.file_name().to_string_lossy().starts_with(".recover-")));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs the Windows symbolic-link privilege or Developer Mode; run by the recovery-link-windows rig"]
    fn windows_symlink_recovery_audio_never_reads_or_writes_through_links() {
        let dir = tempfile::tempdir().unwrap();
        let (_outside, sentinel, bytes) = outside_sentinel();
        let part = dir.path().join(".mic-wav-link.part");
        std::os::windows::fs::symlink_file(&sentinel, &part).expect("symlink privilege");
        let dest = dir.path().join("mic.recovered.wav");
        assert!(matches!(
            repair_wav_copy(&part, &dest).unwrap(),
            Repair::Refused(_)
        ));
        let real = dir.path().join(".mic-wav-real.part");
        std::fs::write(&real, unfinished_wav(&[9; 960], 2)).unwrap();
        std::os::windows::fs::symlink_file(&sentinel, &dest).expect("symlink privilege");
        assert!(matches!(
            repair_wav_copy(&real, &dest).unwrap(),
            Repair::Refused(_)
        ));
        assert_eq!(std::fs::read(&sentinel).unwrap(), bytes);
    }
}
