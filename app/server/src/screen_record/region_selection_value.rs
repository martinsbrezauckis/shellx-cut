//! Private Region-selection values, including topology-bound Windows tickets.

use super::region_selection::{NativeMonitorIdentity, NativeRegionCrop};

const TOPOLOGY_FINGERPRINT_BYTES: usize = 32;
const WINDOWS_GDI_DEVICE_UNITS: usize = 32;
const WINDOWS_TARGET_PATH_UNITS: usize = 128;

/// Opaque fixed-size digest of the exact active native topology that issued a
/// Region crop. It is private process state, never serde/wire/receipt data.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeTopologyFingerprint([u8; TOPOLOGY_FINGERPRINT_BYTES]);

impl NativeTopologyFingerprint {
    pub(crate) fn new(value: [u8; TOPOLOGY_FINGERPRINT_BYTES]) -> Option<Self> {
        value.iter().any(|byte| *byte != 0).then_some(Self(value))
    }

    pub(crate) fn matches(&self, value: &[u8; TOPOLOGY_FINGERPRINT_BYTES]) -> bool {
        self.0 == *value
    }

    pub(crate) fn bytes(&self) -> [u8; TOPOLOGY_FINGERPRINT_BYTES] {
        self.0
    }
}

/// The private, non-serializable portion of a Windows DisplayConfig snapshot.
/// It survives only from authenticated shell admission until atomic ticket
/// consumption, when cutd repeats the native topology check. The physical
/// target path never reaches a verb, browser response, marker, or receipt.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NativeWindowsTopologySnapshot {
    source_gdi: String,
    target_path: String,
    fingerprint: NativeTopologyFingerprint,
}

impl NativeWindowsTopologySnapshot {
    pub(crate) fn new(
        source_gdi: String,
        target_path: String,
        fingerprint: [u8; TOPOLOGY_FINGERPRINT_BYTES],
    ) -> Option<Self> {
        let source_units = source_gdi.encode_utf16().count();
        let target_units = target_path.encode_utf16().count();
        let valid = source_gdi.starts_with(r"\\.\DISPLAY")
            && target_path.starts_with(r"\\?\")
            && !source_gdi.contains('\0')
            && !target_path.contains('\0')
            && source_units < WINDOWS_GDI_DEVICE_UNITS
            && target_units < WINDOWS_TARGET_PATH_UNITS;
        if !valid {
            return None;
        }
        Some(Self {
            source_gdi,
            target_path,
            fingerprint: NativeTopologyFingerprint::new(fingerprint)?,
        })
    }

    pub(crate) fn source_gdi(&self) -> &str {
        &self.source_gdi
    }

    pub(crate) fn target_path(&self) -> &str {
        &self.target_path
    }

    pub(crate) fn fingerprint(&self) -> NativeTopologyFingerprint {
        self.fingerprint
    }
}

/// Validated native-picker value. Construction has no clamping or default
/// monitor path, so a caller must provide one exact target and crop.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RegionSelectionValue {
    monitor: NativeMonitorIdentity,
    crop: NativeRegionCrop,
    topology: Option<NativeTopologyFingerprint>,
    windows_topology: Option<NativeWindowsTopologySnapshot>,
}

impl RegionSelectionValue {
    pub(crate) fn new(monitor: NativeMonitorIdentity, crop: NativeRegionCrop) -> Self {
        Self {
            monitor,
            crop,
            topology: None,
            windows_topology: None,
        }
    }

    /// Windows Region admission supplies this only from the exact foreground
    /// DisplayConfig snapshot that produced `monitor` and `crop`. Its later
    /// ticket consumer must require the same digest, never match dimensions.
    pub(crate) fn with_topology(
        monitor: NativeMonitorIdentity,
        crop: NativeRegionCrop,
        topology: NativeTopologyFingerprint,
    ) -> Self {
        Self {
            monitor,
            crop,
            topology: Some(topology),
            windows_topology: None,
        }
    }

    /// Windows Region admission retains the full native snapshot only until
    /// consume-time validation. Its opaque fingerprint remains available to
    /// existing generic topology tests, while only the Windows owner may read
    /// the raw fields required by the native ABI.
    pub(crate) fn with_windows_topology(
        monitor: NativeMonitorIdentity,
        crop: NativeRegionCrop,
        topology: NativeWindowsTopologySnapshot,
    ) -> Self {
        Self {
            monitor,
            crop,
            topology: Some(topology.fingerprint()),
            windows_topology: Some(topology),
        }
    }

    pub(crate) fn monitor_identity(&self) -> &NativeMonitorIdentity {
        &self.monitor
    }

    pub(crate) fn crop(&self) -> NativeRegionCrop {
        self.crop
    }

    pub(crate) fn topology_fingerprint(&self) -> Option<NativeTopologyFingerprint> {
        self.topology
    }

    pub(crate) fn windows_topology(&self) -> Option<&NativeWindowsTopologySnapshot> {
        self.windows_topology.as_ref()
    }
}

/// Exact internal value returned only after one-use consumption and exact
/// monitor/topology revalidation.
pub(crate) struct ConsumedRegionSelection(pub(crate) RegionSelectionValue);

impl ConsumedRegionSelection {
    pub(crate) fn monitor_identity(&self) -> &NativeMonitorIdentity {
        self.0.monitor_identity()
    }

    pub(crate) fn crop(&self) -> NativeRegionCrop {
        self.0.crop()
    }

    pub(crate) fn topology_fingerprint(&self) -> Option<NativeTopologyFingerprint> {
        self.0.topology_fingerprint()
    }

    pub(crate) fn windows_topology(&self) -> Option<&NativeWindowsTopologySnapshot> {
        self.0.windows_topology()
    }
}
