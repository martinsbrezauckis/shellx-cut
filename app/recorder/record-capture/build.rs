// build.rs — compile the macOS Core Audio system-audio tap shim (mac_systemaudio.mm) and
// link the CoreAudio + Foundation frameworks, ONLY when building the macOS capture backend
// (target_os = macos AND feature capture-macos). On every other target this is a no-op, so
// the Linux/Windows builds are unaffected. See src/mac_systemaudio.mm for why the tap exists
// (the SCK capturesAudio path is broken on macOS 15+/26).

fn main() {
    println!("cargo:rerun-if-changed=src/linux_camera_native.c");
    println!("cargo:rerun-if-changed=src/linux_camera_native_test_support.h");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var("CARGO_FEATURE_CAPTURE_LINUX").is_ok()
    {
        let gst = pkg_config::Config::new()
            .atleast_version("1.20")
            .probe("gstreamer-1.0")
            .expect("Linux camera capture needs GStreamer core development headers >= 1.20");
        let mut build = cc::Build::new();
        build.file("src/linux_camera_native.c");
        for include in gst.include_paths {
            build.include(include);
        }
        build
            .flag_if_supported("-std=c11")
            .compile("sxc_linux_camera_native");
    }
    println!("cargo:rerun-if-changed=src/mac_systemaudio.mm");
    println!("cargo:rerun-if-changed=src/mic_endpoint/macos.mm");
    println!("cargo:rerun-if-changed=src/macos_camera_native.mm");
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
            .file("src/mic_endpoint/macos.mm")
            .file("src/macos_camera_native.mm")
            .flag("-fobjc-arc") // ARC for the CATapDescription / NSDictionary objects
            .cpp_link_stdlib(None) // we add libc++ explicitly below (rustc does the final link)
            .compile("sxc_mac_systemaudio");
        // The shim uses std::vector/std::mutex → needs the C++ runtime. Rust's link line is
        // -nodefaultlibs, so name libc++ explicitly or the std:: symbols are undefined.
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }

    let has_windows_capture = std::env::var("CARGO_FEATURE_CAPTURE_WINDOWS").is_ok();
    if target_os == "windows" && has_windows_capture {
        // The foreground desktop owns the Win32 Region overlay, while cutd
        // needs this smaller topology-only ABI to burn a one-use selection
        // after a fresh DisplayConfig/physical-target revalidation. Keep the
        // picker itself out of the capture child: it must never present UI.
        let topology = "../../server/src/screen_record/windows_region_topology.cpp";
        println!("cargo:rerun-if-changed={topology}");
        println!("cargo:rerun-if-changed=../../server/src/screen_record/windows_region_topology.h");
        cc::Build::new()
            .file(topology)
            .cpp(true)
            // xwin can provision a newer MSVC STL than Ubuntu's maintained
            // clang-cl. The topology shim uses only stable Win32/C++17
            // facilities, so admit that supported cross-toolchain pairing
            // instead of making a native Windows helper depend on the host's
            // MSVC header release cadence.
            .define("_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH", None)
            .compile("sxc_windows_region_topology");
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=bcrypt");
    }
}
