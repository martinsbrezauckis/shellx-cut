#!/usr/bin/env python3
"""instruments.py — ShellX Cut perception sidecar (perception contract).

Role: one JSON-in/JSON-out CLI the Rust crate (cut-perception::sidecar) shells
out to. Gives the system measured FACTS about a media file: word-level
transcript, silences, scene cuts, black/frozen video spans, beat grid,
loudness. Output is the PerceptionReport shape (app/perception/src/types.rs).

Invocation — two modes, same report either way:
  1. Wire mode (what Rust uses): stdin JSON request -> stdout JSON report.
         .venv/bin/python instruments.py < request.json > report.json
  2. Human/CLI mode: positional media path + flags.
         .venv/bin/python instruments.py <media> --out perception.json \
             [--instruments words,silence,...] [--model small]

Request JSON (wire mode):
    {
      "media_path": "/abs/path/file.mp4",
      "asset_id": "a1",
      "asset_hash": "sha256:…",          # echoed into the report (cache key)
      "instruments": ["words","silence","scenes","beats","loudness"],  # subset ok
      "whisper_model": "small"           # optional, default "small" (dev)
    }

Response JSON: PerceptionReport — schema "shellx-cut/perception/1"; times in
ms (ints). The "scenes" instrument ALSO emits black_spans/frozen_spans
(ffmpeg blackdetect d=0.3s / freezedetect d=2.0s — thresholds live HERE; the
Rust black_or_frozen_frames check asserts the lists are empty) AND content_bbox
(ffmpeg cropdetect sampled across the clip → the content rectangle + a
uniform_border flag for baked-in letterbox/pillarbox; the Rust uniform_border
check guards renders, edit.crop fixes sources).

On error: exit nonzero with one-line JSON {"error":{code,message,cause}} on
stdout — Rust maps it to CutError code "sidecar". Progress/log lines go to
stderr only; stdout carries exactly one JSON document.

Engines (perception contract, permissive licenses only):
    words    — DEFAULT: NVIDIA Parakeet-TDT-0.6B via ONNX (onnx-asr, MIT) — native
               word timestamps, no torch. WEAK-LANGUAGE:
               Canary-1B-v2 text via onnx-asr + torchaudio MMS_FA forced alignment
               for word timestamps. FALLBACK: whisperX (BSD-2) then faster-whisper,
               when the selected ONNX path is absent/errors. The engine that ran
               is recorded in words.model (e.g.
               "parakeet-tdt/nemo-parakeet-tdt-0.6b-v3@onnx",
               "canary/nemo-canary-1b-v2+mms-fa@onnx", "whisperx-large-v3@cuda").
    silence  — silero-vad (MIT) + ffmpeg silencedetect cross-check; each span
               tagged source: "both" | "silero" | "ffmpeg". Falls back to
               ffmpeg-only when the optional Silero stack is unavailable.
    scenes   — PySceneDetect (BSD-3) ContentDetector with an ffmpeg scene-select
               fallback + blackdetect/freezedetect.
    beats    — lightweight NumPy/WAV energy peak grid. Deliberately NOT madmom
               (CC BY-NC-SA) and not librosa.beat_track (native segfault risk).
    loudness — ffmpeg ebur128: integrated LUFS, true peak (dBTP), 1s momentary windows.

GPU: torch.cuda first (RTX 5080), automatic CPU fallback on any CUDA failure —
the fallback is logged to stderr and visible in words.model provenance.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unicodedata
from decimal import Decimal, InvalidOperation
from pathlib import Path

# on a cold install ffmpeg/ffprobe are NOT on
# PATH — they live in the app's bundled/app-data tools dir. The Rust sidecar
# orchestrator forwards that dir as SHELLX_CUT_FFMPEG_DIR; prepend it to PATH
# here, then resolve both tools once to absolute paths. That keeps every
# subprocess call on the same binaries the Rust media engine uses and avoids
# PATH changes after startup influencing analysis. No-op on a dev box where
# ffmpeg is already on PATH (the var is just unset). Accepts the dir itself or
# its `bin/` subfolder (BtbN zips extract to bin/).
_ff_dir = os.environ.get("SHELLX_CUT_FFMPEG_DIR", "").strip()
if _ff_dir:
    _candidates = [_ff_dir, os.path.join(_ff_dir, "bin")]
    _present = [d for d in _candidates if os.path.isdir(d)]
    if _present:
        os.environ["PATH"] = os.pathsep.join(_present + [os.environ.get("PATH", "")])

FFMPEG_BIN = shutil.which("ffmpeg") or "ffmpeg"
FFPROBE_BIN = shutil.which("ffprobe") or "ffprobe"

SCHEMA = "shellx-cut/perception/1"

# Wire discipline: stdout must carry EXACTLY one JSON document. whisperX (and
# friends) configure logging onto sys.stdout at import time, so main() swaps
# sys.stdout→sys.stderr before any heavy import and the report/error is
# written to this saved real stdout handle only.
REAL_STDOUT = sys.stdout

# Detection thresholds — the Rust check (black_or_frozen_frames) documents
# these same numbers; change them in BOTH places or not at all.
BLACKDETECT_MIN_S = 0.3
FREEZEDETECT_MIN_S = 2.0
# cropdetect (content_bbox / uniform-border detection). limit=24 = the
# luma threshold below which a pixel counts as "border" (ffmpeg default is
# higher; 24 is conservative so faint UI chrome is NOT mistaken for a band).
# round=2 keeps the bbox even (yuv420 chroma alignment, matches the renderer).
# A content edge inset by MORE than this many px on any side is treated as a
# real uniform border (sub-tolerance insets are cropdetect jitter, not bands)
# — mirrored in checks.rs uniform_border; change in BOTH or neither.
CROPDETECT_LIMIT = 24
CROPDETECT_ROUND = 2
CONTENT_BBOX_EDGE_TOL_PX = 8
# Mirrored in checks.rs silence_at_edges details.detector (detector-visibility contract) —
# change them in BOTH places or not at all.
SILENCE_NOISE_DB = -35  # silencedetect noise floor
SILENCE_MIN_S = 0.3  # minimum silence span both detectors report


def log(msg: str) -> None:
    """Progress/diagnostics to stderr — stdout is reserved for the report."""
    print(f"[instruments] {msg}", file=sys.stderr, flush=True)


def die(code: str, message: str, cause: str):
    """Emit the error contract on the REAL stdout and exit nonzero."""
    print(json.dumps({"error": {"code": code, "message": message, "cause": cause}}),
          file=REAL_STDOUT, flush=True)
    sys.exit(1)


def finite_float(value, default=None) -> float:
    """Parse an external numeric token, rejecting NaN/Inf before conversion."""
    try:
        dec = Decimal(str(value).strip())
    except (InvalidOperation, ValueError):
        if default is not None:
            return default
        raise ValueError(f"not a number: {value!r}")
    if not dec.is_finite():
        if default is not None:
            return default
        raise ValueError(f"non-finite number: {value!r}")
    return dec.__float__()


def finite_number(value, default=0.0) -> float:
    """Convert trusted numeric scalars while rejecting NaN/Inf."""
    try:
        if isinstance(value, str):
            return finite_float(value, default)
        if hasattr(value, "item"):
            value = value.item()
        if hasattr(value, "__float__"):
            out = value.__float__()
        else:
            out = finite_float(value, default)
    except (TypeError, ValueError, InvalidOperation):
        if default is not None:
            return default
        raise
    if not math.isfinite(out):
        if default is not None:
            return default
        raise ValueError(f"non-finite number: {value!r}")
    return out


def run_ffmpeg(args: list) -> str:
    """Run ffmpeg, return stderr text (where all filter logs go)."""
    proc = subprocess.run(
        [FFMPEG_BIN, "-nostats", "-hide_banner", *args],
        capture_output=True,
        text=True,
    )
    # ffmpeg -f null exits 0 on success; nonzero means the INPUT is bad.
    if proc.returncode != 0:
        die("sidecar", "ffmpeg analysis failed", proc.stderr.strip()[-500:])
    return proc.stderr


def media_duration_ms(path: str) -> int:
    """Container duration via ffprobe (ms)."""
    proc = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-show_entries", "format=duration",
         "-of", "default=noprint_wrappers=1:nokey=1", path],
        capture_output=True, text=True,
    )
    if proc.returncode != 0 or not proc.stdout.strip():
        die("sidecar", "ffprobe failed", proc.stderr.strip()[-300:])
    return int(finite_float(proc.stdout.strip()) * 1000)


def has_video_stream(path: str) -> bool:
    """True when the file carries at least one video stream (ffprobe).
    audio-only media guard defense: video instruments (scenes/blackdetect/freezedetect)
    crash on audio-only files — PySceneDetect raises VideoOpenFailure — so
    they are skipped (and dropped from instruments_run, keeping the cache
    honest) when no video stream exists. The Rust side normally never
    requests them for kind=="audio"; this guard covers direct CLI use."""
    proc = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-select_streams", "v",
         "-show_entries", "stream=codec_type", "-of", "csv=p=0", path],
        capture_output=True, text=True,
    )
    return proc.returncode == 0 and proc.stdout.strip() != ""


def has_audio_stream(path: str) -> bool:
    """True when the file carries at least one audio stream (ffprobe).
    Audio instruments (words/silence/beats/loudness) crash on a file with no
    audio — whisperX yields no segments and IndexErrors during alignment;
    extract_wav16k's ffmpeg produces 'Output file does not contain any stream';
    ebur128 has nothing to measure. A video-only clip (b-roll, screen demo with
    no narration) and a legitimately SILENT render are normal inputs, so those
    instruments are skipped (and dropped from instruments_run, keeping the cache
    honest) rather than failing the whole job."""
    proc = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-select_streams", "a",
         "-show_entries", "stream=codec_type", "-of", "csv=p=0", path],
        capture_output=True, text=True,
    )
    return proc.returncode == 0 and proc.stdout.strip() != ""


def audio_energy_envelope(path: str, step_ms: int = 100) -> list:
    """CPU-only: RMS energy per step_ms bucket (ffprobe astats).

    Returns a list of floats (RMS dBFS, roughly -inf to 0) at `step_ms` intervals.
    Used for the floor-heuristic active-speaker gate in the subject instrument:
    - any bucket < SILENCE_FLOOR (-50 dBFS) means silence; speaking = above.
    - Zero-cost: a single ffprobe pass, no model/weights, no extra deps.
    Returns [] if the file has no audio stream or ffprobe fails."""
    proc = subprocess.run(
        [FFPROBE_BIN, "-v", "error",
         "-f", "lavfi",
         "-i", f"amovie={path},astats=metadata=1:reset={max(1, step_ms // 10)}",
         "-show_entries", "frame_tags=lavfi.astats.Overall.RMS_level",
         "-of", "csv=p=0"],
        capture_output=True, text=True, timeout=60,
    )
    if proc.returncode != 0:
        return []
    out = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        out.append(finite_float(line, -90.0) if line else -90.0)
    return out


def extract_wav16k(path: str, tmpdir: str) -> str:
    """Extract mono 16 kHz WAV — shared input for silero/whisper/beat analysis."""
    wav = str(Path(tmpdir) / "audio16k.wav")
    proc = subprocess.run(
        [FFMPEG_BIN, "-nostats", "-hide_banner", "-y", "-i", path,
         "-vn", "-ac", "1", "-ar", "16000", "-f", "wav", wav],
        capture_output=True, text=True,
    )
    if proc.returncode != 0:
        die("sidecar", "audio extraction failed", proc.stderr.strip()[-500:])
    return wav


# ---------------------------------------------------------------------------
# words — PRIMARY: NVIDIA Parakeet-TDT via ONNX (onnx-asr). FALLBACK: whisperX
# then faster-whisper.
# ---------------------------------------------------------------------------

# Long audio is transcribed in fixed chunks to report real sub-progress and keep
# memory bounded. A word that straddles a 5-min seam can split into two
# partial words — negligible for captions/cut at one boundary every 300 s.
STT_CHUNK_S = 300
# Default Parakeet model: the ~25-language MULTILINGUAL checkpoint (v3). Chosen as
# the default over the English-only v2 to remove the friction of a non-English user
# getting an empty transcript and having to know to switch models — v3 also handles
# English well. Override with SHELLX_CUT_STT_MODEL (e.g. nemo-canary-1b-v2 for
# the smaller-language tier, or whisperx-large-v3 for the compatibility fallback).
STT_MODEL_DEFAULT = "nemo-parakeet-tdt-0.6b-v3"
STT_CANARY_MODEL = "nemo-canary-1b-v2"
STT_CANARY_CHUNK_S = 30
# onnx-asr gives token START times only (no durations). A word's END is the last
# token's start + a TAIL — but the raw gap to the next word ALSO contains any
# silence, so using it verbatim inflates a word before a pause (cuts in that
# silence then fail cut_on_word). Measured on real footage: speech token tails
# are <=320 ms (p90), silences are 1280-2560 ms — so cap the tail at 400 ms and
# treat any >600 ms inter-token gap as a word boundary (silence) even when the
# BPE emitted no leading space. This yields tight word spans with real gaps.
STT_TAIL_CAP_MS = 400
STT_GAP_SPLIT_MS = 600
_MMS_FA_CACHE = None


def _emit_progress(frac: float, label: str) -> None:
    """Machine-parseable progress line for the Rust sidecar streamer.
    `cut-perception::sidecar` greps stderr for `PROGRESS <frac> <label>` and
    forwards it to the enrich job so the UI shows real transcription progress
    instead of a frozen number."""
    print(f"[instruments] PROGRESS {max(0.0, min(1.0, frac)):.3f} {label}",
          file=sys.stderr, flush=True)


def _aggregate_parakeet_words(tokens: list, timestamps: list,
                              offset_ms: int, audio_end_ms: int) -> list:
    """Aggregate Parakeet BPE tokens → words with TIGHT, silence-excluding spans.
    A new word opens on a leading-space token OR a >STT_GAP_SPLIT_MS inter-token
    gap (a pause is a word boundary even when the BPE emitted no space). A word's
    end = its last token's start + the gap to the next word, CAPPED at
    STT_TAIL_CAP_MS so silence after the word is not absorbed into it (which
    would make cuts in that silence fail cut_on_word). `offset_ms` shifts a
    chunk's relative timestamps into absolute media time; `audio_end_ms` bounds
    the final word. Returns word dicts WITHOUT `idx` (the caller assigns them).

    GUARD: if `tokens` is non-empty but `timestamps` is EMPTY, raise instead of
    silently returning []. An attention-based model (e.g. Canary) returns text
    tokens with NO frame-aligned timestamps; `zip(tokens, [])` would yield nothing
    and DISCARD the whole transcript with no error. Raising lets the caller fall
    back to a timestamped model (whisperX) rather than producing an empty result."""
    if tokens and not timestamps:
        raise RuntimeError(
            f"STT model returned {len(tokens)} text tokens but NO timestamps — "
            "needs a timestamped model (Parakeet/whisperX) or forced alignment; "
            "refusing to emit an empty transcript")
    # Pass 1 — group tokens into words, tracking each word's first + last token
    # start (absolute ms).
    groups = []  # {"text": str, "start_ms": int, "last_start_ms": int}
    prev_ts = None
    for tok, ts in zip(tokens, timestamps):
        start_ms = int(ts * 1000) + offset_ms
        big_gap = prev_ts is not None and (ts - prev_ts) * 1000 > STT_GAP_SPLIT_MS
        if tok.startswith(" ") or big_gap or not groups:
            groups.append({"text": tok, "start_ms": start_ms, "last_start_ms": start_ms})
        else:
            groups[-1]["text"] += tok
            groups[-1]["last_start_ms"] = start_ms
        prev_ts = ts
    # Pass 2 — compute each word's end from the (capped) tail to the next word.
    out = []
    for i, g in enumerate(groups):
        next_start = groups[i + 1]["start_ms"] if i + 1 < len(groups) else audio_end_ms
        tail = min(max(next_start - g["last_start_ms"], 0), STT_TAIL_CAP_MS)
        end_ms = g["last_start_ms"] + tail
        text = g["text"].strip()
        if not text:
            continue
        out.append({
            "word": text,
            "start_ms": g["start_ms"],
            # guarantee a strictly positive span even for a single-token word.
            "end_ms": max(end_ms, g["start_ms"] + 1),
            "confidence": None,
        })
    return out


_MMS_TRANSLIT = {
    "ß": "ss", "æ": "ae", "ø": "o", "å": "a", "ð": "d", "þ": "th",
    "đ": "d", "ł": "l", "ħ": "h", "ı": "i", "ĸ": "k", "ŧ": "t",
    "а": "a", "б": "b", "в": "v", "г": "g", "д": "d", "е": "e",
    "ё": "e", "ж": "zh", "з": "z", "и": "i", "й": "i", "к": "k",
    "л": "l", "м": "m", "н": "n", "о": "o", "п": "p", "р": "r",
    "с": "s", "т": "t", "у": "u", "ф": "f", "х": "h", "ц": "ts",
    "ч": "ch", "ш": "sh", "щ": "sht", "ъ": "a", "ы": "y", "ь": "",
    "э": "e", "ю": "yu", "я": "ya", "і": "i", "ї": "yi", "є": "ye",
    "ґ": "g",
    "α": "a", "β": "v", "γ": "g", "δ": "d", "ε": "e", "ζ": "z",
    "η": "i", "θ": "th", "ι": "i", "κ": "k", "λ": "l", "μ": "m",
    "ν": "n", "ξ": "x", "ο": "o", "π": "p", "ρ": "r", "σ": "s",
    "ς": "s", "τ": "t", "υ": "y", "φ": "f", "χ": "ch", "ψ": "ps",
    "ω": "o",
}


def _selected_is_canary(selected: str) -> bool:
    s = (selected or "").split("@", 1)[0].strip().lower()
    return s == "canary" or s.startswith("canary-") or s.startswith("nemo-canary")


def _canary_model_id(selected: str) -> str:
    s = (selected or "").split("@", 1)[0].strip()
    if not s or s.lower() in {"canary", "canary-1b-v2"}:
        return STT_CANARY_MODEL
    return s


def _extract_asr_text(result) -> str:
    """Best-effort text extraction across onnx-asr model result shapes."""
    if result is None:
        return ""
    if isinstance(result, str):
        return result.strip()
    if isinstance(result, dict):
        for key in ("text", "transcript", "sentence"):
            val = result.get(key)
            if isinstance(val, str):
                return val.strip()
        return ""
    for attr in ("text", "transcript", "sentence"):
        val = getattr(result, attr, None)
        if isinstance(val, str):
            return val.strip()
    tokens = getattr(result, "tokens", None)
    if tokens:
        return "".join(str(t) for t in tokens).strip()
    return str(result).strip()


def _recognize_text(model, audio, lang_hint: str | None) -> str:
    calls = []
    if lang_hint:
        calls.extend((
            lambda: model.recognize(audio, language=lang_hint),
            lambda: model.recognize(audio, lang=lang_hint),
        ))
    calls.append(lambda: model.recognize(audio))
    last_type_error = None
    for call in calls:
        try:
            return _extract_asr_text(call())
        except TypeError as e:
            last_type_error = e
            continue
    if last_type_error:
        raise last_type_error
    return ""


def _normalize_for_mms(word: str) -> str:
    chars = []
    for ch in unicodedata.normalize("NFKD", (word or "").lower()):
        if unicodedata.combining(ch):
            continue
        mapped = _MMS_TRANSLIT.get(ch, ch)
        for out in mapped:
            if "a" <= out <= "z":
                chars.append(out)
    return "".join(chars)


def _mms_word_pairs(text: str) -> list[tuple[str, str]]:
    pairs = []
    for raw in re.findall(r"\S+", text or ""):
        display = re.sub(r"^\W+|\W+$", "", raw, flags=re.UNICODE).strip()
        norm = _normalize_for_mms(display or raw)
        if display and norm:
            pairs.append((display, norm))
    return pairs


def _mms_fa_bundle():
    global _MMS_FA_CACHE
    if _MMS_FA_CACHE is None:
        import torch
        import torchaudio

        bundle = torchaudio.pipelines.MMS_FA
        device = "cuda" if torch.cuda.is_available() else "cpu"
        model = bundle.get_model().to(device).eval()
        _MMS_FA_CACHE = {
            "bundle": bundle,
            "model": model,
            "tokenizer": bundle.get_tokenizer(),
            "aligner": bundle.get_aligner(),
            "device": device,
            "torch": torch,
            "torchaudio": torchaudio,
        }
    return _MMS_FA_CACHE


def _token_span_score(spans: list) -> float | None:
    total = 0
    weighted = 0.0
    for span in spans:
        try:
            width = max(1, len(span))
        except TypeError:
            width = max(1, int(getattr(span, "end", 0)) - int(getattr(span, "start", 0)))
        score = getattr(span, "score", None)
        if score is None:
            continue
        total += width
        weighted += finite_float(score, 0.0) * width
    return round(weighted / total, 4) if total else None


def _mms_spans_to_words(
    word_pairs: list[tuple[str, str]],
    token_spans: list,
    ratio: float,
    offset_ms: int,
    audio_end_ms: int,
) -> list:
    """Map MMS_FA token spans to ShellX word spans.

    Kept pure so tests can lock the timestamp math without downloading Canary,
    torch, or MMS_FA model weights.
    """
    words = []
    for (display, _norm), spans in zip(word_pairs, token_spans):
        if not spans:
            continue
        raw_start_ms = offset_ms + int(finite_float(spans[0].start) * ratio * 1000)
        raw_end_ms = offset_ms + int(finite_float(spans[-1].end) * ratio * 1000)
        if audio_end_ms <= offset_ms or raw_end_ms <= offset_ms or raw_start_ms >= audio_end_ms:
            continue
        start_ms = max(offset_ms, min(raw_start_ms, audio_end_ms - 1))
        end_ms = max(offset_ms, min(raw_end_ms, audio_end_ms))
        if end_ms <= start_ms:
            end_ms = min(audio_end_ms, start_ms + 1)
        if end_ms <= start_ms:
            continue
        words.append({
            "word": display,
            "start_ms": start_ms,
            "end_ms": end_ms,
            "confidence": _token_span_score(spans),
        })
    return words


def _mms_align_words(audio, sample_rate: int, word_pairs: list[tuple[str, str]], offset_ms: int) -> list:
    if not word_pairs:
        return []
    fa = _mms_fa_bundle()
    bundle = fa["bundle"]
    torch = fa["torch"]
    torchaudio = fa["torchaudio"]

    waveform = torch.as_tensor(audio, dtype=torch.float32).unsqueeze(0)
    if waveform.numel() == 0:
        return []
    if sample_rate != bundle.sample_rate:
        waveform = torchaudio.functional.resample(waveform, sample_rate, bundle.sample_rate)

    transcript = [norm for _display, norm in word_pairs]
    with torch.inference_mode():
        emission, _ = fa["model"](waveform.to(fa["device"]))
        token_spans = fa["aligner"](emission[0], fa["tokenizer"](transcript))

    ratio = waveform.size(1) / emission.size(1) / bundle.sample_rate
    audio_end_ms = offset_ms + int(waveform.size(1) / bundle.sample_rate * 1000)
    return _mms_spans_to_words(word_pairs, token_spans, ratio, offset_ms, audio_end_ms)


def _words_via_canary(media_path: str, asset_id: str) -> dict:
    """Transcribe with Canary-1B-v2 text, then force-align words with MMS_FA.
    Canary's ONNX result supports the weak-language tier, but its
    timestamp arrays are empty. This route refuses to return text unless forced
    alignment produced real word spans, so transcript edits never see an empty
    or untimed transcript masquerading as success."""
    import onnx_asr
    import soundfile as sf

    selected = os.environ.get("SHELLX_CUT_STT_MODEL") or ""
    model_id = _canary_model_id(selected)
    lang_hint = (os.environ.get("SHELLX_CUT_STT_LANG") or "").strip() or None
    load_kwargs = {}
    if sys.platform == "darwin":
        load_kwargs["providers"] = ["CPUExecutionProvider"]
    _emit_progress(0.05, "transcribe:loading-canary")
    log(f"words: canary onnx-asr model={model_id} + MMS_FA aligner lang={lang_hint or 'auto'}")
    model = onnx_asr.load_model(model_id, **load_kwargs)

    with tempfile.TemporaryDirectory(prefix="cut-stt-canary-") as td:
        wav = extract_wav16k(media_path, td)
        data, sr = sf.read(wav, dtype="float32")
    if getattr(data, "ndim", 1) > 1:
        data = data.mean(axis=1)

    chunk = STT_CANARY_CHUNK_S * sr
    words = []
    if len(data) <= chunk:
        _emit_progress(0.2, "transcribe:canary-text")
        text = _recognize_text(model, data, lang_hint)
        pairs = _mms_word_pairs(text)
        _emit_progress(0.55, "transcribe:mms-align")
        words = _mms_align_words(data, sr, pairs, 0)
        _emit_progress(0.95, "transcribe:done")
    else:
        n = (len(data) + chunk - 1) // chunk
        for i in range(n):
            seg = data[i * chunk:(i + 1) * chunk]
            offset_ms = int(i * STT_CANARY_CHUNK_S * 1000)
            text = _recognize_text(model, seg, lang_hint)
            pairs = _mms_word_pairs(text)
            words.extend(_mms_align_words(seg, sr, pairs, offset_ms))
            _emit_progress(0.1 + 0.85 * (i + 1) / n, f"transcribe:canary chunk {i + 1}/{n}")

    if not words and len(data) > 0:
        raise RuntimeError("Canary produced no aligned word spans via MMS_FA")
    for j, w in enumerate(words):
        w["idx"] = j
    suffix = "-cpu" if sys.platform == "darwin" else ""
    return {
        "asset": asset_id,
        "model": f"canary/{model_id}+mms-fa@onnx{suffix}",
        "language": lang_hint,
        "words": words,
    }


def _words_via_parakeet(media_path: str, asset_id: str, model_override: str | None = None) -> dict:
    """Transcribe with Parakeet-TDT via onnx-asr (no torch). Raises on any
    failure so instrument_words can fall back to whisperX. Chunks audio > 5 min
    for sub-progress + bounded memory. macOS forces the CPU EP (CoreML fails on
    this model's external-data initialization)."""
    import onnx_asr  # raises ImportError on an older venv → caller falls back
    import soundfile as sf

    model_id = model_override or os.environ.get("SHELLX_CUT_STT_MODEL") or STT_MODEL_DEFAULT
    load_kwargs = {}
    if sys.platform == "darwin":
        load_kwargs["providers"] = ["CPUExecutionProvider"]
    _emit_progress(0.05, "transcribe:loading-model")
    log(f"words: parakeet onnx-asr model={model_id} provider="
        f"{'cpu(macos)' if sys.platform == 'darwin' else 'auto'}")
    model = onnx_asr.load_model(model_id, **load_kwargs).with_timestamps()

    # Decode to the same mono 16 kHz the rest of the sidecar uses, then read it
    # as a float32 array so chunking is in-memory (one ffmpeg call, not N).
    with tempfile.TemporaryDirectory(prefix="cut-stt-") as td:
        wav = extract_wav16k(media_path, td)
        data, _sr = sf.read(wav, dtype="float32")
    if getattr(data, "ndim", 1) > 1:  # defensive — extract_wav16k forces mono
        data = data.mean(axis=1)
    chunk = STT_CHUNK_S * 16000
    total_ms = int(len(data) / 16000.0 * 1000)

    words = []
    if len(data) <= chunk:
        _emit_progress(0.2, "transcribe:running")
        r = model.recognize(data)
        words = _aggregate_parakeet_words(r.tokens or [], r.timestamps or [], 0, total_ms)
        _emit_progress(0.95, "transcribe:done")
    else:
        n = (len(data) + chunk - 1) // chunk
        for i in range(n):
            seg = data[i * chunk:(i + 1) * chunk]
            offset_ms = i * STT_CHUNK_S * 1000
            seg_end_ms = offset_ms + int(len(seg) / 16000.0 * 1000)
            r = model.recognize(seg)
            words.extend(_aggregate_parakeet_words(r.tokens or [], r.timestamps or [], offset_ms, seg_end_ms))
            _emit_progress(0.1 + 0.85 * (i + 1) / n, f"transcribe:chunk {i + 1}/{n}")

    for j, w in enumerate(words):
        w["idx"] = j
    suffix = "-cpu" if sys.platform == "darwin" else ""
    # Language: the user's SHELLX_CUT_STT_LANG hint wins; else v2 is
    # English and v3/multilingual auto-detects (reported as None).
    lang_hint = (os.environ.get("SHELLX_CUT_STT_LANG") or "").strip() or None
    return {
        "asset": asset_id,
        "model": f"parakeet-tdt/{model_id}@onnx{suffix}",
        "language": lang_hint or ("en" if model_id.endswith("v2") else None),
        "words": words,
    }


# Languages where Parakeet-TDT v3's weights underperform (FLEURS WER well
# above its English ~5%). Canary-1B-v2 is the preferred tier here, with MMS_FA
# forced alignment supplying the word spans Canary does not emit natively.
# Tunable; the strong tier (en/de/fr/es/it/pt/nl/pl/ru/uk) stays on fast Parakeet.
PARAKEET_WEAK_LANGS = {
    "lv", "lt", "et", "mt", "sl", "hr", "sk", "fi", "bg", "da", "el", "hu", "ro", "sv",
}


def instrument_words(media_path: str, asset_id: str, model_name: str) -> dict:
    """Word-level transcript. PRIMARY engine: NVIDIA Parakeet-TDT via ONNX
    (onnx-asr) — native word timestamps, no torch, immune to the torch>=2.6
    `weights_only` break that disabled whisper on fresh installs. FALLS BACK to
    whisperX (transcribe + forced alignment), then faster-whisper, when onnx-asr
    is absent (older venv) or errors. The engine that ran is recorded in `model`.

    If the user EXPLICITLY selected a Whisper model (SHELLX_CUT_STT_MODEL =
    `whisperx-<name>` / `whisper-<name>`, optionally `@device`), honor it directly —
    parse its name and skip Parakeet entirely. Without this, a chosen Whisper id
    failed the Parakeet (onnx-asr) load and silently fell through to the whisper
    FALLBACK default (`small`) instead of the model the user picked.

    Canary-1B-v2 is the weak-language tier, but it has no native word
    timestamps. Route it through MMS_FA forced alignment before exposing it to
    transcript edits; if that path fails, degrade through the existing Whisper
    fallback chain instead of returning empty words."""
    selected = (os.environ.get("SHELLX_CUT_STT_MODEL") or "").strip()
    lang_hint = (os.environ.get("SHELLX_CUT_STT_LANG") or "").strip().lower() or None
    force_whisper = False
    explicit_whisper = False
    canary_failed = False
    if selected.lower().startswith(("whisperx-", "whisper-")):
        name = selected.split("@", 1)[0]
        for pfx in ("whisperx-", "whisper-"):
            if name.lower().startswith(pfx):
                name = name[len(pfx):]
                break
        if name:
            model_name = name
        force_whisper = True
        explicit_whisper = True
    elif _selected_is_canary(selected):
        try:
            return _words_via_canary(media_path, asset_id)
        except Exception as e:  # noqa: BLE001 — degrade to the timestamped fallback chain
            canary_failed = True
            force_whisper = True
            model_name = "large-v3"
            log(f"words: Canary/MMS_FA failed ({e!r}) — falling back to whisperx large-v3")
    elif lang_hint in PARAKEET_WEAK_LANGS and not selected:
        # The user PINNED a weak-tier language and did NOT explicitly pick a
        # model -> prefer Canary-1B-v2 + MMS_FA timestamps. SOFT preference: if
        # Canary or whisperX are unavailable, fall back to Parakeet at the end.
        try:
            return _words_via_canary(media_path, asset_id)
        except Exception as e:  # noqa: BLE001
            canary_failed = True
            force_whisper = True
            model_name = "large-v3"
            log(
                f"words: language '{lang_hint}' Canary/MMS_FA unavailable ({e!r}) "
                "— falling back to whisperx large-v3"
            )

    if force_whisper and not explicit_whisper and model_name != "large-v3":
        model_name = "large-v3"

    # --- primary: Parakeet-TDT via onnx-asr (unless a Whisper model was chosen) --
    parakeet_tried = False
    if not force_whisper:
        try:
            return _words_via_parakeet(media_path, asset_id)
        except Exception as e:  # noqa: BLE001 — degrade to the whisper fallback chain
            parakeet_tried = True
            log(f"words: parakeet/onnx-asr unavailable ({e!r}) — falling back to whisperx")
    else:
        log(f"words: Whisper preferred ({selected or lang_hint}) -> model={model_name}, skipping parakeet")

    # --- fallback: whisperX → faster-whisper (GPU→CPU) --------------------
    import torch  # imported first so torch's bundled cuDNN is loaded for ctranslate2

    devices = ["cuda", "cpu"] if torch.cuda.is_available() else ["cpu"]
    last_err = None
    for device in devices:
        compute = "float16" if device == "cuda" else "int8"
        # --- primary engine: whisperX ----------------------------------
        try:
            import whisperx

            # SHELLX_CUT_STT_LANG pins the language for the whisperX
            # fallback; unset → whisperX auto-detects (its default).
            lang_hint = (os.environ.get("SHELLX_CUT_STT_LANG") or "").strip() or None
            log(f"words: whisperx model={model_name} device={device} lang={lang_hint or 'auto'}")
            wmodel = whisperx.load_model(model_name, device, compute_type=compute)
            audio = whisperx.load_audio(media_path)
            result = wmodel.transcribe(audio, batch_size=8, language=lang_hint)
            # No detectable speech (music/tone/ambient/applause) is a LEGITIMATE
            # input, not an error — return zero words honestly instead of running
            # forced alignment, which IndexErrors on empty segments and used to
            # fail the whole import chain. This is the no-speech regression path.
            if not result.get("segments"):
                log(f"words: no speech detected (whisperx, {device}) — empty transcript")
                return {
                    "asset": asset_id,
                    "model": f"whisperx-{model_name}@{device}",
                    "language": result.get("language"),
                    "words": [],
                }
            align_model, meta = whisperx.load_align_model(
                language_code=result["language"], device=device
            )
            aligned = whisperx.align(
                result["segments"], align_model, meta, audio, device,
                return_char_alignments=False,
            )
            words = []
            for w in aligned.get("word_segments", []):
                # Alignment occasionally yields words without timestamps
                # (digits, OOV) — skip them rather than invent timings.
                if "start" not in w or "end" not in w:
                    continue
                words.append({
                    "idx": len(words),
                    "word": w["word"].strip(),
                    "start_ms": int(w["start"] * 1000),
                    "end_ms": int(w["end"] * 1000),
                    "confidence": round(finite_float(w.get("score", 0.0), 0.0), 4) or None,
                })
            return {
                "asset": asset_id,
                "model": f"whisperx-{model_name}@{device}",
                "language": result.get("language"),
                "words": words,
            }
        except Exception as e:  # noqa: BLE001 — fall through to next engine
            last_err = e
            log(f"words: whisperx failed on {device} ({e!r}) — trying faster-whisper")
        # --- documented fallback: faster-whisper ------------------------
        try:
            from faster_whisper import WhisperModel

            log(f"words: faster-whisper model={model_name} device={device}")
            fw = WhisperModel(model_name, device=device, compute_type=compute)
            segments, info = fw.transcribe(media_path, word_timestamps=True)
            words = []
            for seg in segments:
                for w in seg.words or []:
                    words.append({
                        "idx": len(words),
                        "word": w.word.strip(),
                        "start_ms": int(w.start * 1000),
                        "end_ms": int(w.end * 1000),
                        "confidence": round(finite_float(w.probability, 0.0), 4),
                    })
            return {
                "asset": asset_id,
                "model": f"faster-whisper-{model_name}@{device}",
                "language": getattr(info, "language", None),
                "words": words,
            }
        except Exception as e:  # noqa: BLE001 — try the next device
            last_err = e
            log(f"words: faster-whisper failed on {device} ({e!r})")
    # If whisperX was only PREFERRED for a weak-tier language (not an explicit
    # user choice) and it's unavailable here, fall back to Parakeet as a last resort —
    # a low-quality transcript beats none on a Parakeet-only cold install.
    if not explicit_whisper and not parakeet_tried:
        try:
            log("words: whisper unavailable — last-resort Parakeet (low quality for this language)")
            fallback_model = STT_MODEL_DEFAULT if canary_failed else None
            return _words_via_parakeet(media_path, asset_id, fallback_model)
        except Exception as e:  # noqa: BLE001
            last_err = e
    die("sidecar", "all transcription engines failed", repr(last_err))


# ---------------------------------------------------------------------------
# silence — silero-vad + ffmpeg silencedetect cross-check
# ---------------------------------------------------------------------------

def _ffmpeg_silences(media_path: str) -> list:
    """ffmpeg silencedetect spans (ms)."""
    stderr = run_ffmpeg([
        "-i", media_path, "-vn",
        "-af", f"silencedetect=noise={SILENCE_NOISE_DB}dB:d={SILENCE_MIN_S}",
        "-f", "null", "-",
    ])
    spans, start = [], None
    for line in stderr.splitlines():
        m = re.search(r"silence_start:\s*(-?[\d.]+)", line)
        if m:
            start = max(0.0, finite_float(m.group(1)))
        m = re.search(r"silence_end:\s*([\d.]+)", line)
        if m and start is not None:
            spans.append((int(start * 1000), int(finite_float(m.group(1)) * 1000)))
            start = None
    # Unclosed silence at EOF: silencedetect omits silence_end.
    if start is not None:
        spans.append((int(start * 1000), media_duration_ms(media_path)))
    return spans


def instrument_silence(media_path: str, wav16k: str) -> list:
    """Silero-vad speech spans inverted to silences, cross-checked against
    ffmpeg silencedetect. source = both|silero|ffmpeg (overlap >= 50% of the
    shorter span counts as agreement). Silero is a quality enhancement, not a
    hard dependency: final-render verification must still measure FFmpeg's
    objective silence spans on a base-only perception install."""
    ffm = _ffmpeg_silences(media_path)
    try:
        import soundfile as sf
        import torch
        from silero_vad import load_silero_vad, get_speech_timestamps

        log("silence: silero-vad")
        model = load_silero_vad()
        data, sr = sf.read(wav16k, dtype="float32")
        if getattr(data, "ndim", 1) > 1:
            data = data.mean(axis=1)
        if sr != 16000:
            raise RuntimeError(f"expected 16 kHz extracted wav, got {sr}")
        wav = torch.from_numpy(data.copy())  # 16 kHz mono tensor; avoids torchaudio I/O.
        speech = get_speech_timestamps(wav, model)  # [{start,end}] in samples
        total_ms = int(len(wav) / 16000 * 1000)

        # Invert speech → silence (gaps + head + tail), min span SILENCE_MIN_S.
        min_ms = int(SILENCE_MIN_S * 1000)
        silero = []
        cursor = 0
        for s in speech:
            start_ms, end_ms = int(s["start"] / 16), int(s["end"] / 16)
            if start_ms - cursor >= min_ms:
                silero.append((cursor, start_ms))
            cursor = end_ms
        if total_ms - cursor >= min_ms:
            silero.append((cursor, total_ms))
    except Exception as e:  # optional full-perception stack
        log(f"silence: silero unavailable ({e!r}) — using ffmpeg silencedetect")
        return [
            {"start_ms": start, "end_ms": end, "source": "ffmpeg"}
            for start, end in ffm
        ]

    def overlaps(a, b) -> bool:
        inter = min(a[1], b[1]) - max(a[0], b[0])
        return inter > 0 and inter * 2 >= min(a[1] - a[0], b[1] - b[0])

    out = []
    matched_ffm = set()
    for span in silero:
        src = "silero"
        for i, f in enumerate(ffm):
            if overlaps(span, f):
                src = "both"
                matched_ffm.add(i)
                break
        out.append({"start_ms": span[0], "end_ms": span[1], "source": src})
    for i, f in enumerate(ffm):
        if i not in matched_ffm:
            out.append({"start_ms": f[0], "end_ms": f[1], "source": "ffmpeg"})
    out.sort(key=lambda s: s["start_ms"])
    return out


# ---------------------------------------------------------------------------
# scenes — PySceneDetect + blackdetect/freezedetect video-defect spans
# ---------------------------------------------------------------------------

def probe_video_geometry(path: str):
    """(width, height, duration_ms) of the first video stream via ffprobe.
    Used by content-bbox detection to know the full frame + sampling span."""
    proc = subprocess.run(
        [FFPROBE_BIN, "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height", "-show_entries", "format=duration",
         "-of", "json", path],
        capture_output=True, text=True,
    )
    if proc.returncode != 0 or not proc.stdout.strip():
        die("sidecar", "ffprobe (geometry) failed", proc.stderr.strip()[-300:])
    data = json.loads(proc.stdout)
    stream = (data.get("streams") or [{}])[0]
    w = int(stream.get("width") or 0)
    h = int(stream.get("height") or 0)
    dur = data.get("format", {}).get("duration")
    dur_ms = int(finite_float(dur, 0.0) * 1000) if dur else 0
    return w, h, dur_ms


def detect_content_bbox(media_path: str):
    """content_bbox: sample ffmpeg cropdetect across several windows of the
    clip and report the content rectangle (non-uniform pixels) + whether a
    baked-in uniform border (letterbox/pillarbox) exists.

    WHY SAMPLED, NOT WHOLE-CLIP: a single cropdetect pass keys on the LAST
    frame's running bbox, which a transient full-frame flash (a fade, a white
    splash) can widen wrongly. Sampling a few short windows and taking the
    INTERSECTION (max x/y, min right/bottom) yields the largest border ALL
    windows agree is uniform — robust against a single odd frame, and exactly
    the bands the OBS-capture driver baked in stay detected.

    Deterministic: fixed sample offsets (fractions of the duration), fixed
    cropdetect params (CROPDETECT_LIMIT/ROUND). Returns the ContentBbox dict
    (app/perception/src/types.rs) or None when geometry is unavailable."""
    w, h, dur_ms = probe_video_geometry(media_path)
    if w == 0 or h == 0:
        return None  # no decodable video geometry — nothing to bound

    # Sample windows: skip the very start/end (intros/outros often differ),
    # spread across the body. 1.5s each is plenty for cropdetect to settle.
    dur_s = max(dur_ms / 1000.0, 0.0)
    if dur_s <= 2.0:
        offsets = [0.0]  # too short to sample — one pass from the start
    else:
        body = [0.15, 0.4, 0.65, 0.85]
        offsets = sorted({round(f * dur_s, 3) for f in body if f * dur_s < dur_s})
    win_s = 1.5

    boxes = []  # (x, y, w, h) per sample that produced a crop line
    for off in offsets:
        log(f"content_bbox: cropdetect @ {off:.2f}s")
        stderr = run_ffmpeg([
            "-ss", f"{off:.3f}", "-i", media_path, "-an", "-t", f"{win_s}",
            "-vf", f"cropdetect=limit={CROPDETECT_LIMIT}:round={CROPDETECT_ROUND}:reset=0",
            "-f", "null", "-",
        ])
        last = None
        for m in re.finditer(r"crop=(\d+):(\d+):(\d+):(\d+)", stderr):
            last = (int(m.group(3)), int(m.group(4)), int(m.group(1)), int(m.group(2)))
        if last is not None:
            boxes.append(last)
    if not boxes:
        # cropdetect found nothing croppable on every sample → full frame.
        boxes = [(0, 0, w, h)]

    # Intersection = the border ALL samples agree is uniform: the content rect
    # is the SMALLEST box (largest x/y, smallest right/bottom edge) across
    # samples, clamped to the frame.
    x = min(max(b[0] for b in boxes), w)
    y = min(max(b[1] for b in boxes), h)
    right = max(min(b[0] + b[2] for b in boxes), x)
    bottom = max(min(b[1] + b[3] for b in boxes), y)
    cw = min(right - x, w - x)
    ch = min(bottom - y, h - y)
    if cw <= 0 or ch <= 0:  # degenerate intersection — treat as full frame
        x, y, cw, ch = 0, 0, w, h

    # Uniform border = the content is inset by more than the tolerance on any
    # edge (sub-tolerance insets are cropdetect jitter, not real bands).
    inset = max(x, y, w - (x + cw), h - (y + ch))
    uniform_border = inset > CONTENT_BBOX_EDGE_TOL_PX

    return {
        "frame_width": w,
        "frame_height": h,
        "x": int(x),
        "y": int(y),
        "width": int(cw),
        "height": int(ch),
        "uniform_border": bool(uniform_border),
        "samples_agreed": len(boxes),
    }


def _ffmpeg_scene_cuts(media_path: str) -> list:
    """Scene-cut fallback requiring only the bundled FFmpeg.

    PySceneDetect remains the preferred detector, but it is installed with the
    best-effort full perception extras. `select` keeps this release check useful
    on a base-only install instead of aborting black/freeze/border measurement.
    """
    log("scenes: ffmpeg select fallback")
    stderr = run_ffmpeg([
        "-i", media_path, "-an",
        "-vf", "select=gt(scene\\,0.30),showinfo",
        "-fps_mode", "vfr", "-f", "null", "-",
    ])
    cuts = []
    for line in stderr.splitlines():
        m = re.search(r"showinfo.*?pts_time:([-+\d.eE]+)", line)
        if not m:
            continue
        at_ms = max(0, int(finite_float(m.group(1)) * 1000))
        if not cuts or cuts[-1]["at_ms"] != at_ms:
            cuts.append({"at_ms": at_ms, "score": None})
    return cuts


def instrument_scenes(media_path: str):
    """Returns (scene_cuts, black_spans, frozen_spans)."""
    try:
        from scenedetect import open_video, SceneManager
        from scenedetect.detectors import ContentDetector

        log("scenes: pyscenedetect ContentDetector")
        video = open_video(media_path)
        sm = SceneManager()
        sm.add_detector(ContentDetector())
        sm.detect_scenes(video)
        scene_list = sm.get_scene_list()
        # Cuts are the starts of every scene after the first.
        cuts = [
            {"at_ms": int(start.get_seconds() * 1000), "score": None}
            for start, _ in scene_list[1:]
        ]
    except Exception as e:  # scenedetect is an optional full-perception extra
        log(f"scenes: pyscenedetect unavailable ({e!r}) — using ffmpeg")
        cuts = _ffmpeg_scene_cuts(media_path)

    log("scenes: blackdetect + freezedetect")
    stderr = run_ffmpeg([
        "-i", media_path, "-an",
        "-vf", f"blackdetect=d={BLACKDETECT_MIN_S}:pix_th=0.10,"
               f"freezedetect=n=-60dB:d={FREEZEDETECT_MIN_S}",
        "-f", "null", "-",
    ])
    black, frozen = [], []
    for line in stderr.splitlines():
        m = re.search(r"black_start:([\d.]+)\s+black_end:([\d.]+)", line)
        if m:
            black.append({"start_ms": int(finite_float(m.group(1)) * 1000),
                          "end_ms": int(finite_float(m.group(2)) * 1000)})
    # freezedetect logs start and end on separate lines.
    fstart = None
    for line in stderr.splitlines():
        m = re.search(r"freeze_start:\s*([\d.]+)", line)
        if m:
            fstart = finite_float(m.group(1))
        m = re.search(r"freeze_end:\s*([\d.]+)", line)
        if m and fstart is not None:
            frozen.append({"start_ms": int(fstart * 1000),
                           "end_ms": int(finite_float(m.group(1)) * 1000)})
            fstart = None
    if fstart is not None:  # frozen through EOF — freeze_end never logged
        frozen.append({"start_ms": int(fstart * 1000),
                       "end_ms": media_duration_ms(media_path)})
    return cuts, black, frozen


# ---------------------------------------------------------------------------
# beats — lightweight energy peak grid
# ---------------------------------------------------------------------------

def instrument_beats(wav16k: str) -> dict:
    """Approximate beat grid over extracted mono audio without native JIT paths."""
    import wave
    import numpy as np

    log("beats: lightweight energy peaks")
    with wave.open(wav16k, "rb") as wf:
        sr = int(wf.getframerate() or 16000)
        channels = max(1, int(wf.getnchannels() or 1))
        width = int(wf.getsampwidth() or 2)
        raw = wf.readframes(wf.getnframes())

    if not raw:
        return {"bpm": 0.0, "beats_ms": []}
    if width == 1:
        samples = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    elif width == 2:
        samples = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif width == 4:
        samples = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648.0
    else:
        log(f"beats: unsupported wav sample width {width}; returning empty grid")
        return {"bpm": 0.0, "beats_ms": []}
    if channels > 1:
        usable = (samples.size // channels) * channels
        samples = samples[:usable].reshape(-1, channels).mean(axis=1)
    if samples.size < max(sr // 2, 1):
        return {"bpm": 0.0, "beats_ms": []}

    frame = max(512, int(sr * 0.064))
    hop = max(160, int(sr * 0.032))
    rms = []
    stop = max(samples.size - frame, 0)
    for start in range(0, stop + 1, hop):
        chunk = samples[start:start + frame]
        rms.append(finite_number(np.sqrt(np.mean(chunk * chunk)))) if chunk.size else None
    if len(rms) < 3:
        return {"bpm": 0.0, "beats_ms": []}

    energy = np.asarray(rms, dtype=np.float32)
    onset = np.maximum(np.diff(energy, prepend=energy[:1]), 0.0)
    threshold = finite_number(np.mean(onset) + 0.75 * np.std(onset))
    min_gap_frames = max(1, int(0.28 * sr / hop))
    peaks = []
    last = -min_gap_frames
    for idx in range(1, len(onset) - 1):
        if idx - last < min_gap_frames:
            continue
        if onset[idx] >= threshold and onset[idx] >= onset[idx - 1] and onset[idx] >= onset[idx + 1]:
            peaks.append(idx)
            last = idx

    beats_ms = [int(idx * hop * 1000 / sr) for idx in peaks]
    intervals = np.diff(np.asarray(beats_ms, dtype=np.float32))
    intervals = intervals[(intervals >= 250.0) & (intervals <= 2000.0)]
    bpm = round(finite_number(60000.0 / np.median(intervals)), 2) if intervals.size else 0.0
    return {"bpm": bpm, "beats_ms": beats_ms}


# ---------------------------------------------------------------------------
# loudness — ffmpeg ebur128
# ---------------------------------------------------------------------------

def instrument_loudness(media_path: str) -> dict:
    """ebur128 integrated LUFS + true peak + ~1s momentary windows."""
    log("loudness: ebur128")
    stderr = run_ffmpeg([
        "-i", media_path, "-vn",
        "-af", "ebur128=peak=true", "-f", "null", "-",
    ])
    # Per-frame lines: "t: 1.00237 ... M: -18.1 S: ... I: -17.9 LUFS ..."
    windows, last_bucket = [], -1
    for line in stderr.splitlines():
        m = re.search(r"t:\s*([\d.]+).*?M:\s*(-?[\d.]+|nan)", line)
        if m and m.group(2) != "nan":
            at_ms = int(finite_float(m.group(1)) * 1000)
            if at_ms // 1000 != last_bucket:  # subsample to one window per second
                last_bucket = at_ms // 1000
                windows.append({"at_ms": at_ms, "momentary_lufs": finite_float(m.group(2))})
    # Summary block: "I: -16.2 LUFS" under "Integrated loudness:", then
    # "Peak: -1.4 dBFS" under "True peak:". Take the LAST occurrences (the
    # summary repeats the same keys the per-frame lines use).
    integrated = None
    for m in re.finditer(r"I:\s*(-?[\d.]+)\s*LUFS", stderr):
        integrated = finite_float(m.group(1))
    peak = None
    # Silent audio prints "Peak: -inf dBFS" (regression: real screen
    # recording killed the whole import chain here). Silence is a MEASUREMENT,
    # not an error — floor -inf to a finite -99.0 dBTP so serde (f64) accepts
    # it; the lufs receipt check still fails honestly against the target.
    for m in re.finditer(r"Peak:\s*(-?(?:[\d.]+|inf))\s*dBFS", stderr):
        peak = max(finite_float(m.group(1), -99.0), -99.0)
    if integrated is None or peak is None:
        die("sidecar", "ebur128 summary not found",
            f"integrated={integrated} peak={peak}; does the file have audio?")
    return {"integrated_lufs": integrated, "true_peak_dbtp": peak, "windows": windows}


# ---------------------------------------------------------------------------
# subject — auto-reframe subject track (local CV; perception contract reframe rework)
# ---------------------------------------------------------------------------
#
# Emits the SubjectTrack (app/perception/src/types.rs): an aspect-INDEPENDENT,
# NORMALIZED per-frame subject path that the render (cut_media::render reframe
# mode) turns into a moving crop for ANY target aspect. Pipeline (all local,
# footage never leaves the box; GPU-optional, CPU-viable — the "same hardware
# floor as today" decision:
#   YOLO-seg + ByteTrack (ultralytics)  → detect+track subjects across frames
#   handcrafted saliency (OpenCV)       → general-subject cue (sports/product/B-roll)
#   PySceneDetect cuts                  → reset the subject choice per scene
#   linear ranker + hysteresis          → pick WHICH subject to frame, stably
# The subject-selection approach is based on the MIT-licensed
# KazKozDev/auto-vertical-reframe reference; this is a clean, self-contained port
# with no third-party runtime import. Face/active-speaker
# (MediaPipe Tasks) are a precision enhancer; the base path frames on boxes and
# saliency. The "which subject matters" hard
# semantic calls are refined by the VLM subject pick.

# COCO category ids we treat as framable subjects. NOTE: torchvision detection
# models use the ORIGINAL COCO 1-indexed labels WITH GAPS (person=1, car=3, …) —
# NOT the contiguous 0-79 set YOLO uses. Indices verified from
# SSDLite..._Weights.DEFAULT.meta["categories"].
_SUBJECT_CLASS_IDS = {
    "person": 1, "bicycle": 2, "car": 3, "motorcycle": 4,
    "bus": 6, "truck": 8, "cat": 17, "dog": 18,
}
_ID_TO_NAME = {v: k for k, v in _SUBJECT_CLASS_IDS.items()}

# Presets = which classes to consider + crop motion/zoom limits. The render reads
# the motion/zoom limits; the instrument only needs the class set (the track is
# aspect-independent), but we keep the full preset here so one place defines them.
_SUBJECT_PRESETS = {
    "talking_head": {"classes": ["person"]},
    "sports": {"classes": ["person", "car", "bicycle", "motorcycle"]},
    "pets": {"classes": ["dog", "cat", "person"]},
    "cars": {"classes": ["car", "truck", "bus", "motorcycle", "person"]},
    "general": {"classes": ["person", "dog", "cat", "car", "bicycle", "motorcycle", "bus", "truck"]},
}


def _sclamp(v: float, lo: float, hi: float) -> float:
    return lo if v < lo else hi if v > hi else v


def _handcrafted_saliency(frame_bgr, prev_gray_small):
    """Spectral-residual saliency (Hou & Zhang 2007) blended with frame motion.

    CPU-only, NO model/weights — the general-subject cue that keeps the reframe
    analysis on the hardware floor. Returns (saliency_map HxW float[0,1],
    gray_small) — pass gray_small back in as prev_gray_small next frame for the
    motion term. Based on the MIT-licensed auto-vertical-reframe reference."""
    import cv2
    import numpy as np
    h, w = frame_bgr.shape[:2]
    scale = min(1.0, 320.0 / max(h, w))
    sw, sh = max(32, int(round(w * scale))), max(32, int(round(h * scale)))
    gray = cv2.cvtColor(
        cv2.resize(frame_bgr, (sw, sh), interpolation=cv2.INTER_AREA),
        cv2.COLOR_BGR2GRAY,
    ).astype(np.float32)
    dft = cv2.dft(gray, flags=cv2.DFT_COMPLEX_OUTPUT)
    real, imag = dft[:, :, 0], dft[:, :, 1]
    log_amp = np.log(cv2.magnitude(real, imag) + 1e-6)
    residual = log_amp - cv2.blur(log_amp, (3, 3))
    phase = cv2.phase(real, imag)
    exp_r = np.exp(residual)
    spec = np.dstack([exp_r * np.cos(phase), exp_r * np.sin(phase)]).astype(np.float32)
    sal = cv2.idft(spec, flags=cv2.DFT_SCALE | cv2.DFT_REAL_OUTPUT)
    sal = cv2.GaussianBlur(sal * sal, (7, 7), 0)
    sal = cv2.normalize(sal, None, 0.0, 1.0, cv2.NORM_MINMAX)
    if prev_gray_small is not None and prev_gray_small.shape == gray.shape:
        motion = cv2.normalize(
            cv2.GaussianBlur(cv2.absdiff(gray, prev_gray_small), (5, 5), 0),
            None, 0.0, 1.0, cv2.NORM_MINMAX,
        )
        sal = sal * 0.72 + motion * 0.28
    return cv2.resize(sal, (w, h), interpolation=cv2.INTER_LINEAR), gray


def _saliency_region(saliency_map, bounds):
    """Saliency-weighted centroid + tight box inside `bounds` (x1,y1,x2,y2).

    Returns (cx, cy, x1, y1, x2, y2, confidence) or None. Confidence = the
    salient mass fraction in the region — feeds the ranker + crop bounds."""
    import numpy as np
    h, w = saliency_map.shape[:2]
    x1, y1, x2, y2 = bounds
    x1 = int(_sclamp(x1, 0, w - 1)); y1 = int(_sclamp(y1, 0, h - 1))
    x2 = int(_sclamp(x2, x1 + 1, w)); y2 = int(_sclamp(y2, y1 + 1, h))
    roi = saliency_map[y1:y2, x1:x2]
    if roi.size == 0 or finite_number(roi.sum()) <= 1e-6:
        return None
    roi_max, roi_mean, roi_std = finite_number(roi.max()), finite_number(roi.mean()), finite_number(roi.std())
    thresh = max(roi_mean + roi_std * 0.75, roi_max * 0.6)
    mask = roi >= thresh
    if not np.any(mask):
        mask = roi >= max(roi_mean + roi_std * 0.25, roi_max * 0.4)
    if not np.any(mask):
        return None
    ys, xs = np.where(mask)
    wts = roi[mask].astype(np.float64)
    wsum = finite_number(wts.sum())
    if wsum <= 1e-6:
        return None
    cx = x1 + finite_number(np.dot(xs, wts) / wsum)
    cy = y1 + finite_number(np.dot(ys, wts) / wsum)
    return (cx, cy, x1 + finite_number(xs.min()), y1 + finite_number(ys.min()),
            x1 + finite_number(xs.max() + 1), y1 + finite_number(ys.max() + 1),
            _sclamp(wsum / max(1.0, roi.size), 0.0, 1.0))


class _SubjectRanker:
    """Hand-tuned LINEAR subject-importance score (NOT a trained model): a class
    bias plus a weighted sum of cheap features, picking WHICH detected subject to
    frame. Deliberately heuristic + CPU-free; the hard semantic calls (which of two
    people matters now) are refined by the VLM subject pick. Weights are ported from
    the MIT-licensed auto-vertical-reframe reference. `face_presence` uses MediaPipe;
    `active_speaker` is a relative, audio-gated boost passed in by the
    caller — 0 unless someone is confidently speaking, so it never perturbs a
    single-subject pick. Pose remains omitted (detector off)."""
    _CLASS_BIAS = {
        "person": 0.22, "dog": 0.12, "cat": 0.10, "car": 0.06,
        "bicycle": 0.02, "motorcycle": 0.02, "bus": 0.01, "truck": 0.01,
    }
    _W = {
        "det_conf": 1.35, "mask_presence": 0.95, "center_affinity": 0.55,
        "saliency_presence": 0.72, "saliency_conf": 0.78, "tracking_match": 1.05,
        "size_logit": 0.26, "face_presence": 0.48, "active_speaker": 0.95,
    }

    def score(self, *, cls, conf, norm_area, dist_center, frame_diag,
              saliency_conf, tracking_match, has_face=False, active_speaker=0.0):
        import math
        feats = {
            "det_conf": _sclamp(conf, 0.0, 1.0),
            "mask_presence": math.sqrt(max(0.0, norm_area)),
            "center_affinity": 1.0 - _sclamp(dist_center / max(frame_diag, 1.0), 0.0, 1.0),
            "saliency_presence": 1.0 if saliency_conf > 0.0 else 0.0,
            "saliency_conf": _sclamp(saliency_conf, 0.0, 1.0),
            "tracking_match": 1.0 if tracking_match else 0.0,
            "size_logit": math.log1p(norm_area * 250.0),
            "face_presence": 1.0 if has_face else 0.0,
            # RELATIVE speaking signal [0,1] (1.0 = the frame's leading speaker);
            # already audio-gated + confidence-gated by the caller (0 when nobody
            # clearly speaks), so it only TIPS multi-person picks — never regresses.
            "active_speaker": _sclamp(active_speaker, 0.0, 1.0),
        }
        s = self._CLASS_BIAS.get(cls, 0.0)
        for k, v in feats.items():
            s += self._W[k] * v
        return s


def _choose_subject(cands, state, min_hold_frames=12, switch_threshold=1.20,
                    speaker_tid=None):
    """Stable subject pick across frames. cands sorted by score desc. Hysteresis:
    keep the previously-tracked subject unless a challenger clearly beats it, so the
    crop doesn't flip-flop between similar subjects. `state` carries tracked id/cls
    + frames_since_switch (mutated here). Based on auto-vertical-reframe (MIT).

    Active-speaker override: `speaker_tid` (when set) is the tid of a person who
    is CONFIDENTLY speaking (audio-gated, clearly leading the others' mouth motion).
    A confident speaker is AUTHORITATIVE — it bypasses the stability hold so the crop
    follows whoever is talking, even away from the incumbent (subject-tracking). This is
    deliberately stronger than the `active_speaker` score nudge, which alone loses to
    the incumbent's `tracking_match` bonus. Jitter is bounded UPSTREAM (EMA smoothing,
    the audio gate, and the dominance margin that sets `speaker_tid`)."""
    if not cands:
        return None
    if speaker_tid is not None:
        spk = next((c for c in cands if c["tid"] == speaker_tid), None)
        if spk is not None:
            return spk
    best = cands[0]
    prev = next((c for c in cands
                 if c["tid"] == state.get("tid") and c["cls_id"] == state.get("cls_id")), None)
    if prev is not None:
        if prev["tid"] == best["tid"] and prev["cls_id"] == best["cls_id"]:
            return prev
        if state.get("since_switch", 0) < min_hold_frames and prev["score"] * switch_threshold >= best["score"]:
            return prev
        if prev["score"] >= best["score"] * 0.92:
            return prev
    return best


def _focus_bounds(subj):
    """Rectangle to keep in frame for the chosen subject (mask/box + headroom,
    unioned with its saliency box). Source px. Drives the crop zoom. v1 (no pose):
    box top for the head, a little headroom, padded; saliency widens it if stronger."""
    left, top, right, bottom = subj["x1"], subj["mask_top"], subj["x2"], subj["y2"]
    height = max(1.0, bottom - top)
    width = max(1.0, right - left)
    left -= width * 0.14; right += width * 0.14
    top -= height * 0.14; bottom += height * 0.16
    sb = subj.get("sal_box")
    if sb is not None:
        pad_x = max(8.0, (sb[2] - sb[0]) * 0.18)
        pad_y = max(8.0, (sb[3] - sb[1]) * 0.22)
        left = min(left, sb[0] - pad_x); top = min(top, sb[1] - pad_y)
        right = max(right, sb[2] + pad_x); bottom = max(bottom, sb[3] + pad_y)
    return left, top, right, bottom


def _resolve_device():
    """GPU when torch reports CUDA, else CPU (the floor path). Returns the torch
    device string + a label (same value here, kept symmetric with the report)."""
    try:
        import torch
        if torch.cuda.is_available():
            return "cuda", "cuda"
    except Exception:
        pass
    return "cpu", "cpu"


def _load_detector(quality: str, device: str):
    """BSD-licensed torchvision detector with portable and high-quality tiers:
      'fast' (default) = SSDlite320-MobileNetV3, the CPU floor path.
      'high'           = Faster R-CNN R50-FPN  — GPU quality tier (finds 2-up subjects
                          more reliably). COCO weights, BSD.
    Returns (predict(frame_bgr) -> (boxes Nx4 xyxy, scores N, labels N) numpy, name).
    Detection-only (no masks) — framing uses the bbox + handcrafted saliency;
    face and active-speaker signals sharpen it later."""
    import torch
    import cv2
    if quality == "high":
        from torchvision.models.detection import (
            fasterrcnn_resnet50_fpn, FasterRCNN_ResNet50_FPN_Weights,
        )
        model = fasterrcnn_resnet50_fpn(weights=FasterRCNN_ResNet50_FPN_Weights.DEFAULT)
        name = "fasterrcnn_resnet50_fpn"
    else:
        from torchvision.models.detection import (
            ssdlite320_mobilenet_v3_large, SSDLite320_MobileNet_V3_Large_Weights,
        )
        model = ssdlite320_mobilenet_v3_large(weights=SSDLite320_MobileNet_V3_Large_Weights.DEFAULT)
        name = "ssdlite320_mobilenet_v3_large"
    model.eval().to(device)

    def predict(frame_bgr):
        t = (torch.from_numpy(cv2.cvtColor(frame_bgr, cv2.COLOR_BGR2RGB))
             .permute(2, 0, 1).float().div(255.0).to(device))
        with torch.no_grad():
            o = model([t])[0]
        return (o["boxes"].cpu().numpy(), o["scores"].cpu().numpy(), o["labels"].cpu().numpy())

    return predict, name


def _load_face_detector():
    """MediaPipe Tasks FaceDetector (Apache-2.0) for FACE-AWARE framing — frames the
    face eye-line for close subjects (subject-tracking 'follows the face'), not the body
    centre. GRACEFUL: returns None if mediapipe or the model is unavailable, so the
    instrument degrades to body+saliency framing. The short-range model is tuned for
    CLOSE / talking-head faces; distant faces simply aren't found → body fallback.
    Model: $SHELLX_CUT_FACE_MODEL, else blaze_face_short_range.tflite beside this file."""
    model = os.environ.get("SHELLX_CUT_FACE_MODEL", "").strip()
    if not model:
        here = Path(__file__).parent / "blaze_face_short_range.tflite"
        model = str(here) if here.is_file() else ""
    if not model or not Path(model).is_file():
        log("subject: no face model — body/saliency framing only")
        return None
    try:
        import mediapipe as mp
        from mediapipe.tasks.python import BaseOptions
        from mediapipe.tasks.python.vision import FaceDetector, FaceDetectorOptions
        det = FaceDetector.create_from_options(FaceDetectorOptions(
            base_options=BaseOptions(model_asset_path=model),
            min_detection_confidence=0.4))
        log(f"subject: face-aware framing ON ({Path(model).name})")
        return (det, mp)
    except Exception as exc:
        log(f"subject: face detector unavailable ({exc}); body/saliency framing")
        return None


def _detect_faces(face, frame_bgr):
    """Face boxes in the frame as (cx, cy, x1, y1, x2, y2) px. [] if no detector/none."""
    if face is None:
        return []
    det, mp = face
    import cv2
    try:
        rgb = cv2.cvtColor(frame_bgr, cv2.COLOR_BGR2RGB)
        r = det.detect(mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb))
    except Exception:
        return []
    out = []
    for d in r.detections:
        b = d.bounding_box
        out.append((b.origin_x + b.width / 2.0, b.origin_y + b.height / 2.0,
                    finite_number(b.origin_x), finite_number(b.origin_y),
                    finite_number(b.origin_x + b.width), finite_number(b.origin_y + b.height)))
    return out


def _match_face(faces, x1, y1, x2, y2):
    """The largest face whose centre falls inside the person bbox, else None. Used to
    swap the framing point from the body centre to the face eye-line for that subject."""
    best, best_area = None, -1.0
    for fb in faces:
        if x1 <= fb[0] <= x2 and y1 <= fb[1] <= y2:
            area = (fb[4] - fb[2]) * (fb[5] - fb[3])
            if area > best_area:
                best, best_area = fb, area
    return best


# --- active-speaker: CPU-floor heuristic, confidence-gated --------------------
# In multi-person dialogue, prefer whoever is SPEAKING (subject-tracking "follows
# the speaker"). The audio is a single MIXED track — it tells us WHEN someone
# speaks, never WHO — so we pair it with per-face MOUTH MOTION: among visible
# faces, the one whose mouth moves while the audio gate is open is the speaker.
# Deliberately a hardware-FLOOR heuristic (frame-diff in the mouth ROI + an
# audio-RMS gate + EMA smoothing), NOT an ASD model — LR-ASD is a future GPU
# quality tier so caption analysis keeps a stable minimum confidence. Applied
# as a RELATIVE, confidence-GATED boost in the ranker: it can only TIP the pick
# between visible people and NEVER regresses the stable single-face framing —
# when the audio is silent or no mouth clearly leads, the boost is 0 and face
# framing stands unchanged.
_SPEAK_AUDIO_FLOOR = 0.12   # normalized RMS below this ⇒ treat as non-speech (gate off)
_SPEAK_EMA_ALPHA = 0.6      # mouth-motion EMA smoothing (higher ⇒ steadier, less jitter)
_SPEAK_MIN = 0.04           # min leading EMA to believe "someone is actually speaking"
_MOUTH_GAIN = 6.0           # scales the raw mouth frame-diff into a usable [0,1] range


def _audio_energy_envelope(media_path: str, fps: float):
    """Per-video-frame RMS energy of the MIXED audio, normalized to [0,1] (95th
    pct), or None when the file has no audio. Drives the active-speaker GATE: it
    says WHEN someone is speaking, never who. 8 kHz mono is ample for an energy
    envelope and keeps memory tiny (~16 KB/s). Decoded ONCE before the frame loop;
    indexed by frame number (audio ≈ video duration on the rendered edit)."""
    import math
    if fps <= 0 or not has_audio_stream(media_path):
        return None
    import numpy as np
    sr = 8000
    proc = subprocess.run(
        [FFMPEG_BIN, "-v", "error", "-i", media_path,
         "-vn", "-ac", "1", "-ar", str(sr), "-f", "s16le", "-"],
        capture_output=True,
    )
    if proc.returncode != 0 or not proc.stdout:
        return None
    samples = np.frombuffer(proc.stdout, dtype=np.int16).astype(np.float32) / 32768.0
    if samples.size == 0:
        return None
    spf = sr / fps  # audio samples per video frame
    n = int(math.ceil(samples.size / spf))
    env = np.zeros(n, dtype=np.float32)
    for i in range(n):
        seg = samples[int(i * spf):int((i + 1) * spf)]
        if seg.size:
            env[i] = finite_number(np.sqrt(np.mean(seg * seg)))
    p95 = finite_number(np.percentile(env, 95)) if env.size else 0.0
    if p95 > 1e-5:
        env = np.clip(env / p95, 0.0, 1.0)
    return env


def _mouth_roi_gray(frame_bgr, fb):
    """Fixed-size (32x24) grayscale crop of a face's MOUTH region (lower-mid of the
    face box) for frame-to-frame motion. Fixed output size makes the absdiff robust
    to the face box growing/shrinking between frames. Returns a float array or None
    when the face is too small to sample. `fb` = (cx,cy,x1,y1,x2,y2) px."""
    import cv2
    import numpy as np
    fw = fb[4] - fb[2]
    fh = fb[5] - fb[3]
    if fw < 6 or fh < 6:
        return None
    h, w = frame_bgr.shape[:2]
    mx1 = int(_sclamp(fb[2] + fw * 0.15, 0, w - 1))
    mx2 = int(_sclamp(fb[4] - fw * 0.15, mx1 + 1, w))
    my1 = int(_sclamp(fb[3] + fh * 0.55, 0, h - 1))
    my2 = int(_sclamp(fb[5] - fh * 0.05, my1 + 1, h))
    roi = frame_bgr[my1:my2, mx1:mx2]
    if roi.size == 0:
        return None
    return cv2.cvtColor(
        cv2.resize(roi, (32, 24), interpolation=cv2.INTER_AREA),
        cv2.COLOR_BGR2GRAY,
    ).astype(np.float32)


def instrument_subject(media_path: str, preset: str = "talking_head", director=None) -> dict:
    """Build the SubjectTrack for auto-reframe (app/perception/src/types.rs).

    `director` is an optional
    per-scene brief from the foundation model — `{scene_idx: {"cx": float}}` to
    follow the subject at that normalized x position (the contact-sheet label
    resolved to a position), or `{scene_idx: {"mode": "widen"}}` to hold a centered
    wide frame. It is the HIGHEST-priority pick for a directed scene (above the
    active-speaker override and the ranker): on the first directed frame it acquires
    the subject closest to the briefed position and LOCKS its track id for the
    scene, re-acquiring by position only if the track is lost. Scenes with no brief
    fall back to the CV ranker exactly as before (so an absent director = no change).

    Aspect-INDEPENDENT + NORMALIZED: extracted once (ideally on a proxy), it drives
    a moving crop to ANY target aspect at full resolution. Heavy (per-frame CV) so
    it is requested ON DEMAND, never in the import set. Raises a sidecar error if
    the CV deps are missing (reframe explicitly needs them)."""
    import cv2
    import numpy as np
    try:
        import torch  # noqa: F401  (device + input tensors)
        import torchvision  # noqa: F401  (the BSD detector backend)
        import supervision as sv
    except Exception as exc:
        die("sidecar", "subject instrument needs the CV deps (torch, torchvision, supervision)",
            f"{exc}; pip install torchvision supervision into the sidecar venv")

    classes = _SUBJECT_PRESETS.get(preset, _SUBJECT_PRESETS["talking_head"])["classes"]
    allowed = {_SUBJECT_CLASS_IDS[c] for c in classes if c in _SUBJECT_CLASS_IDS}
    # 'fast' = CPU-floor SSDlite (default); 'high' = Faster R-CNN (GPU quality tier).
    quality = os.environ.get("SHELLX_CUT_DETECTOR", "fast")
    device, device_label = _resolve_device()

    cap = cv2.VideoCapture(media_path)
    raw_fps = finite_number(cap.get(cv2.CAP_PROP_FPS), 0.0)
    fps_source = "measured"
    fps_warning = None
    if raw_fps > 0.0:
        fps = raw_fps
    else:
        fps = 30.0
        fps_source = "fallback"
        fps_warning = "OpenCV did not report a usable FPS; using 30fps for subject timestamps"
        log(f"subject: {fps_warning}")
    frame_w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    frame_h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    if frame_w == 0 or frame_h == 0:
        cap.release()
        die("sidecar", "subject: no decodable video geometry", media_path)

    # Scene starts (0-based frame indices) — the render resets the crop at each.
    from scenedetect import open_video, SceneManager
    from scenedetect.detectors import ContentDetector
    sm = SceneManager()
    sm.add_detector(ContentDetector())
    sm.detect_scenes(open_video(media_path))
    # FrameTimecode → 0-based start frame (frame_num in scenedetect 0.7+, get_frames older).
    def _start_frame(tc):
        return tc.frame_num if hasattr(tc, "frame_num") else tc.get_frames()
    scene_starts = sorted({0} | {_start_frame(s) for s, _ in sm.get_scene_list()[1:]})
    scene_set = set(scene_starts)

    predict, model_name = _load_detector(quality, device)
    face = _load_face_detector()  # None → body/saliency framing (graceful)
    face_aware = face is not None
    log(f"subject: {model_name} on {device_label}, classes={classes}, {frame_w}x{frame_h}")
    ranker = _SubjectRanker()
    import math

    frame_diag = math.hypot(frame_w, frame_h)
    frame_area = finite_number(frame_w * frame_h)
    frame_cx, frame_cy = frame_w / 2.0, frame_h / 2.0
    prev_gray = None
    state = {"tid": None, "cls_id": None, "since_switch": 0}
    # ByteTrack (supervision, MIT) — recreated at each scene cut so track ids never
    # bleed across a hard cut (a new scene is a fresh tracking context).
    tracker = sv.ByteTrack(frame_rate=max(1, int(round(fps))))
    # Active-speaker path: the audio-energy gate (decoded once) + per-track mouth
    # state, all reset per scene (track ids restart at each cut). `speaker_aware`
    # records whether the heuristic was even available (audio present) for the
    # receipt; `speaker_frames` counts frames where it actually led a pick.
    audio_env = _audio_energy_envelope(media_path, fps)
    speaker_aware = audio_env is not None
    mouth_prev = {}   # tid -> last mouth-ROI gray crop
    speak_ema = {}    # tid -> EMA-smoothed speaking score
    speaker_frames = 0
    # Director brief: per-scene override from the model.
    # Keys normalized to int scene indices. `director_lock` holds the acquired
    # track id per directed scene; `directed_scenes` records which scenes the
    # director actually decided (receipt honesty).
    director = {int(k): v for k, v in (director or {}).items()}
    director_lock = {}
    directed_scenes = set()
    scene_idx = 0
    frames = []
    f = 0

    while True:
        ok, frame = cap.read()
        if not ok:
            break
        if f in scene_set and f > 0:
            scene_idx += 1
            prev_gray = None
            state = {"tid": None, "cls_id": None, "since_switch": 0}
            tracker = sv.ByteTrack(frame_rate=max(1, int(round(fps))))
            mouth_prev = {}   # track ids restart at a cut → drop stale mouth state
            speak_ema = {}
        sal_map, prev_gray = _handcrafted_saliency(frame, prev_gray)
        # Audio gate for active-speaker: is someone speaking on THIS frame?
        env_f = finite_number(audio_env[min(f, len(audio_env) - 1)]) if audio_env is not None else 0.0
        audio_on = env_f >= _SPEAK_AUDIO_FLOOR

        # Detect (torchvision) → keep allowed classes over threshold → track (ByteTrack).
        boxes, scores, labels = predict(frame)
        keep = [i for i in range(len(labels))
                if int(labels[i]) in allowed and finite_number(scores[i]) > 0.30]
        if keep:
            dets = sv.Detections(
                xyxy=boxes[keep].astype(float),
                confidence=scores[keep].astype(float),
                class_id=labels[keep].astype(int),
            )
            dets = tracker.update_with_detections(dets)
        else:
            dets = sv.Detections.empty()

        # Face-aware framing: detect faces ONCE per frame, only when a person is present
        # (skip the cost on people-less frames). Matched per-person below → eye-line.
        person_present = any(int(labels[i]) == _SUBJECT_CLASS_IDS["person"] for i in keep)
        faces = _detect_faces(face, frame) if (face is not None and person_present) else []

        # --- Pass A: geometry, face-eye-line framing, per-face mouth-motion -----
        # (bbox + saliency framing; detection models give no masks). Scoring is
        # deferred to pass B so the active-speaker boost can be RELATIVE across the
        # frame's people (needs every candidate's speaking score first).
        cands = []
        for i in range(len(dets)):
            x1, y1, x2, y2 = (finite_number(v) for v in dets.xyxy[i])
            cls_id = int(dets.class_id[i])
            conf = finite_number(dets.confidence[i])
            tid = int(dets.tracker_id[i]) if dets.tracker_id is not None else None
            cls_name = _ID_TO_NAME.get(cls_id, str(cls_id))
            mask_area = max(1.0, (x2 - x1) * (y2 - y1))
            mask_cx, mask_cy, mask_top = (x1 + x2) / 2, (y1 + y2) / 2, y1
            sal = _saliency_region(sal_map, (int(x1), int(y1), int(x2), int(y2)))
            # Framing point: FACE eye-line for a person whose face we found (subject-
            # parity); else bbox centre (or saliency centroid for non-person subjects).
            framing_cx, framing_cy = mask_cx, mask_cy
            has_face = False
            speak = 0.0
            if cls_name == "person":
                fb = _match_face(faces, x1, y1, x2, y2)
                if fb is not None:
                    has_face = True
                    framing_cx = fb[0]
                    # eye-line ≈ upper-third of the face box (natural headroom).
                    framing_cy = fb[3] + (fb[5] - fb[3]) * 0.42
                    # Mouth motion is ALWAYS sampled (keeps `mouth_prev` fresh) but
                    # only CONTRIBUTES while the audio gate is open; the EMA decays
                    # toward 0 during silence, so the boost vanishes off-speech.
                    if tid is not None:
                        roi = _mouth_roi_gray(frame, fb)
                        prev = mouth_prev.get(tid)
                        motion = 0.0
                        if roi is not None and prev is not None and prev.shape == roi.shape:
                            motion = _sclamp(
                                finite_number(np.mean(np.abs(roi - prev))) / 255.0 * _MOUTH_GAIN,
                                0.0, 1.0)
                        if roi is not None:
                            mouth_prev[tid] = roi
                        contrib = motion if audio_on else 0.0
                        speak = (_SPEAK_EMA_ALPHA * speak_ema.get(tid, 0.0)
                                 + (1.0 - _SPEAK_EMA_ALPHA) * contrib)
                        speak_ema[tid] = speak
            elif sal is not None:
                framing_cx, framing_cy = sal[0], sal[1]
            dist_center = math.hypot(framing_cx - frame_cx, framing_cy - frame_cy)
            tracking_match = tid is not None and tid == state["tid"] and cls_id == state["cls_id"]
            cands.append({
                "cls_id": cls_id, "cls_name": cls_name, "conf": conf, "tid": tid,
                "x1": x1, "y1": y1, "x2": x2, "y2": y2, "mask_top": mask_top,
                "framing_cx": framing_cx, "framing_cy": framing_cy,
                "sal_box": (sal[2], sal[3], sal[4], sal[5]) if sal else None,
                "norm_area": mask_area / max(frame_area, 1.0),
                "dist_center": dist_center,
                "saliency_conf": (sal[6] if sal else 0.0),
                "tracking_match": tracking_match, "has_face": has_face,
                "speak_ema": speak,
            })

        # Active-speaker boost is RELATIVE among the frame's people and GATED: it
        # only engages when the audio is active AND one mouth's smoothed motion
        # clearly leads (>= _SPEAK_MIN). Otherwise rel_speak = 0 for everyone, so
        # The second pass reproduces face-aware framing exactly.
        max_speak = max((c["speak_ema"] for c in cands if c["cls_name"] == "person"),
                        default=0.0)
        speaker_active = audio_on and max_speak >= _SPEAK_MIN
        # A CONFIDENT speaker = one person whose smoothed mouth-motion clearly leads
        # (>= 1.5x the runner-up) while the audio gate is open. Only then is the
        # speaker authoritative (overrides the stability hold); overlapping/ambiguous
        # speech leaves speaker_tid = None, so framing stays stable.
        speaker_tid = None
        if speaker_active:
            ranked = sorted(((c["speak_ema"], c["tid"]) for c in cands
                             if c["cls_name"] == "person" and c["tid"] is not None),
                            reverse=True)
            if ranked and ranked[0][0] >= _SPEAK_MIN:
                runner = ranked[1][0] if len(ranked) > 1 else 0.0
                if ranked[0][0] >= 1.5 * max(runner, 1e-6):
                    speaker_tid = ranked[0][1]
        if speaker_tid is not None:
            speaker_frames += 1

        # --- Pass B: score each candidate (with the gated relative speaker term) -
        for c in cands:
            rel_speak = (c["speak_ema"] / max_speak) if (speaker_active and max_speak > 1e-6) else 0.0
            c["score"] = ranker.score(
                cls=c["cls_name"], conf=c["conf"], norm_area=c["norm_area"],
                dist_center=c["dist_center"], frame_diag=frame_diag,
                saliency_conf=c["saliency_conf"], tracking_match=c["tracking_match"],
                has_face=c["has_face"], active_speaker=rel_speak,
            )
        cands.sort(key=lambda c: c["score"], reverse=True)

        # Director override (highest priority): in a briefed scene, follow the
        # subject at the briefed position (lock its tid, re-acquire by position if
        # lost) or hold a centered wide frame ("widen"). Absent brief → CV ranker.
        brief = director.get(scene_idx)
        if brief is not None:
            if brief.get("mode") == "widen":
                subj = None
                directed_scenes.add(scene_idx)
            elif cands:
                locked = director_lock.get(scene_idx)
                subj = next((c for c in cands if c["tid"] == locked), None) if locked is not None else None
                if subj is None:
                    target = finite_number(brief.get("cx", 0.5), 0.5) * frame_w
                    subj = min(cands, key=lambda c: abs(c["framing_cx"] - target))
                    if subj.get("tid") is not None:
                        director_lock[scene_idx] = subj["tid"]
                directed_scenes.add(scene_idx)
            else:
                subj = None
        else:
            subj = _choose_subject(cands, state, speaker_tid=speaker_tid)
        if os.environ.get("SHELLX_CUT_SUBJECT_DEBUG") and f % 10 == 0:
            dbg = [(c["tid"], round(c["speak_ema"], 2), round(c["score"], 2)) for c in cands]
            log(f"DBG f={f} audio_on={audio_on} max_speak={max_speak:.2f} "
                f"spk_tid={speaker_tid} cands(tid,speak,score)={dbg} "
                f"chosen={subj['tid'] if subj else None}")
        if subj is None:
            frames.append({"f": f, "t_ms": int(f / fps * 1000), "conf": 0.0, "scene": scene_idx})
            state["since_switch"] += 1
            f += 1
            continue
        changed = (subj["tid"], subj["cls_id"]) != (state["tid"], state["cls_id"])
        state["since_switch"] = 0 if changed else state["since_switch"] + 1
        state["tid"], state["cls_id"] = subj["tid"], subj["cls_id"]

        fx1, fy1, fx2, fy2 = _focus_bounds(subj)
        frames.append({
            "f": f, "t_ms": int(f / fps * 1000),
            "cx": round(_sclamp(subj["framing_cx"] / frame_w, 0.0, 1.0), 5),
            "cy": round(_sclamp(subj["framing_cy"] / frame_h, 0.0, 1.0), 5),
            "fx1": round(_sclamp(fx1 / frame_w, 0.0, 1.0), 5),
            "fy1": round(_sclamp(fy1 / frame_h, 0.0, 1.0), 5),
            "fx2": round(_sclamp(fx2 / frame_w, 0.0, 1.0), 5),
            "fy2": round(_sclamp(fy2 / frame_h, 0.0, 1.0), 5),
            "conf": round(finite_number(subj["conf"]), 4),
            "scene": scene_idx, "cls": subj["cls_name"],
            "tid": subj["tid"],
        })
        f += 1

    cap.release()
    n_subj = sum(1 for fr in frames if "cx" in fr)
    speaker_note = (f"active-speaker on, led {speaker_frames}f" if speaker_aware
                    else "active-speaker off (no audio)")
    log(f"subject: {len(frames)} frames, {n_subj} with subject "
        f"({100 * n_subj / max(1, len(frames)):.0f}%), {scene_idx + 1} scenes; {speaker_note}")
    out = {
        "fps": finite_number(fps),
        "fps_source": fps_source,
        "frame_width": frame_w, "frame_height": frame_h,
        "scenes": list(scene_starts),
        "seg_model": model_name, "device": device_label,
        "classes": classes, "frames": frames,
        # Receipt honesty: was the active-speaker heuristic available/applied?
        "speaker_aware": speaker_aware,
        # Receipt honesty: was face/eye-line framing available, or did we fall
        # back to body/saliency centers?
        "face_aware": face_aware,
        # Scenes whose subject the director model decided.
        "directed_scenes": sorted(directed_scenes),
    }
    if fps_warning:
        out["fps_warning"] = fps_warning
    return out


def build_contact_sheet(media_path: str, preset: str = "talking_head", out_dir=None) -> dict:
    """DIRECTOR pass — the sparse input the
    foundation model reads to direct the whole clip in ONE call.

    Scene-detects, samples ONE representative keyframe per scene, detects the
    candidate subjects on it, and tiles the annotated keyframes into a single
    contact-sheet image. Candidates are labeled A/B/C ordered LEFT→RIGHT so a
    director brief ("scene 2: follow B") maps to a stable per-scene POSITION the
    executor resolves at render time — no need to share ByteTrack ids across the
    sparse director pass and the dense execution pass. Cheap: one detection per
    scene, never per-frame. Returns the sheet path + structured per-scene candidates."""
    import cv2
    import numpy as np
    try:
        import torch  # noqa: F401
        import torchvision  # noqa: F401
    except Exception as exc:
        die("sidecar", "contact_sheet needs the CV deps (torch, torchvision)",
            f"{exc}; pip install torchvision into the sidecar venv")

    classes = _SUBJECT_PRESETS.get(preset, _SUBJECT_PRESETS["talking_head"])["classes"]
    allowed = {_SUBJECT_CLASS_IDS[c] for c in classes if c in _SUBJECT_CLASS_IDS}
    quality = os.environ.get("SHELLX_CUT_DETECTOR", "fast")
    device, device_label = _resolve_device()

    cap = cv2.VideoCapture(media_path)
    fps = cap.get(cv2.CAP_PROP_FPS) or 30.0
    frame_w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    frame_h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT)) or 0
    if frame_w == 0 or frame_h == 0:
        cap.release()
        die("sidecar", "contact_sheet: no decodable video geometry", media_path)

    # Scene spans [start, end) (0-based frame indices).
    from scenedetect import open_video, SceneManager
    from scenedetect.detectors import ContentDetector
    sm = SceneManager()
    sm.add_detector(ContentDetector())
    sm.detect_scenes(open_video(media_path))

    def _sf(tc):
        return tc.frame_num if hasattr(tc, "frame_num") else tc.get_frames()

    scene_list = sm.get_scene_list()
    spans = [(_sf(s), _sf(e)) for s, e in scene_list] if scene_list else [(0, total or int(fps))]

    predict, model_name = _load_detector(quality, device)
    face = _load_face_detector()
    log(f"contact_sheet: {model_name} on {device_label}, {len(spans)} scenes, {frame_w}x{frame_h}")

    def _detect_on(frame):
        """allowed-class detections over threshold on one frame → [(x1,y1,x2,y2,cls,conf)]."""
        boxes, scores, labels = predict(frame)
        return [(finite_number(boxes[i][0]), finite_number(boxes[i][1]), finite_number(boxes[i][2]), finite_number(boxes[i][3]),
                 _ID_TO_NAME.get(int(labels[i]), str(int(labels[i]))), round(finite_number(scores[i]), 3))
                for i in range(len(labels))
                if int(labels[i]) in allowed and finite_number(scores[i]) > 0.30]

    tiles = []
    scenes_json = []
    for si, (s, e) in enumerate(spans):
        e2 = max(s + 1, e)
        # Sample a few positions in the scene and keep the RICHEST keyframe (most
        # candidates) — a single midpoint frame can miss the subject (motion blur,
        # head turn). Still sparse: a few detections per scene, never per-frame.
        best_frame, best_dets, best_at = None, [], (s + e2) // 2
        for frac in (0.35, 0.5, 0.65):
            fi = int(s + (e2 - s) * frac)
            cap.set(cv2.CAP_PROP_POS_FRAMES, fi)
            ok, frame = cap.read()
            if not ok:
                continue
            dets = _detect_on(frame)
            if best_frame is None or len(dets) > len(best_dets):
                best_frame, best_dets, best_at = frame, dets, fi
        if best_frame is None:
            continue
        frame, mid = best_frame, best_at
        person_present = any(d[4] == "person" for d in best_dets)
        faces = _detect_faces(face, frame) if (face is not None and person_present) else []
        cands = []
        for (x1, y1, x2, y2, cls, conf) in best_dets:
            has_face = cls == "person" and _match_face(faces, x1, y1, x2, y2) is not None
            cands.append({"cls": cls, "conf": conf,
                          "x1": x1, "y1": y1, "x2": x2, "y2": y2,
                          "cx": (x1 + x2) / 2, "cy": (y1 + y2) / 2, "has_face": has_face})
        cands.sort(key=lambda c: c["cx"])  # left→right → stable A/B/C labels
        for j, c in enumerate(cands):
            c["label"] = chr(ord("A") + j) if j < 26 else f"Z{j}"

        # Annotate a tile: candidate boxes + labels + the scene index.
        tile = frame.copy()
        for c in cands:
            p1 = (int(c["x1"]), int(c["y1"]))
            p2 = (int(c["x2"]), int(c["y2"]))
            cv2.rectangle(tile, p1, p2, (0, 200, 255), 2)
            txt = c["label"] + ":" + c["cls"] + ("/face" if c["has_face"] else "")
            cv2.putText(tile, txt, (p1[0], max(16, p1[1] - 6)),
                        cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 200, 255), 2)
        cv2.putText(tile, f"scene {si}", (8, 26),
                    cv2.FONT_HERSHEY_SIMPLEX, 0.8, (255, 255, 255), 2)
        tw = 480
        th = max(1, int(round(frame_h * tw / frame_w)))
        tiles.append(cv2.resize(tile, (tw, th)))
        scenes_json.append({
            "scene": si, "start_frame": int(s), "keyframe_frame": int(mid),
            "t_ms": int(mid / fps * 1000),
            "candidates": [{
                "label": c["label"], "cls": c["cls"], "conf": c["conf"],
                "cx": round(c["cx"] / frame_w, 4), "cy": round(c["cy"] / frame_h, 4),
                "box": [round(c["x1"] / frame_w, 4), round(c["y1"] / frame_h, 4),
                        round(c["x2"] / frame_w, 4), round(c["y2"] / frame_h, 4)],
                "has_face": c["has_face"],
            } for c in cands],
        })

    cap.release()

    sheet_path = None
    if tiles:
        cols = 1 if len(tiles) == 1 else (2 if len(tiles) <= 4 else 3)
        rows = (len(tiles) + cols - 1) // cols
        th, tw = tiles[0].shape[0], tiles[0].shape[1]
        sheet = np.full((rows * th, cols * tw, 3), 24, np.uint8)
        for idx, t in enumerate(tiles):
            r, cc = idx // cols, idx % cols
            sheet[r * th:(r + 1) * th, cc * tw:(cc + 1) * tw] = t
        out_dir = out_dir or tempfile.mkdtemp(prefix="cut-contact-")
        Path(out_dir).mkdir(parents=True, exist_ok=True)
        sheet_path = str(Path(out_dir) / "contact_sheet.jpg")
        cv2.imwrite(sheet_path, sheet, [cv2.IMWRITE_JPEG_QUALITY, 85])
    log(f"contact_sheet: {len(scenes_json)} scene tiles → {sheet_path}")
    return {
        "contact_sheet": sheet_path,
        "fps": finite_number(fps), "frame_width": frame_w, "frame_height": frame_h,
        "preset": preset, "detector": model_name, "device": device_label,
        "scene_count": len(scenes_json), "scenes": scenes_json,
    }


# QC thresholds (heuristic hints; the model's vision is the real judge).
_QC_OFFCENTER = 0.22   # |face_cx - 0.5| above this ⇒ subject pushed to the edge
_QC_HEAD_CUT = 0.02    # face-top normalized y below this ⇒ head clipped at the top


def build_qc_sheet(media_path: str, preset: str = "talking_head", out_dir=None) -> dict:
    """Director QC pass — review the REFRAMED
    OUTPUT and surface what to fix. Samples one keyframe per scene of the finished
    reframe, detects the framed subject + face, and tiles them into a review contact
    sheet WITH cheap CV quality hints: subject_present, face_present, face centering
    (|cx-0.5|), and headroom (face-top y; a head clipped at the top edge). The model
    READS the sheet (its vision is the real judge of 'wrong subject / bad framing')
    and re-issues a corrected render.reframe{direction}; the hints + `needs_review`
    flag focus its attention. Same sparse, never-per-frame discipline as the director
    pass — one detection per scene on the OUTPUT this time."""
    import cv2
    import numpy as np
    try:
        import torch  # noqa: F401
        import torchvision  # noqa: F401
    except Exception as exc:
        die("sidecar", "qc_sheet needs the CV deps (torch, torchvision)",
            f"{exc}; pip install torchvision into the sidecar venv")

    quality = os.environ.get("SHELLX_CUT_DETECTOR", "fast")
    device, device_label = _resolve_device()
    cap = cv2.VideoCapture(media_path)
    fps = cap.get(cv2.CAP_PROP_FPS) or 30.0
    frame_w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    frame_h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT)) or 0
    if frame_w == 0 or frame_h == 0:
        cap.release()
        die("sidecar", "qc_sheet: no decodable video geometry", media_path)
    person_id = _SUBJECT_CLASS_IDS["person"]

    from scenedetect import open_video, SceneManager
    from scenedetect.detectors import ContentDetector
    sm = SceneManager()
    sm.add_detector(ContentDetector())
    sm.detect_scenes(open_video(media_path))

    def _sf(tc):
        return tc.frame_num if hasattr(tc, "frame_num") else tc.get_frames()

    scene_list = sm.get_scene_list()
    spans = [(_sf(s), _sf(e)) for s, e in scene_list] if scene_list else [(0, total or int(fps))]

    predict, model_name = _load_detector(quality, device)
    face = _load_face_detector()
    log(f"qc_sheet: {model_name} on {device_label}, {len(spans)} scenes, {frame_w}x{frame_h}")

    tiles = []
    scenes_json = []
    review_count = 0
    for si, (s, e) in enumerate(spans):
        mid = (s + max(s + 1, e)) // 2
        cap.set(cv2.CAP_PROP_POS_FRAMES, mid)
        ok, frame = cap.read()
        if not ok:
            cap.set(cv2.CAP_PROP_POS_FRAMES, s)
            ok, frame = cap.read()
        if not ok:
            continue
        boxes, scores, labels = predict(frame)
        persons = [i for i in range(len(labels))
                   if int(labels[i]) == person_id and finite_number(scores[i]) > 0.30]
        subject_present = len(persons) > 0
        faces = _detect_faces(face, frame) if (face is not None and subject_present) else []
        # The largest face = the framed subject; measure its composition.
        fb = max(faces, key=lambda b: (b[4] - b[2]) * (b[5] - b[3]), default=None)
        face_present = fb is not None
        face_cx = round(fb[0] / frame_w, 4) if fb is not None else None
        offcenter = round(abs(face_cx - 0.5), 4) if face_cx is not None else None
        headroom = round(fb[3] / frame_h, 4) if fb is not None else None  # face-top y (norm)
        # Heuristic flag (the model's vision is authoritative; this just focuses it).
        needs_review = (not subject_present) or (
            face_present and ((offcenter is not None and offcenter > _QC_OFFCENTER)
                              or (headroom is not None and headroom < _QC_HEAD_CUT)))
        issues = []
        if not subject_present:
            issues.append("no_subject")
        if face_present and offcenter is not None and offcenter > _QC_OFFCENTER:
            issues.append("off_center")
        if face_present and headroom is not None and headroom < _QC_HEAD_CUT:
            issues.append("head_cut")
        if needs_review:
            review_count += 1

        tile = frame.copy()
        if fb is not None:
            cv2.rectangle(tile, (int(fb[2]), int(fb[3])), (int(fb[4]), int(fb[5])),
                          (0, 0, 255) if needs_review else (0, 200, 0), 2)
        tag = f"scene {si}" + (" REVIEW:" + ",".join(issues) if needs_review else " ok")
        color = (0, 0, 255) if needs_review else (0, 220, 0)
        cv2.putText(tile, tag, (8, 26), cv2.FONT_HERSHEY_SIMPLEX, 0.7, color, 2)
        tw = 360
        th = max(1, int(round(frame_h * tw / frame_w)))
        tiles.append(cv2.resize(tile, (tw, th)))
        scenes_json.append({
            "scene": si, "keyframe_frame": int(mid), "t_ms": int(mid / fps * 1000),
            "subject_present": subject_present, "face_present": face_present,
            "face_cx": face_cx, "off_center": offcenter, "headroom": headroom,
            "needs_review": needs_review, "issues": issues,
        })

    cap.release()
    sheet_path = None
    if tiles:
        cols = 1 if len(tiles) == 1 else (2 if len(tiles) <= 4 else 3)
        rows = (len(tiles) + cols - 1) // cols
        th, tw = tiles[0].shape[0], tiles[0].shape[1]
        sheet = np.full((rows * th, cols * tw, 3), 24, np.uint8)
        for idx, t in enumerate(tiles):
            r, cc = idx // cols, idx % cols
            sheet[r * th:(r + 1) * th, cc * tw:(cc + 1) * tw] = t
        out_dir = out_dir or tempfile.mkdtemp(prefix="cut-qc-")
        Path(out_dir).mkdir(parents=True, exist_ok=True)
        sheet_path = str(Path(out_dir) / "qc_sheet.jpg")
        cv2.imwrite(sheet_path, sheet, [cv2.IMWRITE_JPEG_QUALITY, 85])
    log(f"qc_sheet: {len(scenes_json)} scenes, {review_count} flagged → {sheet_path}")
    return {
        "qc_sheet": sheet_path,
        "fps": finite_number(fps), "frame_width": frame_w, "frame_height": frame_h,
        "detector": model_name, "device": device_label,
        "scene_count": len(scenes_json), "review_count": review_count,
        "scenes": scenes_json,
    }


# ---------------------------------------------------------------------------
# entrypoint
# ---------------------------------------------------------------------------

# Instruments that run by DEFAULT (at import / when a request names none). The
# heavy per-frame `subject` instrument (auto-reframe) is deliberately EXCLUDED —
# it runs ONLY when reframe explicitly requests instruments=["subject"], so a
# normal import never pays its cost.
DEFAULT_INSTRUMENTS = ["words", "silence", "scenes", "beats", "loudness"]
# All VALID instrument names (request validation accepts these).
ALL_INSTRUMENTS = DEFAULT_INSTRUMENTS + ["subject"]


def build_request_from_argv(argv: list):
    """Human/CLI mode: positional media + flags → wire request + --out path."""
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("media", help="media file to analyze")
    ap.add_argument("--out", help="also write the report to this path")
    ap.add_argument("--instruments", default=",".join(DEFAULT_INSTRUMENTS),
                    help="comma list, subset of: " + ",".join(ALL_INSTRUMENTS)
                         + " (default excludes the heavy on-demand 'subject')")
    ap.add_argument("--subject-preset", default="talking_head",
                    help="auto-reframe subject preset: " + ",".join(_SUBJECT_PRESETS))
    ap.add_argument("--contact-sheet", default=None, metavar="DIR",
                    help="DIRECTOR mode: build a per-scene contact sheet into DIR "
                         "(+ candidate JSON to stdout) instead of running instruments")
    ap.add_argument("--qc-sheet", default=None, metavar="DIR",
                    help="DIRECTOR v2 QC mode: review a reframed OUTPUT — per-scene "
                         "frames + composition hints into DIR (+ QC JSON to stdout)")
    ap.add_argument("--direction", default=None, metavar="JSON",
                    help="DIRECTOR brief for the subject instrument: a JSON map "
                         '{scene_idx: {"cx": float} | {"mode": "widen"}}')
    ap.add_argument("--model", default="small", help="whisper model (default small)")
    ap.add_argument("--asset-id", default=None, help="asset id (default: file stem)")
    ap.add_argument("--hash", default=None, help="sha256:… (default: computed)")
    args = ap.parse_args(argv)
    media = str(Path(args.media).resolve())
    asset_hash = args.hash
    if asset_hash is None and Path(media).is_file():
        h = hashlib.sha256()
        with open(media, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        asset_hash = f"sha256:{h.hexdigest()}"
    return {
        "media_path": media,
        "asset_id": args.asset_id or Path(media).stem,
        "asset_hash": asset_hash or "",
        "instruments": [i.strip() for i in args.instruments.split(",") if i.strip()],
        "whisper_model": args.model,
        "subject_preset": args.subject_preset,
        "contact_sheet": args.contact_sheet,
        "qc_sheet": args.qc_sheet,
        "direction": json.loads(args.direction) if args.direction else None,
    }, args.out


def main() -> int:
    # Wire discipline (see REAL_STDOUT): any library that prints or logs to
    # sys.stdout from here on actually writes to stderr. The one JSON document
    # this process emits goes to REAL_STDOUT at the end (or through die()).
    sys.stdout = sys.stderr
    if len(sys.argv) > 1:
        req, out_path = build_request_from_argv(sys.argv[1:])
    else:
        req, out_path = json.load(sys.stdin), None

    media = req.get("media_path", "")
    if not media or not Path(media).is_file():
        die("sidecar", "media file not found", f"media_path={media!r}")
    # `subject` is on-demand only, so it is NEVER in the default set — a request
    # must name it explicitly. Validation still accepts it (it's in ALL_INSTRUMENTS).
    instruments = req.get("instruments") or DEFAULT_INSTRUMENTS
    unknown = [i for i in instruments if i not in ALL_INSTRUMENTS]
    if unknown:
        die("sidecar", "unknown instruments requested",
            f"{unknown}; valid: {ALL_INSTRUMENTS}")
    # audio-only media guard: never run video instruments on a file with no video stream
    # (see has_video_stream). instruments_run reflects what ACTUALLY ran. `subject`
    # (auto-reframe) is video-only too — drop it on an audio-only file.
    video_only = ("scenes", "subject")
    if any(i in instruments for i in video_only) and not has_video_stream(media):
        log("video instruments skipped — no video stream in input (audio-only file)")
        instruments = [i for i in instruments if i not in video_only]
    # Symmetric guard for AUDIO instruments: a video-only clip or a legitimately
    # silent render has no audio stream, so words/silence/beats/loudness are
    # skipped (and dropped from instruments_run) instead of crashing the job.
    # No-audio regression coverage includes b-roll, music-less screen demos,
    # and silent renders.
    audio_instruments = ("words", "silence", "beats", "loudness")
    if any(i in instruments for i in audio_instruments) and not has_audio_stream(media):
        log("audio instruments skipped — no audio stream in input")
        instruments = [i for i in instruments if i not in audio_instruments]
    asset_id = req.get("asset_id", Path(media).stem)
    model_name = req.get("whisper_model") or "small"
    subject_preset = req.get("subject_preset") or "talking_head"

    # Director contact-sheet mode emits its OWN JSON (sheet
    # path + per-scene candidates) and returns — it does NOT run the per-frame
    # instruments. Video-only, like the auto-reframe analysis it feeds.
    if req.get("contact_sheet") is not None:
        if not has_video_stream(media):
            die("sidecar", "contact_sheet needs a video stream", media)
        sheet = build_contact_sheet(media, subject_preset, req.get("contact_sheet") or None)
        print(json.dumps(sheet), file=REAL_STDOUT, flush=True)
        return 0

    # Director QC mode: review a reframed OUTPUT (per-scene frames + composition
    # hints) for the model to judge + correct. Emits its own JSON, then returns.
    if req.get("qc_sheet") is not None:
        if not has_video_stream(media):
            die("sidecar", "qc_sheet needs a video stream", media)
        sheet = build_qc_sheet(media, subject_preset, req.get("qc_sheet") or None)
        print(json.dumps(sheet), file=REAL_STDOUT, flush=True)
        return 0

    report = {
        "schema": SCHEMA,
        "asset_hash": req.get("asset_hash", ""),
        "source_path": media,
        "instruments_run": instruments,
        "silences": [],
        "scenes": [],
        "black_spans": [],
        "frozen_spans": [],
        "content_bbox": None,
    }

    with tempfile.TemporaryDirectory(prefix="cut-perception-") as tmpdir:
        needs_wav = any(i in instruments for i in ("silence", "beats"))
        wav16k = extract_wav16k(media, tmpdir) if needs_wav else None
        # Per-instrument PROGRESS so the enrich job never sits at a FROZEN number
        # while the perception battery runs. Scene-detect on a multi-GB file is the
        # bottleneck; emitting before each instrument keeps the label + % moving
        # ("perception:scenes" at 40% tells the user it's detecting scenes, not
        # stuck). `words` also streams its own finer-grained PROGRESS. The Rust
        # enrich job maps these into its 0.6..1.0 band.
        _battery = [i for i in ("words", "silence", "scenes", "beats", "loudness") if i in instruments]
        _total = max(len(_battery), 1)
        _done = 0
        if "words" in instruments:
            _emit_progress(_done / _total, "perception:words")
            report["words"] = instrument_words(media, asset_id, model_name)
            _done += 1
        if "silence" in instruments:
            _emit_progress(_done / _total, "perception:silence")
            report["silences"] = instrument_silence(media, wav16k)
            _done += 1
        if "scenes" in instruments:
            _emit_progress(_done / _total, "perception:scenes")
            cuts, black, frozen = instrument_scenes(media)
            report["scenes"], report["black_spans"], report["frozen_spans"] = cuts, black, frozen
            # content_bbox / uniform-border: rides on the "scenes"
            # (video-only) instrument so audio-only assets never trigger it
            # (the has_video_stream gate above already dropped "scenes" for
            # them — preserves the audio-only media guard).
            report["content_bbox"] = detect_content_bbox(media)
            _done += 1
        if "beats" in instruments:
            _emit_progress(_done / _total, "perception:beats")
            report["beats"] = instrument_beats(wav16k)
            _done += 1
        if "loudness" in instruments:
            _emit_progress(_done / _total, "perception:loudness")
            report["loudness"] = instrument_loudness(media)
            _done += 1
        if "subject" in instruments:
            # On-demand auto-reframe analysis → the SubjectTrack (aspect-independent,
            # normalized). Heavy per-frame CV; only runs because it was explicitly
            # requested. The render derives the moving crop from this. An optional
            # The director brief overrides the per-scene subject pick.
            report["subject_track"] = instrument_subject(media, subject_preset, req.get("direction"))

    print(json.dumps(report), file=REAL_STDOUT, flush=True)
    if out_path:
        Path(out_path).write_text(json.dumps(report, indent=2))
        log(f"report written to {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
