# ShellX Cut — UI surface sweep (release check)

`surface-sweep.mjs` is the **reusable UI release check**. It does NOT just open
panels — it **performs the real action and asserts the desired result landed in
cutd's project state** (the source of truth), each with a hang-timeout so a frozen
or no-effect action fails loudly. The whole point: catch wiring malfunctions, not
"the panel rendered."

## What it covers

**Editing workflow — action → assert EFFECT in cutd state (the load-bearing part):**
- **wf-clip-present** — import + insert put a clip on the timeline
- **wf-split** — split-at-playhead actually **raises the clip count**
- **wf-grade** — Grade → Apply actually **records an `edit.grade` op**
- **wf-ripple-delete** — Delete actually **removes the clip**
- **wf-undo** — Ctrl+Z actually **reverts** the delete
- **wf-transport-seek** — frame-step actually **moves the playhead** (no proxy)

**Surface health — open every surface, assert it renders (catch crashes):**
- boot (title + brand + **0 non-favicon console errors**), Export menu (15 opts),
  left tabs, ⌘K palette (open + precise fuzzy filter), **all 12 drawers** (right
  titles), drawer scrim-close, keymap overlay.

## Prerequisites (a running dev stack)

```bash
# 1. cutd headless (the engine) on :6161
./app/target/release/cutd serve --headless --addr 127.0.0.1:6161 &
# 2. vite dev (the UI) on :5173 — proxies /api -> 6161
cd ui && npm run dev &
```

The sweep sets up its OWN demo project (a talking-head clip on the timeline) over
the cutd API if one isn't already loaded — so it's self-contained.

## Run

```bash
cd ui && npm run sweep          # or: node public-tests/surface-sweep.mjs
```

- Exit 0 = all green; non-zero if any check **FAILED** (SKIPs don't fail the run).
- Evidence (gitignored, regenerated each run): `ui/public-tests/__evidence__/`
  - `NN-<name>.png` — one screenshot per surface
  - `report.md` / `report.json` — the pass/fail table

## Accessibility surface gate

`accessibility-surface-verify.mjs` creates a disposable project, opens every
surface in the shared `ui.open` registry, and reads Chromium's accessibility
tree. It fails when a focusable control or dialog has no programmatic name,
when a registered surface cannot be confirmed open, or when the live document
contains duplicate IDs.

```bash
cd ui
SWEEP_APP=http://127.0.0.1:6171 \
SWEEP_CUTD=http://127.0.0.1:6171 \
npm run verify-accessibility-surfaces
```

## Keeping it fresh

When you add a surface (a new drawer, a new mode, the Record flagship), **add a
check here in the same change** — this file is the contract that the UI works end
to end. Drawers, including Music Bed, use the shared `.cd-*` structure so the
sweep can apply the same layout, action, and accessibility expectations.

## Focused Generate Gates

These prove result evidence for the Generate module instead of only checking that
tabs can be clicked:

```bash
CUTD_GENERATE_STORYBOARD_ADAPTER=$PWD/ui/public-tests/fixtures/generate-storyboard-adapter.py \
  app/target/debug/cutd serve --addr 127.0.0.1:6178 --headless
cd ui
CUTD_DEV_TARGET=http://127.0.0.1:6178 npm run dev -- --host 127.0.0.1 --port 5178
SWEEP_CUTD=http://127.0.0.1:6178 SWEEP_APP=http://127.0.0.1:5178 \
  node tests/generate-storyboard-ui-verify.mjs
```

`generate-storyboard-ui-verify.mjs` opens the Storyboard tab through
`ui.open{panel:"generate-storyboard"}`, confirms `ui.state` reports
`generate:storyboard`, then proves plan scene rows, preview PNGs, insert
checkpoint/clip evidence, project state, and cleanup revert.

## Focused Timeline Save/Drop Gate

`timeline-save-drop-verify.mjs` proves the Timeline toolbar and Assets drop
bridge with actual result evidence:

```bash
SWEEP_CUTD=http://127.0.0.1:6161 SWEEP_APP=http://127.0.0.1:6161 \
  node ui/public-tests/timeline-save-drop-verify.mjs
```

It creates a temporary project, imports `testdata/talking_head.mp4`, clicks the
real `Save to Assets` and `GIF` toolbar actions, verifies the returned files and
imported assets, then dispatches the same `cut:asset-dragmove` / `cut:asset-drop`
events emitted by the Assets tray and verifies a new timeline line appears in
`project.state`.

## macOS Assets/Library Drag Gate

The test-only Tauri WebDriver build can exercise the WKWebView drag bridge with
real media on a Mac host:

```bash
node scripts/macos-wdio-track-controls.mjs \
  --suite media-drag \
  --host your-macos-ssh-alias \
  --clip /path/on/mac/talking_head.mp4 \
  --library-clip /path/on/mac/insert_clip.mp4 \
  --clean-after
```

The gate proves Assets placement, pointer-cancel cleanup, pointer placement,
and Library import-plus-placement through `project.state`. It uses the embedded
driver's mouse-event lane by default because that driver intentionally emits
`MouseEvent`, not `PointerEvent`. `--native-input` switches to the Swift
`CGEvent` fixture when the invoking Mac process has Accessibility input access.
Shipping builds still reject the `webdriver-test` feature.

## Native exhaustive action candidate gates

The shared `full-coverage-verify.mjs` scenarios can run inside the native
WKWebView/WebKitGTK shell instead of Chromium. These commands build a
test-instrumented candidate from the current source, isolate the project index,
Library, and managed projects directory, then write per-action screenshots and
a machine-readable result receipt:

```bash
# macOS / WKWebView
node scripts/macos-wdio-track-controls.mjs \
  --suite full-coverage --host your-macos-ssh-alias

# Linux / WebKitGTK under Xvfb
node scripts/linux-wdio-full-coverage.mjs --host your-linux-ssh-alias
```

Use `--section library,drawers`, `--only <action substring>`, or `--trace` only
for diagnosis. A pre-release all-actions run must be unfiltered. These are
test-feature candidate receipts (`installedApp:false`), not proof of a
shipping/installed binary; the final surface matrix pairs them with the exact
installed-artifact gates. Native file/folder/save pickers are deliberately not
opened by the DOM sweep: an OS-modal chooser cannot be closed by the WebView
driver and would invalidate every later action. Those rows remain visible as
`CLICK=N/A` until the paired installed OS-action receipt proves open plus
select-or-cancel on that host.

## Final installed all-actions gate

Immediately before release, run one complete, unfiltered installed-app
qualification on each supported desktop surface: macOS, Windows, and native
Linux. This is a single final release gate, not a substitute for focused
development checks:

```bash
FCV_REQUIRE_FULL=1 \
FCV_INSTALLED_APP=1 \
FCV_UI_DRIVER=<native-installed-driver> \
FCV_ACTION_MANIFEST=ui/public-tests/full-ui-action-manifest.json \
node scripts/release/full-coverage-gate.mjs --final-all-actions
```

Do not set `FCV_SECTION`, `FCV_ONLY`, or `FCV_NO_AGENT` for this run. A surface
passes only when the installed artifact matches the candidate, the observed
action registry exactly matches the committed manifest, and every action row
passes `PRESENT`, `RENDER`, `CLICK`, and `RESULT`. Candidate-only, filtered,
development-server, or split partial receipts do not satisfy this gate.

During remediation, filtered diagnostic receipts can be unioned against the
same committed manifest to identify the exact actions still lacking runtime
evidence:

```bash
node scripts/release/merge-runtime-action-receipts.mjs \
  --out /path/to/runtime-action-union.json \
  /path/to/results-part-1.json /path/to/results-part-2.json
```

This union is a development diagnostic only. It cannot replace the one
unfiltered installed-app receipt required on each release surface.

The Windows installed WebView2 CDP adapter is accepted. Native Linux also has
an exact shipping-package path: `scripts/linux-wdio-full-coverage.mjs
--installed-final` builds the normal `.deb`, extracts and hashes that package,
then attaches official external Tauri/WebKit WebDriver without enabling
`webdriver-test`. macOS remains release-red because the official Tauri driver
has no external WKWebView backend; its candidate sweep is valuable diagnostic
coverage but cannot be relabeled as installed proof.

## macOS Composed Playback Gate

Use the same test-only native app to cover the graded playback regression:

```bash
node scripts/macos-wdio-track-controls.mjs \
  --suite composed-playback \
  --host your-macos-ssh-alias \
  --clip /path/on/mac/talking_head.mp4 \
  --clean-after
```

The gate applies a brightness grade to real media, proves the paused exact
frame has non-black pixels, starts Composed playback, verifies the live video
clock and grade filter, then pauses and proves a fresh exact composed frame is
loaded. The pixel check reads the same-origin poster in-page because macOS
WebDriver screenshots can omit accelerated media content.
