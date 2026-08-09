//! Build script for the `server` crate (the `cutd` binary).
//!
//! On macOS the recorder links ScreenCaptureKit via the `screencapturekit` crate, which
//! pulls in Apple's Swift concurrency runtime (`libswift_Concurrency.dylib`). That dylib
//! is referenced as `@rpath/libswift_Concurrency.dylib`, but a plain Rust binary carries
//! NO `LC_RPATH`, so cutd aborted at launch with:
//!   dyld: Library not loaded: @rpath/libswift_Concurrency.dylib — no LC_RPATH's found
//! The Swift runtime ships in the OS at `/usr/lib/swift` (macOS 10.14.4+; resolved via the
//! dyld shared cache). Add that rpath to the binaries on the macOS target only — a no-op on
//! Windows/Linux, including Mac ScreenCaptureKit capture parity.

fn main() {
    // CARGO_CFG_TARGET_OS is the TARGET os (cross-compile-safe), not the host.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,/usr/lib/swift");

        // The apple_metal Swift bridge pulls Swift back-compat archives
        // (swiftCompatibility56 / swiftCompatibilityConcurrency). rustc emits a
        // link-search path under `XcodeDefault.xctoolchain/usr/lib/swift/macosx`,
        // but on a Command-Line-Tools-only Mac (no Xcode.app) those archives live
        // under the CLT path instead, so the link fails with
        //   undefined symbol: __swift_FORCE_LOAD_$_swiftCompatibility56
        // Add every Swift `macosx` lib dir that ACTUALLY EXISTS as a link-search
        // path (no-op when absent), so the build links on both CLT-only and
        // full-Xcode machines.
        for dir in [
            "/Library/Developer/CommandLineTools/usr/lib/swift/macosx",
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx",
        ] {
            if std::path::Path::new(dir).is_dir() {
                println!("cargo:rustc-link-arg-bins=-L{dir}");
            }
        }
    }
}
