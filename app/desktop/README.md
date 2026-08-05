# ShellX Cut — desktop shell (Tauri 2)

Native Windows/macOS/Linux wrap of ShellX Cut. **Engine-first:** the shell is
just another client of `cutd` — it spawns the bundled engine binary on
loopback and points its webview at it. Nothing in `app/{core,media,perception,
server,export}` changed for this; a headless `cutd serve` keeps working
unchanged, and the desktop app preserves 100% of the debug/agent surface
(REST verbs, WS events, `cutd mcp` proxy, `cutd verb` CLI) because the engine
it runs IS the normal cutd binary, bundled beside the shell exe.

## Architecture

```
shellx-cut.exe (Tauri shell, this crate)
  ├─ setup hook: pick addr (reuse cutd on :6161 → spawn on :6161 → free port)
  ├─ spawn:  cutd.exe serve --addr <chosen-loopback-addr> --ui-dist <resources>/ui-dist
  ├─ ready-poll GET /api/verbs  → navigate webview to the chosen loopback URL
  ├─ IPC: ping, engine_status   (fallback/index.html is the honest airlock)
  └─ on window destroy: kill the spawned child (never an adopted external one)
```

The default is still 127.0.0.1:6161. If an unrelated process owns that port,
the shell uses a free loopback port and the bundled engine writes discovery so
`cutd mcp` and `cutd verb` proxy to the actual running server.

The shell spawns the normal `cutd` sidecar instead of linking the server as a
library. This keeps the headless binary and desktop engine behavior aligned;
see `src-tauri/src/lib.rs` for the port and process-lifecycle policy.

## Build (Windows installer, cross-compiled FROM WSL)

```
scripts/build-windows.sh [debug|release]   # repo root; see script header
```

Pipeline: build `ui/dist` → cargo-xwin cross-build `cutd.exe`
(x86_64-pc-windows-msvc) → copy to `src-tauri/binaries/` (externalBin) →
`cargo tauri build --runner cargo-xwin` → NSIS setup exe under
`src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/`.

Prerequisites:
`cargo-xwin`, `rustup target add x86_64-pc-windows-msvc`, `makensis`,
`cargo-tauri` (tauri-cli 2.11.2), node/npm for the UI build.

## Runtime dependencies (honest, per-verb)

The app launches self-contained. Verbs that orchestrate external tools report
actionable errors when those are missing on the host:

| Surface | Needs | Missing → |
| --- | --- | --- |
| media probe/proxy/render | ffmpeg/ffprobe on PATH | per-verb error envelope |
| transcribe/perception | Python sidecar (app/perception/py) | per-verb error envelope |

## Icons

`src-tauri/icons/` is generated from `branding/shellx-cut-icon.svg` via
`cargo tauri icon`; see
`src-tauri/icons/ICON_SIZES.md`.
