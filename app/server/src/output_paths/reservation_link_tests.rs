//! The hidden reservation files beside an export are created and locked only
//! as ordinary files: a link planted at their names by a received project
//! never creates, locks or changes a file elsewhere.

use super::*;

fn project_with_exports(root: &Path) -> PathBuf {
    let project = root.join("p.cutproj");
    std::fs::create_dir_all(project.join("exports")).unwrap();
    project
}

#[cfg(unix)]
#[test]
fn linked_reservation_lease_creates_nothing_outside_the_project() {
    let _fixture = test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let project = project_with_exports(root.path());
    let target = root.path().join("created-by-a-link.txt");
    std::os::unix::fs::symlink(
        &target,
        project.join("exports/.render.mp4.shellx-cut-reservation.lock"),
    )
    .unwrap();

    let reserved = fence_output_path(
        &project,
        Some("exports/render.mp4"),
        "exports/render.mp4",
        OutputPathPolicy::MP4,
    );

    assert!(!target.exists(), "a file was created through the link");
    assert!(reserved.is_err());
}

#[test]
fn shared_reservation_lease_is_refused() {
    let _fixture = test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let project = project_with_exports(root.path());
    let outside = root.path().join("outside.txt");
    std::fs::write(&outside, b"outside").unwrap();
    std::fs::hard_link(
        &outside,
        project.join("exports/.render.mp4.shellx-cut-reservation.lock"),
    )
    .unwrap();

    let reserved = fence_output_path(
        &project,
        Some("exports/render.mp4"),
        "exports/render.mp4",
        OutputPathPolicy::MP4,
    );

    assert!(reserved.is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
}

#[cfg(windows)]
#[test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
fn windows_symlink_reservation_lease_creates_nothing_outside_the_project() {
    let _fixture = test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let project = project_with_exports(root.path());
    let target = root.path().join("created-by-a-link.txt");
    std::os::windows::fs::symlink_file(
        &target,
        project.join("exports/.render.mp4.shellx-cut-reservation.lock"),
    )
    .unwrap();

    let reserved = fence_output_path(
        &project,
        Some("exports/render.mp4"),
        "exports/render.mp4",
        OutputPathPolicy::MP4,
    );

    assert!(!target.exists(), "a file was created through the link");
    assert!(reserved.is_err());
}

/// A junction (a name-surrogate reparse point that needs no privilege to
/// create) in a project output folder chain is refused.
#[cfg(windows)]
#[test]
#[ignore = "creates a directory junction with cmd mklink /J on an NTFS volume"]
fn windows_junction_in_the_output_folder_chain_is_refused() {
    let _fixture = test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("p.cutproj");
    std::fs::create_dir_all(&project).unwrap();
    let outside = root.path().join("outside-folder");
    std::fs::create_dir(&outside).unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(project.join(".scratch"))
        .arg(&outside)
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed: {status}");

    let admitted = ensure_plain_project_relative_dir(&project, Path::new(".scratch/judge"));

    assert!(admitted.is_err(), "{admitted:?}");
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
}

/// A compressed (WOF) file in a project output folder is an ordinary file:
/// its reparse point is not a name surrogate.
#[cfg(windows)]
#[test]
#[ignore = "requires an NTFS volume and compact.exe with /EXE (Windows 10 or later)"]
fn windows_compressed_reservation_lease_is_an_ordinary_file() {
    let _fixture = test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let project = project_with_exports(root.path());
    let lease = project.join("exports/.render.mp4.shellx-cut-reservation.lock");
    std::fs::write(&lease, "lease ".repeat(8192)).unwrap();
    let status = std::process::Command::new("compact.exe")
        .args(["/C", "/EXE:XPRESS4K"])
        .arg(&lease)
        .status()
        .unwrap();
    assert!(status.success(), "compact.exe /EXE failed: {status}");

    let reserved = fence_output_path(
        &project,
        Some("exports/render.mp4"),
        "exports/render.mp4",
        OutputPathPolicy::MP4,
    );

    assert!(reserved.is_ok(), "{:?}", reserved.err());
}
