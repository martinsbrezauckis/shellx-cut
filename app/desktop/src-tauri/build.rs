// ShellX Cut — Tauri build script.
// Runs tauri-build's codegen (parses tauri.conf.json, generates the context,
// capability schemas under gen/, and platform metadata). All configuration
// lives in tauri.conf.json.
fn main() {
    tauri_build::build();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // The foreground Tauri process owns the AppKit modal picker. The
        // server source is intentionally shared only for the tiny C ABI and
        // visual implementation; cutd does not compile or link this object.
        let picker = "../../server/src/screen_record/macos_region_picker.mm";
        println!("cargo:rerun-if-changed={picker}");
        cc::Build::new()
            .file(picker)
            .flag("-fobjc-arc")
            .cpp_link_stdlib(None)
            .compile("sxc_macos_region_picker");
        // The picker is Objective-C++. Rust's final linker invocation uses
        // -nodefaultlibs, so name libc++ explicitly just as record-capture's
        // macOS shims do.
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Windows Graphics Capture has no arbitrary-region system picker. The
        // foreground Tauri shell therefore owns this private Win32 overlay;
        // cutd never links or presents it from its child-side capture process.
        let picker = "../../server/src/screen_record/windows_region_picker.cpp";
        let topology = "../../server/src/screen_record/windows_region_topology.cpp";
        println!("cargo:rerun-if-changed={picker}");
        println!("cargo:rerun-if-changed={topology}");
        println!("cargo:rerun-if-changed=../../server/src/screen_record/windows_region_topology.h");
        cc::Build::new()
            .file(picker)
            .file(topology)
            .cpp(true)
            // See record-capture/build.rs: xwin's MSVC headers may be newer
            // than the maintained clang-cl on the central Linux builder.
            .define("_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH", None)
            .compile("sxc_windows_region_picker");
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=bcrypt");
    }
}
