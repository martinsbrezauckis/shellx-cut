# ShellX Cut Icon Set

Source: `branding/shellx-cut-icon.png` (dark family tile, canonical ShellX X,
and blue filmstrip card; vector master `branding/shellx-cut-icon.svg`).

- Source size: 512 x 512 PNG.
- Source SHA-256: `9ed0954f0e21a5918ed7a51842321989a1af8786a6c4409717e415b05f3bc245`.
- Generator: `cargo tauri icon ../../branding/shellx-cut-icon.png -o src-tauri/icons`
  (run from `app/desktop/`).
- android/ and ios/ output removed — desktop-only crate (regenerate with the
  same command if mobile targets ever land).

## Desktop / Store

| File | Size |
| --- | ---: |
| `icon.png` | 512 x 512 |
| `32x32.png` | 32 x 32 |
| `64x64.png` | 64 x 64 |
| `128x128.png` | 128 x 128 |
| `128x128@2x.png` | 256 x 256 |
| `StoreLogo.png` | 50 x 50 |
| `Square30x30Logo.png` … `Square310x310Logo.png` | Windows store tile sizes |
| `icon.ico` | Windows multi-size icon container |
| `icon.icns` | macOS multi-size icon container |
