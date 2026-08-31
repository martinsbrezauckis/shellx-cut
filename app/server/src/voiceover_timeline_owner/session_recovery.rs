//! Hash-verified recovery for an already-private voiceover WAV leaf.

use std::fs;
use std::io::Read;
use std::path::Path;

use cut_core::{error_codes, CutError};
use record_capture::VoiceoverArtifact;
use record_recovery::CaptureRoot;
use sha2::{Digest, Sha256};

use super::journal::{
    VoiceoverSealedArtifact, VoiceoverTakeJournalEntry, VoiceoverTakePhase, VoiceoverTerminalResult,
};
use super::journal_io::VoiceoverTakeJournalFile;
use super::session::{VoiceoverRecoveredTake, WAV_LEAF};

pub(super) fn recover(
    journal: &mut VoiceoverTakeJournalFile,
    root: &CaptureRoot,
    take_id: &str,
) -> Result<VoiceoverRecoveredTake, CutError> {
    let (sealed, terminal) = {
        let state = journal.state();
        (state.sealed.clone(), state.terminal)
    };
    if let (Some(artifact), Some(result)) = (sealed.as_ref(), terminal) {
        if !matches!(
            result,
            VoiceoverTerminalResult::Saved | VoiceoverTerminalResult::DeviceLostSavedPrefix
        ) {
            return Err(io_error(
                "recovered sealed voiceover has a non-placeable terminal result",
            ));
        }
        verify_sealed(root, take_id, artifact)?;
        return Ok(VoiceoverRecoveredTake::CommitEligible(artifact.clone()));
    }
    if let Some(artifact) = sealed.as_ref() {
        verify_sealed(root, take_id, artifact)?;
        journal.append(VoiceoverTakeJournalEntry::Terminal {
            result: terminal_for(artifact),
        })?;
        return Ok(VoiceoverRecoveredTake::CommitEligible(artifact.clone()));
    }
    if let Some(result) = terminal {
        return Ok(VoiceoverRecoveredTake::Terminal(result));
    }
    let bridge_epoch = journal.state().bridge_epoch;
    journal.append(VoiceoverTakeJournalEntry::Phase {
        phase: VoiceoverTakePhase::Finalizing,
        bridge_epoch,
    })?;
    journal.append(VoiceoverTakeJournalEntry::Terminal {
        result: VoiceoverTerminalResult::Interrupted,
    })?;
    Ok(VoiceoverRecoveredTake::Interrupted)
}

pub(super) fn artifact_facts(
    artifact: &VoiceoverArtifact,
    device_lost_after_prefix: bool,
) -> Result<VoiceoverSealedArtifact, CutError> {
    let facts = VoiceoverSealedArtifact {
        sha256: artifact.sha256.clone(),
        bytes: artifact.bytes,
        duration_ms: artifact.duration_ms,
        sample_rate_hz: artifact.sample_rate_hz,
        channels: artifact.channels,
        device_lost_after_prefix,
    };
    facts.validate()?;
    Ok(facts)
}

fn terminal_for(artifact: &VoiceoverSealedArtifact) -> VoiceoverTerminalResult {
    if artifact.device_lost_after_prefix {
        VoiceoverTerminalResult::DeviceLostSavedPrefix
    } else {
        VoiceoverTerminalResult::Saved
    }
}

fn verify_sealed(
    root: &CaptureRoot,
    take_id: &str,
    artifact: &VoiceoverSealedArtifact,
) -> Result<(), CutError> {
    let path = root
        .capture_file(take_id, WAV_LEAF)
        .map_err(|_| io_error("derive the recovered voiceover WAV leaf"))?;
    verify_artifact(&path, artifact)
}

fn verify_artifact(path: &Path, expected: &VoiceoverSealedArtifact) -> Result<(), CutError> {
    let mut file = open_artifact_nofollow(path)?;
    let metadata = file
        .metadata()
        .map_err(|_| io_error("inspect a recovered voiceover artifact"))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || is_reparse(&metadata)
        || metadata.len() != expected.bytes
    {
        return Err(io_error(
            "recovered voiceover artifact is not its sealed local file",
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| io_error("hash a recovered voiceover artifact"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    (format!("{:x}", digest.finalize()) == expected.sha256)
        .then_some(())
        .ok_or_else(|| io_error("recovered voiceover artifact hash does not match the journal"))
}

fn open_artifact_nofollow(path: &Path) -> Result<fs::File, CutError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
        .open(path)
        .map_err(|_| io_error("open a recovered voiceover artifact without following links"))
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

pub(super) fn reject_existing_wav(path: &Path) -> Result<(), CutError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(io_error("private voiceover WAV leaf already exists")),
        Err(_) => Err(io_error("inspect private voiceover WAV leaf")),
    }
}

pub(super) fn io_error(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::IO,
        "voiceover private lifecycle I/O failed",
        detail.into(),
    )
}
