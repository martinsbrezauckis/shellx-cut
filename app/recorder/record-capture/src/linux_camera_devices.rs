//! Passive V4L2 inventory. Never opens a video node or starts a capture.

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use record_core::{error_codes, RecordError, Result};
use sha2::{Digest, Sha256};

const MAX_NODES: usize = 128;
const MAX_METADATA: u64 = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinuxCameraNode {
    pub(crate) id: String,
    pub(crate) label: String,
    /// Internal only: an explicit Start must revalidate before opening this node.
    pub(crate) node: PathBuf,
    pub(crate) rdev: u64,
    pub(crate) node_inode: u64,
    pub(crate) sysfs_inode: u64,
}

#[derive(Clone, Copy)]
struct NodeStat {
    rdev: u64,
    inode: u64,
    character: bool,
}

struct Roots {
    class: PathBuf,
    udev: PathBuf,
    dev: PathBuf,
}

#[allow(
    dead_code,
    reason = "Linux camera stays private until its native owner is admitted"
)]
pub(crate) fn devices() -> Result<Vec<LinuxCameraNode>> {
    inventory(
        Roots {
            class: PathBuf::from("/sys/class/video4linux"),
            udev: PathBuf::from("/run/udev/data"),
            dev: PathBuf::from("/dev"),
        },
        |path| {
            let metadata = fs::metadata(path).map_err(|error| failure("stat video node", error))?;
            Ok(NodeStat {
                rdev: metadata.rdev(),
                inode: metadata.ino(),
                character: metadata.file_type().is_char_device(),
            })
        },
    )
}

fn inventory(
    roots: Roots,
    stat_node: impl Fn(&Path) -> Result<NodeStat>,
) -> Result<Vec<LinuxCameraNode>> {
    let entries = match fs::read_dir(&roots.class) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(failure("enumerate camera class", error)),
    };
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| failure("read camera class", error))
        })
        .collect::<Result<Vec<_>>>()?;
    if paths.len() > MAX_NODES {
        return Err(bad("camera class exceeds bounded inventory"));
    }
    paths.sort();
    let mut devices = Vec::new();
    for class_entry in paths {
        let Some(name) = class_entry.file_name().and_then(|name| name.to_str()) else {
            return Err(bad("camera class entry has no UTF-8 name"));
        };
        if !name.starts_with("video")
            || name[5..].is_empty()
            || !name[5..].bytes().all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let class_path = fs::canonicalize(&class_entry)
            .map_err(|error| failure("resolve camera class", error))?;
        let dev_pair = bounded_text(&class_entry.join("dev"), "camera device number")?;
        let (major, minor) = parse_dev_pair(&dev_pair)?;
        let node = roots.dev.join(name);
        let stat = stat_node(&node)?;
        if !stat.character || stat.rdev != libc::makedev(major, minor) {
            return Err(bad("camera node is not the class's character device"));
        }
        let udev = bounded_text(
            &roots.udev.join(format!("c{major}:{minor}")),
            "camera udev data",
        )?;
        let properties = parse_udev(&udev)?;
        let capability = properties
            .get("ID_V4L_CAPABILITIES")
            .ok_or_else(|| bad("camera udev capabilities are not ready"))?;
        if !capability.split(':').any(|item| item == "capture") {
            continue; // A metadata-only or output-only V4L2 node is not a camera.
        }
        let label = bounded_text(&class_entry.join("name"), "camera label")?;
        if label.chars().any(char::is_control) {
            return Err(bad("camera label contains control characters"));
        }
        let index = bounded_text(&class_entry.join("index"), "camera index")?;
        let index = index
            .parse::<u32>()
            .map_err(|_| bad("camera index is invalid"))?;
        let class_inode = fs::metadata(&class_entry)
            .map_err(|error| failure("stat camera class", error))?
            .ino();
        let physical = match fs::symlink_metadata(class_entry.join("device")) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(failure("inspect camera physical route", error)),
        };
        let route = if physical {
            fs::canonicalize(class_entry.join("device"))
                .map_err(|error| failure("resolve camera physical route", error))?
        } else {
            class_path.clone()
        };
        let serial = properties
            .get("ID_SERIAL_SHORT")
            .filter(|value| !value.is_empty());
        let mut identity = Sha256::new();
        identity.update(b"shellx-cut-linux-camera-v1\0");
        identity.update(route.as_os_str().as_encoded_bytes());
        identity.update(b"\0");
        identity.update(index.to_le_bytes());
        identity.update(b"\0");
        identity.update(label.as_bytes());
        identity.update(b"\0");
        if let Some(serial) = serial {
            identity.update(serial.as_bytes());
        } else {
            // Without a unique serial, do not silently adopt a replacement at
            // the same port or node. Such selections are generation-specific.
            identity.update(class_inode.to_le_bytes());
            identity.update(stat.inode.to_le_bytes());
        }
        let id = format!("linux-camera-v1-{:x}", identity.finalize());
        if devices
            .iter()
            .any(|device: &LinuxCameraNode| device.id == id)
        {
            return Err(bad(
                "camera inventory contains an ambiguous duplicate identity",
            ));
        }
        devices.push(LinuxCameraNode {
            id,
            label,
            node,
            rdev: stat.rdev,
            node_inode: stat.inode,
            sysfs_inode: class_inode,
        });
    }
    Ok(devices)
}

fn bounded_text(path: &Path, label: &str) -> Result<String> {
    let metadata = fs::metadata(path).map_err(|error| failure(label, error))?;
    if metadata.len() > MAX_METADATA {
        return Err(bad(&format!("{label} exceeds the bounded size")));
    }
    let bytes = fs::read(path).map_err(|error| failure(label, error))?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(bad(&format!("{label} exceeds the bounded size")));
    }
    let text = String::from_utf8(bytes).map_err(|_| bad(&format!("{label} is not UTF-8")))?;
    let text = text.trim();
    if text.is_empty() {
        return Err(bad(&format!("{label} is empty")));
    }
    Ok(text.into())
}

fn parse_dev_pair(value: &str) -> Result<(u32, u32)> {
    let (major, minor) = value
        .split_once(':')
        .ok_or_else(|| bad("camera device number is invalid"))?;
    let major = major
        .parse()
        .map_err(|_| bad("camera major number is invalid"))?;
    let minor = minor
        .parse()
        .map_err(|_| bad("camera minor number is invalid"))?;
    Ok((major, minor))
}

fn parse_udev(value: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut props = std::collections::HashMap::new();
    for line in value.lines() {
        let Some(property) = line.strip_prefix("E:") else {
            continue;
        };
        let (key, val) = property
            .split_once('=')
            .ok_or_else(|| bad("camera udev property is malformed"))?;
        if key.is_empty() || props.insert(key.to_owned(), val.to_owned()).is_some() {
            return Err(bad("camera udev property key is invalid or duplicated"));
        }
    }
    Ok(props)
}

fn failure(label: &str, error: std::io::Error) -> RecordError {
    RecordError::new(error_codes::CAPTURE, label, error.to_string())
}

fn bad(message: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        message,
        "camera inventory metadata is incomplete or unsafe",
    )
}

#[cfg(test)]
#[path = "linux_camera_devices_tests.rs"]
mod tests;
