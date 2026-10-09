#[cfg(any(windows, test))]
const WINDOWS_PREFIX: &str = "windows-hwnd-v1";
#[cfg(any(target_os = "macos", test))]
const MACOS_PREFIX: &str = "macos-scwindow-v1";

#[cfg(any(windows, test))]
pub(crate) fn windows_window_id(hwnd: usize, pid: u32) -> String {
    format!("{WINDOWS_PREFIX}:{hwnd:016x}:{pid}")
}

#[cfg(any(windows, test))]
pub(crate) fn parse_windows_window_id(value: &str) -> Option<(usize, u32)> {
    let mut parts = value.split(':');
    if parts.next()? != WINDOWS_PREFIX {
        return None;
    }
    let hwnd = usize::from_str_radix(parts.next()?, 16).ok()?;
    let pid = parts.next()?.parse::<u32>().ok()?;
    (parts.next().is_none() && hwnd != 0 && pid != 0).then_some((hwnd, pid))
}

/// Bind the opaque identity to the HWND already admitted by the source picker.
/// Never re-enumerate a second transient native snapshot or admit a different item.
#[cfg(any(windows, test))]
pub(crate) fn admitted_windows_window_identity(
    id: &str,
    admitted_hwnd: usize,
) -> Option<(usize, u32)> {
    let identity = parse_windows_window_id(id)?;
    (identity.0 == admitted_hwnd).then_some(identity)
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_window_id(window_id: u32) -> String {
    format!("{MACOS_PREFIX}:{window_id}")
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn parse_macos_window_id(value: &str) -> Option<u32> {
    let mut parts = value.split(':');
    if parts.next()? != MACOS_PREFIX {
        return None;
    }
    let id = parts.next()?.parse::<u32>().ok()?;
    (parts.next().is_none() && id != 0).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_identity_binds_handle_and_process() {
        let id = windows_window_id(0x1234_abcd, 4312);
        assert_eq!(id, "windows-hwnd-v1:000000001234abcd:4312");
        assert_eq!(parse_windows_window_id(&id), Some((0x1234_abcd, 4312)));
        assert_eq!(parse_windows_window_id("windows-hwnd-v1:0:4312"), None);
        assert_eq!(parse_windows_window_id("windows-hwnd-v1:1234:0"), None);
        assert_eq!(
            parse_windows_window_id("windows-hwnd-v1:1234:4312:extra"),
            None
        );
        assert_eq!(parse_windows_window_id("Fixture Window"), None);
    }

    #[test]
    fn click_owner_binds_already_admitted_window_and_refuses_mismatch() {
        let id = windows_window_id(0x1234_abcd, 4312);
        assert_eq!(
            admitted_windows_window_identity(&id, 0x1234_abcd),
            Some((0x1234_abcd, 4312))
        );
        assert_eq!(admitted_windows_window_identity(&id, 0x5678), None);
        assert_eq!(admitted_windows_window_identity(&id, 0), None);
        assert_eq!(
            admitted_windows_window_identity("windows-hwnd-v1:1234:0", 0x1234),
            None
        );
        assert_eq!(
            admitted_windows_window_identity("Fixture Window", 0x1234),
            None
        );
    }

    #[test]
    fn macos_identity_is_native_and_not_an_ordinal() {
        let id = macos_window_id(87);
        assert_eq!(id, "macos-scwindow-v1:87");
        assert_eq!(parse_macos_window_id(&id), Some(87));
        assert_eq!(parse_macos_window_id("macos-scwindow-v1:0"), None);
        assert_eq!(parse_macos_window_id("87"), None);
    }
}
