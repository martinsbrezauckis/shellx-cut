#!/usr/bin/env bash
# silent-profile-check.sh — silent-footage receipt-profile scenario
# (silent-screen profile regression harness).
#
# Proves on REAL measured facts that the check battery's footage profiles
# are honest: a correct silent screen-demo clip
#   - FAILS under talking_head (the silent-screen regression case), and
#   - PASSES under silent_screen_demo with every waiver RECORDED in the
#     receipt (waived_by_profile — never silently dropped), and
#   - is PROPOSED (not applied) as silent_screen_demo by auto-detect.
#
# Steps: (1) generate testdata/silent_screen.mp4 (pure ffmpeg, ~2s),
# (2) run the gated integration test, which spawns the real python sidecar
# (whisperX/silero/scenedetect/ebur128 — needs app/perception/py/.venv).
#
# Dependencies: ffmpeg/ffprobe, cargo, the perception venv.
# Primary callers: developers; CI once a venv-provisioned runner exists.
# Exit: nonzero on any failure (cargo test propagates).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "[silent-profile-check] generating silent screen clip..." >&2
bash "$ROOT/scripts/make-test-assets.sh" --silent-only

echo "[silent-profile-check] running profile integration test (real sidecar)..." >&2
cargo test --manifest-path "$ROOT/app/Cargo.toml" -p cut-perception \
  --test silent_profile -- --ignored --nocapture

echo "[silent-profile-check] PASS" >&2
