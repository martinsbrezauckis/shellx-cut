use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(crate) fn existing_image_cache_is_complete(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("jpg" | "jpeg") => complete_jpeg(path),
        Some("png") => complete_png(path),
        _ => path
            .metadata()
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false),
    }
}

fn complete_jpeg(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.len() < 4 {
        return false;
    }
    let mut head = [0u8; 2];
    let mut tail = [0u8; 2];
    if file.read_exact(&mut head).is_err() {
        return false;
    }
    if file.seek(SeekFrom::End(-2)).is_err() || file.read_exact(&mut tail).is_err() {
        return false;
    }
    head == [0xff, 0xd8] && tail == [0xff, 0xd9]
}

fn complete_png(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.len() < 8 {
        return false;
    }
    let mut head = [0u8; 8];
    file.read_exact(&mut head).is_ok() && head == *b"\x89PNG\r\n\x1a\n"
}
