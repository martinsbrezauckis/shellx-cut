# Building ShellX Cut from source

Role: the build-from-source and dev-environment guide for contributors and
agents on a fresh machine. Companion docs: `docs/public/FEATURES.md` (what the app
does), `docs/public/DEBUG_API.md` (how to drive it), and
`docs/public/FEATURE_SURFACE_CONTRACT.md` (which public surfaces a supported
feature exposes). The verb contract lives in `schema/verbs.json`.

## Prerequisites

| Tool | Needed for | Notes |
|---|---|---|
| Rust (stable, via [rustup](https://rustup.rs)) | the engine (`cutd`) and all crates | edition 2021 workspace; CI and release builds use rustc 1.99.0 |
| Node.js + npm | the UI bundle and the JS test harnesses | CI uses Node 22.16.0 |
| ffmpeg + ffprobe | media probe, proxies, rendering | on `PATH`, or point at a dir with `SHELLX_CUT_FFMPEG_DIR`; a source-built `cutd` (and every test binary) in the cargo target folder uses the first FFmpeg in the search order (unless `SHELLX_CUT_FFMPEG` or a saved Choose ffmpeg setting names one), and `SHELLX_CUT_FFMPEG_AUTO=1` turns on the installed engine's automatic hardware pick; the consented `system.fetch_tool` downloader installs a separate GPL runtime from its publisher on Windows/Linux x86_64 and Apple silicon (the macOS build has no vid.stab); Intel Macs need a compatible local install; FFmpeg is not bundled in Cut installers |
| `jq`, `curl` | schema checks, examples, and `verbargs-sync.sh` | standard on most systems |

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
capture is not exercised at runtime. A cached Cargo target can hide missing
PipeWire, XInput/XTest, or Clang resource headers, so contributors should
occasionally verify a clean-target build.

Installing the Linux packages needs no extra steps on a supported system. The
`.deb` and `.rpm` declare every library the shell and the engine link (the
webview, GTK, ALSA, PipeWire, GStreamer, X11 and XTest libraries) plus the
GStreamer tools and PipeWire plugin the screen recorder runs, and a minimum libc
of glibc 2.39 (older systems refuse the install instead of failing at start).
The supported distributions are Ubuntu 24.04 or newer, Debian 13 or newer and
Fedora 43 or newer. The `.rpm` names Fedora packages (for example
`pipewire-gstreamer`), so other RPM-based distributions such as openSUSE are not
supported. The glibc of the machine that builds the packages sets the minimum
the binaries need: build release Linux packages on Ubuntu 24.04 (glibc 2.39).
`scripts/build-linux.sh` reads every shipped binary and fails when a linked
library is not declared or the binaries need a newer glibc than the packages
declare. The H.264/AAC decoders for preview
and the x264 encoder for screen recording come from GStreamer packages that the
package manager installs as recommendations (`gstreamer1.0-plugins-bad` and
`-ugly` on Debian and Ubuntu, where they are in `universe`). If a system left
them out, Doctor names the missing GStreamer element and the package that
provides it. On Fedora, x264enc comes from RPM Fusion. The packages also declare
`tar`, `xz` and `gzip`: the in-app FFmpeg download is a `.tar.xz` that `tar`
unpacks through `xz`, and it checks for them before downloading. An engine
started by hand from the package (`/usr/bin/cutd serve --headless`) finds its
packaged agent documents, UI and captions payload from its own location; the
`SHELLX_CUT_AGENT_DOCS_DIR` and `SHELLX_CUT_SIDECAR_DIR` variables only override
that.

Optional, feature-gated (the app degrades honestly without them):

- **Python 3.12 + [uv](https://docs.astral.sh/uv/)** — the perception sidecar
  (transcription, captions, scene detection, reframe). Installable from inside
  the app via `system.setup_perception`; nothing is bundled. In a source build
  the sidecar uses `SHELLX_CUT_PYTHON` when set, else the `.venv` of its
  perception folder (`app/perception/py/.venv`), else a `python3`/`python.exe`
  on PATH only when it answers as Python 3 within 5 seconds (the Microsoft
  Store shortcut and Apple's command-line stub are skipped), else the app-data
  environment that `system.setup_perception` creates. An installed app never
  uses a Python found on PATH.
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

The desktop shell (Tauri) lives under `app/desktop/`. The public GitHub
**Smoke build** workflow shows the portable, unsigned packaging sequence on
Windows, macOS, and Linux. Local contributor builds should likewise disable
updater artifacts and produce unsigned packages.

The workflow compiles `cutd` from the checked-out source and
stages that exact host binary as Tauri's target-qualified external sidecar before
making its unsigned, updater-free workflow artifacts. It is packaging evidence,
not a signed or updater-capable release.

The desktop resource map is the canonical manifest for documentation shipped
inside an installer. Package checks fail if a source is absent and compare the
bundled documents with the checked-out source.

For an unsigned macOS package, run the Smoke build sequence from the repository
root after installing the prerequisites above:

```bash
npm --prefix ui ci --ignore-scripts
npm --prefix ui run build
cargo build --release --manifest-path app/Cargo.toml -p server --bin cutd
node scripts/release/stage-tauri-cutd.mjs
(cd app/desktop && npx --yes @tauri-apps/cli@2.12.1 build --config '{"bundle":{"createUpdaterArtifacts":false}}')
```

### Official release packages

GitHub Releases provide the signed Windows installer, notarized macOS disk
image, Linux packages, updater payloads, and checksums. Signing and publication
are maintainer operations and are deliberately outside the public source-build
workflow. Contributors can validate the same product source through the
unsigned Smoke build without access to release credentials.

## Verification gates (the definition of done)

Run these before claiming a change works; CI-grade, all exit 0 on green:

```bash
cargo fmt --all --check --manifest-path app/Cargo.toml
cargo fmt --check --manifest-path app/desktop/src-tauri/Cargo.toml
cargo clippy --workspace --all-targets --manifest-path app/Cargo.toml -- -D warnings
cargo test --workspace --manifest-path app/Cargo.toml
node scripts/schema-validation-parity.mjs    # identical schema failures on dispatch/REST/CLI/MCP
scripts/verbargs-sync.sh                     # every verb has a typed UI client binding
node scripts/generate-verbs.mjs --check      # schema/verbs.json matches its fragments
node scripts/generate-verb-contract.mjs --check
node scripts/generate-requirements-docs.mjs --check   # SKILL/reference "what needs what" matches schema/requirements.json
npm --prefix ui run build
npm --prefix ui run test:lib
node scripts/check-published-test-inventory.mjs
```

`npm --prefix ui run test:lib` includes browser tests that need Playwright's
Chromium (`npx --prefix ui playwright install chromium`). The perception
helpers also have Python unit tests (`app/perception/py/*_test.py`). Changing a
Rust or npm dependency makes `scripts/public-tests/third-party-notice.test.mjs`
fail until `THIRD-PARTY-NOTICES.txt` is regenerated by the maintainers.

`docs/public/FEATURE_SURFACE_CONTRACT.md` explains the observable human, agent,
and debug surfaces that the gates above validate.

Native/UI test environments must isolate both kinds of user data. Set
`SHELLX_CUT_HOME` for the internal project index, global Library, and mutable
tool preferences exercised by the tests (currently FFmpeg and transcription
model choices), and set `SHELLX_CUT_PROJECTS_DIR` for the user-visible managed
`.cutproj` directory. The remembered export folder is part of the first: it is
kept in `SHELLX_CUT_HOME` (`export-folder.json`) with the project index.
Using only the first variable still lets `project.list` discover projects in the
real default projects folder.
