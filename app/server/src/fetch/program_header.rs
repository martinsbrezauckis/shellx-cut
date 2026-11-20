//! program_header.rs — is an unpacked file the kind of program this machine
//! runs? A cheap check of the executable header, after the SHA-256 match, so
//! a wrong pin (another platform's build) is refused with a plain reason
//! instead of failing at the first import.

const MH_MAGIC_64: [u8; 4] = [0xcf, 0xfa, 0xed, 0xfe];
const CPU_TYPE_ARM64: [u8; 4] = [0x0c, 0x00, 0x00, 0x01];

/// Accept only a thin 64-bit arm64 Mach-O file (the start of its header).
pub(crate) fn check_arm64_macho(header: &[u8]) -> Result<(), String> {
    match header.get(..8) {
        Some(h) if h[..4] == MH_MAGIC_64 && h[4..8] == CPU_TYPE_ARM64 => Ok(()),
        Some(h) if h[..4] == MH_MAGIC_64 => {
            Err("the program is not built for Apple silicon".into())
        }
        _ => Err("the file is not a single-architecture macOS program".into()),
    }
}

/// Accept only a 64-bit x86 Windows PE executable.
pub(crate) fn check_windows_x64_pe(header: &[u8]) -> Result<(), String> {
    if header.get(..2) != Some(b"MZ") {
        return Err("the file is not a Windows program".into());
    }
    let pe = header
        .get(0x3c..0x40)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or("the Windows program header is truncated")?;
    if header.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("the file is not a Windows program".into());
    }
    match header.get(pe + 4..pe + 6) {
        Some([0x64, 0x86]) => Ok(()),
        Some(_) => Err("the Windows program is not built for 64-bit x86".into()),
        None => Err("the Windows program header is truncated".into()),
    }
}

/// Accept only a 64-bit little-endian x86-64 ELF executable.
pub(crate) fn check_linux_x64_elf(header: &[u8]) -> Result<(), String> {
    if header.get(..4) != Some(b"\x7fELF") {
        return Err("the file is not a Linux program".into());
    }
    if header.get(4) != Some(&2) || header.get(5) != Some(&1) {
        return Err("the Linux program is not a 64-bit little-endian build".into());
    }
    match header.get(18..20) {
        Some([0x3e, 0x00]) => Ok(()),
        Some(_) => Err("the Linux program is not built for 64-bit x86".into()),
        None => Err("the Linux program header is truncated".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_thin_arm64_programs_pass_the_mac_check() {
        assert!(check_arm64_macho(&[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1, 9, 9]).is_ok());
        assert!(check_arm64_macho(&[0xcf, 0xfa, 0xed, 0xfe, 0x07, 0, 0, 1]).is_err());
        assert!(check_arm64_macho(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]).is_err());
        assert!(check_arm64_macho(b"\x7fELF\x02\x01\x01\x00").is_err());
        assert!(check_arm64_macho(b"").is_err());
    }

    fn pe(machine: [u8; 2]) -> Vec<u8> {
        let mut h = vec![0u8; 0x100];
        h[..2].copy_from_slice(b"MZ");
        h[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        h[0x80..0x84].copy_from_slice(b"PE\0\0");
        h[0x84..0x86].copy_from_slice(&machine);
        h
    }

    #[test]
    fn only_x64_windows_programs_pass_the_windows_check() {
        assert!(check_windows_x64_pe(&pe([0x64, 0x86])).is_ok());
        let arm = check_windows_x64_pe(&pe([0x64, 0xaa])).unwrap_err();
        assert!(arm.contains("64-bit x86"), "{arm}");
        assert!(check_windows_x64_pe(b"#!/bin/sh\n").is_err());
        assert!(check_windows_x64_pe(&pe([0x64, 0x86])[..0x82]).is_err());
    }

    fn elf(class: u8, machine: u8) -> Vec<u8> {
        let mut h = vec![0u8; 64];
        h[..4].copy_from_slice(b"\x7fELF");
        h[4] = class;
        h[5] = 1;
        h[18] = machine;
        h
    }

    #[test]
    fn only_x86_64_linux_programs_pass_the_linux_check() {
        assert!(check_linux_x64_elf(&elf(2, 0x3e)).is_ok());
        assert!(check_linux_x64_elf(&elf(2, 0xb7))
            .unwrap_err()
            .contains("64-bit x86"));
        assert!(check_linux_x64_elf(&elf(1, 0x3e)).is_err());
        assert!(check_linux_x64_elf(b"#!/bin/sh\n").is_err());
    }
}
