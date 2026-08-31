// Private Windows Region topology snapshot ABI shared only by the foreground
// picker and its Rust shell wrapper. It is never a browser or server wire type.

#pragma once

#include <cstddef>
#include <cstdint>
#include <windows.h>

constexpr std::size_t SXC_WINDOWS_GDI_DEVICE_UNITS = CCHDEVICENAME;
constexpr std::size_t SXC_WINDOWS_TARGET_PATH_UNITS = 128;
constexpr std::size_t SXC_WINDOWS_TOPOLOGY_DIGEST_BYTES = 32;

struct SXCWindowsTopologySnapshot {
    uint16_t source_gdi[SXC_WINDOWS_GDI_DEVICE_UNITS];
    uint16_t target_path[SXC_WINDOWS_TARGET_PATH_UNITS];
    uint8_t digest[SXC_WINDOWS_TOPOLOGY_DIGEST_BYTES];
};

bool sxc_windows_topology_snapshot_for_gdi(
    const WCHAR* source_gdi,
    uint32_t parent_width,
    uint32_t parent_height,
    SXCWindowsTopologySnapshot* output);

bool sxc_windows_topology_snapshot_for_monitor(
    HMONITOR monitor,
    RECT* monitor_rect,
    SXCWindowsTopologySnapshot* output);

bool sxc_windows_topology_snapshot_is_current(
    const SXCWindowsTopologySnapshot* snapshot,
    uint32_t parent_width,
    uint32_t parent_height);

// Narrow C ABI for the cutd-side recorder. The foreground picker never shares
// a UI handle with cutd; this only checks the already-issued private snapshot.
extern "C" int32_t sxc_windows_topology_snapshot_is_current_ffi(
    const SXCWindowsTopologySnapshot* snapshot,
    uint32_t parent_width,
    uint32_t parent_height);
