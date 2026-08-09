# ShellX Cut branding

## App icon

The vector master is `shellx-cut-icon.svg`; `shellx-cut-icon.png` is its
512 x 512 raster source for the desktop icon generator. The mark uses the
canonical ShellX X, a dark tile, and ShellX Cut's blue filmstrip motif.

The X is the **canonical traced path** (512 viewBox, filled). Reuse this path
verbatim for ShellX-family module icons; do not redraw it:
`M74 90 L200 90 L256 168 L312 90 L438 90 L330 256 L438 422 L312 422 L256 344 L200 422 L74 422 L182 256 Z`

## Regenerate

Regenerate desktop icons with Tauri from the checked-in raster source:

```bash
cd app/desktop
cargo tauri icon ../../branding/shellx-cut-icon.png -o src-tauri/icons
```
