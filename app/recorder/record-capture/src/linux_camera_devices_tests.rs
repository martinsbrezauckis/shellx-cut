//! Passive Linux camera inventory fixtures.

use super::*;
use std::os::unix::fs::symlink;

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for name in ["class", "udev", "dev", "physical"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        Self { root }
    }

    fn roots(&self) -> Roots {
        Roots {
            class: self.root.path().join("class"),
            udev: self.root.path().join("udev"),
            dev: self.root.path().join("dev"),
        }
    }

    fn add(&self, number: u32, serial: Option<&str>, capability: &str) {
        let physical = self.root.path().join("physical/usb-port-a");
        let class_target = physical.join(format!("video4linux/video{number}"));
        fs::create_dir_all(&class_target).unwrap();
        fs::write(class_target.join("dev"), format!("81:{number}\n")).unwrap();
        fs::write(class_target.join("name"), "Fixture camera\n").unwrap();
        fs::write(class_target.join("index"), "0\n").unwrap();
        symlink(&physical, class_target.join("device")).unwrap();
        symlink(
            &class_target,
            self.root.path().join(format!("class/video{number}")),
        )
        .unwrap();
        let mut data = format!("E:ID_V4L_CAPABILITIES={capability}\n");
        if let Some(serial) = serial {
            data.push_str(&format!("E:ID_SERIAL_SHORT={serial}\n"));
        }
        fs::write(self.root.path().join(format!("udev/c81:{number}")), data).unwrap();
    }

    fn add_virtual(&self, number: u32) {
        let target = self
            .root
            .path()
            .join(format!("virtual/video4linux/video{number}"));
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("dev"), format!("81:{number}\n")).unwrap();
        fs::write(target.join("name"), "Cut virtual camera\n").unwrap();
        fs::write(target.join("index"), "0\n").unwrap();
        symlink(
            &target,
            self.root.path().join(format!("class/video{number}")),
        )
        .unwrap();
        fs::write(
            self.root.path().join(format!("udev/c81:{number}")),
            "E:ID_V4L_CAPABILITIES=:capture:\n",
        )
        .unwrap();
    }

    fn inventory(&self) -> Result<Vec<LinuxCameraNode>> {
        // No /dev/videoN exists in this fixture. Only an injected stat may
        // inspect a device: discovery cannot open or start one.
        inventory(self.roots(), |path| {
            assert!(
                !path.exists(),
                "inventory unexpectedly created a video device"
            );
            let number: u32 = path.file_name().unwrap().to_str().unwrap()[5..]
                .parse()
                .unwrap();
            Ok(NodeStat {
                rdev: libc::makedev(81, number),
                inode: number as u64 + 900,
                character: true,
            })
        })
    }
}

#[test]
fn serial_backed_physical_camera_id_survives_video_node_renumbering() {
    let first = Fixture::new();
    first.add(3, Some("unique-unit"), ":capture:");
    let a = first.inventory().unwrap().pop().unwrap();
    fs::remove_file(first.root.path().join("class/video3")).unwrap();
    first.add(8, Some("unique-unit"), ":capture:");
    let b = first.inventory().unwrap().pop().unwrap();
    assert_eq!(a.id, b.id);
    assert_ne!(a.rdev, b.rdev);
    assert!(!a.id.contains("usb-port-a"));
    assert!(!a.id.contains("unique-unit"));
    first.add(9, Some("unique-unit"), ":capture:");
    assert!(
        first.inventory().is_err(),
        "two indistinguishable selected nodes fail closed"
    );
}

#[test]
fn serial_less_generation_and_serial_replacement_do_not_reuse_selection() {
    let fixture = Fixture::new();
    fixture.add(2, None, ":capture:");
    let a = fixture.inventory().unwrap().pop().unwrap();
    let target = fixture
        .root
        .path()
        .join("physical/usb-port-a/video4linux/video2");
    fs::remove_file(fixture.root.path().join("class/video2")).unwrap();
    // Retain the retired inode so the replacement cannot reuse it.
    fs::rename(&target, target.with_file_name("retired-video2")).unwrap();
    fixture.add(2, None, ":capture:");
    let replacement = fixture.inventory().unwrap().pop().unwrap();
    assert_ne!(a.id, replacement.id);
    fs::write(
        fixture.root.path().join("udev/c81:2"),
        "E:ID_V4L_CAPABILITIES=:capture:\nE:ID_SERIAL_SHORT=unit-a\n",
    )
    .unwrap();
    let serial_a = fixture.inventory().unwrap().pop().unwrap();
    fs::write(
        fixture.root.path().join("udev/c81:2"),
        "E:ID_V4L_CAPABILITIES=:capture:\nE:ID_SERIAL_SHORT=unit-b\n",
    )
    .unwrap();
    assert_ne!(serial_a.id, fixture.inventory().unwrap()[0].id);
    fs::remove_file(fixture.root.path().join("class/video2")).unwrap();
    assert!(fixture.inventory().unwrap().is_empty());
}

#[test]
fn serial_less_selection_rejects_node_replacement_with_unchanged_sysfs() {
    let fixture = Fixture::new();
    fixture.add(6, None, ":capture:");
    let node = |inode| {
        inventory(fixture.roots(), |_| {
            Ok(NodeStat {
                rdev: libc::makedev(81, 6),
                inode,
                character: true,
            })
        })
        .unwrap()
        .pop()
        .unwrap()
    };
    let original = node(101);
    let replacement = node(202);
    assert_ne!(original.id, replacement.id);
    assert_eq!(original.sysfs_inode, replacement.sysfs_inode);
    assert_eq!(original.rdev, replacement.rdev);
    assert_eq!(original.label, replacement.label);
}

#[test]
fn metadata_only_nodes_are_skipped_and_missing_udev_is_an_error() {
    let fixture = Fixture::new();
    fixture.add(4, Some("unit"), ":metadata:");
    assert!(fixture.inventory().unwrap().is_empty());
    fs::remove_file(fixture.root.path().join("udev/c81:4")).unwrap();
    assert!(fixture.inventory().is_err());
}

#[test]
fn malformed_capability_and_mismatched_char_identity_fail_closed() {
    let fixture = Fixture::new();
    fixture.add(5, Some("unit"), ":capture:");
    fs::write(
        fixture.root.path().join("udev/c81:5"),
        "E:ID_V4L_CAPABILITIES\n",
    )
    .unwrap();
    assert!(fixture.inventory().is_err());
    fs::write(
        fixture.root.path().join("udev/c81:5"),
        "E:ID_V4L_CAPABILITIES=:capture:\n",
    )
    .unwrap();
    assert!(inventory(fixture.roots(), |_| Ok(NodeStat {
        rdev: 0,
        inode: 1,
        character: true
    }))
    .is_err());
    assert!(inventory(fixture.roots(), |_| Ok(NodeStat {
        rdev: libc::makedev(81, 5),
        inode: 1,
        character: false
    }))
    .is_err());
}

#[test]
fn virtual_loopback_inventory_is_passive_and_generation_scoped() {
    let fixture = Fixture::new();
    fixture.add_virtual(7);
    let original = fixture.inventory().unwrap().pop().unwrap();
    assert_eq!(original.label, "Cut virtual camera");
    assert!(original.id.starts_with("linux-camera-v1-"));
    fs::remove_file(fixture.root.path().join("class/video7")).unwrap();
    let target = fixture.root.path().join("virtual/video4linux/video7");
    fs::rename(&target, target.with_file_name("retired-video7")).unwrap();
    fixture.add_virtual(7);
    assert_ne!(original.id, fixture.inventory().unwrap()[0].id);
}
