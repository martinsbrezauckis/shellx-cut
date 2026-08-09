//! ScreenCast restore-token location and persistence.

pub(crate) fn token_path_from(
    xdg_cache_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    let absolute = |value: Option<&std::ffi::OsStr>| {
        value
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .filter(|value| value.is_absolute())
    };
    let base =
        absolute(xdg_cache_home).or_else(|| absolute(home).map(|home| home.join(".cache")))?;
    Some(base.join("shellx-record/screencast.token"))
}

fn token_path() -> Option<std::path::PathBuf> {
    let xdg_cache_home = std::env::var_os("XDG_CACHE_HOME");
    let home = std::env::var_os("HOME");
    token_path_from(xdg_cache_home.as_deref(), home.as_deref())
}

pub(crate) fn read_token() -> Option<String> {
    std::fs::read_to_string(token_path()?)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn write_token(token: &str) {
    let Some(path) = token_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, token);
}

#[cfg(test)]
mod tests {
    use super::token_path_from;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn restore_token_path_requires_an_absolute_cache_base() {
        assert_eq!(
            token_path_from(Some(OsStr::new("/cache")), Some(OsStr::new("/home/user"))),
            Some(PathBuf::from("/cache/shellx-record/screencast.token"))
        );
        assert_eq!(
            token_path_from(None, Some(OsStr::new("/home/user"))),
            Some(PathBuf::from(
                "/home/user/.cache/shellx-record/screencast.token"
            ))
        );
        assert_eq!(token_path_from(None, None), None);
        assert_eq!(
            token_path_from(Some(OsStr::new("relative")), Some(OsStr::new("home"))),
            None
        );
    }
}
