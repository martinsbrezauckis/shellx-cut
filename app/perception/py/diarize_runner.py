#!/usr/bin/env python3
"""diarize_runner.py — the one-shot SPEAKER DIARIZATION sidecar (extract + POST).

Role: turn an asset's audio into normalized speaker turns ("who spoke when") by
extracting its audio to 16 kHz mono WAV and POSTing it to the Sortformer v2
diarization service (loopback ``:9002``). cutd spawns this per-``media.diarize`` exactly
like the perception / matte / dub sidecars (JSON-in on stdin, ONE JSON line out on
stdout) — invisible plumbing inside the one ShellX Cut app. The Rust side
(``server/diarize.rs`` + the ``media.diarize`` dispatch handler) owns the receipt,
the word↔speaker alignment and the report writes; THIS runner owns only the ffmpeg
audio extract + the HTTP call.

Why this shape (mirrors dub_runner.py):
- One-shot CLI = the SAME pattern as every other sidecar (cutd manages the
  lifecycle; the user never sees it). The network touch (the diarization service)
  happens here only — replay/render never need diarization (turns are baked into
  the perception receipt at plan time).
- STDLIB ONLY (urllib + wave + json) so the runner runs on any python3 — no torch /
  numpy / requests / NeMo dependency (the heavy Sortformer/NeMo stack lives on the
  GPU box behind the service). ffmpeg on PATH is the only external tool, used ONLY
  to decode the asset's audio to the 16 kHz mono WAV the service expects.

Engine contract:
  GET  {endpoint}/health
    -> {"status":"ok","model":"sortformer-v2","loaded":true,"device":"cuda", ...}
  POST {endpoint}/diarize  (Content-Type: audio/wav, body = the WAV bytes,
                            optional ?max_speakers=N)
    -> {"turns":[{"start_ms","end_ms","speaker":"S1"}...], "n_speakers", "model",
        "rtf", "audio_s", "infer_s"}
  Speakers are normalized "S1".."Sn" in ARRIVAL order by the service. We POST the
  WAV BYTES (Mode B) rather than a wav_path, because the asset lives on the CLIENT
  (Mac/Win/Linux) and the GPU service is reached over an SSH tunnel — its filesystem
  is not the client's. Optional header X-LV-VA-Auth: <secret> when set.

I/O contract (matches server/diarize.rs):
  stdin  (JSON): {endpoint, media, max_speakers?, secret?, timeout_s?}
  stdout (ONE JSON line): {schema, turns:[{start_ms,end_ms,speaker}], n_speakers,
                  model, backend, endpoint, device?, sample_rate, rtf?, audio_s?,
                  infer_s?}
  Non-zero exit + a human message on stderr on any failure (service down/loading, a
  bad asset, an HTTP error) — NEVER faked/empty turns.

Dependencies: ffmpeg on PATH (cutd forwards SHELLX_CUT_FFMPEG_DIR). Primary caller:
server/diarize.rs (media.diarize).
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import ipaddress
from safe_numbers import finite_number

# Cold Windows installs keep ffmpeg/ffprobe in the app tools dir, not PATH.
# cutd forwards that directory as SHELLX_CUT_FFMPEG_DIR; mirror dub_runner.py.
_ff_dir = os.environ.get("SHELLX_CUT_FFMPEG_DIR", "").strip()
if _ff_dir:
    _candidates = [_ff_dir, os.path.join(_ff_dir, "bin")]
    _present = [d for d in _candidates if os.path.isdir(d)]
    if _present:
        os.environ["PATH"] = os.pathsep.join(_present + [os.environ.get("PATH", "")])

FFMPEG_BIN = shutil.which("ffmpeg") or "ffmpeg"

SAMPLE_RATE = 16000  # the service's expected rate (it resamples anyway; we pin it)
SCHEMA = "shellx-cut/diarize/1"
_SERVICE_OPENER = urllib.request.build_opener()
_BLOCKED_SERVICE_HOSTS = {
    "metadata",
    "metadata.google.internal",
}


def _die(msg: str, code: int = 1) -> "None":
    sys.stderr.write(msg.rstrip() + "\n")
    sys.exit(code)


def extract_wav16k(media: str, out_wav: str) -> None:
    """Decode the asset's audio to 16 kHz MONO PCM-s16 WAV (the diarizer input).

    Mirrors the perception sidecar's extract_wav16k / dub's atempo decode: bare
    ``ffmpeg`` (resolved via SHELLX_CUT_FFMPEG_DIR), audio-only (``-vn``), downmixed
    to mono and resampled to 16 kHz. Raises RuntimeError on failure so the caller
    fails the whole diarize honestly (never faked turns)."""
    cmd = [
        FFMPEG_BIN, "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
        "-i", media,
        "-vn", "-ac", "1", "-ar", str(SAMPLE_RATE),
        "-c:a", "pcm_s16le", "-f", "wav", out_wav,
    ]
    proc = subprocess.run(cmd, capture_output=True, timeout=120)
    if proc.returncode != 0:
        raise RuntimeError(
            "ffmpeg audio extract failed (no audio stream, or a bad/again unreadable "
            "asset): " + proc.stderr.decode("utf-8", "replace").strip()[:300]
        )
    if not os.path.isfile(out_wav) or os.path.getsize(out_wav) <= 44:  # 44 = WAV hdr
        raise RuntimeError(
            "ffmpeg produced no audio — the asset has no decodable audio stream to "
            "diarize (diarization needs speech)."
        )


def service_url(endpoint: str, route: str, query: dict[str, str] | None = None) -> str:
    raw = endpoint.strip()
    if any(ch in raw for ch in ("\r", "\n", "\t", "\\")):
        raise RuntimeError("diarization endpoint contains disallowed URL characters")
    parsed = urllib.parse.urlsplit(raw)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise RuntimeError("diarization endpoint must be an http(s) URL with a host")
    if parsed.username or parsed.password:
        raise RuntimeError("diarization endpoint must not include credentials in the URL")
    host = (parsed.hostname or "").strip().lower()
    if not host or host in _BLOCKED_SERVICE_HOSTS:
        raise RuntimeError("diarization endpoint host is not allowed")
    try:
        ip = ipaddress.ip_address(host.strip("[]"))
    except ValueError:
        ip = None
    if ip is not None and (ip.is_link_local or ip.is_multicast or ip.is_unspecified):
        raise RuntimeError("diarization endpoint IP range is not allowed")
    path = (parsed.path.rstrip("/") + route) or route
    qs = urllib.parse.urlencode(query or {})
    return urllib.parse.urlunparse((parsed.scheme, parsed.netloc, path, "", qs, ""))


def check_health(endpoint: str, secret: str | None, timeout: float) -> None:
    """GET {endpoint}/health and raise an ACTIONABLE error when the service is
    unreachable or its model is still loading (the SETUP-error contract, like
    media.index). Mirrors the blueprint's health-gated provisioning."""
    url = service_url(endpoint, "/health")
    req = urllib.request.Request(url, method="GET")
    if secret:
        req.add_header("X-LV-VA-Auth", secret)
    try:
        with _SERVICE_OPENER.open(req, timeout=timeout) as r:
            info = json.loads(r.read())
    except urllib.error.URLError as e:
        raise RuntimeError(
            f"diarization service unreachable ({endpoint}): {e.reason}. "
            f"Start the configured diarization service or set CUT_DIARIZE_ENDPOINT."
        )
    if not info.get("loaded", False):
        raise RuntimeError(
            f"diarization model not ready (status={info.get('status')!r}); the "
            f"service at {endpoint} is still loading — retry in a few seconds."
        )


def diarize_bytes(
    endpoint: str, wav: bytes, max_speakers: int | None, secret: str | None,
    timeout: float,
) -> dict:
    """POST the WAV bytes to {endpoint}/diarize and return the parsed JSON. Raises
    RuntimeError with an actionable message on any transport/HTTP error."""
    query = {"max_speakers": str(int(max_speakers))} if max_speakers else None
    url = service_url(endpoint, "/diarize", query)
    req = urllib.request.Request(
        url, data=wav, headers={"Content-Type": "audio/wav"}, method="POST"
    )
    if secret:
        req.add_header("X-LV-VA-Auth", secret)
    try:
        with _SERVICE_OPENER.open(req, timeout=timeout) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        detail = ""
        try:
            detail = e.read().decode("utf-8", "replace")[:300]
        except Exception as detail_error:
            detail = f"could not read error body: {detail_error}"[:300]
        if e.code == 503:
            raise RuntimeError(
                f"diarization service is still loading the model (HTTP 503 at "
                f"{endpoint}); retry shortly. {detail}"
            )
        raise RuntimeError(
            f"diarization /diarize returned HTTP {e.code} at {endpoint}: {detail}"
        )
    except urllib.error.URLError as e:
        raise RuntimeError(
            f"diarization service unreachable ({endpoint}): {e.reason}. "
            f"Start the configured diarization service or set CUT_DIARIZE_ENDPOINT."
        )


def normalize_turns(raw_turns: list) -> list:
    """Coerce the service turns to {start_ms:int, end_ms:int, speaker:str}, drop
    degenerate (end<=start) turns, and sort by start_ms (defensive — the service
    already arrival-orders + sorts, but a runner that re-normalizes is robust to a
    backend swap)."""
    out = []
    for t in raw_turns or []:
        try:
            s = int(round(finite_number(t["start_ms"])))
            e = int(round(finite_number(t["end_ms"])))
            spk = str(t["speaker"])
        except (KeyError, TypeError, ValueError):
            continue
        if e <= s:
            continue
        out.append({"start_ms": s, "end_ms": e, "speaker": spk})
    out.sort(key=lambda x: (x["start_ms"], x["end_ms"]))
    return out


def main() -> int:
    raw = sys.stdin.read()
    if not raw.strip():
        _die("diarize_runner: empty stdin (expected the diarize job JSON)")
    try:
        job = json.loads(raw)
    except json.JSONDecodeError as e:
        _die(f"diarize_runner: stdin is not valid JSON: {e}")

    endpoint = str(job.get("endpoint") or "http://127.0.0.1:9002")
    media = job.get("media")
    max_speakers = job.get("max_speakers")
    secret = job.get("secret") or os.environ.get("CUT_DIARIZE_SECRET") or None
    timeout = finite_number(job.get("timeout_s") or 120.0, 120.0)
    if not media:
        _die("diarize_runner: 'media' (the asset path) is required")
    if not os.path.isfile(media):
        _die(f"diarize_runner: media not found: {media}")

    # 1. Fail fast with an actionable message if the service isn't ready.
    check_health(endpoint, secret, min(timeout, 15.0))

    # 2. Decode the asset's audio → 16 kHz mono WAV (a temp file, auto-cleaned).
    with tempfile.TemporaryDirectory(prefix="cutd-diarize-") as td:
        wav_path = os.path.join(td, "audio16k.wav")
        extract_wav16k(media, wav_path)
        with open(wav_path, "rb") as f:
            wav = f.read()

        # 3. POST the bytes → normalized speaker turns.
        resp = diarize_bytes(endpoint, wav, max_speakers, secret, timeout)

    turns = normalize_turns(resp.get("turns"))
    n_speakers = len({t["speaker"] for t in turns})
    model = str(resp.get("model") or "sortformer-v2")
    backend = "pyannote" if "pyannote" in model.lower() else "sortformer"

    out = {
        "schema": SCHEMA,
        "turns": turns,
        "n_speakers": n_speakers,
        "model": model,
        "backend": backend,
        "endpoint": endpoint,
        "sample_rate": SAMPLE_RATE,
    }
    # Pass through the service's performance/provenance fields when present.
    for k in ("rtf", "audio_s", "infer_s", "device"):
        if k in resp:
            out[k] = resp[k]

    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except RuntimeError as e:
        _die(f"diarize_runner: {e}")
    except Exception as e:  # noqa: BLE001 — surface ANY failure as a clean message
        _die(f"diarize_runner: unexpected error: {e}")
