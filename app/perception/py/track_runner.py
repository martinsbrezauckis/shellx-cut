#!/usr/bin/env python3
"""track_runner.py — the SHIPPABLE local MOTION-TRACKING runtime (one-shot CLI).

Role: track a single region (a face, a bottle, a label, a license plate…) across
a clip's frames and emit its trajectory as a list of sampled boxes. cutd spawns
this per `edit.track` call exactly like the perception sidecar and the matte
runner (`python <script> …`), so it is invisible plumbing inside the one ShellX
Cut app — no HTTP service, no port, no second window.

Why this shape:
- One-shot CLI = the SAME pattern as the perception sidecar / matte runner
  (consistency); cutd manages the lifecycle, the user never sees it.
- cv2 ONLY (already in the perception venv via scenedetect's opencv-python dep) —
  no new dependency. Leads with the CSRT DCF tracker (the accurate single-object
  tracker); if the contrib trackers are unavailable it degrades GRACEFULLY to a
  dependency-free normalized-cross-correlation template tracker (base opencv's
  `matchTemplate`), so tracking works on ANY opencv build. This mirrors the
  perception sidecar's graceful mediapipe→saliency fallback.
- Generic (tracks an arbitrary user box), NOT class-restricted — so it serves
  "blur the moving face" AND "make the label follow the bottle" AND redaction of
  a moving plate, unlike a COCO-class detector+ByteTrack stack.

Contract (matches what server/track.rs expects):
  python track_runner.py <in_video> --bbox x,y,w,h | --point x,y
                         [--start-ms N] [--end-ms N] [--every-ms N]
                         [--engine auto|csrt|kcf|mil|template] [--point-size F]
  All box coordinates are FRACTIONS of the frame (x,y = top-left, 0..1). Times are
  in SOURCE milliseconds. Prints ONE line of JSON on STDOUT:
    {"ok":true,"engine":"csrt","fps":30.0,"width":1280,"height":720,
     "points":[{"t_ms":0,"cx":0.5,"cy":0.5,"w":0.1,"h":0.1,"ok":true}, …]}
  where cx,cy = box CENTRE (fractions), w,h = box SIZE (fractions). `ok:false` on
  a point marks a frame where the tracker lost lock (the box is the last good one).
  Non-zero exit + stderr on a hard failure (unreadable video, no seed, no cv2).

Dependencies: opencv-python (cv2), numpy, ffmpeg not required (cv2 decodes).
Primary caller: server/track.rs (local-CLI transport).
"""
from __future__ import annotations

import argparse
import json
import sys

import cv2  # type: ignore
import numpy as np
from safe_numbers import finite_number


def _make_cv_tracker(name: str):
    """Construct a named cv2 tracker, tolerating the main-module vs `legacy`
    namespace split across OpenCV versions (CSRT/KCF/MIL moved to the main module
    in 4.5.1 but stayed mirrored under cv2.legacy). Returns the tracker or None if
    this build lacks it (→ caller falls back to the template tracker)."""
    ctor = f"Tracker{name}_create"
    for ns in (cv2, getattr(cv2, "legacy", None)):
        if ns is not None and hasattr(ns, ctor):
            try:
                return getattr(ns, ctor)()
            except Exception as exc:
                print(f"track_runner: {ctor} unavailable in {ns.__name__}: {exc}", file=sys.stderr)
    return None


def _seed_px(bbox_frac, w: int, h: int):
    """Fractional [x,y,w,h] (top-left) → integer pixel box, clamped inside the
    frame with a >=2px minimum size (a degenerate seed breaks every tracker)."""
    x = int(round(bbox_frac[0] * w))
    y = int(round(bbox_frac[1] * h))
    bw = int(round(bbox_frac[2] * w))
    bh = int(round(bbox_frac[3] * h))
    x = max(0, min(x, w - 2))
    y = max(0, min(y, h - 2))
    bw = max(2, min(bw, w - x))
    bh = max(2, min(bh, h - y))
    return x, y, bw, bh


class TemplateTracker:
    """Dependency-free NCC template tracker (base opencv `matchTemplate`): each
    frame, search a window around the last centre for the best normalized-cross-
    correlation match of the seed template. No appearance update (rigid-region
    assumption) — robust enough for "follow the moving label" over a short clip,
    and the universal floor when the CSRT/KCF contrib trackers are absent."""

    def __init__(self, frame, box):
        x, y, w, h = box
        self.tmpl = frame[y:y + h, x:x + w].copy()
        self.w, self.h = w, h
        self.x, self.y = x, y  # top-left of the last match

    def update(self, frame):
        fh, fw = frame.shape[:2]
        # Search window: the template box grown by half its size on every side.
        mx, my = self.w // 2, self.h // 2
        x0 = max(0, self.x - mx)
        y0 = max(0, self.y - my)
        x1 = min(fw, self.x + self.w + mx)
        y1 = min(fh, self.y + self.h + my)
        region = frame[y0:y1, x0:x1]
        if region.shape[0] < self.h or region.shape[1] < self.w:
            return False, (self.x, self.y, self.w, self.h)
        res = cv2.matchTemplate(region, self.tmpl, cv2.TM_CCOEFF_NORMED)
        _, max_val, _, max_loc = cv2.minMaxLoc(res)
        self.x = x0 + max_loc[0]
        self.y = y0 + max_loc[1]
        # A weak peak (NCC < 0.3) = lost lock; keep the box but flag it.
        return bool(max_val >= 0.30), (self.x, self.y, self.w, self.h)


def track(video, seed_frac, start_ms, end_ms, every_ms, engine):
    """Run the tracker over [start_ms, end_ms], updating EVERY frame (CSRT needs
    a continuous stream) but EMITTING a sample at most every `every_ms`. Returns
    (engine_used, fps, w, h, points)."""
    cap = cv2.VideoCapture(video)
    if not cap.isOpened():
        raise SystemExit(f"track_runner: cannot open video: {video}")
    fps = cap.get(cv2.CAP_PROP_FPS) or 0.0
    w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    if fps <= 0 or w <= 0 or h <= 0:
        raise SystemExit("track_runner: video has no readable fps/size")

    # Seek to the start frame and grab the seed frame.
    cap.set(cv2.CAP_PROP_POS_MSEC, finite_number(start_ms))
    ok, frame = cap.read()
    if not ok or frame is None:
        raise SystemExit(f"track_runner: cannot read a frame at {start_ms}ms")
    box = _seed_px(seed_frac, w, h)

    # Pick the engine. "auto" → CSRT (best), else KCF, else template.
    used = engine
    trk = None
    if engine in ("auto", "csrt"):
        trk = _make_cv_tracker("CSRT")
        used = "csrt"
    if trk is None and engine in ("auto", "kcf"):
        trk = _make_cv_tracker("KCF")
        used = "kcf"
    if trk is None and engine == "mil":
        trk = _make_cv_tracker("MIL")
        used = "mil"
    if trk is None:
        trk = TemplateTracker(frame, box)
        used = "template"
    else:
        trk.init(frame, tuple(box))

    def emit(t_ms, b, good):
        bx, by, bw, bh = b
        return {
            "t_ms": int(t_ms),
            "cx": round((bx + bw / 2.0) / w, 6),
            "cy": round((by + bh / 2.0) / h, 6),
            "w": round(bw / finite_number(w), 6),
            "h": round(bh / finite_number(h), 6),
            "ok": bool(good),
        }

    points = [emit(start_ms, box, True)]
    last_emit = start_ms
    while True:
        ok, frame = cap.read()
        if not ok or frame is None:
            break
        t_ms = cap.get(cv2.CAP_PROP_POS_MSEC)
        if end_ms is not None and t_ms > end_ms:
            break
        good, b = trk.update(frame)
        # Keep the box integer + inside the frame even when the tracker drifts out.
        bx, by, bw, bh = (int(round(v)) for v in b)
        bx = max(0, min(bx, w - 1))
        by = max(0, min(by, h - 1))
        if t_ms - last_emit >= every_ms:
            points.append(emit(t_ms, (bx, by, bw, bh), good))
            last_emit = t_ms
    cap.release()
    return used, fps, w, h, points


def main() -> int:
    ap = argparse.ArgumentParser(description="ShellX Cut motion tracker (one-shot)")
    ap.add_argument("video")
    ap.add_argument("--bbox", help="x,y,w,h as FRACTIONS of the frame (top-left)")
    ap.add_argument("--point", help="x,y as FRACTIONS — a seed box is built around it")
    ap.add_argument("--point-size", type=float, default=0.08,
                    help="seed box size (fraction of frame) for --point [0.08]")
    ap.add_argument("--start-ms", type=int, default=0)
    ap.add_argument("--end-ms", type=int, default=None)
    ap.add_argument("--every-ms", type=int, default=100)
    ap.add_argument("--engine", default="auto",
                    choices=["auto", "csrt", "kcf", "mil", "template"])
    a = ap.parse_args()

    if a.bbox:
        seed = [finite_number(v) for v in a.bbox.split(",")]
        if len(seed) != 4:
            raise SystemExit("track_runner: --bbox needs x,y,w,h")
    elif a.point:
        p = [finite_number(v) for v in a.point.split(",")]
        if len(p) != 2:
            raise SystemExit("track_runner: --point needs x,y")
        s = max(0.01, a.point_size)
        seed = [p[0] - s / 2.0, p[1] - s / 2.0, s, s]
    else:
        raise SystemExit("track_runner: one of --bbox / --point is required")

    used, fps, w, h, points = track(
        a.video, seed, a.start_ms, a.end_ms, a.every_ms, a.engine
    )
    n_ok = sum(1 for p in points if p["ok"])
    out = {
        "ok": True,
        "engine": used,
        "fps": round(finite_number(fps), 4),
        "width": w,
        "height": h,
        "coverage": round(n_ok / len(points), 4) if points else 0.0,
        "points": points,
    }
    sys.stdout.write(json.dumps(out) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
