//! Field-stable `screen_record.doctor` display projection.
//!
//! Native enumeration owns the opaque identity. This server layer copies it
//! exactly and deliberately does not derive replacement identities from display
//! copy, ordinal, primary status, or geometry.

use super::MonitorInfo;

pub(super) fn monitors(
    source: impl IntoIterator<Item = record_capture::MonitorInfo>,
) -> Vec<MonitorInfo> {
    source
        .into_iter()
        .map(|monitor| MonitorInfo {
            id: monitor.id,
            index: monitor.index,
            name: monitor.name,
            width: monitor.width,
            height: monitor.height,
            primary: monitor.primary,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_the_opaque_native_id_without_rederiving_it() {
        let id = "shellx-monitor-v1:windows:0123456789abcdef";
        let result = monitors([record_capture::MonitorInfo {
            id: Some(id.into()),
            index: 2,
            name: "Renamed display".into(),
            width: 1_920,
            height: 1_080,
            primary: false,
        }]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_deref(), Some(id));
        assert_eq!(result[0].index, 2);
        assert_eq!(result[0].name, "Renamed display");
    }

    #[test]
    fn projection_keeps_an_identity_unavailable_row_usable_by_the_legacy_picker() {
        let result = monitors([record_capture::MonitorInfo {
            id: None,
            index: 3,
            name: "Display with no exact identity".into(),
            width: 1_280,
            height: 720,
            primary: false,
        }]);
        assert_eq!(result[0].id, None);
        assert_eq!(result[0].index, 3);
    }
}
