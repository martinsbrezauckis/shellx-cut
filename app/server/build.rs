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

fn embed_windows_version_resource() {
    let version = std::env::var("CARGO_PKG_VERSION")
        .expect("Cargo must provide CARGO_PKG_VERSION to the cutd build script");
    let component = |name: &str| {
        std::env::var(name)
            .unwrap_or_else(|_| panic!("Cargo must provide {name} to the cutd build script"))
            .parse::<u16>()
            .unwrap_or_else(|_| panic!("{name} exceeds the Windows version-resource range"))
    };
    let [major, minor, patch, build] = [
        component("CARGO_PKG_VERSION_MAJOR"),
        component("CARGO_PKG_VERSION_MINOR"),
        component("CARGO_PKG_VERSION_PATCH"),
        0,
    ];
    let out_dir = std::path::PathBuf::from(
        std::env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR to the cutd build script"),
    );
    let resource = out_dir.join("cutd-version.rc");
    let source = format!(
        r#"1 VERSIONINFO
FILEVERSION {major},{minor},{patch},{build}
PRODUCTVERSION {major},{minor},{patch},{build}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "ShellX\0"
      VALUE "FileDescription", "ShellX Cut engine\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "cutd\0"
      VALUE "OriginalFilename", "cutd.exe\0"
      VALUE "ProductName", "ShellX Cut\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#,
    );
    std::fs::write(&resource, source).expect("write generated cutd Windows version resource");
    embed_resource::compile_for(&resource, ["cutd"], embed_resource::NONE)
        .manifest_required()
        .expect("compile and link cutd Windows version resource");
}

fn main() {
    // CARGO_CFG_TARGET_OS is the TARGET os (cross-compile-safe), not the host.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS");
    if target_os.as_deref() == Ok("windows") {
        embed_windows_version_resource();
    }
    if target_os.as_deref() == Ok("macos") {
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
