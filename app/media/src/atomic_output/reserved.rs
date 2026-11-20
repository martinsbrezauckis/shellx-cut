//! The staged output's identity and the output-folder permission rule.

use std::fs;
use std::path::Path;

/// Everyone can write (other-write) and nothing stops them replacing each
/// other's files (no sticky bit). Group-write alone is not refused.
#[cfg(unix)]
pub(super) fn everyone_can_replace_files(mode: u32) -> bool {
    mode & 0o002 != 0 && mode & 0o1000 == 0
}

/// The identity of the reserved staging file (device, inode, owner), so a file a
/// group member swapped in after the reservation is recognised and never
/// published. Unix only; elsewhere there is nothing to compare.
pub(super) struct ReservedFile {
    #[cfg(unix)]
    identity: Option<(u64, u64, u32)>,
}

impl ReservedFile {
    #[cfg(unix)]
    pub(super) fn of(path: &Path) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            identity: fs::symlink_metadata(path)
                .ok()
                .map(|meta| (meta.dev(), meta.ino(), meta.uid())),
        }
    }

    #[cfg(not(unix))]
    pub(super) fn of(_path: &Path) -> Self {
        Self {}
    }

    #[cfg(unix)]
    pub(super) fn still_ours(&self, now: &fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        self.identity == Some((now.dev(), now.ino(), now.uid()))
            && now.nlink() == 1
            && now.mode() & 0o077 == 0
    }

    #[cfg(not(unix))]
    pub(super) fn still_ours(&self, _now: &fs::Metadata) -> bool {
        true
    }
}
