//! wl_selftest — exercise the REAL Wayland capture module (`record_capture::wayland_pw`)
//! end-to-end without a portal consent dialog, by driving `org.gnome.Mutter.ScreenCast`
//! directly. Lets the frame + cursor-metadata path be regression-tested repeatably
//! (headless/over SSH), e.g. after touching `wayland_pw.rs`.
//!
//! Requires a logged-in GNOME Wayland session and its user bus:
//!   XDG_RUNTIME_DIR=/run/user/$UID DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$UID/bus \
//!   cargo run -p record-capture --example wl_selftest --features capture-linux -- [seconds]
//!
//! Set `SHELLX_CONNECTOR` to your monitor's connector if it isn't `DP-1`
//! (e.g. `eDP-1`, `HDMI-1`; list with `gnome-monitor-config list` or wlr-randr).
//! For cursor motion, run a pointer jiggler in another shell during the window.
//!
//! Expected on success: `OK: <N> cursor samples` and a valid random temporary output path.
//! This is what proves the size-negotiation fix + de-pad + re-entrancy guard still work.

#[cfg(all(target_os = "linux", feature = "capture-linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Instant;

    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);

    let rt = tokio::runtime::Runtime::new()?;
    // Hold the connection (and thus the mutter session/node) alive for the capture.
    let (_conn, node) = rt.block_on(direct_mutter_node())?;

    let out_dir = tempfile::Builder::new()
        .prefix("shellx-cut-wl-selftest-")
        .tempdir()?
        .keep();
    let raw = out_dir.join("raw.mp4");
    let raw_display = raw.display();
    let raw = raw.to_str().ok_or("temporary output path is not UTF-8")?;
    let dur_ms = secs.checked_mul(1000).ok_or("duration is too large")?;
    let stop = Arc::new(AtomicBool::new(false));
    println!("capturing {secs}s from mutter node {node} -> {raw_display} ...");
    match record_capture::wayland_pw::capture(
        None, // None = connect to the default user PipeWire (no portal fd)
        node,
        dur_ms,
        Instant::now(),
        stop,
        raw,
        "ffmpeg",
    ) {
        Ok(cursor) => {
            println!("OK: {} cursor samples, raw={raw_display}", cursor.len());
            if cursor.is_empty() {
                eprintln!("(note: 0 cursor samples — move the pointer during the window)");
            }
        }
        Err(e) => {
            eprintln!("ERR: {e}");
            std::process::exit(1);
        }
    }
    drop(rt);
    Ok(())
}

/// Create a mutter ScreenCast session with `cursor-mode=2` (metadata) on a monitor and
/// return (connection, pipewire node id). No portal, so no consent dialog.
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
async fn direct_mutter_node() -> Result<(zbus::Connection, u32), Box<dyn std::error::Error>> {
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, Value};

    let conn = zbus::Connection::session().await?;
    let sc = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.ScreenCast",
        "/org/gnome/Mutter/ScreenCast",
        "org.gnome.Mutter.ScreenCast",
    )
    .await?;
    let create: HashMap<&str, Value> = HashMap::new();
    let session_path: OwnedObjectPath = sc.call("CreateSession", &(create,)).await?;
    let session = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.ScreenCast",
        session_path.as_str().to_owned(),
        "org.gnome.Mutter.ScreenCast.Session",
    )
    .await?;

    let mut props: HashMap<&str, Value> = HashMap::new();
    props.insert("cursor-mode", Value::U32(2)); // 2 = METADATA
    let connector = std::env::var("SHELLX_CONNECTOR").unwrap_or_else(|_| "DP-1".to_string());
    let stream_path: OwnedObjectPath = session
        .call("RecordMonitor", &(connector.as_str(), props))
        .await
        .map_err(|e| {
            format!("RecordMonitor({connector}) failed ({e}); set SHELLX_CONNECTOR to your monitor")
        })?;
    let stream = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.ScreenCast",
        stream_path.as_str().to_owned(),
        "org.gnome.Mutter.ScreenCast.Stream",
    )
    .await?;

    let mut added = stream.receive_signal("PipeWireStreamAdded").await?;
    session.call_method("Start", &()).await?;
    let msg = added.next().await.ok_or("no PipeWireStreamAdded signal")?;
    let node: u32 = msg.body().deserialize()?;
    Ok((conn, node))
}

#[cfg(not(all(target_os = "linux", feature = "capture-linux")))]
fn main() {
    eprintln!("wl_selftest requires Linux + --features capture-linux");
}
