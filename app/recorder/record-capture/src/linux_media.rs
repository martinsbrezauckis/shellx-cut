//! Linux finalized-source metadata probes kept outside the live portal owner.

use std::process::Command;

pub(crate) fn probe_dims(ffprobe: &str, path: &str) -> Option<(u32, u32)> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=,",
            path,
        ])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut values = stdout.lines().next()?.trim().split(',');
    Some((values.next()?.parse().ok()?, values.next()?.parse().ok()?))
}
