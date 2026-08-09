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
node scripts/generate-verb-contract.mjs --check
npm --prefix ui run build
npm --prefix ui run test:lib
node --test scripts/public-tests/*.test.mjs
```

`docs/public/FEATURE_CHANGE_WORKFLOW.md` explains which surfaces every feature change
must touch; the gates above are what enforce it.

Native/UI test environments must isolate both kinds of user data. Set
`SHELLX_CUT_HOME` for the internal project index, global Library, and mutable
tool preferences exercised by the tests (currently FFmpeg and transcription
model choices), and set `SHELLX_CUT_PROJECTS_DIR` for the user-visible managed
`.cutproj` directory.
Using only the first variable still lets `project.list` discover projects in the
real default projects folder.
