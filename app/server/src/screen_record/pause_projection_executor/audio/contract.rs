//! Receipt-bound private audio leaves for one completed pause projection.

use super::{audio_error, invalid, read_nofollow, MICROPHONE_FILE, SYSTEM_FILE};
use cut_core::CutError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

const SYSTEM_TIMING_FILE: &str = crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE;
const SYSTEM_TIMING_SCHEMA: &str = "shellx-cut/system-audio-timing/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProjectionAudioLeaf {
    file_name: String,
    bytes: u64,
    sha256: String,
}

impl ProjectionAudioLeaf {
    pub(super) fn from_local(file_name: &str, path: &Path) -> Result<Self, CutError> {
        Self::from_bytes(file_name, &read_nofollow(path)?)
    }

    fn from_bytes(file_name: &str, bytes: &[u8]) -> Result<Self, CutError> {
        if bytes.is_empty() {
            return Err(invalid("private audio projection leaf is empty"));
        }
        Ok(Self {
            file_name: file_name.into(),
            bytes: u64::try_from(bytes.len())
                .map_err(|_| invalid("private audio projection leaf is too large"))?,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })
    }

    pub(super) fn file_name(&self) -> &str {
        &self.file_name
    }

    fn verify_at(&self, path: &Path) -> Result<Vec<u8>, CutError> {
        let bytes = read_nofollow(path)?;
        if u64::try_from(bytes.len()).ok() != Some(self.bytes)
            || format!("{:x}", Sha256::digest(&bytes)) != self.sha256
        {
            return Err(invalid(
                "private audio projection leaf differs from its completed receipt contract",
            ));
        }
        Ok(bytes)
    }

    #[cfg(test)]
    pub(super) fn test_from_bytes(file_name: &str, bytes: &[u8]) -> Self {
        Self::from_bytes(file_name, bytes).expect("test audio leaf is non-empty")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectionSystemTiming {
    leaf: ProjectionAudioLeaf,
    canonical_bytes: Vec<u8>,
    first_packet_offset_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProjectionAudioContract {
    microphone: Option<ProjectionAudioLeaf>,
    system: Option<ProjectionAudioLeaf>,
    system_timing: Option<ProjectionSystemTiming>,
}

impl ProjectionAudioContract {
    pub(super) fn new(
        microphone: Option<ProjectionAudioLeaf>,
        system: Option<(ProjectionAudioLeaf, Option<u64>)>,
    ) -> Result<Self, CutError> {
        if microphone
            .as_ref()
            .is_some_and(|leaf| leaf.file_name != MICROPHONE_FILE)
        {
            return Err(invalid(
                "private microphone projection leaf has an unexpected name",
            ));
        }
        let (system, system_timing) = match system {
            None => (None, None),
            Some((system, first_packet_offset_ms)) => {
                if system.file_name != SYSTEM_FILE {
                    return Err(invalid(
                        "private system projection leaf has an unexpected name",
                    ));
                }
                let timing = crate::screen_record::system_audio::SystemAudioTiming {
                    schema: SYSTEM_TIMING_SCHEMA.into(),
                    first_packet_offset_ms,
                };
                let canonical_bytes = serde_json::to_vec(&timing).map_err(|error| {
                    invalid(format!("serialize private system-audio timing: {error}"))
                })?;
                let leaf = ProjectionAudioLeaf::from_bytes(SYSTEM_TIMING_FILE, &canonical_bytes)?;
                (
                    Some(system),
                    Some(ProjectionSystemTiming {
                        leaf,
                        canonical_bytes,
                        first_packet_offset_ms,
                    }),
                )
            }
        };
        Ok(Self {
            microphone,
            system,
            system_timing,
        })
    }

    pub(super) fn microphone_file(&self) -> Option<&'static str> {
        self.microphone.as_ref().map(|_| MICROPHONE_FILE)
    }

    pub(super) fn verify_leaf(
        &self,
        path: &Path,
        expected: &ProjectionAudioLeaf,
    ) -> Result<(), CutError> {
        expected.verify_at(path).map(|_| ())
    }

    pub(crate) fn verify_published(&self, capture_dir: &Path) -> Result<(), CutError> {
        if self
            .microphone
            .as_ref()
            .is_some_and(|leaf| leaf.file_name != MICROPHONE_FILE)
            || self
                .system
                .as_ref()
                .is_some_and(|leaf| leaf.file_name != SYSTEM_FILE)
            || (self.system.is_some() != self.system_timing.is_some())
            || self.system_timing.as_ref().is_some_and(|timing| {
                timing.leaf.file_name != SYSTEM_TIMING_FILE
                    || timing.leaf.bytes
                        != u64::try_from(timing.canonical_bytes.len()).unwrap_or(u64::MAX)
                    || timing.leaf.sha256
                        != format!("{:x}", Sha256::digest(&timing.canonical_bytes))
            })
        {
            return Err(invalid(
                "private audio receipt contract has an invalid selected-leaf shape",
            ));
        }
        if let Some(leaf) = &self.microphone {
            leaf.verify_at(&capture_dir.join(leaf.file_name()))?;
        }
        if let Some(leaf) = &self.system {
            leaf.verify_at(&capture_dir.join(leaf.file_name()))?;
        }
        if let Some(timing) = &self.system_timing {
            let bytes = timing
                .leaf
                .verify_at(&capture_dir.join(timing.leaf.file_name()))?;
            if bytes != timing.canonical_bytes {
                return Err(invalid(
                    "private system-audio timing bytes differ from the completed receipt contract",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn publish_system_timing(&self, capture_dir: &Path) -> Result<(), CutError> {
        let Some(timing) = &self.system_timing else {
            return Ok(());
        };
        let path = capture_dir.join(timing.leaf.file_name());
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                let bytes = timing.leaf.verify_at(&path)?;
                if bytes == timing.canonical_bytes {
                    Ok(())
                } else {
                    Err(invalid(
                        "existing private system-audio timing is not the exact canonical receipt bytes",
                    ))
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let (stage, mut file) =
                    record_recovery::create_staging_file(capture_dir, "pause-system-audio-timing")
                        .map_err(audio_error)?;
                file.write_all(&timing.canonical_bytes)
                    .map_err(audio_error)?;
                file.sync_all().map_err(audio_error)?;
                drop(file);
                record_recovery::publish_new_synced(&stage, &path).map_err(audio_error)
            }
            Err(error) => Err(audio_error(error)),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_with_selected_audio(
        microphone: Option<&[u8]>,
        system: Option<&[u8]>,
        first_packet_offset_ms: Option<u64>,
    ) -> Self {
        Self::new(
            microphone.map(|bytes| ProjectionAudioLeaf::test_from_bytes(MICROPHONE_FILE, bytes)),
            system.map(|bytes| {
                (
                    ProjectionAudioLeaf::test_from_bytes(SYSTEM_FILE, bytes),
                    first_packet_offset_ms,
                )
            }),
        )
        .expect("test audio contract is valid")
    }

    #[cfg(test)]
    pub(crate) fn test_system_timing_bytes(&self) -> Option<&[u8]> {
        self.system_timing
            .as_ref()
            .map(|timing| timing.canonical_bytes.as_slice())
    }
}
