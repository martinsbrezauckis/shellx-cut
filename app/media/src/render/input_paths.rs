use std::path::{Path, PathBuf};

/// Strip the Windows verbatim / extended-length prefix (`\\?\C:\…`,
/// `\\?\UNC\…`) that `canonicalize()` stamps on project and asset paths.
/// External FFmpeg builds cannot reliably read that form as an input or inside
/// a filtergraph. `\\?\C:\x` becomes `C:\x`, `\\?\UNC\srv\sh` becomes
/// `\\srv\sh`, and POSIX/native plain paths are unchanged.
pub(super) fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}
