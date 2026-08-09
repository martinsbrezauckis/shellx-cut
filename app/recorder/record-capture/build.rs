// build.rs — compile the macOS Core Audio system-audio tap shim (mac_systemaudio.mm) and
// link the CoreAudio + Foundation frameworks, ONLY when building the macOS capture backend
// (target_os = macos AND feature capture-macos). On every other target this is a no-op, so
// the Linux/Windows builds are unaffected. See src/mac_systemaudio.mm for why the tap exists
// (the SCK capturesAudio path is broken on macOS 15+/26).

fn main() {
    println!("cargo:rerun-if-changed=src/mac_systemaudio.mm");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let has_macos_capture = std::env::var("CARGO_FEATURE_CAPTURE_MACOS").is_ok();
    if target_os == "macos" && has_macos_capture {
        // ScreenCaptureKit's Swift bridge loads the concurrency runtime through
        // @rpath. Test and example binaries do not inherit cutd's server-level
        // linker flags, so give every record-capture link target the runtime
        // path and the compatibility-archive search paths it needs.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        for dir in [
            "/Library/Developer/CommandLineTools/usr/lib/swift/macosx",
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx",
        ] {
            if std::path::Path::new(dir).is_dir() {
                println!("cargo:rustc-link-search=native={dir}");
            }
        }

        cc::Build::new()
            .file("src/mac_systemaudio.mm")
            .flag("-fobjc-arc") // ARC for the CATapDescription / NSDictionary objects
            .cpp_link_stdlib(None) // we add libc++ explicitly below (rustc does the final link)
            .compile("sxc_mac_systemaudio");
        // The shim uses std::vector/std::mutex → needs the C++ runtime. Rust's link line is
        // -nodefaultlibs, so name libc++ explicitly or the std:: symbols are undefined.
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
