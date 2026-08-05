#[cfg(windows)]
fn main() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let mut args = std::env::args().skip(1);
    let output = args
        .next()
        .expect("usage: windows_loopback_probe <output.wav> [duration-ms]");
    let duration_ms = args
        .next()
        .map(|value| {
            value
                .parse::<u64>()
                .expect("duration-ms must be an integer")
        })
        .unwrap_or(5_000);
    let stop = Arc::new(AtomicBool::new(false));

    match record_capture::capture_system_loopback(&output, Some(duration_ms), stop) {
        Ok(path) => println!("{path}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("windows_loopback_probe requires a Windows target");
    std::process::exit(2);
}
