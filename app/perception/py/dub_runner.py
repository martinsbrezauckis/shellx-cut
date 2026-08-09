#!/usr/bin/env python3
"""dub_runner.py — the one-shot AI DUBBING sidecar (synth + time-fit + assemble).

Role: turn a list of ALREADY-TRANSLATED, timestamped segments into a single
continuous dubbed WAV (24 kHz mono), each segment re-voiced by the OmniVoice TTS
service and placed at its ORIGINAL start time (silence between). cutd spawns this
per-dub exactly like the perception / matte / translate sidecars (JSON-in on
stdin, ONE JSON line out on stdout) — invisible plumbing inside the one ShellX
Cut app. The Rust side (server/dub.rs, dispatch::audio_dub) owns the transcript
read, the translation (reuses transcript.translate machinery) and the timeline
ops; THIS runner owns only the TTS HTTP calls + ffmpeg time-fit + WAV assembly.

Why this shape (mirrors translate_runner / matte_runner):
- One-shot CLI = the SAME pattern as every other sidecar (cutd manages the
  lifecycle; the user never sees it). The network touch (the OmniVoice service)
  happens here, at synth time only — replay/render stay fully offline (they read
  the assembled WAV asset).
- STDLIB ONLY (urllib + wave + struct + subprocess) so the runner runs on any
  python3 — no numpy / requests / torch dependency. ffmpeg/ffprobe on PATH is the
  only external tool, and it is used ONLY for `atempo` residual time-fit.

Engine contract:
  POST {endpoint}/synthesize {text, voice, speed?, duration?}
    -> raw streamed PCM, signed 16-bit little-endian, 24000 Hz, mono (s16le),
       NO wav header. `duration` (seconds) re-paces the utterance to a target
       length with a near-constant +~0.19 s additive offset (postprocess edge
       fade) that `atempo` trims trivially.
  Optional header X-LV-VA-Auth: <secret> when the service runs with a secret.

I/O contract (matches server/dub.rs):
  stdin  (JSON): {endpoint, voice, lang?, secret?, sample_rate?, out_wav,
                  segments:[{i, start_ms, slot_ms, text}, ...]}
  stdout (ONE JSON line): {out_wav, sample_rate, voice, n_segments, total_ms,
                  endpoint, segments:[{i, start_ms, slot_ms, synth_ms,
                  fit_ratio, atempo, placed_at_ms, placed_ms}, ...]}
  Non-zero exit + a human message on stderr on any failure (service down, a bad
  segment, an HTTP error) — NEVER a fake/empty track.

Per-segment time-fit policy:
  target = slot_ms (the segment's own [start,end] span).
  - synth LONGER than the slot (the common case — the +0.19 s offset): speed it
    up with `atempo` to hit the slot (fit BEFORE placement → higher quality than
    truncation), chaining atempo stages to stay inside ffmpeg's 0.5..2.0 per
    stage.
  - synth SHORTER than the slot: leave it (it sits on silence) rather than
    over-stretch to unnaturally slow speech (prefer silence over
    over-compression). `fit_ratio` surfaces the mismatch so the agent sees it.
  placed_ms is clamped to the gap to the NEXT segment's start so a segment can
  never clobber the next (a safety net; atempo already keeps it within the slot).

Dependencies: ffmpeg/ffprobe on PATH (cutd forwards SHELLX_CUT_FFMPEG_DIR).
Primary caller: server/dub.rs (audio.dub).
"""
from __future__ import annotations

import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import wave
import ipaddress
from pathlib import Path
from safe_numbers import finite_number

try:
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8", errors="backslashreplace")
except AttributeError as exc:
    _RECONFIGURE_ERROR = exc

# Cold Windows installs keep ffmpeg/ffprobe in the app tools dir, not PATH.
# cutd forwards that directory as SHELLX_CUT_FFMPEG_DIR; mirror matte_runner.py.
_ff_dir = os.environ.get("SHELLX_CUT_FFMPEG_DIR", "").strip()
if _ff_dir:
    _candidates = [_ff_dir, os.path.join(_ff_dir, "bin")]
    _present = [d for d in _candidates if os.path.isdir(d)]
    if _present:
        os.environ["PATH"] = os.pathsep.join(_present + [os.environ.get("PATH", "")])

FFMPEG_BIN = shutil.which("ffmpeg") or "ffmpeg"

BYTES_PER_SAMPLE = 2  # s16le, mono
_SERVICE_OPENER = urllib.request.build_opener()
_BLOCKED_SERVICE_HOSTS = {
    "metadata",
    "metadata.google.internal",
}


def _die(msg: str, code: int = 1) -> "None":
    sys.stderr.write(msg.rstrip() + "\n")
    sys.exit(code)


def resolve_output_path(path_value) -> Path:
    if not path_value:
        _die("dub_runner: 'out_wav' is required")
    path = Path(str(path_value)).expanduser()
    if not path.is_absolute():
        _die("dub_runner: 'out_wav' must be an absolute path")
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def service_url(endpoint: str, route: str) -> str:
    raw = endpoint.strip()
    if any(ch in raw for ch in ("\r", "\n", "\t", "\\")):
        raise RuntimeError("OmniVoice endpoint contains disallowed URL characters")
    parsed = urllib.parse.urlsplit(raw)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise RuntimeError("OmniVoice endpoint must be an http(s) URL with a host")
    if parsed.username or parsed.password:
        raise RuntimeError("OmniVoice endpoint must not include credentials in the URL")
    host = (parsed.hostname or "").strip().lower()
    if not host or host in _BLOCKED_SERVICE_HOSTS:
        raise RuntimeError("OmniVoice endpoint host is not allowed")
    try:
        ip = ipaddress.ip_address(host.strip("[]"))
    except ValueError:
        ip = None
    if ip is not None and (ip.is_link_local or ip.is_multicast or ip.is_unspecified):
        raise RuntimeError("OmniVoice endpoint IP range is not allowed")
    path = (parsed.path.rstrip("/") + route) or route
    return urllib.parse.urlunparse((parsed.scheme, parsed.netloc, path, "", "", ""))


def ms_to_bytes(ms: float, sr: int) -> int:
    """Absolute ms → a SAMPLE-aligned byte offset in an s16le mono stream."""
    samples = round(ms / 1000.0 * sr)
    return samples * BYTES_PER_SAMPLE


def bytes_to_ms(n: int, sr: int) -> float:
    """s16le mono byte count → milliseconds."""
    return (n / BYTES_PER_SAMPLE) / sr * 1000.0


def atempo_factors(ratio: float) -> "list[float]":
    """Decompose a tempo `ratio` (>1 = faster/shorter) into a chain of factors
    each inside ffmpeg's supported [0.5, 2.0] window. Product == ratio."""
    factors: "list[float]" = []
    r = finite_number(ratio, 1.0)
    # Guard against pathological values.
    if r <= 0:
        return [1.0]
    while r > 2.0:
        factors.append(2.0)
        r /= 2.0
    while r < 0.5:
        factors.append(0.5)
        r *= 2.0
    factors.append(r)
    return factors


def synth_segment(
    endpoint: str, voice: str, text: str, duration_s: float, secret: str | None, timeout: float
) -> bytes:
    """POST one segment to {endpoint}/synthesize and return the raw s16le PCM.

    Raises RuntimeError with an actionable message on any transport/HTTP error so
    the caller fails the whole dub honestly (never a fake/empty track)."""
    payload = {"text": text, "voice": voice}
    if duration_s and duration_s > 0:
        payload["duration"] = round(finite_number(duration_s), 4)
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        service_url(endpoint, "/synthesize"),
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    if secret:
        req.add_header("X-LV-VA-Auth", secret)
    try:
        with _SERVICE_OPENER.open(req, timeout=timeout) as resp:
            return resp.read()
    except urllib.error.HTTPError as e:
        detail = ""
        try:
            detail = e.read().decode("utf-8", "replace")[:300]
        except Exception as detail_error:
            detail = f"could not read error body: {detail_error}"[:300]
        raise RuntimeError(
            f"OmniVoice /synthesize returned HTTP {e.code} for voice '{voice}': {detail}"
        )
    except urllib.error.URLError as e:
        raise RuntimeError(
            f"OmniVoice :endpoint unreachable ({endpoint}): {e.reason}. "
            f"Start the configured dubbing service or set CUT_DUB_ENDPOINT."
        )


def synth_segment_resilient(
    endpoint: str, voice: str, text: str, duration_s: float, secret: str | None, timeout: float
) -> "tuple[bytes, int]":
    """Recover from a successful-but-empty model stream without hiding it.

    OmniVoice generation is stochastic and can very occasionally return an
    empty waveform for a valid short segment. Retry the exact constrained
    request once, then let the model choose an unconstrained duration; the
    existing atempo path still fits that fallback to the requested slot.
    Transport and HTTP errors remain immediate failures.
    """
    attempts = (duration_s, duration_s, 0.0)
    for attempt, requested_duration in enumerate(attempts, 1):
        pcm = synth_segment(
            endpoint, voice, text, requested_duration, secret, timeout
        )
        if pcm:
            return pcm, attempt
    raise RuntimeError(
        "OmniVoice returned 0 bytes after 3 synthesis attempts "
        "(two duration-fitted and one model-timed)"
    )


def atempo_fit(pcm: bytes, ratio: float, sr: int) -> bytes:
    """Run ffmpeg `atempo` to multiply the segment's tempo by `ratio`
    (>1 = shorter). Pure raw s16le in → raw s16le out via temp files."""
    chain = ",".join(f"atempo={f:.6f}" for f in atempo_factors(ratio))
    with tempfile.TemporaryDirectory(prefix="cutd-dub-") as td:
        src = os.path.join(td, "in.pcm")
        dst = os.path.join(td, "out.pcm")
        with open(src, "wb") as f:
            f.write(pcm)
        cmd = [
            FFMPEG_BIN, "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
            "-f", "s16le", "-ar", str(sr), "-ac", "1", "-i", src,
            "-filter:a", chain,
            "-f", "s16le", "-ar", str(sr), "-ac", "1", dst,
        ]
        proc = subprocess.run(cmd, capture_output=True, timeout=120)
        if proc.returncode != 0:
            raise RuntimeError(
                "ffmpeg atempo time-fit failed: "
                + proc.stderr.decode("utf-8", "replace").strip()[:300]
            )
        with open(dst, "rb") as f:
            return f.read()


def main() -> int:
    raw = sys.stdin.read()
    if not raw.strip():
        _die("dub_runner: empty stdin (expected the dub job JSON)")
    try:
        job = json.loads(raw)
    except json.JSONDecodeError as e:
        _die(f"dub_runner: stdin is not valid JSON: {e}")

    endpoint = str(job.get("endpoint") or "http://127.0.0.1:9001")
    voice = str(job.get("voice") or "rebeka")
    secret = job.get("secret") or os.environ.get("OMNIVOICE_TTS_SECRET") or None
    sr = int(job.get("sample_rate") or 24000)
    out_wav = resolve_output_path(job.get("out_wav"))
    timeout = finite_number(job.get("timeout_s") or 180.0, 180.0)
    segments = job.get("segments") or []
    if not segments:
        _die("dub_runner: no segments to dub")

    # Order by start (the placement is absolute) and pre-compute each segment's
    # gap to the NEXT one so a fitted segment can never clobber its successor.
    segments = sorted(segments, key=lambda s: int(s["start_ms"]))
    starts = [int(s["start_ms"]) for s in segments]

    master = bytearray()
    out_segs = []
    for n, seg in enumerate(segments):
        i = int(seg.get("i", n))
        start_ms = int(seg["start_ms"])
        slot_ms = max(1, int(seg["slot_ms"]))
        text = str(seg.get("text") or "").strip()
        gap_to_next = (starts[n + 1] - start_ms) if n + 1 < len(starts) else None

        if not text:
            # Empty translation → contribute pure silence (no synth call).
            out_segs.append({
                "i": i, "start_ms": start_ms, "slot_ms": slot_ms,
                "synth_ms": 0.0, "fit_ratio": 0.0, "atempo": 1.0,
                "placed_at_ms": start_ms, "placed_ms": 0,
                "synth_attempts": 0, "skipped": True,
            })
            continue

        pcm, synth_attempts = synth_segment_resilient(
            endpoint, voice, text, slot_ms / 1000.0, secret, timeout
        )
        synth_ms = bytes_to_ms(len(pcm), sr)
        fit_ratio = synth_ms / slot_ms if slot_ms else 1.0

        # Time-fit: only COMPRESS when the synth overran its slot (the common
        # +0.19 s case). A shorter synth is left to sit on silence.
        atempo = 1.0
        fitted = pcm
        if synth_ms > slot_ms + 15.0:
            atempo = synth_ms / slot_ms
            fitted = atempo_fit(pcm, atempo, sr)

        # Clamp the placed length to the gap to the next segment (safety net).
        placed = fitted
        if gap_to_next is not None:
            max_bytes = ms_to_bytes(gap_to_next, sr)
            if len(placed) > max_bytes:
                placed = placed[:max_bytes]
        placed_ms = round(bytes_to_ms(len(placed), sr))

        # Write the placed PCM into the master buffer at the absolute offset.
        off = ms_to_bytes(start_ms, sr)
        end = off + len(placed)
        if len(master) < end:
            master.extend(b"\x00" * (end - len(master)))
        master[off:end] = placed

        out_segs.append({
            "i": i,
            "start_ms": start_ms,
            "slot_ms": slot_ms,
            "synth_ms": round(synth_ms, 1),
            "fit_ratio": round(fit_ratio, 4),
            "atempo": round(atempo, 4),
            "placed_at_ms": start_ms,
            "placed_ms": placed_ms,
            "synth_attempts": synth_attempts,
        })

    # Pad to a whole sample frame and write the continuous dubbed WAV.
    if len(master) % BYTES_PER_SAMPLE != 0:
        master.append(0)
    with wave.open(str(out_wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(BYTES_PER_SAMPLE)
        w.setframerate(sr)
        w.writeframes(bytes(master))
    total_ms = round(bytes_to_ms(len(master), sr))

    sys.stdout.write(json.dumps({
        "out_wav": str(out_wav),
        "sample_rate": sr,
        "voice": voice,
        "endpoint": endpoint,
        "n_segments": len(out_segs),
        "total_ms": total_ms,
        "segments": out_segs,
    }) + "\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except RuntimeError as e:
        _die(f"dub_runner: {e}")
    except Exception as e:  # noqa: BLE001 — surface ANY failure as a clean message
        _die(f"dub_runner: unexpected error: {e}")
