use super::*;

/// A minimal x64 PE image whose import table names `imports` and whose
/// delay-load table names `delayed`. Only the parts the reader uses exist.
fn fixture_pe(imports: &[&str], delayed: &[&str]) -> Vec<u8> {
    let section_rva: u32 = 0x1000;
    let mut section = vec![0u8; 0x400];
    let import_table = 0usize;
    let delay_table = (imports.len() + 1) * 20;
    let mut cursor = delay_table + (delayed.len() + 1) * 32;
    let mut name_rvas = Vec::new();
    for name in imports.iter().chain(delayed) {
        name_rvas.push(section_rva + cursor as u32);
        section[cursor..cursor + name.len()].copy_from_slice(name.as_bytes());
        cursor += name.len() + 1;
    }
    for (index, rva) in name_rvas.iter().take(imports.len()).enumerate() {
        let at = import_table + index * 20;
        section[at + 12..at + 16].copy_from_slice(&rva.to_le_bytes());
        section[at + 16..at + 20].copy_from_slice(&(section_rva + 0x300).to_le_bytes());
    }
    for (index, rva) in name_rvas.iter().skip(imports.len()).enumerate() {
        let at = delay_table + index * 32;
        section[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        section[at + 4..at + 8].copy_from_slice(&rva.to_le_bytes());
    }
    let mut headers = vec![0u8; 0x200];
    headers[0..2].copy_from_slice(b"MZ");
    let pe = 0x40usize;
    headers[0x3c..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
    headers[pe..pe + 4].copy_from_slice(b"PE\0\0");
    headers[pe + 4..pe + 6].copy_from_slice(&0x8664u16.to_le_bytes());
    headers[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes());
    headers[pe + 20..pe + 22].copy_from_slice(&240u16.to_le_bytes());
    let optional = pe + 24;
    headers[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    headers[optional + 24..optional + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    headers[optional + 108..optional + 112].copy_from_slice(&16u32.to_le_bytes());
    let dirs = optional + 112;
    if !imports.is_empty() {
        headers[dirs + 8..dirs + 12]
            .copy_from_slice(&(section_rva + import_table as u32).to_le_bytes());
    }
    if !delayed.is_empty() {
        let at = dirs + 13 * 8;
        headers[at..at + 4].copy_from_slice(&(section_rva + delay_table as u32).to_le_bytes());
    }
    let header = optional + 240;
    headers[header..header + 6].copy_from_slice(b".idata");
    headers[header + 8..header + 12].copy_from_slice(&0x400u32.to_le_bytes());
    headers[header + 12..header + 16].copy_from_slice(&section_rva.to_le_bytes());
    headers[header + 16..header + 20].copy_from_slice(&0x400u32.to_le_bytes());
    headers[header + 20..header + 24].copy_from_slice(&0x200u32.to_le_bytes());
    headers.extend_from_slice(&section);
    headers
}

#[test]
fn reads_normal_and_delay_load_imports() {
    let image = fixture_pe(&["KERNEL32.dll", "MSVCP140.dll"], &["VCRUNTIME140_1.dll"]);
    assert_eq!(
        imported_dlls(&image).unwrap(),
        vec!["KERNEL32.dll", "MSVCP140.dll", "VCRUNTIME140_1.dll"]
    );
    assert!(imported_dlls(b"not a program").is_none());
}

#[test]
fn visual_cpp_runtime_names_are_recognised() {
    for name in [
        "MSVCP140.dll",
        "msvcp140_1.dll",
        "VCRUNTIME140_1.dll",
        "vcomp140.dll",
        "concrt140.dll",
        "MSVCR120.dll",
    ] {
        assert!(is_vc_runtime_dll(name), "{name}");
    }
    for name in [
        "kernel32.dll",
        "python312.dll",
        "msvcrt.dll",
        "msvcp_win.dll",
        "api-ms-win-crt-runtime-l1-1-0.dll",
        // A wheel's own renamed copy in its .libs folder (numpy: delvewheel).
        "msvcp140-a4c2229bdc2a2a630acdc095b4d86008.dll",
    ] {
        assert!(!is_vc_runtime_dll(name), "{name}");
    }
}

#[test]
fn gate_reports_a_runtime_dll_that_no_searched_folder_has() {
    let root = tempfile::tempdir().unwrap();
    let python = root.path().join("python");
    let site = root.path().join("venv").join("Lib").join("site-packages");
    let capi = site.join("onnxruntime").join("capi");
    std::fs::create_dir_all(&python).unwrap();
    std::fs::create_dir_all(&capi).unwrap();
    std::fs::write(python.join("vcruntime140.dll"), b"x").unwrap();
    std::fs::write(
        capi.join("onnxruntime_pybind11_state.pyd"),
        fixture_pe(&["python312.dll", "MSVCP140.dll", "VCRUNTIME140.dll"], &[]),
    )
    .unwrap();
    std::fs::write(
        site.join("plain.pyd"),
        fixture_pe(&["VCRUNTIME140.dll"], &[]),
    )
    .unwrap();

    let missing = missing_runtime_dlls(&site, &python, None);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].dll, "MSVCP140.dll");
    assert!(missing[0]
        .module
        .ends_with("onnxruntime_pybind11_state.pyd"));

    let error = missing_runtime_error(&missing, "base");
    assert!(error.message.contains("MSVCP140.DLL"), "{}", error.message);
    assert!(error.message.contains("Microsoft Visual C++ runtime"));
    assert!(error
        .cause
        .contains("onnxruntime_pybind11_state.pyd needs MSVCP140.dll"));
    assert!(error
        .suggested_action
        .unwrap()
        .contains("Install captions again"));

    // App-local copy beside python.exe satisfies it (case-insensitively).
    std::fs::write(python.join("msvcp140.dll"), b"x").unwrap();
    assert!(missing_runtime_dlls(&site, &python, None).is_empty());
}

#[test]
fn a_copy_in_the_module_folder_or_the_system_folder_also_counts() {
    let root = tempfile::tempdir().unwrap();
    let python = root.path().join("python");
    let system = root.path().join("System32");
    let site = root.path().join("site-packages");
    let pkg = site.join("pkg");
    for dir in [&python, &system, &pkg] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(
        pkg.join("mod.pyd"),
        fixture_pe(&["vcomp140.dll", "msvcp140_1.dll"], &[]),
    )
    .unwrap();
    std::fs::write(pkg.join("vcomp140.dll"), b"x").unwrap();
    std::fs::write(system.join("MSVCP140_1.DLL"), b"x").unwrap();
    assert!(missing_runtime_dlls(&site, &python, Some(&system)).is_empty());
}

#[test]
fn uv_stays_inside_the_perception_folder() {
    let env = uv_environment_for(Path::new("/data/ShellX Cut/perception"));
    let get = |key: &str| env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
    assert_eq!(
        get("UV_PYTHON_INSTALL_DIR").unwrap(),
        Path::new("/data/ShellX Cut/perception")
            .join("python")
            .into_os_string()
    );
    assert_eq!(
        get("UV_CACHE_DIR").unwrap(),
        Path::new("/data/ShellX Cut/perception")
            .join("uv-cache")
            .into_os_string()
    );
    assert_eq!(get("UV_PYTHON_INSTALL_BIN").unwrap(), "0");
    assert_eq!(get("UV_PYTHON_INSTALL_REGISTRY").unwrap(), "0");
}

#[test]
fn the_gate_points_at_microsofts_runtime_entry_not_a_third_party_package() {
    let missing = vec![MissingRuntime {
        dll: "MSVCP140.dll".to_string(),
        module: PathBuf::from("onnxruntime_pybind11_state.pyd"),
    }];
    let action = missing_runtime_error(&missing, "base")
        .suggested_action
        .unwrap();
    assert!(
        action.contains("Settings > AI & transcription > Microsoft Visual C++ runtime"),
        "{action}"
    );
    assert!(
        action.contains(r#"system.fetch_tool {"tool":"vc-runtime"}"#),
        "{action}"
    );
    assert!(
        action.contains("https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist"),
        "{action}"
    );
    assert!(!action.contains("own Python folder"), "{action}");
    // The removed third-party route leaves no trace in this module.
    let source = include_str!("windows_runtime.rs");
    assert!(!source.contains("msvc-runtime=="));
    assert!(!source.contains("--require-hashes"));
}
