//! The title-movie path rules: every form a received project could record.
use super::*;

const NAME: &str = "title_0123456789abcdef.mov";

#[test]
fn only_the_projects_own_titles_folder_is_ours() {
    // An absolute project folder on this system (none of it exists).
    let dir = Path::new(if cfg!(windows) {
        r"C:\p\a.cutproj"
    } else {
        "/p/a.cutproj"
    });
    let d = dir.display();
    let ours = TitleMovie::Ours(dir.join("titles").join(NAME));
    assert_eq!(title_movie(dir, &format!("{d}/titles/{NAME}")), ours);
    assert_eq!(title_movie(dir, &format!("titles/{NAME}")), ours);
    // A `.` inside a path is the same folder (Path drops it): still ours, and
    // the place written is built from the project folder anyway.
    assert_eq!(title_movie(dir, &format!("{d}/titles/./{NAME}")), ours);
    // Rejected forms: never a place Cut writes.
    for path in [
        format!("titles/../../somewhere/{NAME}"),
        format!("{d}/titles/../../etc/{NAME}"),
        format!("/elsewhere/titles/{NAME}"),
        format!("{d}/../other.cutproj/titles/{NAME}"),
        format!("other/titles/{NAME}"),
        NAME.to_string(),
    ] {
        assert_eq!(title_movie(dir, &path), TitleMovie::Elsewhere, "{path}");
    }
    // Not title movies at all.
    for path in [
        "/p/a.cutproj/titles/notes.mov",
        "/p/a.cutproj/titles/title_.mov",
        "/p/a.cutproj/titles/title_a b.mov",
        "/media/clip.mp4",
    ] {
        assert_eq!(title_movie(dir, path), TitleMovie::NotATitle, "{path}");
    }
}

#[cfg(unix)]
#[test]
fn a_backslash_is_an_ordinary_character_off_windows() {
    let dir = Path::new("/p/a.cutproj");
    // One file name with backslashes in it, not a path that climbs out.
    assert_eq!(
        title_movie(dir, r"titles\..\..\title_0123.mov"),
        TitleMovie::NotATitle
    );
    assert_eq!(
        title_movie(dir, r"/p/a.cutproj/titles/..\..\title_0123.mov"),
        TitleMovie::NotATitle
    );
}

#[cfg(windows)]
#[test]
fn the_same_titles_folder_counts_in_any_windows_spelling() {
    let verbatim = Path::new(r"\\?\C:\Users\me\p.cutproj");
    let plain = Path::new(r"C:\Users\me\p.cutproj");
    let ours = |dir: &Path, path: &str| matches!(title_movie(dir, path), TitleMovie::Ours(_));
    assert!(ours(
        verbatim,
        &format!(r"C:\Users\me\p.cutproj\titles\{NAME}")
    ));
    assert!(ours(
        plain,
        &format!(r"\\?\C:\Users\me\p.cutproj\titles\{NAME}")
    ));
    assert!(ours(plain, &format!("C:/Users/me/p.cutproj/titles/{NAME}")));
    assert!(ours(
        verbatim,
        &format!(r"c:\users\ME\P.cutproj\Titles\{NAME}")
    ));
    assert!(ours(
        Path::new(r"\\?\UNC\srv\share\p.cutproj"),
        &format!(r"\\srv\share\p.cutproj\titles\{NAME}")
    ));
    assert_eq!(
        title_movie(
            verbatim,
            &format!(r"C:\Users\me\p.cutproj\titles\..\..\{NAME}")
        ),
        TitleMovie::Elsewhere
    );
    assert_eq!(
        title_movie(
            verbatim,
            &format!(r"C:\Users\me\other.cutproj\titles\{NAME}")
        ),
        TitleMovie::Elsewhere
    );
    // The written path is built from the project folder, not from the record.
    assert_eq!(
        title_movie(verbatim, &format!(r"c:\users\ME\P.cutproj\Titles\{NAME}")),
        TitleMovie::Ours(verbatim.join("titles").join(NAME))
    );
}

#[cfg(unix)]
#[test]
fn a_linked_titles_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("a.cutproj");
    let outside = root.path().join("outside");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, project.join("titles")).unwrap();
    assert!(writable_titles_folder(&project).is_err());
    std::fs::remove_file(project.join("titles")).unwrap();
    assert_eq!(
        writable_titles_folder(&project).unwrap(),
        project.join("titles")
    );
}

#[cfg(unix)]
#[test]
fn a_recorded_path_through_a_linked_folder_is_not_ours() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("a.cutproj");
    let outside = root.path().join("outside");
    std::fs::create_dir_all(project.join("titles")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.path().join("link")).unwrap();
    // A folder outside, reached through a link: resolves elsewhere.
    let through_link = root.path().join("link").join(NAME);
    assert_eq!(
        title_movie(&project, &through_link.display().to_string()),
        TitleMovie::Elsewhere
    );
    // The project's own folder, written through a link to the project: ours,
    // and the place written is built from the project folder.
    std::os::unix::fs::symlink(&project, root.path().join("plink")).unwrap();
    let via = root.path().join("plink").join("titles").join(NAME);
    assert_eq!(
        title_movie(&project, &via.display().to_string()),
        TitleMovie::Ours(project.join("titles").join(NAME))
    );
}
