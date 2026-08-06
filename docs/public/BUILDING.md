# Building ShellX Cut from source

Role: the build-from-source and dev-environment guide for contributors and
agents on a fresh machine. Companion docs: `docs/public/FEATURES.md` (what the app
does), `docs/public/DEBUG_API.md` (how to drive it), `docs/public/FEATURE_CHANGE_WORKFLOW.md`
(how to change it). The verb contract lives in `schema/verbs.json`.

## Prerequisites

| Tool | Needed for | Notes |
|---|---|---|
| Rust (stable, via [rustup](https://rustup.rs)) | the engine (`cutd`) and all crates | edition 2021 workspace; tested with rustc 1.94 |
| Node.js + npm | the UI bundle and the JS test harnesses | tested with Node 24; anything recent works |
| ffmpeg + ffprobe | media probe, proxies, rendering | on `PATH`, or point at a dir with `SHELLX_CUT_FFMPEG_DIR`; the consented `system.fetch_tool` downloader installs a separate BtbN GPL runtime on Windows/Linux, while macOS requires a compatible local install; FFmpeg is not bundled in Cut installers |
| `jq`, `curl` | the shell gate scripts (`coverage-audit.sh`, `verbargs-sync.sh`) | standard on most systems |

Linux builds also need the native development headers used by capture and
audio backends. On Ubuntu 24.04, install the complete workspace set with:

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config clang libclang-dev \
  libasound2-dev libpipewire-0.3-dev libdbus-1-dev \
  libx11-dev libxi-dev libxtst-dev
```

These packages are required to compile the Linux workspace even when screen
capture is not exercised at runtime. In particular, a cached Cargo target can
hide missing PipeWire, XInput/XTest, or Clang resource headers; release checks
should include a clean-target build on a Linux rig.

Optional, feature-gated (the app degrades honestly without them):

- **Python 3.12 + [uv](https://docs.astral.sh/uv/)** — the perception sidecar
  (transcription, captions, scene detection, reframe). Installable from inside
  the app via `system.setup_perception`; nothing is bundled.
- **espeak-ng** — only for `scripts/make-test-assets.sh` (generates `testdata/`
  with known ground truth).

macOS contributors need full Xcode, not only Command Line Tools, to link and
run the complete workspace test matrix including `record-capture`. Media tests
and runtime features also require an FFmpeg build with the filters they exercise:
`ass` for caption burn-in, `zscale` for managed color conversions, and
`vidstabdetect`/`vidstabtransform` for stabilization. `system.doctor` reports
missing runtime capabilities. Homebrew's regular `ffmpeg` formula does not
provide all of them; install the keg-only complete build with
`brew install ffmpeg-full`. Cut detects
`/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg` on Apple Silicon and
`/usr/local/opt/ffmpeg-full/bin/ffmpeg` on Intel automatically.

## Build and run

```bash
# 1. UI bundle (ui/dist is gitignored — cutd serves whatever was last built)
cd ui && npm ci && npm run build && cd ..

# 2. Engine + server (one binary: cutd)
cargo build --release -p server --manifest-path app/Cargo.toml
# → app/target/release/cutd

# Or let the dev script do both and serve at http://127.0.0.1:6161
scripts/dev.sh              # builds ui/, runs cutd serving it
scripts/dev.sh --headless   # API only, no UI build (background-run friendly)
```

For UI hot-reload, run `npm run dev` inside `ui/` against a separately running
`cutd` (the Vite dev server proxies to it).

The desktop shells (Tauri) live under `app/desktop/`. Use
`scripts/build-windows.sh` for NSIS, `scripts/build-macos.sh` for the signed app
and DMG, and `scripts/build-linux.sh` on native Linux for `.deb` and `.rpm`
packages. Do not run the Linux package build under WSL. Code signing is opt-in
and off by default; dev builds are unsigned.

`scripts/lib/agent-docs.mjs` is the canonical manifest for documentation shipped
inside an installer. Every platform build fails if a manifest source is absent;
the macOS, Debian, and RPM builders also extract/inspect their package and compare
all manifest files byte-for-byte. Windows proves the same property after a clean
install with `scripts/windows/install-cut-current.ps1`, including exact bytes
served by the installed engine's `/api/agent-doc/*path` endpoint.

### Signed updater feed

Windows and Apple-silicon macOS release builds produce the Tauri updater
artifact and its minisign `.sig` beside the normal installer/DMG. A signed
release build fails if those updater files are absent or stale. Consolidate the
fresh files under one private staging root before creating a GitHub release:

```text
~/shellx-cut-builds/v0.6.106/
├── windows/
│   ├── ShellX Cut_0.6.106_x64-setup.exe
│   └── ShellX Cut_0.6.106_x64-setup.exe.sig
└── macos/
    ├── ShellX Cut.app.tar.gz
    └── ShellX Cut.app.tar.gz.sig
```

Generate the static Tauri feed only after both platform artifacts have passed
their native signing and installed-app qualification:

```bash
node scripts/release/generate-updater-manifest.mjs \
  --artifact-root "$HOME/shellx-cut-builds/v0.6.106"
```

The command cryptographically verifies each `.sig` against the updater public
key embedded in `tauri.conf.json`, requires both release platforms, binds every
download URL to tag `v0.6.106`, and then writes `latest.json` plus a local
`updater-manifest-verify.json` exact-source receipt. Retain the verification
receipt outside the published assets. Never publish a hand-written or
partially populated feed.

A complete GitHub release carries the DOWNLOAD assets as well as the updater
feed — the updater set alone leaves the macOS DMG and both Linux packages
missing from every download surface (product page, `/download/cut/*` routes,
release page). Upload all of:

```text
ShellX Cut_0.6.106_x64-setup.exe        (+ .sig)   Windows installer + updater
ShellX Cut_0.6.106_aarch64.dmg                     macOS download
ShellX Cut.app.tar.gz                   (+ .sig)   macOS updater payload
ShellX Cut_0.6.106_amd64.deb                       Debian/Ubuntu
ShellX Cut-0.6.106-1.x86_64.rpm                    Fedora/RHEL-compatible
latest.json                                        updater feed
SHA256SUMS.txt                                     hashes of every asset above
```

Before the first public release the configured
`releases/latest/download/latest.json` URL returns no usable manifest and the
launch check stays quiet. An installed older build can exercise the real
download/install/restart path only after the signed release assets are public;
retain that post-publish canary receipt separately from pre-release source and
native-dialog coverage.

## Verification gates (the definition of done)

Run these before claiming a change works; CI-grade, all exit 0 on green:

```bash
cargo fmt --all --check --manifest-path app/Cargo.toml
cargo fmt --check --manifest-path app/desktop/src-tauri/Cargo.toml
cargo clippy --workspace --all-targets --manifest-path app/Cargo.toml -- -D warnings
cargo test --workspace --manifest-path app/Cargo.toml
scripts/coverage-audit.sh                    # every verb answers on REST + MCP
node scripts/schema-validation-parity.mjs    # identical schema failures on dispatch/REST/CLI/MCP
scripts/verbargs-sync.sh                     # every verb has a typed UI client binding
node --test scripts/public-tests/feature-contract.test.mjs   # docs/skill/README stay synced to the schema
node ui/public-tests/full-coverage-verify.mjs --coverage-check # every verb UI-covered or explicitly non-UI
scripts/e2e.sh                               # full pipeline end-to-end
```

With an isolated app stack running, also execute the live accessibility surface
gate. It creates and cleans up its own disposable project, opens every surface
in the shared `ui.open` registry, and fails on unnamed controls/dialogs,
unreachable surfaces, or duplicate DOM IDs:

```bash
SWEEP_APP=http://127.0.0.1:6161 \
SWEEP_CUTD=http://127.0.0.1:6161 \
npm --prefix ui run verify-accessibility-surfaces
```

`docs/public/FEATURE_CHANGE_WORKFLOW.md` explains which surfaces every feature change
must touch; the gates above are what enforce it.

Native/UI test rigs must isolate both kinds of user data. Set
`SHELLX_CUT_HOME` for the internal project index, global Library, and mutable
tool preferences exercised by the rig (currently FFmpeg and transcription
model choices), and set `SHELLX_CUT_PROJECTS_DIR` for the user-visible managed
`.cutproj` directory.
Using only the first variable still lets `project.list` discover projects in the
real default projects folder.

The final pre-release UI qualification is stricter than the development gates:
one unfiltered installed-app all-actions run must pass on macOS, Windows, and
native Linux. See `docs/public/TEST_RIGS.md`; candidate, filtered, or development-server
receipts do not satisfy that final gate. Run the host wrappers with
`--clean-after` so rebuildable build trees and isolated test state do not remain
on the three test machines; the runner retains the evidence needed for audit.
On Windows, use `scripts/windows-installed-full-coverage.mjs`; it drives the
installed shipping artifact through a validated WebView2 CDP port and runs its
verifier under native Windows Node even when the orchestration command starts
from WSL. Each run also receives an isolated, cleanup-owned WebView2 profile.
