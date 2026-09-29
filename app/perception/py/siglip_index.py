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
# This optional indexer needs the local perception runtime, a SigLIP2 model, and
# a GPU or CPU. In a prepared native context it receives one sealed local model.
# Ordinary mode passes its explicit ``--model`` source to Transformers, which
# resolves that source through its normal local-cache/download API.
from local_media_io import input_args
import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

from native_runtime_models import (
    SIGLIP_MODEL_ID,
    NativeRuntimeModelError,
    siglip_directory,
)

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
MAX_VISUAL_INDEX_BYTES = 512 * 1024 * 1024  # Keep in sync with vissearch.rs.


def write_index_bounded(index: dict, path: str, max_bytes: int) -> None:
    # Count the exact JSON encoding in chunks before opening the final cache.
    # A long index is already resident as frame vectors; avoid a second giant
    # string just to discover it cannot be read back by Cut.
    size = 0
    for chunk in json.JSONEncoder().iterencode(index):
        size += len(chunk.encode("utf-8"))
        if size > max_bytes:
            raise ValueError(f"visual index exceeds {max_bytes} byte limit")
    parent = os.path.dirname(os.path.abspath(path))
    parent_stat = os.lstat(parent)
    if not stat.S_ISDIR(parent_stat.st_mode) or is_reparse(parent_stat):
        raise ValueError("embeddings is not a plain directory")
    try:
        leaf_stat = os.lstat(path)
    except FileNotFoundError:
        pass
    else:
        if not stat.S_ISREG(leaf_stat.st_mode) or is_reparse(leaf_stat):
            raise ValueError("index is not a plain file")
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=parent,
                                         prefix=".index-", suffix=".tmp", delete=False) as output:
            temporary = output.name
            json.dump(index, output)
        parent_stat = os.lstat(parent)
        if not stat.S_ISDIR(parent_stat.st_mode) or is_reparse(parent_stat):
            raise ValueError("embeddings is not a plain directory")
        os.replace(temporary, path)
        temporary = None
    finally:
        if temporary is not None:
            os.unlink(temporary)


def is_reparse(info: os.stat_result) -> bool:
    return stat.S_ISLNK(info.st_mode) or (
        os.name == "nt" and bool(getattr(info, "st_file_attributes", 0) & 0x400)
    )


def log(msg: str) -> None:
    print(f"[siglip_index] {msg}", file=sys.stderr, flush=True)


def pooled_embedding(torch, output, tower: str) -> list[float]:
    """Return one finite pooled SigLIP embedding from a Transformers output.

    ``BaseModelOutputWithPooling`` is tuple-like, but item zero is its token
    ``last_hidden_state``. The image and text towers must instead use the
    documented ``pooler_output``: one vector for the one-item batch. Writing
    token states makes a nested JSON array that the Rust ``Vec<f32>`` contract
    correctly rejects.
    """
    # Transformers 4.57 returns the pooled Tensor directly; 5.16 returns its
    # BaseModelOutputWithPooling. Accept only those two explicit contracts, never
    # tuple-like output where item zero is a hidden/token state.
    pooled = output if torch.is_tensor(output) else getattr(output, "pooler_output", None)
    if pooled is None:
        raise RuntimeError(f"SigLIP {tower} tower returned no pooled embedding")
    if pooled.ndim != 2 or pooled.shape[0] != 1:
        raise RuntimeError(
            f"SigLIP {tower} pooled embedding must have shape [1, dim], got {tuple(pooled.shape)}"
        )
    vector = torch.nn.functional.normalize(pooled, dim=-1)[0]
    if vector.ndim != 1 or vector.numel() == 0:
        raise RuntimeError(f"SigLIP {tower} pooled embedding must be one non-empty vector")
    if not torch.isfinite(vector).all():
        raise RuntimeError(f"SigLIP {tower} pooled embedding contains non-finite values")
    return vector.float().cpu().tolist()


def resolve_model_source(requested: str, model_id: str) -> tuple[str, dict, str]:
    """Keep ordinary HF selection, but make a supplied context fully offline.

    The Rust consumer supplies the exact admitted directory and stable model ID.
    A native invocation cannot replace either with a host cache, an alternate
    path, or a provider/download route.
    """
    prepared = siglip_directory()
    if prepared is None:
        return requested, {}, requested.split("/")[-1]
    if Path(requested) != prepared or model_id != SIGLIP_MODEL_ID:
        raise NativeRuntimeModelError("prepared SigLIP invocation does not match the admitted model")
    return str(prepared), {"local_files_only": True}, SIGLIP_MODEL_ID


def ffprobe_dims(path: str) -> tuple[int, int]:
    """(width, height) of the first video stream via ffprobe."""
    out = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x", *input_args(path)],
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
        [FFMPEG_BIN, "-v", "error", *input_args(path),
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
    ap.add_argument("--model-id", default="", help=argparse.SUPPRESS)
    ap.add_argument("--fps", type=float, default=1.0)
    ap.add_argument("--asset", default="")
    ap.add_argument("--max-index-bytes", type=int, default=MAX_VISUAL_INDEX_BYTES)
    ap.add_argument("--size", type=int, default=224, help="fixed encoder input size")
    ap.add_argument("--embed-text", default=None,
                    help="TEXT-QUERY mode: embed this string with the SigLIP2 text "
                         "tower and print {\"v\":[...]} to stdout (no indexing).")
    args = ap.parse_args()

    source, source_options, index_model = resolve_model_source(args.model, args.model_id)

    # transformers SiglipModel handles preprocessing + the image/text towers. (An
    # ONNX path is the deployment optimization; this is the faithful reference.)
    import torch  # noqa: PLC0415
    from transformers import AutoModel, AutoProcessor  # noqa: PLC0415

    device = "cuda" if torch.cuda.is_available() else "cpu"
    log(f"loading {index_model} on {device}")
    model = AutoModel.from_pretrained(source, **source_options).to(device).eval()
    processor = AutoProcessor.from_pretrained(source, **source_options)

    # TEXT-QUERY mode (media.search): embed the query → stdout, then exit. The
    # vector is L2-normalized to match the indexed (also-normalized) frames.
    if args.embed_text is not None:
        with torch.inference_mode():
            inp = processor(text=[args.embed_text], return_tensors="pt",
                            padding="max_length").to(device)
            output = model.get_text_features(**inp, return_dict=True)
            vec = pooled_embedding(torch, output, "text")
        # The ONLY stdout line is the JSON document (wire discipline).
        print(json.dumps({"v": vec}))
        return 0

    frames = []
    dim = 0
    with torch.inference_mode():
        for ms, img in iter_frames(args.inp, args.fps, args.size):
            inputs = processor(images=img, return_tensors="pt").to(device)
            output = model.get_image_features(**inputs, return_dict=True)
            vec = pooled_embedding(torch, output, "image")
            if dim and len(vec) != dim:
                raise RuntimeError(
                    f"SigLIP image pooled embedding dimension changed from {dim} to {len(vec)}"
                )
            dim = len(vec)
            frames.append({"ms": ms, "v": vec})

    if not frames:
        log("no frames embedded (empty/short input?)")
        return 1

    index = {
        "schema": "shellx-cut/vissearch/1",
        "model": index_model,
        "dim": dim,
        "asset": args.asset,
        "frames": frames,
    }
    write_index_bounded(index, args.out, args.max_index_bytes)
    log(f"wrote {len(frames)} frame embeddings (dim={dim}) → {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
