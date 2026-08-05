#!/usr/bin/env python3
"""matanyone_runner.py — the SHIPPABLE local MatAnyone2 matte runtime (one-shot CLI).

Role: the PREMIUM (opt-in) form of the matting runtime. Bakes a clip's
straight-alpha matte with **MatAnyone2** (NTU S-Lab, CVPR-2026) on the user's
own machine, given a FIRST-FRAME subject mask. Same plumbing shape as the RVM
runner (`matte_runner.py`) and the perception sidecar: cutd spawns it per-bake,
the user never sees a second window — one ShellX Cut app.

Why MatAnyone2 is the premium tier (vs the RVM default):
- TARGET-ASSIGNED: the first-frame mask picks WHICH subject to keep (RVM mattes
  "the human" automatically, no choice). The mask is seeded by RVM (zero-click
  bootstrap) or by a SAM2 click ("pick the subject") — the server makes it.
- Memory-propagation transformer → much better hair/edge detail + temporal
  stability on hard real-world footage. GPU/NVIDIA realistically (CPU = unusably
  slow); NON-COMMERCIAL (NTU S-Lab License 1.0) → fetch-on-consent, opt-in.

Why this shape (vs upstream `inference_matanyone2.py`):
- STREAMING ffmpeg rawvideo pipe in → FFV1 lossless gray alpha pipe out, exactly
  like the RVM runner. Upstream loads the WHOLE video into a RAM tensor and writes
  a LOSSY h264 alpha (quality=7) — both wrong for us: a lossy matte fringes the
  composite edges, and a long clip OOMs. MatAnyone2 is recurrent (one
  `processor.step()` per frame with an internal memory bank), so we CAN stream:
  only the small recurrent memory lives across frames.
- Output is LOSSLESS FFV1 gray at the SOURCE width/height/fps (CFR-pinned), so the
  renderer's `[fg][alpha]alphamerge` reads a frame-for-frame aligned matte — the
  exact same cache contract that the RVM runner writes (server/matte.rs is model-blind).

Contract (matches server/matte.rs `bake_local`, parallel to matte_runner.py):
  python matanyone_runner.py <in_video> <out_alpha.mkv> --mask <seed.png>
                             --model <matanyone2.pth>
                             [--max-size N] [--warmup 10] [--erode 10] [--dilate 10]
  → writes the FFV1 gray alpha to <out_alpha.mkv>; prints ONE line of JSON stats
    (frames, fps, width, height, coverage_mean, cov_min, cov_max,
     temporal_flicker, edge_softness, model, device) to STDOUT.
    Non-zero exit + stderr on failure.

Model: matanyone2.pth (PeiqingYang/MatAnyone2, ~141 MB; auto-downloaded by upstream
from its GitHub release, or fetched sha-pinned by system.setup_matte{model:matanyone}).

Dependencies: torch+torchvision (CUDA), the matanyone2 package + its inference deps
(kornia, hydra-core, einops, safetensors, opencv, av, numpy, Pillow), ffmpeg/ffprobe
on PATH. Primary caller: server/matte.rs (local-CLI transport, model=matanyone).
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
import torch
import torch.nn.functional as F
from PIL import Image
from safe_numbers import finite_number

Image.MAX_IMAGE_PIXELS = 200_000_000


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


def load_model(ckpt: str, device: torch.device):
    """Load MatAnyone2 + an InferenceCore. Imports are local so an import error is
    a clean non-zero exit with a useful message (not a top-level crash)."""
    from matanyone2.inference.inference_core import InferenceCore
    from matanyone2.utils.get_default_model import get_matanyone2_model
    model = get_matanyone2_model(ckpt, device)
    processor = InferenceCore(model, cfg=model.cfg)
    return processor


def load_seed_mask(mask_path: str, r_erode: int, r_dilate: int) -> np.ndarray:
    """First-frame subject mask → eroded/dilated uint8 (matanyone2's own kernels,
    so a COARSE seed — e.g. an RVM threshold — is fine; the model is robust to it)."""
    from matanyone2.utils.inference_utils import gen_dilate, gen_erosion
    m = np.array(Image.open(mask_path).convert("L"))
    if r_dilate > 0:
        m = gen_dilate(m, r_dilate, r_dilate)
    if r_erode > 0:
        m = gen_erosion(m, r_erode, r_erode)
    return m


def wait_child(proc: subprocess.Popen, label: str, timeout_s: int = 30) -> None:
    try:
        proc.wait(timeout=timeout_s)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)
        raise SystemExit(f"matanyone_runner: {label} did not exit within {timeout_s}s")
    if proc.returncode != 0:
        stderr = ""
        if proc.stderr:
            stderr = proc.stderr.read().decode("utf-8", "replace").strip()[-600:]
        detail = f": {stderr}" if stderr else ""
        raise SystemExit(f"matanyone_runner: {label} failed with exit {proc.returncode}{detail}")


def run(in_path: str, out_path: str, mask_path: str, ckpt: str,
        max_size: int, n_warmup: int, r_erode: int, r_dilate: int) -> dict:
    fps, w, h = ffprobe(in_path)
    frame_size = w * h * 3  # rgb24
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")

    processor = load_model(ckpt, device)

    # Processing size: downsample for VRAM when min side exceeds max_size; the
    # alpha is always UPSCALED back to source w×h before encode (so it aligns
    # frame-for-frame with the foreground in the renderer's alphamerge).
    proc_h, proc_w = h, w
    if max_size > 0 and min(h, w) > max_size:
        scale = max_size / min(h, w)
        proc_h, proc_w = int(round(h * scale)), int(round(w * scale))

    mask_np = load_seed_mask(mask_path, r_erode, r_dilate)
    mask = torch.from_numpy(mask_np).float().to(device)
    if (proc_h, proc_w) != (h, w):
        mask = F.interpolate(mask[None, None], size=(proc_h, proc_w), mode="nearest")[0, 0]
    objects = [1]

    # Decoder: source → CFR rawvideo rgb24 (CFR pins frame↔matte 1:1).
    dec = subprocess.Popen(
        [FFMPEG_BIN, "-v", "error", "-i", in_path,
         "-fps_mode", "cfr", "-r", f"{fps}", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    # Encoder: rawvideo gray → LOSSLESS FFV1 .mkv at source res + fps.
    enc = subprocess.Popen(
        [FFMPEG_BIN, "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "gray",
         "-s", f"{w}x{h}", "-framerate", f"{fps}", "-i", "-",
         "-c:v", "ffv1", "-pix_fmt", "gray", out_path],
        stdin=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    def to_proc(buf: bytes) -> torch.Tensor:
        """rgb24 bytes → [3,proc_h,proc_w] float[0,1] on device."""
        rgb = np.frombuffer(buf, dtype=np.uint8).reshape(h, w, 3)
        t = torch.from_numpy(rgb.copy()).permute(2, 0, 1).float().to(device) / 255.0
        if (proc_h, proc_w) != (h, w):
            t = F.interpolate(t[None], size=(proc_h, proc_w), mode="area")[0]
        return t

    def emit(prob) -> np.ndarray:
        """output_prob → alpha float[0,1] HxW (proc res); upscale to source; encode."""
        a = processor.output_prob_to_mask(prob).float()        # proc_h × proc_w, [0,1]
        if (proc_h, proc_w) != (h, w):
            a = F.interpolate(a[None, None], size=(h, w), mode="bilinear",
                              align_corners=False)[0, 0]
        a = a.clamp(0, 1).cpu().numpy()
        enc.stdin.write((a * 255.0).round().astype(np.uint8).tobytes())
        return a

    covs: list[float] = []
    flick: list[float] = []
    soft: list[float] = []
    prev = None
    n = 0
    # Match upstream: inference_mode + autocast on CUDA (the model assumes it; also
    # halves the working memory of the propagation transformer).
    autocast = (torch.autocast(device_type="cuda", dtype=torch.float16)
                if device.type == "cuda" else torch.autocast(device_type="cpu", enabled=False))
    try:
      with torch.inference_mode(), autocast:
        # --- read the real frame 0 (drives warmup AND the first emitted alpha) ---
        buf0 = dec.stdout.read(frame_size)
        if len(buf0) < frame_size:
            raise SystemExit("matanyone_runner: no decodable video frames")
        img0 = to_proc(buf0)

        # Warmup on frame 0 (upstream prepends n_warmup copies of f0): encode the
        # mask, then re-init as first-frame prediction n_warmup times. None emitted.
        processor.step(img0, mask, objects=objects)
        prob = processor.step(img0, first_frame_pred=True)
        for _ in range(max(0, n_warmup - 1)):
            prob = processor.step(img0, first_frame_pred=True)
        # The REAL frame 0 prediction (upstream's ti == n_warmup) → first emit.
        prob = processor.step(img0, first_frame_pred=True)
        a = emit(prob); covs.append(finite_number((a > 0.5).mean()))
        soft.append(finite_number(((a > 0.05) & (a < 0.95)).mean()))
        prev = a; n += 1

        # --- stream the rest ---
        while True:
            buf = dec.stdout.read(frame_size)
            if len(buf) < frame_size:
                break
            img = to_proc(buf)
            prob = processor.step(img)
            a = emit(prob)
            covs.append(finite_number((a > 0.5).mean()))
            soft.append(finite_number(((a > 0.05) & (a < 0.95)).mean()))
            if prev is not None:
                flick.append(finite_number(np.abs(a - prev).mean()))
            prev = a; n += 1
    finally:
        if dec.stdout:
            dec.stdout.close()
        wait_child(dec, "decoder")
        if enc.stdin:
            enc.stdin.close()
        wait_child(enc, "encoder")

    if n == 0:
        raise SystemExit("matanyone_runner: produced no frames")
    return {
        "frames": n,
        "fps": round(fps, 6),
        "width": w,
        "height": h,
        "proc_width": proc_w,
        "proc_height": proc_h,
        "coverage_mean": round(finite_number(np.mean(covs)), 5) if covs else 0.0,
        "cov_min": round(finite_number(np.min(covs)), 5) if covs else 0.0,
        "cov_max": round(finite_number(np.max(covs)), 5) if covs else 0.0,
        "temporal_flicker": round(finite_number(np.mean(flick)), 6) if flick else 0.0,
        "edge_softness": round(finite_number(np.mean(soft)), 6) if soft else 0.0,
        "model": "matanyone2",
        "device": str(device),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("in_video")
    ap.add_argument("out_alpha")
    ap.add_argument("--mask", required=True, help="first-frame subject mask (PNG, L)")
    ap.add_argument("--model", required=True, help="matanyone2.pth checkpoint")
    ap.add_argument("--max-size", type=int, default=1080,
                    help="downsample for VRAM if min(w,h) exceeds this; alpha upscaled back")
    ap.add_argument("--warmup", type=int, default=10)
    ap.add_argument("--erode", type=int, default=10)
    ap.add_argument("--dilate", type=int, default=10)
    a = ap.parse_args()
    for p, what in ((a.model, "model"), (a.mask, "mask"), (a.in_video, "input video")):
        if not Path(p).is_file():
            print(f"matanyone_runner: {what} not found: {p}", file=sys.stderr)
            return 2
    stats = run(a.in_video, a.out_alpha, a.mask, a.model,
                a.max_size, a.warmup, a.erode, a.dilate)
    print(json.dumps(stats))  # ONE line of JSON on stdout — the caller parses it
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
