#!/usr/bin/env python3
"""face_runner.py — the SHIPPABLE local FACE-DETECT runtime for redaction.

Role: find every FACE + its bounding box in ONE video frame, so `edit.redact{faces}`
can auto-blur people's faces (privacy) without the user drawing boxes. cutd spawns
this per `faces` call exactly like the OCR / track / matte runners (one-shot CLI, no
port, no second window) — the multi-region redact engine then blurs the UNION of
the returned boxes.

Why this engine — YuNet via cv2.FaceDetectorYN (OpenCV's built-in, model
`face_detection_yunet_2023mar.onnx`, ~230 KB, BUNDLED beside this script so a cold
install needs NO download): rides the perception venv's existing opencv (cv2) +
onnxruntime, runs in a few ms on CPU, fits our talking-head/screen-demo wedge where
faces are prominent. (CenterFace — the `deface` tool's detector — is the future
crowd/distant upgrade; YuNet is weaker on tiny faces but adds zero dependency.)

The boxes are EXPANDED by a safety margin (privacy fail-safe — never under-cover a
face), mirroring the OCR redaction's margin. The MATCHING / region-building is in Rust
(server/faces.rs) — this runner only extracts face boxes.

Contract (matches server/faces.rs):
  python face_runner.py <in_video> [--at-ms N] [--margin 0.18] [--score 0.6]
  → prints ONE JSON line: {"ok":true,"width":W,"height":H,"boxes":[
      {"cx":..,"cy":..,"w":..,"h":..,"conf":..}, …]} where cx,cy,w,h are the box
    centre+size as FRACTIONS of the frame (0..1), already margin-expanded + clamped.
    Non-zero exit + stderr on failure.

Dependencies: opencv-python (cv2 ≥ 4.5.4 for FaceDetectorYN), numpy. Caller:
server/faces.rs.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from safe_numbers import finite_number

MODEL_NAME = "face_detection_yunet_2023mar.onnx"


def _eprint(*a):
    print(*a, file=sys.stderr)


def _read_frame(path: str, at_ms: int):
    """Decode ONE frame at `at_ms` (ms) → a BGR numpy array, or raise."""
    import cv2

    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        raise RuntimeError(f"cannot open {path}")
    try:
        if at_ms > 0:
            cap.set(cv2.CAP_PROP_POS_MSEC, finite_number(at_ms))
        ok, frame = cap.read()
        if not ok or frame is None:
            # Fall back to the first frame (some containers refuse a mid-seek read).
            cap.set(cv2.CAP_PROP_POS_FRAMES, 0)
            ok, frame = cap.read()
        if not ok or frame is None:
            raise RuntimeError("frame decode failed")
        return frame
    finally:
        cap.release()


def _make_csrt():
    """A CSRT tracker, tolerating the main-module vs `legacy` namespace split
    (mirrors track_runner.py). None if this opencv build lacks CSRT."""
    import cv2

    for ns in (cv2, getattr(cv2, "legacy", None)):
        if ns is not None and hasattr(ns, "TrackerCSRT_create"):
            try:
                return ns.TrackerCSRT_create()
            except Exception as exc:
                _eprint(f"face_runner: CSRT tracker unavailable in {ns.__name__}: {exc}")
    return None


def _track_faces(path, seed_at_ms, seeds_px, margin, sample_ms=120):
    """CSRT-track each seeded face FORWARD from `seed_at_ms` to the clip end, so a
    moving face stays covered. `seeds_px` = [(x,y,w,h)] in pixels at the seed
    frame. Returns [track] aligned to seeds, each a list of {t_ms,cx,cy,w,h} fractions
    (margin-expanded), sampled ~every `sample_ms`. Degrades to a single static point
    if CSRT is unavailable."""
    import cv2

    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        return [[] for _ in seeds_px]
    try:
        w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH)) or 1
        h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT)) or 1
        fps = cap.get(cv2.CAP_PROP_FPS) or 24.0
        if fps <= 0:
            fps = 24.0
        cap.set(cv2.CAP_PROP_POS_MSEC, finite_number(seed_at_ms))
        ok, frame = cap.read()
        if not ok or frame is None:
            return [[(seed_at_ms, *_box_frac(b, w, h, margin)) for b in seeds_px] for _ in [0]]  # type: ignore
        trackers = []
        tracks = []
        for b in seeds_px:
            t = _make_csrt()
            if t is not None:
                try:
                    t.init(frame, tuple(int(v) for v in b))
                except Exception:
                    t = None
            trackers.append(t)
            tracks.append([(seed_at_ms, *_box_frac(b, w, h, margin))])
        last_rec = seed_at_ms
        frame_i = 0
        m = max(0.0, finite_number(margin))
        while True:
            ok, frame = cap.read()
            if not ok or frame is None:
                break
            frame_i += 1
            t_ms = int(seed_at_ms + frame_i * 1000.0 / fps)
            if t_ms - last_rec < sample_ms:
                continue
            last_rec = t_ms
            for i, tr in enumerate(trackers):
                if tr is None:
                    # Static fallback: repeat the seed box.
                    tracks[i].append((t_ms, *_box_frac(seeds_px[i], w, h, margin)))
                    continue
                ok2, box = tr.update(frame)
                if ok2:
                    tracks[i].append((t_ms, *_box_frac(box, w, h, margin)))
                # lost lock → just stop extending this track (last good point holds)
        return [[{"t_ms": tm, "cx": cx, "cy": cy, "w": bw, "h": bh} for (tm, cx, cy, bw, bh) in tk] for tk in tracks]
    finally:
        cap.release()


def _box_frac(box, w, h, margin):
    """(x,y,w,h)px → (cx,cy,w,h) fractions, margin-expanded + clamped."""
    fx, fy, fw, fh = finite_number(box[0]), finite_number(box[1]), finite_number(box[2]), finite_number(box[3])
    ex, ey = fw * margin, fh * margin
    x0 = max(0.0, fx - ex)
    y0 = max(0.0, fy - ey)
    x1 = min(finite_number(w), fx + fw + ex)
    y1 = min(finite_number(h), fy + fh + ey)
    return (
        round(((x0 + x1) / 2.0) / w, 5),
        round(((y0 + y1) / 2.0) / h, 5),
        round((x1 - x0) / w, 5),
        round((y1 - y0) / h, 5),
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("in_video")
    ap.add_argument("--at-ms", type=int, default=0)
    ap.add_argument("--margin", type=float, default=0.18, help="box expand fraction (privacy fail-safe)")
    ap.add_argument("--score", type=float, default=0.6, help="YuNet score threshold")
    ap.add_argument("--track", action="store_true", help="CSRT-track each moving face forward")
    args = ap.parse_args()

    try:
        import cv2
        import numpy as np  # noqa: F401  (cv2 returns numpy; import asserts the dep)
    except Exception as e:  # pragma: no cover
        _eprint(f"face_runner: missing dep: {e}")
        return 3

    model = Path(__file__).resolve().parent / MODEL_NAME
    if not model.exists():
        _eprint(f"face_runner: bundled YuNet model not found at {model}")
        return 4

    try:
        frame = _read_frame(args.in_video, args.at_ms)
    except Exception as e:
        _eprint(f"face_runner: {e}")
        return 5

    h, w = frame.shape[:2]
    try:
        det = cv2.FaceDetectorYN.create(model, "", (w, h), args.score, 0.3, 5000)
        det.setInputSize((w, h))
        _, faces = det.detect(frame)
    except Exception as e:
        _eprint(f"face_runner: YuNet detect failed: {e}")
        return 6

    boxes = []
    seeds_px = []
    if faces is not None:
        for f in faces:
            # YuNet row: x, y, w, h (px), 5 landmarks (10), score → 15 cols.
            fx, fy, fw, fh = finite_number(f[0]), finite_number(f[1]), finite_number(f[2]), finite_number(f[3])
            conf = finite_number(f[14]) if len(f) >= 15 else 1.0
            if fw <= 0 or fh <= 0:
                continue
            cx, cy, bw, bh = _box_frac((fx, fy, fw, fh), w, h, args.margin)
            boxes.append({"cx": cx, "cy": cy, "w": bw, "h": bh, "conf": round(conf, 4)})
            seeds_px.append((fx, fy, fw, fh))

    # CSRT-track each face forward so a moving face stays covered. Attach a
    # per-face track (centre+size over time) to each box.
    if args.track and seeds_px:
        try:
            tracks = _track_faces(args.in_video, args.at_ms, seeds_px, args.margin)
            for i, tk in enumerate(tracks):
                if i < len(boxes) and tk:
                    boxes[i]["track"] = tk
        except Exception as e:  # tracking is best-effort; static boxes still ship
            _eprint(f"face_runner: track warn: {e}")

    print(json.dumps({"ok": True, "width": w, "height": h, "boxes": boxes}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
