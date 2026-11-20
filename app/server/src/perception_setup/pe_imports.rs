//! Read the DLL names a Windows PE image imports (normal and delay-load
//! tables). Pure byte parsing with bounds checks; used by the captions runtime
//! gate (windows_runtime.rs) on every platform's tests.

/// DLL names in a PE image's import and delay-load import tables. None when
/// the bytes are not a readable PE image.
pub(crate) fn imported_dlls(bytes: &[u8]) -> Option<Vec<String>> {
    let u16_at = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if bytes.get(0..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(0x3c)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = u16_at(pe + 6)? as usize;
    let optional_size = u16_at(pe + 20)? as usize;
    let optional = pe + 24;
    let (count_at, dirs_at, image_base) = match u16_at(optional)? {
        0x10b => (
            optional + 92,
            optional + 96,
            u64::from(u32_at(optional + 28)?),
        ),
        0x20b => {
            let base = bytes.get(optional + 24..optional + 32)?;
            (
                optional + 108,
                optional + 112,
                u64::from_le_bytes(base.try_into().ok()?),
            )
        }
        _ => return None,
    };
    let dir_count = u32_at(count_at)? as usize;
    let table = optional + optional_size;
    let mut spans = Vec::with_capacity(sections);
    for index in 0..sections {
        let at = table + index * 40;
        spans.push((
            u32_at(at + 12)?,
            u32_at(at + 8)?.max(u32_at(at + 16)?),
            u32_at(at + 20)?,
        ));
    }
    let offset = |rva: u32| {
        spans
            .iter()
            .find(|(va, size, _)| rva >= *va && rva < va.saturating_add(*size))
            .and_then(|(va, _, raw)| raw.checked_add(rva - va))
            .map(|at| at as usize)
    };
    let name_at = |rva: u32| {
        let start = offset(rva)?;
        let end = start + bytes.get(start..)?.iter().take(512).position(|b| *b == 0)?;
        Some(String::from_utf8_lossy(&bytes[start..end]).into_owned())
    };
    let directory = |index: usize| {
        (index < dir_count)
            .then(|| u32_at(dirs_at + index * 8))
            .flatten()
            .filter(|rva| *rva != 0)
    };
    let mut names = Vec::new();
    if let Some(rva) = directory(1) {
        let mut at = offset(rva)?;
        for _ in 0..4096 {
            let (name, thunk) = (u32_at(at + 12)?, u32_at(at + 16)?);
            if name == 0 && thunk == 0 {
                break;
            }
            names.push(name_at(name)?);
            at += 20;
        }
    }
    if let Some(rva) = directory(13) {
        let mut at = offset(rva)?;
        for _ in 0..4096 {
            let (attributes, mut name) = (u32_at(at)?, u32_at(at + 4)?);
            if name == 0 {
                break;
            }
            if attributes & 1 == 0 {
                name = u32::try_from(u64::from(name).checked_sub(image_base)?).ok()?;
            }
            names.push(name_at(name)?);
            at += 32;
        }
    }
    Some(names)
}
