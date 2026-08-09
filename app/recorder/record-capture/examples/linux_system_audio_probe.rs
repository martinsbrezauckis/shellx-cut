//! Focused PipeWire system-audio timing probe for a logged-in Linux desktop.
//!
//! This deliberately has no portal interaction. It captures the PipeWire default
//! sink monitor and reports the first nonempty native buffer on the supplied
//! recorder clock. An optional pre-connect delay makes the shared-clock contract
//! observable without treating process creation as audio timing.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let mut args = std::env::args().skip(1);
    let output = args.next().expect(
        "usage: linux_system_audio_probe <output.wav> [duration-ms] [pre-connect-delay-ms]",
    );
    let duration_ms = args
        .next()
        .map(|value| {
            value
                .parse::<u64>()
                .expect("duration-ms must be an integer")
        })
        .unwrap_or(2_000);
    let delay_ms = args
        .next()
        .map(|value| {
            value
                .parse::<u64>()
                .expect("pre-connect-delay-ms must be an integer")
        })
        .unwrap_or(0);
    let started = Instant::now();
    std::thread::sleep(Duration::from_millis(delay_ms));

    match record_capture::capture_system_pipewire(
        Path::new(&output),
        Some(duration_ms),
        Arc::new(AtomicBool::new(false)),
        started,
    ) {
        Ok(capture) => println!(
            "{} first_packet_offset_ms={:?}",
            capture.path, capture.first_packet_offset_ms
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("linux_system_audio_probe requires Linux + the capture-linux feature");
    std::process::exit(2);
}
