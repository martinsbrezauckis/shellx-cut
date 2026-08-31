// windows_region_topology.cpp -- exact active DisplayConfig snapshots.

#include "windows_region_topology.h"

#include <bcrypt.h>

#include <algorithm>
#include <cstring>
#include <string>
#include <utility>
#include <vector>

namespace {

struct ActivePath {
    std::wstring source_gdi;
    std::wstring target_path;
};

// DisplayConfig reports physical modes, and the low-level input hook reports
// per-monitor-aware points. Keep every topology/monitor lookup in that same
// coordinate space even though cutd is a console child without the shell's UI
// manifest. A failure must refuse the ticket rather than compare virtualized
// dimensions to the selected WGC parent frame.
class PerMonitorDpiContext {
public:
    bool enter() {
        previous_ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        return previous_ != nullptr;
    }

    ~PerMonitorDpiContext() {
        if (previous_) SetThreadDpiAwarenessContext(previous_);
    }

private:
    DPI_AWARENESS_CONTEXT previous_ = nullptr;
};

std::wstring utf16_value(const WCHAR* value, size_t capacity) {
    const size_t length = wcsnlen(value, capacity);
    return length == 0 || length == capacity ? std::wstring{} : std::wstring(value, length);
}

bool device_names(const DISPLAYCONFIG_PATH_INFO& path, ActivePath* output) {
    if (!output) return false;
    DISPLAYCONFIG_SOURCE_DEVICE_NAME source{};
    source.header = {
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        static_cast<UINT32>(sizeof(source)),
        path.sourceInfo.adapterId,
        path.sourceInfo.id,
    };
    DISPLAYCONFIG_TARGET_DEVICE_NAME target{};
    target.header = {
        DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        static_cast<UINT32>(sizeof(target)),
        path.targetInfo.adapterId,
        path.targetInfo.id,
    };
    if (DisplayConfigGetDeviceInfo(&source.header) != ERROR_SUCCESS
        || DisplayConfigGetDeviceInfo(&target.header) != ERROR_SUCCESS) {
        return false;
    }
    output->source_gdi = utf16_value(source.viewGdiDeviceName, CCHDEVICENAME);
    output->target_path = utf16_value(target.monitorDevicePath, SXC_WINDOWS_TARGET_PATH_UNITS);
    return !output->source_gdi.empty() && !output->target_path.empty();
}

bool active_paths(std::vector<ActivePath>* output) {
    if (!output) return false;
    UINT32 path_count = 0;
    UINT32 mode_count = 0;
    if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &path_count, &mode_count) != ERROR_SUCCESS
        || path_count == 0) {
        return false;
    }
    std::vector<DISPLAYCONFIG_PATH_INFO> paths(path_count);
    std::vector<DISPLAYCONFIG_MODE_INFO> modes(mode_count);
    if (QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &path_count,
            paths.data(),
            &mode_count,
            modes.data(),
            nullptr) != ERROR_SUCCESS) {
        return false;
    }
    paths.resize(path_count);
    output->clear();
    output->reserve(paths.size());
    for (const auto& path : paths) {
        ActivePath active;
        if (!device_names(path, &active)) return false;
        output->push_back(std::move(active));
    }
    std::sort(output->begin(), output->end(), [](const ActivePath& left, const ActivePath& right) {
        return left.source_gdi == right.source_gdi ? left.target_path < right.target_path
                                                   : left.source_gdi < right.source_gdi;
    });
    return !output->empty();
}

void append_u32(std::vector<UCHAR>* bytes, uint32_t value) {
    for (unsigned shift = 0; shift < 32; shift += 8) {
        bytes->push_back(static_cast<UCHAR>(value >> shift));
    }
}

bool append_utf16(std::vector<UCHAR>* bytes, const std::wstring& value) {
    if (!bytes || value.size() > UINT32_MAX) return false;
    append_u32(bytes, static_cast<uint32_t>(value.size()));
    for (const WCHAR unit : value) append_u32(bytes, static_cast<uint16_t>(unit));
    return true;
}

bool topology_digest(const std::vector<ActivePath>& paths, uint8_t output[32]) {
    static constexpr char prefix[] = "shellx-cut-windows-topology-v1";
    std::vector<UCHAR> bytes(prefix, prefix + sizeof(prefix));
    if (paths.size() > UINT32_MAX) return false;
    append_u32(&bytes, static_cast<uint32_t>(paths.size()));
    for (const auto& path : paths) {
        if (!append_utf16(&bytes, path.source_gdi) || !append_utf16(&bytes, path.target_path)) return false;
    }

    if (bytes.size() > ULONG_MAX) return false;

    BCRYPT_ALG_HANDLE algorithm = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;
    ULONG object_size = 0;
    ULONG received = 0;
    NTSTATUS status = BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0);
    if (status >= 0) status = BCryptGetProperty(algorithm, BCRYPT_OBJECT_LENGTH, reinterpret_cast<PUCHAR>(&object_size), sizeof(object_size), &received, 0);
    std::vector<UCHAR> object(object_size);
    if (status >= 0) status = BCryptCreateHash(algorithm, &hash, object.data(), object_size, nullptr, 0, 0);
    if (status >= 0) status = BCryptHashData(hash, bytes.data(), static_cast<ULONG>(bytes.size()), 0);
    if (status >= 0) status = BCryptFinishHash(hash, output, SXC_WINDOWS_TOPOLOGY_DIGEST_BYTES, 0);
    if (hash) BCryptDestroyHash(hash);
    if (algorithm) BCryptCloseAlgorithmProvider(algorithm, 0);
    return status >= 0;
}

bool copy_wide(const std::wstring& value, uint16_t* output, size_t capacity) {
    if (!output || value.empty() || value.size() >= capacity) return false;
    std::fill(output, output + capacity, 0);
    for (size_t index = 0; index < value.size(); ++index) output[index] = static_cast<uint16_t>(value[index]);
    return true;
}

std::wstring snapshot_value(const uint16_t* value, size_t capacity) {
    if (!value) return {};
    size_t length = 0;
    while (length < capacity && value[length] != 0) ++length;
    if (length == 0 || length == capacity) return {};
    std::wstring output;
    output.reserve(length);
    for (size_t index = 0; index < length; ++index) output.push_back(static_cast<WCHAR>(value[index]));
    return output;
}

struct MonitorLookup {
    const std::wstring* gdi = nullptr;
    uint32_t width = 0;
    uint32_t height = 0;
    unsigned matches = 0;
};

BOOL CALLBACK monitor_callback(HMONITOR monitor, HDC, LPRECT, LPARAM lparam) {
    auto* lookup = reinterpret_cast<MonitorLookup*>(lparam);
    MONITORINFOEXW info{};
    info.cbSize = sizeof(info);
    if (!lookup || !GetMonitorInfoW(monitor, &info) || *lookup->gdi != info.szDevice) return TRUE;
    const int64_t width = static_cast<int64_t>(info.rcMonitor.right) - info.rcMonitor.left;
    const int64_t height = static_cast<int64_t>(info.rcMonitor.bottom) - info.rcMonitor.top;
    if (width <= 0 || height <= 0 || width > UINT32_MAX || height > UINT32_MAX) return TRUE;
    lookup->width = static_cast<uint32_t>(width);
    lookup->height = static_cast<uint32_t>(height);
    ++lookup->matches;
    return TRUE;
}

bool current_monitor_dimensions(const std::wstring& gdi, uint32_t* width, uint32_t* height) {
    MonitorLookup lookup{&gdi};
    if (!EnumDisplayMonitors(nullptr, nullptr, monitor_callback, reinterpret_cast<LPARAM>(&lookup))
        || lookup.matches != 1 || !width || !height) {
        return false;
    }
    *width = lookup.width;
    *height = lookup.height;
    return true;
}

}  // namespace

bool sxc_windows_topology_snapshot_for_gdi(
    const WCHAR* source_gdi,
    uint32_t parent_width,
    uint32_t parent_height,
    SXCWindowsTopologySnapshot* output) {
    if (!source_gdi || !output || parent_width == 0 || parent_height == 0) return false;
    PerMonitorDpiContext dpi;
    if (!dpi.enter()) return false;
    const std::wstring selected = utf16_value(source_gdi, SXC_WINDOWS_GDI_DEVICE_UNITS);
    std::vector<ActivePath> paths;
    if (selected.empty() || !active_paths(&paths)) return false;
    const auto match = std::find_if(paths.begin(), paths.end(), [&selected](const ActivePath& path) {
        return path.source_gdi == selected;
    });
    const unsigned source_matches = static_cast<unsigned>(std::count_if(
        paths.begin(), paths.end(), [&selected](const ActivePath& path) {
            return path.source_gdi == selected;
        }));
    uint32_t current_width = 0;
    uint32_t current_height = 0;
    if (match == paths.end() || source_matches != 1
        || !current_monitor_dimensions(selected, &current_width, &current_height)
        || current_width != parent_width || current_height != parent_height) {
        return false;
    }
    *output = {};
    return copy_wide(match->source_gdi, output->source_gdi, SXC_WINDOWS_GDI_DEVICE_UNITS)
        && copy_wide(match->target_path, output->target_path, SXC_WINDOWS_TARGET_PATH_UNITS)
        && topology_digest(paths, output->digest);
}

bool sxc_windows_topology_snapshot_for_monitor(
    HMONITOR monitor,
    RECT* monitor_rect,
    SXCWindowsTopologySnapshot* output) {
    if (!monitor || !monitor_rect || !output) return false;
    PerMonitorDpiContext dpi;
    if (!dpi.enter()) return false;
    MONITORINFOEXW first{};
    first.cbSize = sizeof(first);
    uint32_t width = 0;
    uint32_t height = 0;
    if (!GetMonitorInfoW(monitor, &first)
        || !current_monitor_dimensions(utf16_value(first.szDevice, CCHDEVICENAME), &width, &height)
        || width == 0 || height == 0) {
        return false;
    }
    SXCWindowsTopologySnapshot snapshot{};
    if (!sxc_windows_topology_snapshot_for_gdi(first.szDevice, width, height, &snapshot)) return false;
    MONITORINFOEXW confirmed{};
    confirmed.cbSize = sizeof(confirmed);
    if (!GetMonitorInfoW(monitor, &confirmed)
        || std::memcmp(&confirmed.rcMonitor, &first.rcMonitor, sizeof(RECT)) != 0
        || wcscmp(confirmed.szDevice, first.szDevice) != 0
        || !sxc_windows_topology_snapshot_is_current(&snapshot, width, height)) {
        return false;
    }
    *monitor_rect = first.rcMonitor;
    *output = snapshot;
    return true;
}

bool sxc_windows_topology_snapshot_is_current(
    const SXCWindowsTopologySnapshot* snapshot,
    uint32_t parent_width,
    uint32_t parent_height) {
    if (!snapshot || parent_width == 0 || parent_height == 0) return false;
    PerMonitorDpiContext dpi;
    if (!dpi.enter()) return false;
    const std::wstring source = snapshot_value(snapshot->source_gdi, SXC_WINDOWS_GDI_DEVICE_UNITS);
    const std::wstring target = snapshot_value(snapshot->target_path, SXC_WINDOWS_TARGET_PATH_UNITS);
    std::vector<ActivePath> paths;
    if (source.empty() || target.empty() || !active_paths(&paths)) return false;
    const bool exact_path = std::any_of(paths.begin(), paths.end(), [&source, &target](const ActivePath& path) {
        return path.source_gdi == source && path.target_path == target;
    });
    uint32_t current_width = 0;
    uint32_t current_height = 0;
    uint8_t digest[SXC_WINDOWS_TOPOLOGY_DIGEST_BYTES]{};
    return exact_path && current_monitor_dimensions(source, &current_width, &current_height)
        && current_width == parent_width && current_height == parent_height
        && topology_digest(paths, digest)
        && std::memcmp(snapshot->digest, digest, SXC_WINDOWS_TOPOLOGY_DIGEST_BYTES) == 0;
}

extern "C" int32_t sxc_windows_topology_snapshot_is_current_ffi(
    const SXCWindowsTopologySnapshot* snapshot,
    uint32_t parent_width,
    uint32_t parent_height) {
    return sxc_windows_topology_snapshot_is_current(snapshot, parent_width, parent_height) ? 1 : 0;
}
