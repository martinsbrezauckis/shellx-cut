#!/usr/bin/env python3
"""ShellX Cut render-review contract, prompts, and sampling helpers.

The bundled access ladder imports this module for one stable
``shellx-cut/judge-review/1`` envelope across Claude, Codex, Antigravity, and
Grok subscription CLIs. It consumes a rendered video plus measured perception
facts, samples visual frames with ShellX Cut's configured ffmpeg tools, and
keeps measured claims separate from model judgment.

This file also keeps the developer-only direct ``dry-run``, Gemini API, and
Ollama harnesses used to exercise the shared prompt and validation contract.
Installed ``verify.judge`` uses the subscription-CLI ladder and needs no API
key.

HONESTY RULES (enforced in code, not just prose):
  - Missing API key / missing model / missing SDK => envelope status
    "not_run" with a reason. NEVER a fabricated verdict.
  - Backend exceptions => status "error" with the exception text.
  - backend.watched / backend.listened record what the model actually
    received; the prompt tells the model the same thing.

Usage:
  judge.py estimate --render R.mp4 --perception P.json [--mode global|window]
      [--fps N] [--windows a:b,c:d] [--resolution low|default] [--backend B]
  judge.py review --render R.mp4 --perception P.json --backend dry-run|gemini|ollama
      [--mode global|window] [--windows a:b,c:d] [--fps N] [--model NAME]
      [--resolution low|default] [--intent TEXT] [--out review.json]
      [--frames-dir DIR] [--max-frames N]

Exit codes: 0 = envelope produced (even not_run — that IS the honest result),
2 = bad invocation/input.
"""

from __future__ import annotations

import argparse
import base64
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from datetime import datetime, timezone

SCHEMA = "shellx-cut/judge-review/1"
PERCEPTION_SCHEMA = "shellx-cut/perception/1"

# ---------------------------------------------------------------------------
# Token + cost math for the developer-only direct Gemini harness. Pricing is
# intentionally treated as an estimate and omitted when unknown.
# ---------------------------------------------------------------------------

TOK_PER_FRAME = {"default": 258, "low": 66}  # Gemini per-frame video tokens
AUDIO_TOK_PER_S = 32                          # Gemini flat audio token rate
TEXT_CHARS_PER_TOK = 4.0                      # rough prompt-text estimate
# Rough local-model image-token estimate; model and build dependent.
OLLAMA_TOK_PER_IMAGE = 250

# $/1M input tokens for the developer-only direct Gemini harness.
# Flash output UNVERIFIED — None means "don't pretend to know".
GEMINI_PRICING_PER_M = {
    "gemini-3-flash-preview": {"in": 0.50, "out": None},
    "gemini-3.1-pro-preview": {"in": 2.00, "out": 12.00},
}

DEFAULT_GEMINI_MODEL = "gemini-3-flash-preview"
DEFAULT_OLLAMA_MODEL = "openbmb/minicpm-o4.5"
DEFAULT_GLOBAL_FPS = 1.0
DEFAULT_WINDOW_FPS = 5.0

# ---------------------------------------------------------------------------
# Structured output schema — the model must return exactly this (docs/public/JUDGE_REVIEW.md §5).
# Used as Gemini response_schema AND Ollama `format`. Core required by task
# spec: verdict, issues[{at_ms,kind,severity,evidence}], confidence.
# ---------------------------------------------------------------------------

ISSUE_KINDS = [
    "content_error",     # missing/duplicated/out-of-order/truncated content
    "cut_artifact",      # perceptually jarring cut (visual jump, audible splice)
    "av_sync_suspect",   # judge may SUSPECT sync issues, never measure them
    "caption_error",     # wrong/garbled/missing caption text
    "caption_timing",    # caption visibly out of step with speech (>~1 word)
    "audio_artifact",    # click/pop/tone jump/music truncation (if listened)
    "visual_artifact",   # black/frozen/garbled frames, stutter, aspect error
    "pacing",            # dead air, rushed section, rhythm complaints
    "narrative",         # result does not flow per stated edit intent
    "other",
]

REVIEW_SCHEMA = {
    "type": "object",
    "properties": {
        "verdict": {"type": "string", "enum": ["pass", "fail", "needs_review"]},
        "issues": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "at_ms": {"type": "integer"},
                    "end_ms": {"type": "integer"},
                    "kind": {"type": "string", "enum": ISSUE_KINDS},
                    "severity": {"type": "string", "enum": ["blocker", "major", "minor"]},
                    "evidence": {"type": "string"},
                    "suggested_fix": {"type": "string"},
                },
                "required": ["at_ms", "kind", "severity", "evidence"],
            },
        },
        "cannot_assess": {"type": "array", "items": {"type": "string"}},
        "confidence": {"type": "number"},
        "summary": {"type": "string"},
    },
    "required": ["verdict", "issues", "confidence", "summary"],
}

# ---------------------------------------------------------------------------
# Prompt templates (docs/public/JUDGE_REVIEW.md §5 — keep in sync)
# ---------------------------------------------------------------------------

SYSTEM_PROMPT = """\
You are the render reviewer ("judge") inside ShellX Cut, an agent-first video
editor. You review a RENDERED video that an editing agent produced, against
(a) deterministic instrument measurements provided as facts in your context
and (b) the editor's stated intent. You are the perceptual layer: you judge
what a human viewer would notice. You are NOT a measuring instrument.

HARD RULES — violating any of these makes your review worthless:
1. INSTRUMENT FACTS ARE GROUND TRUTH for anything measured: loudness (LUFS,
   true peak), exact timestamps, word timings, durations, AV offset. Never
   contradict them, never re-estimate them, never report a measurement of
   your own. If a question hinges on a measured quantity you were not given,
   list it in cannot_assess.
2. You CANNOT perceive and must never opine on: absolute loudness level;
   audio-video offsets below ~1 sampled frame; exact millisecond positions
   of cuts; whether a cut lands on a musical beat; encoder/bitrate quality
   beyond gross visible breakage. These go in cannot_assess when relevant.
3. Timestamp honesty: you see video sampled at {fps} fps, so any visual
   finding carries about ±{granularity_ms} ms uncertainty. Report at_ms as
   your best estimate; do not invent precision beyond that.
4. Modality honesty: watched={watched}, listened={listened}. If you did not
   receive audio, say nothing about sound beyond quoting instrument facts;
   put audio questions in cannot_assess.
5. Evidence or silence: every issue must describe what you actually saw or
   heard at that point. If the render is clean, verdict "pass" with an empty
   issues list is the correct answer — do not manufacture findings.
6. confidence (0-1) is your confidence in the overall verdict. Below 0.6
   signals that a stronger judge should re-review; honesty is cheap,
   escalation is cheap, a wrong confident verdict is expensive.

Respond ONLY with JSON matching the provided schema. at_ms/end_ms are
absolute milliseconds from the start of the full render.
"""

GLOBAL_REVIEW_PROMPT = """\
REVIEW TYPE: global pass over the complete render ({duration_s:.1f} s).

EDIT INTENT (what the editing agent meant to do):
{intent}

INSTRUMENT FACTS (deterministic ground truth):
{digest}

{frame_manifest}\
ANSWER THESE via issues[] / cannot_assess / summary:
1. Content integrity — anything missing, duplicated, out of order? Sentences
   that start or end truncated near cuts?
2. Cut quality (perceptual) — any cut a viewer would notice as jarring:
   visual jump, mid-gesture snap{audio_cut_clause}?
3. Captions — present during speech, readable, no garbled text, roughly in
   step with speech (within ~1 word)?
4. {audio_question}
5. Visual integrity — black/frozen/garbled frames, stutter, wrong aspect.
6. Pacing & narrative — does the result flow as the edit intent describes?
"""

WINDOW_REVIEW_PROMPT = """\
REVIEW TYPE: flagged-window re-probe at {fps} fps.
WINDOW: {start_ms}-{end_ms} ms of the full render. Reason flagged:
{reason}

EDIT INTENT (context):
{intent}

INSTRUMENT FACTS for this window (deterministic ground truth):
{digest}

{frame_manifest}\
QUESTION: at this sampling rate, does this window look{listen_clause} natural
to a viewer? Look specifically for: truncated words at the splice, visual
jump/stutter, caption flicker or mistiming{audio_artifact_clause}.
Report only what you perceive INSIDE this window. Use absolute at_ms
timestamps (window start = {start_ms} ms).
"""

# ---------------------------------------------------------------------------
# Subprocess helpers (ffmpeg/ffprobe)
# ---------------------------------------------------------------------------


def run(cmd: list[str]) -> subprocess.CompletedProcess:
    """Run a subprocess, raising with stderr attached on failure."""
    cp = subprocess.run(cmd, capture_output=True, text=True)
    if cp.returncode != 0:
        raise RuntimeError(f"{cmd[0]} failed ({cp.returncode}): {cp.stderr[-800:]}")
    return cp


def configured_media_tool(name: str) -> str:
    """Resolve ffmpeg/ffprobe with the same explicit/dir/PATH order as cutd."""
    explicit = os.environ.get(
        "SHELLX_CUT_FFMPEG" if name == "ffmpeg" else "SHELLX_CUT_FFPROBE"
    )
    if explicit:
        return explicit
    directory = os.environ.get("SHELLX_CUT_FFMPEG_DIR")
    if directory:
        candidate = os.path.join(directory, name + (".exe" if os.name == "nt" else ""))
        if os.path.isfile(candidate):
            return candidate
    return shutil.which(name) or name


def probe_duration_s(path: str) -> float:
    """Container duration in seconds via ffprobe."""
    cp = run([
        configured_media_tool("ffprobe"), "-v", "error",
        "-show_entries", "format=duration",
        "-of", "json", path,
    ])
    return float(json.loads(cp.stdout)["format"]["duration"])


def extract_frames(render: str, out_dir: str, fps: float,
                   start_ms: int | None = None, end_ms: int | None = None,
                   max_frames: int = 120, width: int = 768) -> list[dict]:
    """Extract JPEG frames for frame-based backends / dry-run inspection.

    Returns a manifest [{path, at_ms}] — at_ms computed from the fps grid so
    frames-only models can be told WHEN each frame is (they cannot infer it).
    Caps at max_frames (drops the tail, recorded by caller in the envelope).
    """
    os.makedirs(out_dir, exist_ok=True)
    cmd = [configured_media_tool("ffmpeg"), "-hide_banner", "-y"]
    if start_ms is not None:
        cmd += ["-ss", f"{start_ms / 1000:.3f}"]
    cmd += ["-i", render]
    if start_ms is not None and end_ms is not None:
        cmd += ["-t", f"{(end_ms - start_ms) / 1000:.3f}"]
    cmd += ["-vf", f"fps={fps},scale={width}:-2", "-q:v", "4",
            os.path.join(out_dir, "f_%05d.jpg")]
    run(cmd)
    files = sorted(f for f in os.listdir(out_dir) if f.endswith(".jpg"))
    base = start_ms or 0
    manifest = [
        {"path": os.path.join(out_dir, f), "at_ms": int(base + i * 1000.0 / fps)}
        for i, f in enumerate(files)
    ]
    return manifest[:max_frames]


# ---------------------------------------------------------------------------
# Perception digest — compact instrument facts for the prompt (docs/public/JUDGE_REVIEW.md §4)
# ---------------------------------------------------------------------------


def load_perception(path: str) -> dict:
    with open(path) as f:
        p = json.load(f)
    if p.get("schema") != PERCEPTION_SCHEMA:
        raise ValueError(
            f"perception schema {p.get('schema')!r} != {PERCEPTION_SCHEMA!r}")
    return p


def sanity_check_perception(p: dict, duration_ms: int) -> None:
    """Reject source-coordinate facts accidentally paired with render frames.

    No perception timestamp may exceed the render duration. Raises ValueError
    listing offenders before frames are extracted or subscription quota is used.

    Slack: instruments may legitimately overshoot the container duration by a
    hair (silero measures the extracted wav, which pads ~16-50 ms past the
    container; observed 46 016 ms on a 46 000 ms render). Allowance is
    max(500 ms, 1% of duration) — coordinate-space mistakes are whole-segment
    sized (tens of seconds), never sub-second.
    """
    slack_ms = max(500, duration_ms // 100)
    limit = duration_ms + slack_ms
    offenders: list[str] = []
    for s in p.get("scenes") or []:
        if s.get("at_ms", 0) > limit:
            offenders.append(f"scene change at {s['at_ms']} ms")
    for key in ("silences", "black_spans", "frozen_spans"):
        for s in p.get(key) or []:
            if s.get("end_ms", 0) > limit:
                offenders.append(f"{key} span {s.get('start_ms')}-{s['end_ms']} ms")
    for w in (p.get("words") or {}).get("words") or []:
        if w.get("end_ms", 0) > limit:
            offenders.append(f"word {w.get('word')!r} ending {w['end_ms']} ms")
    for w in (p.get("loudness") or {}).get("windows") or []:
        if w.get("at_ms", 0) > limit:
            offenders.append(f"loudness window at {w['at_ms']} ms")
    if offenders:
        shown = "; ".join(offenders[:5])
        more = f" (+{len(offenders) - 5} more)" if len(offenders) > 5 else ""
        raise ValueError(
            f"perception timestamps exceed the render duration "
            f"({duration_ms} ms + {slack_ms} ms slack): {shown}{more}. "
            "This is almost certainly SOURCE-asset perception fed to a judge "
            "watching RENDER frames (coordinate-space mismatch). Bundle the "
            "render output's own perception "
            "(receipts/<render_id>.output.perception.json) or omit "
            "--perception to auto-resolve it by content hash.")


def digest_perception(p: dict, window: tuple[int, int] | None = None,
                      max_transcript_chars: int = 1500) -> str:
    """Render perception.json into a compact text block for the judge prompt.

    Window mode filters facts to the window span. Transcript text is included
    (capped) — it doubles as the 'ear proxy' for deaf frame-based backends.
    """

    def in_win(ms: int) -> bool:
        return window is None or (window[0] <= ms <= window[1])

    # Every span field is read via .get() and a span missing a timestamp
    # it needs is SKIPPED, not fatal. A malformed-but-schema-tagged perception
    # (e.g. a silence span with no end_ms) must not crash mid-prompt-build with
    # an uncaught KeyError outside the adapter's try/except — mirror the
    # defensive posture of sanity_check_perception above. A skipped partial span
    # is the honest outcome (we cannot place a fact we cannot fully read).
    lines: list[str] = []
    loud = p.get("loudness") or {}
    if loud:
        lines.append(
            f"- loudness (ebur128): integrated {loud.get('integrated_lufs')} LUFS, "
            f"true peak {loud.get('true_peak_dbtp')} dBTP  "
            f"[MEASURED — never re-estimate]")
    sil = [s for s in (p.get("silences") or [])
           if s.get("start_ms") is not None and s.get("end_ms") is not None
           and (in_win(s["start_ms"]) or in_win(s["end_ms"]))]
    if sil:
        spans = ", ".join(f"{s['start_ms']}-{s['end_ms']}ms" for s in sil[:20])
        more = f" (+{len(sil) - 20} more)" if len(sil) > 20 else ""
        lines.append(f"- silence spans (silero+ffmpeg): {spans}{more}")
    scenes = [s for s in (p.get("scenes") or [])
              if s.get("at_ms") is not None and in_win(s["at_ms"])]
    if scenes:
        cuts = ", ".join(f"{s['at_ms']}ms" for s in scenes[:20])
        more = f" (+{len(scenes) - 20} more)" if len(scenes) > 20 else ""
        lines.append(f"- visual scene changes (PySceneDetect): {cuts}{more}")
    beats = p.get("beats") or {}
    if beats.get("bpm"):
        lines.append(f"- music beat grid (librosa): {beats['bpm']:.1f} bpm, "
                     f"{len(beats.get('beats_ms', []))} beats")
    words = (p.get("words") or {}).get("words") or []
    # A word needs start_ms (to window-filter), a word string AND end_ms (the
    # speech-span line dereferences end_ms on the last word) to be usable.
    wwin = [w for w in words
            if w.get("start_ms") is not None and w.get("end_ms") is not None
            and w.get("word") is not None and in_win(w["start_ms"])]
    if wwin:
        text = " ".join(w["word"] for w in wwin)
        if len(text) > max_transcript_chars:
            text = text[:max_transcript_chars] + " …[truncated]"
        lines.append(
            f"- transcript (whisperX, word-timed ±50ms; {len(wwin)} words"
            f"{' in window' if window else ''}): \"{text}\"")
        lines.append(
            f"  speech span: {wwin[0]['start_ms']}-{wwin[-1]['end_ms']}ms")
    if not lines:
        lines.append("- (no instrument facts available for this span)")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Prompt assembly
# ---------------------------------------------------------------------------


def build_prompts(mode: str, perception: dict, duration_s: float, fps: float,
                  intent: str, listened: bool, watched: bool,
                  window: tuple[int, int] | None, window_reason: str,
                  frame_manifest: list[dict] | None) -> tuple[str, str]:
    """Returns (system_prompt, user_prompt) for the review call."""
    granularity_ms = int(1000.0 / fps / 2)  # ± half a sample interval
    sys_p = SYSTEM_PROMPT.format(
        fps=fps, granularity_ms=granularity_ms,
        watched=str(watched).lower(), listened=str(listened).lower())

    # Frames-only backends need an explicit time map; native-video backends
    # (Gemini) get timestamps from the model side, so the block is empty.
    fm = ""
    if frame_manifest:
        rows = ", ".join(f"frame {i + 1}={m['at_ms']}ms"
                         for i, m in enumerate(frame_manifest))
        fm = f"FRAME TIME MAP (images attached in this order): {rows}\n\n"

    if mode == "global":
        user_p = GLOBAL_REVIEW_PROMPT.format(
            duration_s=duration_s,
            intent=intent,
            digest=digest_perception(perception),
            frame_manifest=fm,
            audio_cut_clause=", abrupt audio tone change" if listened else "",
            audio_question=(
                "Audio continuity — audible splices, music/room-tone jumps, "
                "clicks or pops at cut points?" if listened else
                "Audio: you did NOT receive audio. Add unanswered audio "
                "questions to cannot_assess."),
        )
    else:
        assert window is not None
        user_p = WINDOW_REVIEW_PROMPT.format(
            fps=fps, start_ms=window[0], end_ms=window[1],
            reason=window_reason or "(no reason recorded)",
            intent=intent,
            digest=digest_perception(perception, window=window),
            frame_manifest=fm,
            listen_clause=" and sound" if listened else "",
            audio_artifact_clause=(
                ", audible click/pop or tone jump" if listened else ""),
        )
    return sys_p, user_p


# ---------------------------------------------------------------------------
# Token + cost estimate (testable without any API key)
# ---------------------------------------------------------------------------


def estimate_tokens(backend: str, duration_s: float, fps: float,
                    resolution: str, prompt_chars: int,
                    n_frames: int | None, model: str) -> dict:
    """Input-token + cost estimate per docs/public/JUDGE_REVIEW.md §6. Honest about unknowns."""
    text_tok = int(prompt_chars / TEXT_CHARS_PER_TOK)
    est: dict = {"text_tokens": text_tok, "notes": []}
    if backend == "gemini":
        frames = math.ceil(duration_s * fps)
        video_tok = frames * TOK_PER_FRAME[resolution]
        audio_tok = math.ceil(duration_s) * AUDIO_TOK_PER_S
        total = video_tok + audio_tok + text_tok
        est.update(frames=frames, video_tokens=video_tok,
                   audio_tokens=audio_tok, input_tokens_total=total)
        price = GEMINI_PRICING_PER_M.get(model)
        if price:
            est["input_cost_usd"] = round(total * price["in"] / 1e6, 6)
            if price["out"] is None:
                est["notes"].append(
                    "output cost excluded: output $/M unverified for this model")
        else:
            est["notes"].append(f"no pricing table entry for {model}")
    elif backend == "ollama":
        imgs = n_frames or 0
        est.update(frames=imgs,
                   image_tokens_rough=imgs * OLLAMA_TOK_PER_IMAGE,
                   input_tokens_total=imgs * OLLAMA_TOK_PER_IMAGE + text_tok,
                   input_cost_usd=0.0)
        est["notes"].append(
            f"image token count is a rough constant ({OLLAMA_TOK_PER_IMAGE}/img),"
            " model-dependent; local inference is $0")
    else:  # dry-run: show the gemini math as the reference plan
        return estimate_tokens("gemini", duration_s, fps, resolution,
                               prompt_chars, n_frames, model)
    return est


# ---------------------------------------------------------------------------
# Backends — each returns the inner review dict (validated) or raises;
# availability problems return None + reason via the (review, not_run_reason)
# tuple so the envelope stays honest.
# ---------------------------------------------------------------------------


# Enum/value constraints pulled DIRECTLY from REVIEW_SCHEMA so validate_review
# can never drift from the declared schema. If the schema changes, these
# follow automatically — never hardcode a parallel guessed list.
_VERDICT_ENUM = REVIEW_SCHEMA["properties"]["verdict"]["enum"]
_ISSUE_SCHEMA = REVIEW_SCHEMA["properties"]["issues"]["items"]["properties"]
_KIND_ENUM = _ISSUE_SCHEMA["kind"]["enum"]            # == ISSUE_KINDS
_SEVERITY_ENUM = _ISSUE_SCHEMA["severity"]["enum"]
# Issue keys the schema types as integer ms (at_ms required, end_ms optional).
_INT_MS_KEYS = ("at_ms", "end_ms")


def _coerce_int_ms(value, key: str, issue: dict) -> int:
    """Coerce a schema-declared integer-ms field, rejecting non-integral input.

    REVIEW_SCHEMA types at_ms/end_ms as `integer`. Models sometimes emit a float
    (9000.0) — accept it ONLY when it is integral (no information lost); reject a
    fractional float (9000.5) and any non-numeric ("around 9000") with a clear
    ValueError. bool is rejected explicitly (a Python bool is an int subtype, but
    `at_ms: true` is garbage, not a timestamp).
    """
    if isinstance(value, bool):
        raise ValueError(f"issue {key!r} must be an int ms, got bool {value!r}: {issue}")
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        if value.is_integer():
            return int(value)
        raise ValueError(
            f"issue {key!r} must be an integer ms, got non-integral float "
            f"{value!r}: {issue}")
    raise ValueError(f"issue {key!r} must be an int ms, got {type(value).__name__} "
                     f"{value!r}: {issue}")


def validate_review(obj: dict) -> dict:
    """Validate model output against REVIEW_SCHEMA's structure AND value enums.

    Not a full JSON-Schema validator, but it enforces every value constraint the
    schema declares — this is the ONLY validation gate the schema-less
    antigravity/grok rungs have, so a weak check here lets garbage verdicts
    through. All enums/types are derived from REVIEW_SCHEMA above so they
    stay in sync. Mutates issues in place to coerce integral-float ms to int.
    """
    for k in REVIEW_SCHEMA["required"]:
        if k not in obj:
            raise ValueError(f"model output missing required key {k!r}")
    if obj["verdict"] not in _VERDICT_ENUM:
        raise ValueError(f"bad verdict {obj['verdict']!r} (allowed: {_VERDICT_ENUM})")
    if not isinstance(obj["issues"], list):
        raise ValueError("issues is not a list")
    # confidence: schema type "number", semantically a probability in [0,1].
    conf = obj.get("confidence")
    if isinstance(conf, bool) or not isinstance(conf, (int, float)):
        raise ValueError(f"confidence must be a number, got {conf!r}")
    if not (0.0 <= conf <= 1.0):
        raise ValueError(f"confidence {conf!r} out of range [0,1]")
    for i in obj["issues"]:
        for k in ("at_ms", "kind", "severity", "evidence"):
            if k not in i:
                raise ValueError(f"issue missing {k!r}: {i}")
        if i["kind"] not in _KIND_ENUM:
            raise ValueError(f"bad issue kind {i['kind']!r} (allowed: {_KIND_ENUM})")
        if i["severity"] not in _SEVERITY_ENUM:
            raise ValueError(
                f"bad issue severity {i['severity']!r} (allowed: {_SEVERITY_ENUM})")
        # at_ms/end_ms are schema integers — coerce integral floats, reject the
        # rest (a float that loses precision, or a non-numeric like "around 9000").
        for k in _INT_MS_KEYS:
            if k in i and i[k] is not None:
                i[k] = _coerce_int_ms(i[k], k, i)
    return obj


def backend_gemini(render: str, sys_p: str, user_p: str, fps: float,
                   resolution: str, model: str,
                   window: tuple[int, int] | None):
    """Native watch+listen review via google-genai. Returns (review|None, reason|None).

    Uploads the render once via the Files API (re-usable ~48 h for window
    re-probes); passes videoMetadata fps + start/end offsets for windows.
    """
    if not os.environ.get("GEMINI_API_KEY"):
        return None, "GEMINI_API_KEY not set — honest not_run (no fabricated verdict)"
    try:
        from google import genai
        from google.genai import types
    except ImportError:
        return None, "google-genai SDK not installed (pip install google-genai)"

    client = genai.Client()  # reads GEMINI_API_KEY
    f = client.files.upload(file=render)
    # Files API processes async; poll until ACTIVE (cap ~120 s).
    import time
    for _ in range(60):
        if f.state and str(f.state).endswith("ACTIVE"):
            break
        time.sleep(2)
        f = client.files.get(name=f.name)
    vm_kwargs: dict = {"fps": fps}
    if window:
        vm_kwargs["start_offset"] = f"{window[0] / 1000:.3f}s"
        vm_kwargs["end_offset"] = f"{window[1] / 1000:.3f}s"
    part = types.Part(
        file_data=types.FileData(file_uri=f.uri, mime_type="video/mp4"),
        video_metadata=types.VideoMetadata(**vm_kwargs),
    )
    cfg = types.GenerateContentConfig(
        system_instruction=sys_p,
        response_mime_type="application/json",
        response_schema=REVIEW_SCHEMA,
        media_resolution=("MEDIA_RESOLUTION_LOW" if resolution == "low"
                          else "MEDIA_RESOLUTION_MEDIUM"),
        temperature=0,
    )
    resp = client.models.generate_content(
        model=model, contents=[part, user_p], config=cfg)
    review = validate_review(json.loads(resp.text))
    usage = getattr(resp, "usage_metadata", None)
    if usage:
        review["_usage"] = {
            "input_tokens": getattr(usage, "prompt_token_count", None),
            "output_tokens": getattr(usage, "candidates_token_count", None),
        }
    return review, None


def backend_ollama(sys_p: str, user_p: str, frames: list[dict], model: str,
                   host: str = "http://127.0.0.1:11434"):
    """Frames-only local review via Ollama /api/chat. Returns (review|None, reason|None).

    listened=false by design: Ollama's MiniCPM-o/Qwen3-VL builds take
    text+image only. Audio context arrives via the
    transcript inside the perception digest.
    """
    # Guard: urllib follows file:// etc.; only allow explicit http(s) hosts
    # (semgrep CWE-939). The host is operator-supplied config, never user data.
    if not (host.startswith("http://") or host.startswith("https://")):
        return None, f"refusing non-http(s) ollama host {host!r}"
    # Availability checks first — honest not_run beats a hang or a fake.
    try:
        # host is scheme-validated http(s) operator config (guard above)
        with urllib.request.urlopen(f"{host}/api/tags", timeout=5) as r:  # nosemgrep
            tags = json.load(r)
    except (urllib.error.URLError, OSError) as e:
        return None, f"ollama not reachable at {host}: {e}"
    names = {m["name"] for m in tags.get("models", [])}
    if model not in names and f"{model}:latest" not in names:
        return None, (f"model {model!r} not present locally "
                      f"(have: {sorted(names)}); pull it first")

    images = []
    for m in frames:
        with open(m["path"], "rb") as fh:
            images.append(base64.b64encode(fh.read()).decode())
    payload = {
        "model": model,
        "messages": [
            {"role": "system", "content": sys_p},
            {"role": "user", "content": user_p, "images": images},
        ],
        "stream": False,
        "format": REVIEW_SCHEMA,   # Ollama structured output (>=0.5)
        "options": {"temperature": 0},
    }
    req = urllib.request.Request(
        f"{host}/api/chat", data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"})
    # same scheme-validated host as above
    with urllib.request.urlopen(req, timeout=600) as r:  # nosemgrep
        out = json.load(r)
    review = validate_review(json.loads(out["message"]["content"]))
    review["_usage"] = {
        "input_tokens": out.get("prompt_eval_count"),
        "output_tokens": out.get("eval_count"),
        "total_duration_ms": int(out.get("total_duration", 0) / 1e6),
    }
    return review, None


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def parse_windows(s: str | None) -> list[tuple[int, int]]:
    if not s:
        return []
    out = []
    for span in s.split(","):
        a, b = span.split(":")
        out.append((int(a), int(b)))
    return out


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["review", "estimate"])
    ap.add_argument("--render", required=True)
    ap.add_argument("--perception", required=True)
    ap.add_argument("--backend", default="dry-run",
                    choices=["dry-run", "gemini", "ollama"])
    ap.add_argument("--mode", default="global", choices=["global", "window"])
    ap.add_argument("--windows", help="comma list of start:end ms spans")
    ap.add_argument("--window-reason", default="flagged by instrument checks")
    ap.add_argument("--fps", type=float, default=None,
                    help="sampling fps (default: 1 global, 5 window)")
    ap.add_argument("--resolution", default="low", choices=["low", "default"])
    ap.add_argument("--model", default=None)
    ap.add_argument("--intent", default="(no edit intent provided)")
    ap.add_argument("--out", default=None, help="write envelope JSON here")
    ap.add_argument("--frames-dir", default=None)
    ap.add_argument("--max-frames", type=int, default=32,
                    help="frame cap for frame-based backends (ctx budget)")
    args = ap.parse_args()

    if not os.path.exists(args.render):
        print(f"render not found: {args.render}", file=sys.stderr)
        return 2
    perception = load_perception(args.perception)
    duration_s = probe_duration_s(args.render)
    # Coordinate-space guard (the coordinate-space guard): refuse source-asset perception
    # BEFORE any backend call — a judge fed wrong-space facts mis-reasons.
    try:
        sanity_check_perception(perception, int(duration_s * 1000))
    except ValueError as e:
        print(f"perception sanity check FAILED: {e}", file=sys.stderr)
        return 2
    windows = parse_windows(args.windows)
    if args.mode == "window" and not windows:
        print("--mode window requires --windows a:b[,c:d]", file=sys.stderr)
        return 2
    fps = args.fps or (DEFAULT_WINDOW_FPS if args.mode == "window"
                       else DEFAULT_GLOBAL_FPS)
    model = args.model or (DEFAULT_OLLAMA_MODEL if args.backend == "ollama"
                           else DEFAULT_GEMINI_MODEL)
    # Modality truth per backend: only Gemini hears.
    listened = args.backend == "gemini"
    window = windows[0] if args.mode == "window" else None
    span_s = ((window[1] - window[0]) / 1000.0) if window else duration_s

    # Frames: needed by ollama; dry-run extracts them too so the extraction
    # path is testable without any model.
    frames: list[dict] = []
    frames_dir = args.frames_dir or tempfile.mkdtemp(prefix="judge_frames_")
    if args.backend in ("ollama", "dry-run") and args.command == "review":
        frames = extract_frames(
            args.render, frames_dir, fps,
            start_ms=window[0] if window else None,
            end_ms=window[1] if window else None,
            max_frames=args.max_frames)

    sys_p, user_p = build_prompts(
        args.mode, perception, duration_s, fps, args.intent,
        listened=listened, watched=True, window=window,
        window_reason=args.window_reason,
        frame_manifest=frames if args.backend in ("ollama", "dry-run") else None)

    est = estimate_tokens(args.backend, span_s, fps, args.resolution,
                          len(sys_p) + len(user_p), len(frames), model)

    if args.command == "estimate":
        print(json.dumps({"mode": args.mode, "fps": fps, "span_s": span_s,
                          "resolution": args.resolution, "model": model,
                          "estimate": est}, indent=2))
        return 0

    # ---- review ----
    review, not_run_reason, status = None, None, "completed"
    if args.backend == "dry-run":
        status, not_run_reason = "not_run", (
            "dry-run backend: prompts assembled, frames extracted, no model called")
    elif args.backend == "gemini":
        try:
            review, not_run_reason = backend_gemini(
                args.render, sys_p, user_p, fps, args.resolution, model, window)
            status = "completed" if review else "not_run"
        except Exception as e:  # noqa: BLE001 — surface honestly, never fake
            status, not_run_reason = "error", f"{type(e).__name__}: {e}"
    elif args.backend == "ollama":
        try:
            review, not_run_reason = backend_ollama(sys_p, user_p, frames, model)
            status = "completed" if review else "not_run"
        except Exception as e:  # noqa: BLE001
            status, not_run_reason = "error", f"{type(e).__name__}: {e}"

    envelope = {
        "schema": SCHEMA,
        "ts": now_iso(),
        "render": os.path.abspath(args.render),
        "mode": args.mode,
        "backend": {
            "name": args.backend, "model": model, "fps": fps,
            "resolution": args.resolution,
            "watched": True, "listened": listened,
            # frames actually sent (0 for native-video gemini path)
            "frames_sent": len(frames) if args.backend != "gemini" else 0,
        },
        "window": ({"start_ms": window[0], "end_ms": window[1],
                    "reason": args.window_reason} if window else None),
        "status": status,
        "not_run_reason": not_run_reason,
        "review": review,
        "estimate": est,
        "prompt_chars": {"system": len(sys_p), "user": len(user_p)},
        "frames_dir": frames_dir if frames else None,
    }
    text = json.dumps(envelope, indent=2)
    if args.out:
        with open(args.out, "w") as f:
            f.write(text + "\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
