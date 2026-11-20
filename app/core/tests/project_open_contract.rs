//! project.open opens an existing project; it never turns an unrelated folder
//! into one. A folder without a journal or project state is refused, left
//! exactly as it was, and the refusal names project.create.

use cut_core::ProjectStore;

fn listing(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_folder_that_is_not_a_project_is_refused_and_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("clip.mp4"), b"not a project").unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"keep me").unwrap();
    let before = listing(dir.path());

    let error = match ProjectStore::open(dir.path()) {
        Ok(_) => panic!("a media folder must not open as a project"),
        Err(error) => error,
    };

    assert_eq!(error.code, "invalid_args");
    assert!(
        error.message.contains("is not a ShellX Cut project"),
        "{error}"
    );
    assert!(error.cause.contains("nothing was written"), "{error}");
    assert!(
        error
            .suggested_action
            .as_deref()
            .unwrap_or("")
            .contains("project.create"),
        "{error}"
    );
    assert_eq!(
        listing(dir.path()),
        before,
        "the folder must not gain project files"
    );
}

#[test]
fn an_empty_folder_is_not_a_project_either() {
    let dir = tempfile::tempdir().unwrap();
    assert!(ProjectStore::open(dir.path()).is_err());
    assert!(listing(dir.path()).is_empty());
}

#[test]
fn a_created_project_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let created = ProjectStore::create(dir.path(), "p", None).unwrap();
    let path = created.dir.clone();
    drop(created);
    assert!(ProjectStore::open(&path).is_ok());
}

cut_core::isolate_test_process!();
