#!/usr/bin/env python3
"""sam2_runner.py — SAM2 click-to-pick-subject → a first-frame matte SEED (one-shot CLI).

Role: the PREMIUM matte's "pick WHICH subject" path. A click (or box) on one frame
→ SAM2 produces a binary subject mask that seeds MatAnyone2's first frame (vs the
RVM auto-seed, which just finds "the human"). On a multi-person clip, clicking
person A vs B selects that one. Ships
in the perception payload; runs in the MATANYONE torch venv (SAM2 needs torch≥2.5.1,
already there) — cutd spawns it per pick, invisible plumbing (one app).

Selection: SAM2's multimask returns nested granularities
(whole/part/subpart); the highest-SCORE mask is usually the small part (a face),
so we pick the **largest-area** mask = the whole subject (~matches RVM coverage on
a single person). Proven: a single click anywhere on the subject → clean full-person
mask.

License: SAM2 is Apache-2.0 (commercial-OK) — unlike MatAnyone2's non-commercial
weights. Model: facebook/sam2-hiera-base-plus (HF, ~80 MB), loaded at a pinned
revision via from_pretrained with HF_HOME pointed at the matanyone appdata dir
(offline after the one-time setup_matte pre-fetch).

Contract:
  python sam2_runner.py <in_video> <out_mask.png> --frame N
        ( --point X,Y [--point X,Y ...] | --box X,Y,W,H )
        [--hf-home DIR] [--hf-id ID] [--hf-revision REV] [--neg X,Y ...]
  → writes a BINARY mask PNG (255=subject, the largest multimask) at SOURCE res;
    prints ONE JSON stats line {width,height,frame,coverage,score} to STDOUT.

Dependencies: sam-2 (Apache-2.0), torch+torchvision (CUDA), numpy, Pillow,
ffmpeg/ffprobe on PATH. Primary caller: server/matte.rs (the matte seed for
edit.matte{model:matanyone, seed}).
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
from PIL import Image
from safe_numbers import finite_number


def ffprobe_whf(path: str) -> tuple[int, int, float]:
    """(width, height, fps) of the first video stream."""
    out = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height,r_frame_rate", "-of", "json", path],
        capture_output=True, text=True, check=True, timeout=60)
    st = json.loads(out.stdout)["streams"][0]
    num, den = st["r_frame_rate"].split("/")
    fps = finite_number(num) / finite_number(den) if finite_number(den) else finite_number(num)
    return int(st["width"]), int(st["height"]), fps


def frame_rgb(path: str, n: int, w: int, h: int) -> np.ndarray:
    out = subprocess.run(
        [FFMPEG_BIN, "-v", "error", "-i", path, "-vf", f"select=eq(n\\,{n})",
         "-vframes", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        capture_output=True, check=True, timeout=120).stdout
    if len(out) < w * h * 3:
        raise SystemExit(f"sam2_runner: could not read frame {n}")
    return np.frombuffer(out, np.uint8).reshape(h, w, 3)


def parse_xy(vals: list[str]) -> list[list[int]]:
    pts = []
    for v in vals or []:
        x, y = v.split(",")
        pts.append([int(finite_number(x)), int(finite_number(y))])
    return pts


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("in_video")
    ap.add_argument("out_mask")
    ap.add_argument("--frame", type=int, default=0)
    ap.add_argument("--at-ms", type=int, default=None, help="source time (ms) → frame via fps (overrides --frame)")
    ap.add_argument("--point", action="append", default=[], help="X,Y positive click (repeatable)")
    ap.add_argument("--neg", action="append", default=[], help="X,Y negative click (repeatable)")
    ap.add_argument("--box", default=None, help="X,Y,W,H subject box")
    ap.add_argument("--hf-home", default=None, help="HF cache dir (offline after pre-fetch)")
    ap.add_argument("--hf-id", default="facebook/sam2-hiera-base-plus")
    ap.add_argument(
        "--hf-revision",
        default="98efa66555fceff5f74ad281fb8003536dcfb6ff",
        help="pinned HF model revision fetched by system.setup_matte",
    )
    a = ap.parse_args()
    if not Path(a.in_video).is_file():
        print(f"sam2_runner: input not found: {a.in_video}", file=sys.stderr)
        return 2
    if a.hf_home:
        os.environ["HF_HOME"] = a.hf_home  # MUST precede the SAM2 import/load
    if not a.point and not a.box:
        print("sam2_runner: need --point X,Y or --box X,Y,W,H", file=sys.stderr)
        return 2

    import torch
    from sam2.sam2_image_predictor import SAM2ImagePredictor

    w, h, fps = ffprobe_whf(a.in_video)
    frame = a.frame if a.at_ms is None else int(round(a.at_ms / 1000.0 * fps))
    img = frame_rgb(a.in_video, frame, w, h)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    predictor = SAM2ImagePredictor.from_pretrained(a.hf_id, revision=a.hf_revision)

    pos = parse_xy(a.point)
    neg = parse_xy(a.neg)
    point_coords = np.array(pos + neg) if (pos or neg) else None
    point_labels = np.array([1] * len(pos) + [0] * len(neg)) if (pos or neg) else None
    box = None
    if a.box:
        bx, by, bw, bh = (int(finite_number(v)) for v in a.box.split(","))
        box = np.array([bx, by, bx + bw, by + bh])

    autocast = torch.autocast(device, dtype=torch.bfloat16) if device == "cuda" \
        else torch.inference_mode()
    with torch.inference_mode(), autocast:
        predictor.set_image(img)
        masks, scores, _ = predictor.predict(
            point_coords=point_coords, point_labels=point_labels,
            box=box, multimask_output=True)
    # Pick the LARGEST-area mask as the whole subject (highest-score is the
    # small part — a face). For a box prompt SAM2 already returns the whole object,
    # but largest-of-multimask is still the safe pick.
    areas = [finite_number(m.mean()) for m in masks]
    idx = int(np.argmax(areas))
    m = masks[idx].astype(bool)
    Image.fromarray((m.astype(np.uint8) * 255), "L").save(a.out_mask)
    print(json.dumps({
        "width": w, "height": h, "frame": frame,
        "coverage": round(finite_number(m.mean()), 5),
        "score": round(finite_number(scores[idx]), 4),
    }))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
