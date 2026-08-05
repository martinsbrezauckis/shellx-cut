#!/usr/bin/env python3
# siglip_index.py — SigLIP2 visual-search indexer. One-shot CLI:
#   python siglip_index.py <in_media> <out_index.json> --model <path|hf-id>
#                          --fps <rate> --asset <id>
#
# Samples frames from <in_media> at --fps, embeds each with the SigLIP2 IMAGE
# encoder, and writes a content index the Rust engine (vissearch.rs) searches:
#   {schema, model, dim, asset, frames:[{ms, v:[float,...]}]}
#
# The default is a fixed-resolution multilingual SigLIP 2 model suited to
# on-device use. The engine is model-
# AGNOSTIC — any same-dim image-text encoder works, so the model is swappable.
#
# ── SCAFFOLD STATUS ──────────────────────────────────────────────────────────
# This optional indexer needs the perception environment, a SigLIP2 model, and
# a GPU or CPU. It follows the existing
# perception-sidecar patterns (ffmpeg frame pipe like matanyone_runner; one-shot
# CLI like matte_runner). The Rust search engine + media.search are proven
# independently with synthetic embeddings. Provision the runtime through
# system.setup_perception and the model-fetch flow before indexing.
# ─────────────────────────────────────────────────────────────────────────────
import argparse
import json
import os
import shutil
import subprocess
import sys

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


def log(msg: str) -> None:
    print(f"[siglip_index] {msg}", file=sys.stderr, flush=True)


def ffprobe_dims(path: str) -> tuple[int, int]:
    """(width, height) of the first video stream via ffprobe."""
    out = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x", path],
        capture_output=True, text=True, check=True, timeout=60,
    ).stdout.strip()
    w, h = out.split("x")[:2]
    return int(w), int(h)


def iter_frames(path: str, fps: float, size: int):
    """Yield (ms, PIL.Image) sampled at `fps`, decoded straight from an ffmpeg
    rawvideo pipe (rgb24, scaled to `size`×`size` for the encoder). Sidesteps any
    Python video reader — the same trick the matte runner uses."""
    from PIL import Image  # noqa: PLC0415 — venv-only dep

    proc = subprocess.Popen(
        [FFMPEG_BIN, "-v", "error", "-i", path,
         "-vf", f"fps={fps},scale={size}:{size}:flags=bicubic",
         "-pix_fmt", "rgb24", "-f", "rawvideo", "-"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        stdin=subprocess.DEVNULL,
    )
    frame_bytes = size * size * 3
    idx = 0
    assert proc.stdout is not None
    while True:
        buf = proc.stdout.read(frame_bytes)
        if len(buf) < frame_bytes:
            break
        img = Image.frombytes("RGB", (size, size), buf)
        ms = int(round(idx * 1000.0 / fps))
        yield ms, img
        idx += 1
    proc.stdout.close()
    try:
        proc.wait(timeout=30)
    except subprocess.TimeoutExpired as exc:
        proc.kill()
        proc.wait(timeout=5)
        raise RuntimeError("ffmpeg frame extraction timed out") from exc
    if proc.returncode not in (0, None):
        err = proc.stderr.read().decode("utf-8", "replace")[:300] if proc.stderr else ""
        raise RuntimeError(f"ffmpeg frame extraction failed: {err}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("inp", nargs="?", default="")
    ap.add_argument("out", nargs="?", default="")
    ap.add_argument("--model", required=True, help="SigLIP2 model path or HF id")
    ap.add_argument("--fps", type=float, default=1.0)
    ap.add_argument("--asset", default="")
    ap.add_argument("--size", type=int, default=224, help="fixed encoder input size")
    ap.add_argument("--embed-text", default=None,
                    help="TEXT-QUERY mode: embed this string with the SigLIP2 text "
                         "tower and print {\"v\":[...]} to stdout (no indexing).")
    args = ap.parse_args()

    # transformers SiglipModel handles preprocessing + the image/text towers. (An
    # ONNX path is the deployment optimization; this is the faithful reference.)
    import torch  # noqa: PLC0415
    from transformers import AutoModel, AutoProcessor  # noqa: PLC0415

    device = "cuda" if torch.cuda.is_available() else "cpu"
    log(f"loading {args.model} on {device}")
    model = AutoModel.from_pretrained(args.model).to(device).eval()
    processor = AutoProcessor.from_pretrained(args.model)

    # TEXT-QUERY mode (media.search): embed the query → stdout, then exit. The
    # vector is L2-normalized to match the indexed (also-normalized) frames.
    if args.embed_text is not None:
        with torch.inference_mode():
            inp = processor(text=[args.embed_text], return_tensors="pt",
                            padding="max_length").to(device)
            feat = model.get_text_features(**inp)[0]
            feat = torch.nn.functional.normalize(feat, dim=-1)
        # The ONLY stdout line is the JSON document (wire discipline).
        print(json.dumps({"v": feat.float().cpu().tolist()}))
        return 0

    frames = []
    dim = 0
    with torch.inference_mode():
        for ms, img in iter_frames(args.inp, args.fps, args.size):
            inputs = processor(images=img, return_tensors="pt").to(device)
            feat = model.get_image_features(**inputs)[0]
            feat = torch.nn.functional.normalize(feat, dim=-1)  # L2 for cosine
            vec = feat.float().cpu().tolist()
            dim = len(vec)
            frames.append({"ms": ms, "v": vec})

    if not frames:
        log("no frames embedded (empty/short input?)")
        return 1

    index = {
        "schema": "shellx-cut/vissearch/1",
        "model": args.model.split("/")[-1],
        "dim": dim,
        "asset": args.asset,
        "frames": frames,
    }
    with open(args.out, "w") as f:
        json.dump(index, f)
    log(f"wrote {len(frames)} frame embeddings (dim={dim}) → {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
