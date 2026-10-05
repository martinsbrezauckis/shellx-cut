//! Handle-anchored Windows camera output reservation and byte-stream bridge.

use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

use record_core::Result;
use windows::Win32::Media::MediaFoundation::{IMFByteStream, MFCreateMFByteStreamOnStream};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileDispositionInfo, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, CREATE_NEW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, OPEN_EXISTING,
};
use windows::Win32::System::Com::IStream;

use super::finalization_error;
use super::windows_stream::handle_backed_stream;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    volume: u32,
    index: u64,
    attributes: u32,
}

struct OpenDirectory {
    handle: OwnedHandle,
    identity: FileIdentity,
    physical_path: PathBuf,
}

/// An exact created leaf plus retained root and parent directory handles. No
/// finalization operation re-resolves a caller path after this reservation.
pub(crate) struct WindowsNoReplaceCameraStage {
    root: OpenDirectory,
    camera: OpenDirectory,
    file: File,
    leaf: FileIdentity,
    pub(super) artifact_id: String,
    pub(super) video: String,
    finalized: bool,
}

pub(crate) fn reserve_windows_no_replace_output(
    capture_directory: &Path,
    artifact_id: String,
    video: String,
) -> Result<(IMFByteStream, IStream, WindowsNoReplaceCameraStage)> {
    if !video.starts_with("camera/") || !video.ends_with(".mp4") || video.contains('\\') {
        return Err(finalization_error(
            "Windows camera output is outside the private camera route",
            "the Capture Engine reservation must name one camera/*.mp4 artifact",
        ));
    }
    let root = open_directory(capture_directory, "open Windows capture root")?;
    let camera_path = root.physical_path.join("camera");
    if let Err(error) = std::fs::create_dir(&camera_path) {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(finalization_error(
                "create Windows camera output directory",
                &error.to_string(),
            ));
        }
    }
    let camera = open_directory(&camera_path, "open Windows camera output directory")?;
    let leaf_name = video.strip_prefix("camera/").expect("validated route");
    let output_path = camera.physical_path.join(leaf_name);
    let raw = open_new_leaf(&output_path)?;
    // SAFETY: `open_new_leaf` returns an owned CREATE_NEW handle. Ownership is
    // transferred exactly once to `File`, whose duplicate stays in the stage.
    let writer = unsafe { File::from_raw_handle(raw.0 as *mut _) };
    let leaf = identity_for_file(&writer, "inspect reserved Windows camera leaf")?;
    verify_leaf(&leaf)?;
    let stage_file = writer.try_clone().map_err(|error| {
        finalization_error("duplicate reserved Windows camera leaf", &error.to_string())
    })?;
    let stage = WindowsNoReplaceCameraStage {
        root,
        camera,
        file: stage_file,
        leaf,
        artifact_id,
        video,
        finalized: false,
    };
    let stream = handle_backed_stream(writer).map_err(|error| {
        finalization_error("create anchored Windows camera writer", &error.to_string())
    })?;
    // SAFETY: this COM stream owns the writer handle and remains retained beside
    // the IMF byte stream for the entire Capture Engine record operation.
    let output = unsafe { MFCreateMFByteStreamOnStream(&stream) }.map_err(|error| {
        finalization_error(
            "create handle-backed camera byte stream",
            &error.to_string(),
        )
    })?;
    Ok((output, stream, stage))
}

impl WindowsNoReplaceCameraStage {
    pub(super) fn verify_anchored_handles(&self) -> Result<()> {
        verify_directory(&self.root, "revalidate Windows capture root")?;
        verify_directory(&self.camera, "revalidate Windows camera output directory")?;
        let current = identity_for_file(&self.file, "revalidate Windows camera leaf")?;
        if current != self.leaf {
            return Err(finalization_error(
                "Windows camera leaf identity changed",
                "the finalizer must retain the exact CREATE_NEW leaf handle",
            ));
        }
        verify_leaf(&current)
    }

    pub(super) fn duplicate_file(&self) -> Result<File> {
        self.verify_anchored_handles()?;
        self.file.try_clone().map_err(|error| {
            finalization_error("duplicate anchored Windows camera leaf", &error.to_string())
        })
    }

    pub(super) fn source_reader_stream(&self) -> Result<(IMFByteStream, IStream)> {
        let mut file = self.duplicate_file()?;
        file.seek(SeekFrom::Start(0)).map_err(|error| {
            finalization_error("rewind anchored Windows camera reader", &error.to_string())
        })?;
        let stream = handle_backed_stream(file).map_err(|error| {
            finalization_error("create anchored Windows camera reader", &error.to_string())
        })?;
        // SAFETY: the stream wraps a duplicate of the retained CREATE_NEW leaf,
        // so Source Reader never follows a mutable parent/leaf path.
        let byte_stream = unsafe { MFCreateMFByteStreamOnStream(&stream) }.map_err(|error| {
            finalization_error("create anchored Windows camera reader", &error.to_string())
        })?;
        Ok((byte_stream, stream))
    }

    pub(super) fn make_read_only(&mut self) -> Result<()> {
        self.verify_anchored_handles()?;
        let metadata = self.file.metadata().map_err(|error| {
            finalization_error("inspect anchored Windows camera leaf", &error.to_string())
        })?;
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        self.file.set_permissions(permissions).map_err(|error| {
            finalization_error("protect anchored Windows camera leaf", &error.to_string())
        })?;
        let current = identity_for_file(&self.file, "reinspect anchored Windows camera leaf")?;
        if !is_owned_read_only_transition(self.leaf, current) {
            return Err(finalization_error(
                "Windows camera leaf identity changed during protection",
                "the exact leaf must gain read-only protection without other identity or attribute changes",
            ));
        }
        self.leaf = current;
        Ok(())
    }

    pub(super) fn mark_finalized(&mut self) {
        self.finalized = true;
    }

    fn discard(&self) {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: this marks only the retained CREATE_NEW leaf for deletion on
        // close; no pathname is resolved and no pre-existing leaf can be hit.
        let _ = unsafe {
            SetFileInformationByHandle(
                raw_handle(&self.file),
                FileDispositionInfo,
                &disposition as *const _ as _,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
    }
}

impl Drop for WindowsNoReplaceCameraStage {
    fn drop(&mut self) {
        if !self.finalized {
            self.discard();
        }
    }
}

fn open_directory(path: &Path, stage: &str) -> Result<OpenDirectory> {
    let raw = unsafe {
        CreateFileW(
            &windows::core::HSTRING::from(path.as_os_str().to_string_lossy().as_ref()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|error| finalization_error(stage, &error.to_string()))?;
    // SAFETY: successful CreateFileW returns one owned handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.0 as *mut _) };
    let identity = identity_for_handle(raw_handle(&handle), stage)?;
    verify_directory_identity(identity, stage)?;
    let physical_path = final_path(raw_handle(&handle), stage)?;
    Ok(OpenDirectory {
        handle,
        identity,
        physical_path,
    })
}

fn open_new_leaf(path: &Path) -> Result<windows::Win32::Foundation::HANDLE> {
    unsafe {
        CreateFileW(
            &windows::core::HSTRING::from(path.as_os_str().to_string_lossy().as_ref()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ,
            None,
            CREATE_NEW,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|error| {
        finalization_error("reserve no-replace Windows camera leaf", &error.to_string())
    })
}

fn verify_directory(directory: &OpenDirectory, stage: &str) -> Result<()> {
    let current = identity_for_handle(raw_handle(&directory.handle), stage)?;
    if current != directory.identity {
        return Err(finalization_error(
            stage,
            "opened directory handle identity changed",
        ));
    }
    verify_directory_identity(current, stage)
}

fn verify_directory_identity(identity: FileIdentity, stage: &str) -> Result<()> {
    if identity.attributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0
        || identity.attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
    {
        return Err(finalization_error(
            stage,
            "capture directory is redirected or not a directory",
        ));
    }
    Ok(())
}

fn verify_leaf(identity: &FileIdentity) -> Result<()> {
    if identity.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
        || identity.attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
    {
        return Err(finalization_error(
            "inspect reserved Windows camera leaf",
            "output leaf is redirected",
        ));
    }
    Ok(())
}

fn is_owned_read_only_transition(before: FileIdentity, after: FileIdentity) -> bool {
    let readonly = FILE_ATTRIBUTE_READONLY.0;
    let normal = FILE_ATTRIBUTE_NORMAL.0;
    let forbidden = FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0;
    before.volume == after.volume
        && before.index == after.index
        && before.attributes & forbidden == 0
        && after.attributes & forbidden == 0
        && before.attributes & readonly == 0
        && after.attributes & readonly != 0
        && before.attributes & !(readonly | normal) == after.attributes & !(readonly | normal)
        // NORMAL is a standalone marker and Windows removes it when READONLY
        // is added. Never accept a new NORMAL bit or retain it alongside READONLY.
        && after.attributes & normal == 0
}

fn identity_for_file(file: &File, stage: &str) -> Result<FileIdentity> {
    identity_for_handle(raw_handle(file), stage)
}

fn identity_for_handle(
    handle: windows::Win32::Foundation::HANDLE,
    stage: &str,
) -> Result<FileIdentity> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut info) }
        .map_err(|error| finalization_error(stage, &error.to_string()))?;
    Ok(FileIdentity {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        attributes: info.dwFileAttributes,
    })
}

fn final_path(handle: windows::Win32::Foundation::HANDLE, stage: &str) -> Result<PathBuf> {
    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(finalization_error(
            stage,
            "opened directory has no bounded final path",
        ));
    }
    Ok(PathBuf::from(String::from_utf16_lossy(
        &buffer[..length as usize],
    )))
}

fn raw_handle(handle: &impl AsRawHandle) -> windows::Win32::Foundation::HANDLE {
    windows::Win32::Foundation::HANDLE(handle.as_raw_handle() as *mut _)
}

#[cfg(test)]
#[path = "camera_finalization_windows_stage_tests.rs"]
mod tests;
