#!/usr/bin/env python3
"""matte_runner.py — the SHIPPABLE local matte runtime (one-shot CLI).

Role: bake a clip's straight-alpha matte with RVM via onnxruntime, on the user's
own machine. This is the LOCAL form of the matting runtime — cutd spawns it
per-bake exactly like the perception sidecar (`python instruments.py …`), so it's
invisible plumbing inside the one ShellX Cut app (no HTTP service, no port, no
second window). The HTTP variant (`matte_service.py`) stays as the optional
dev/remote-GPU path; this one-shot CLI is the default user-side runtime.

Why this shape:
- One-shot CLI = the SAME pattern as the perception sidecar (consistency); cutd
  manages the lifecycle, the user never sees it.
- onnxruntime (not torch) = PORTABLE across CPU, CoreML (Mac), and CUDA without
  requiring a torch/CUDA install. The whole runtime = a 14 MB
  RVM ONNX model + onnxruntime; small enough to fetch on consent.
- Fully STREAMING (ffmpeg rawvideo pipe in → FFV1 lossless gray alpha pipe out),
  so GPU/CPU memory is bounded (only RVM's small recurrent state lives across
  frames) and there's no PNG-on-disk churn. Alpha is LOSSLESS (a lossy matte
  fringes the composite edges). CFR-pinned so the matte is frame-for-frame
  aligned with the clip (the renderer alphamerges it).

Contract (matches what server/matte.rs expects, same as the HTTP body+header):
  python matte_runner.py <in_video> <out_alpha.mkv> [--model PATH]
                         [--downsample R] [--providers cpu|coreml|cuda,...]
  → writes the FFV1 gray alpha to <out_alpha.mkv>; prints ONE line of JSON stats
    (frames, fps, width, height, downsample_ratio, coverage_mean, cov_min,
    cov_max, temporal_flicker) to STDOUT. Non-zero exit + stderr on failure.

Model: rvm_mobilenetv3_fp32.onnx (PeterL1n/RobustVideoMatting, opset 12, 14 MB).
RVM is GPL — we ship integration, the USER installs the model (fetch-on-consent /
autodetect / browse-to-existing, the ffmpeg pattern). Default model path =
$MATTE_MODEL or rvm.onnx beside this script.

Dependencies: onnxruntime, numpy, ffmpeg/ffprobe on PATH. Primary caller:
server/matte.rs (local-CLI transport).
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

# Cold Windows installs keep ffmpeg/ffprobe in the app tools dir, not PATH.
# cutd forwards that directory as SHELLX_CUT_FFMPEG_DIR; mirror instruments.py.
_ff_dir = os.environ.get("SHELLX_CUT_FFMPEG_DIR", "").strip()
if _ff_dir:
    _candidates = [_ff_dir, os.path.join(_ff_dir, "bin")]
    _present = [d for d in _candidates if os.path.isdir(d)]
    if _present:
        os.environ["PATH"] = os.pathsep.join(_present + [os.environ.get("PATH", "")])

FFMPEG_BIN = shutil.which("ffmpeg") or "ffmpeg"
FFPROBE_BIN = shutil.which("ffprobe") or "ffprobe"

import numpy as np
import onnxruntime as ort
from PIL import Image
from safe_numbers import finite_number


def ffprobe(path: str) -> tuple[float, int, int]:
    """(fps, width, height) of the first video stream."""
    out = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=r_frame_rate,width,height",
         "-of", "json", path],
        capture_output=True, text=True, check=True,
    )
    st = json.loads(out.stdout)["streams"][0]
    num, den = st["r_frame_rate"].split("/")
    fps = finite_number(num) / finite_number(den) if finite_number(den) else finite_number(num)
    return fps, int(st["width"]), int(st["height"])


def auto_dsr(w: int, h: int) -> float:
    """RVM downsample ratio — aim the longer side at ~512 px, clamp [0.25, 1.0].
    720p→0.4, 1080p→~0.27 (RVM's own HD guidance)."""
    return finite_number(min(1.0, max(0.25, 512.0 / max(w, h))))


def pick_providers(requested: str | None) -> list[str]:
    """Resolve the onnxruntime execution providers, best-available first. The
    runtime is portable: CUDA (NVIDIA) → CoreML (Mac) → CPU (everywhere)."""
    avail = set(ort.get_available_providers())
    if requested:
        want = {"cpu": "CPUExecutionProvider", "coreml": "CoreMLExecutionProvider",
                "cuda": "CUDAExecutionProvider"}
        chosen = [want[r.strip().lower()] for r in requested.split(",") if r.strip().lower() in want]
        chosen = [p for p in chosen if p in avail]
        if chosen:
            return chosen + (["CPUExecutionProvider"] if "CPUExecutionProvider" not in chosen else [])
    order = ["CUDAExecutionProvider", "CoreMLExecutionProvider", "CPUExecutionProvider"]
    return [p for p in order if p in avail] or ["CPUExecutionProvider"]


def wait_child(proc: subprocess.Popen, label: str, timeout_s: int = 30) -> None:
    try:
        proc.wait(timeout=timeout_s)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)
        raise SystemExit(f"matte_runner: {label} did not exit within {timeout_s}s")
    if proc.returncode != 0:
        stderr = ""
        if proc.stderr:
            stderr = proc.stderr.read().decode("utf-8", "replace").strip()[-600:]
        detail = f": {stderr}" if stderr else ""
        raise SystemExit(f"matte_runner: {label} failed with exit {proc.returncode}{detail}")


def run(in_path: str, out_path: str, model: str, dsr: float | None, providers: list[str]) -> dict:
    fps, w, h = ffprobe(in_path)
    if dsr is None:
        dsr = auto_dsr(w, h)
    frame_size = w * h * 3  # rgb24

    sess = ort.InferenceSession(model, ort.SessionOptions(), providers=providers)
    rec = [np.zeros([1, 1, 1, 1], dtype=np.float32) for _ in range(4)]
    dsr_arr = np.array([dsr], dtype=np.float32)

    # Decoder: source → CFR rawvideo rgb24 (CFR pins frame↔matte 1:1).
    dec = subprocess.Popen(
        [FFMPEG_BIN, "-v", "error", "-i", in_path,
         "-fps_mode", "cfr", "-r", f"{fps}", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    # Encoder: rawvideo gray → LOSSLESS FFV1 .mkv at the source fps.
    enc = subprocess.Popen(
        [FFMPEG_BIN, "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "gray",
         "-s", f"{w}x{h}", "-framerate", f"{fps}", "-i", "-",
         "-c:v", "ffv1", "-pix_fmt", "gray", out_path],
        stdin=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    covs: list[float] = []
    flick: list[float] = []
    prev = None
    n = 0
    try:
        while True:
            buf = dec.stdout.read(frame_size)
            if len(buf) < frame_size:
                break
            frame = np.frombuffer(buf, dtype=np.uint8).reshape(h, w, 3).astype(np.float32) / 255.0
            src = np.transpose(frame, (2, 0, 1))[None]  # 1,3,H,W
            fgr, pha, *rec = sess.run(
                [], {"src": src, "r1i": rec[0], "r2i": rec[1], "r3i": rec[2],
                     "r4i": rec[3], "downsample_ratio": dsr_arr}
            )
            a = pha[0, 0]
            covs.append(finite_number((a > 0.5).mean()))
            if prev is not None:
                flick.append(finite_number(np.abs(a - prev).mean()))
            prev = a
            enc.stdin.write((a * 255.0).astype(np.uint8).tobytes())
            n += 1
    finally:
        if dec.stdout:
            dec.stdout.close()
        wait_child(dec, "decoder")
        if enc.stdin:
            enc.stdin.close()
        wait_child(enc, "encoder")

    if n == 0:
        raise SystemExit("matte_runner: no decodable video frames")
    return {
        "frames": n,
        "fps": round(fps, 6),
        "width": w,
        "height": h,
        "downsample_ratio": round(dsr, 4),
        "coverage_mean": round(finite_number(np.mean(covs)), 5) if covs else 0.0,
        "cov_min": round(finite_number(np.min(covs)), 5) if covs else 0.0,
        "cov_max": round(finite_number(np.max(covs)), 5) if covs else 0.0,
        "temporal_flicker": round(finite_number(np.mean(flick)), 6) if flick else 0.0,
        "providers": providers,
    }


def first_frame_mask(in_path: str, out_png: str, model: str, threshold: float,
                     providers: list[str]) -> dict:
    """Run RVM on ONLY frame 0 and write a BINARY subject mask PNG (white=keep).

    This seeds the MatAnyone2 premium runtime, which REQUIRES a first-frame mask
    (RVM produces it automatically, zero-click — the bootstrap path; a SAM2 click
    is the later 'pick which subject' upgrade). MatAnyone2 erodes/dilates the seed,
    so a coarse RVM threshold is a fine seed. Runs in the onnx/perception venv the
    user already has for RVM."""
    fps, w, h = ffprobe(in_path)
    dsr = auto_dsr(w, h)
    sess = ort.InferenceSession(model, ort.SessionOptions(), providers=providers)
    rec = [np.zeros([1, 1, 1, 1], dtype=np.float32) for _ in range(4)]
    dec = subprocess.Popen(
        [FFMPEG_BIN, "-v", "error", "-i", in_path, "-frames:v", "1",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    buf = dec.stdout.read(w * h * 3)
    dec.stdout.close(); wait_child(dec, "first-frame decoder")
    if len(buf) < w * h * 3:
        raise SystemExit("matte_runner: could not read frame 0 for the seed mask")
    frame = np.frombuffer(buf, dtype=np.uint8).reshape(h, w, 3).astype(np.float32) / 255.0
    src = np.transpose(frame, (2, 0, 1))[None]
    _fgr, pha, *_ = sess.run(
        [], {"src": src, "r1i": rec[0], "r2i": rec[1], "r3i": rec[2],
             "r4i": rec[3], "downsample_ratio": np.array([dsr], dtype=np.float32)})
    a = pha[0, 0]
    mask = (a > threshold).astype(np.uint8) * 255
    Image.fromarray(mask, "L").save(out_png)
    return {"width": w, "height": h, "coverage": round(finite_number((mask > 0).mean()), 5),
            "threshold": threshold}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("in_video")
    ap.add_argument("out_alpha", nargs="?", default=None,
                    help="FFV1 gray alpha out (omit when --first-frame-mask is set)")
    ap.add_argument("--model", default=os.environ.get(
        "MATTE_MODEL", str(Path(__file__).resolve().parent / "rvm.onnx")))
    ap.add_argument("--downsample", type=float, default=None)
    ap.add_argument("--providers", default=os.environ.get("MATTE_PROVIDERS"))
    ap.add_argument("--first-frame-mask", default=None,
                    help="seed mode: write a binary first-frame subject mask PNG here (for MatAnyone2)")
    ap.add_argument("--threshold", type=float, default=0.43,
                    help="alpha threshold for the binary seed mask (first-frame-mask mode)")
    a = ap.parse_args()
    if not Path(a.model).is_file():
        print(f"matte_runner: model not found: {a.model}", file=sys.stderr)
        return 2
    providers = pick_providers(a.providers)
    if a.first_frame_mask:
        stats = first_frame_mask(a.in_video, a.first_frame_mask, a.model, a.threshold, providers)
    else:
        if not a.out_alpha:
            print("matte_runner: out_alpha is required unless --first-frame-mask is set", file=sys.stderr)
            return 2
        stats = run(a.in_video, a.out_alpha, a.model, a.downsample, providers)
    print(json.dumps(stats))  # ONE line of JSON on stdout — the caller parses it
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
