#!/usr/bin/env python3
"""ocr_runner.py — the SHIPPABLE local OCR runtime for redaction auto-detect.

Role: read text + its bounding boxes from ONE video frame, so `edit.redact{ocr_auto}`
can auto-find on-screen secrets (passwords / API keys / emails / PII) and redact
their regions. cutd spawns this per `ocr_auto` call exactly like the perception
sidecar / matte / track runners (one-shot CLI, no port, no 2nd window).

RapidOCR (rapidocr-onnxruntime, Apache-2.0) uses the perception environment's
existing ONNX Runtime, bundles compact PP-OCR detection and recognition models,
and returns text + 4-point boxes + confidence without requiring a torch runtime.
The PII MATCHING is done in Rust (server/ocr.rs, deterministic regex),
NOT here — this runner only extracts text+boxes.

Contract (matches server/ocr.rs):
  python ocr_runner.py <in_video> [--at-ms N]
  → prints ONE JSON line: {"ok":true,"width":W,"height":H,"boxes":[
      {"text":"…","cx":..,"cy":..,"w":..,"h":..,"conf":..}, …]} where cx,cy,w,h are
    the box centre+size as FRACTIONS of the frame. Non-zero exit + stderr on failure.

Dependencies: rapidocr-onnxruntime, opencv-python (cv2), numpy. Caller: server/ocr.rs.
"""
from __future__ import annotations

import argparse
import json
import sys

import cv2  # type: ignore
from safe_numbers import finite_number


def main() -> int:
    ap = argparse.ArgumentParser(description="ShellX Cut OCR (one-shot, redaction auto-detect)")
    ap.add_argument("video")
    ap.add_argument("--at-ms", type=int, default=0, help="frame timestamp to OCR (ms)")
    a = ap.parse_args()

    cap = cv2.VideoCapture(a.video)
    if not cap.isOpened():
        raise SystemExit(f"ocr_runner: cannot open video: {a.video}")
    w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    cap.set(cv2.CAP_PROP_POS_MSEC, finite_number(a.at_ms))
    ok, frame = cap.read()
    cap.release()
    if not ok or frame is None or w <= 0 or h <= 0:
        raise SystemExit(f"ocr_runner: cannot read a frame at {a.at_ms}ms")

    # Import here so a missing model/dep surfaces a clear error AFTER we've proven
    # the video is readable (keeps the failure messages crisp).
    from rapidocr_onnxruntime import RapidOCR  # type: ignore

    ocr = RapidOCR()
    # RapidOCR takes a BGR ndarray (cv2's native) or a path; returns
    # [[box4pts, text, conf], …] or None when nothing is found.
    result, _ = ocr(frame)
    boxes = []
    for item in result or []:
        box, text, conf = item[0], item[1], item[2]
        xs = [p[0] for p in box]
        ys = [p[1] for p in box]
        x0, x1 = min(xs), max(xs)
        y0, y1 = min(ys), max(ys)
        boxes.append(
            {
                "text": text,
                "cx": round((x0 + x1) / 2.0 / w, 6),
                "cy": round((y0 + y1) / 2.0 / h, 6),
                "w": round((x1 - x0) / finite_number(w), 6),
                "h": round((y1 - y0) / finite_number(h), 6),
                "conf": round(finite_number(conf), 4),
            }
        )
    sys.stdout.write(json.dumps({"ok": True, "width": w, "height": h, "boxes": boxes}) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
