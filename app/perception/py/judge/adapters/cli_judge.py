#!/usr/bin/env python3
"""Claude subscription-CLI render judge adapter.

Samples frames from the rendered output, combines them with measured render
facts and edit intent, invokes Claude Code with only the Read tool, validates
the structured verdict, and applies the shared vision-only honesty filter.
Missing CLI/auth produces not_run; attempted failures produce error.
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

# Single source of truth for prompts/schema/digest lives in ../judge.py.
_JUDGE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _JUDGE_DIR)
import judge  # noqa: E402

ADAPTER_NAME = "cli"
DEFAULT_PROVIDER = "claude"          # codex / gemini adapters slot in later
DEFAULT_CLI_MODEL = "sonnet"         # cheap quota; judge work, not authoring
DEFAULT_GLOBAL_FPS = 1.0             # docs/public/JUDGE_REVIEW.md §2.1
DEFAULT_WINDOW_FPS = 5.0             # docs/public/JUDGE_REVIEW.md §2.2
DEFAULT_MAX_FRAMES = 20              # CLI context cap (each frame = 1 Read img)
DEFAULT_FRAME_WIDTH = 512            # low-res for iteration reviews
DEFAULT_TIMEOUT_S = 600
SIDECAR_TIMEOUT_S = 1800             # whisperX on a long render takes minutes

# judge/ lives immediately inside the shipped perception payload.
_SIDECAR_DIR = os.path.dirname(_JUDGE_DIR)


# ---------------------------------------------------------------------------
# Render-perception resolution — the judge bundle must
# carry the RENDER's own instrument facts, never the source asset's.
# ---------------------------------------------------------------------------


def sha256_file(path: str) -> str:
    """Content hash in the receipts convention: 'sha256:<hex>'."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def find_receipt_perception(render: str, render_hash: str) -> str | None:
    """Search the cutproj receipts pattern for THIS render's perception.

    cutd writes renders to <proj>/exports/ and their perception to
    <proj>/receipts/<render_id>.output.perception.json with asset_hash =
    sha256 of the output file — matching by hash is exact, not name-based.
    Searched: the render's dir, ./receipts, ../receipts.
    """
    rdir = os.path.dirname(os.path.abspath(render))
    candidates_dirs = [rdir, os.path.join(rdir, "receipts"),
                       os.path.join(os.path.dirname(rdir), "receipts")]
    for d in candidates_dirs:
        if not os.path.isdir(d):
            continue
        for name in sorted(os.listdir(d)):
            if not name.endswith(".perception.json"):
                continue
            path = os.path.join(d, name)
            try:
                p = judge.load_perception(path)
            except (ValueError, OSError, json.JSONDecodeError):
                continue  # not a valid perception report — skip, keep looking
            if p.get("asset_hash") == render_hash:
                return path
    return None


def generate_render_perception(render: str, render_hash: str,
                               out_path: str) -> str:
    """Run the app/perception sidecar on the render itself (receipts absent).

    Uses the instruments.py shipped beside the judge payload and the same
    interpreter cutd selected for this adapter.
    Raises RuntimeError on any failure (caller fails loud; never a fake).
    """
    script = os.path.join(_SIDECAR_DIR, "instruments.py")
    python = sys.executable
    if not os.path.exists(script):
        raise RuntimeError(f"sidecar not found at {script} — pass --perception")
    print(f"[cli_judge] no receipt perception for this render — running "
          f"instruments on it (may take minutes; whisperX included)",
          file=sys.stderr)
    cp = subprocess.run(
        [python, script, render, "--out", out_path,
         "--asset-id", "render", "--hash", render_hash],
        capture_output=True, text=True, timeout=SIDECAR_TIMEOUT_S)
    if cp.returncode != 0 or not os.path.exists(out_path):
        raise RuntimeError(
            f"perception sidecar failed (exit {cp.returncode}): "
            f"{cp.stderr[-600:]}")
    return out_path


def resolve_perception(render: str, explicit: str | None,
                       bundle: str) -> tuple[dict, dict, list[str]]:
    """Returns (perception, source_meta, warnings) for the judge bundle.

    Order: explicit --perception > receipts match by content hash > generate
    via sidecar. The source is RECORDED in the envelope so a reviewer can see
    which facts the judge reasoned from (provenance, judge honesty rules).
    """
    warnings: list[str] = []
    render_hash = sha256_file(render)
    if explicit:
        perception = judge.load_perception(explicit)
        meta = {"mode": "explicit", "path": os.path.abspath(explicit),
                "render_hash": render_hash,
                "hash_match": perception.get("asset_hash") == render_hash}
        if not meta["hash_match"]:
            warnings.append(
                "explicit --perception asset_hash does not match the render's "
                "content hash — verify these facts describe THIS render "
                "(timestamp sanity check still applies)")
        return perception, meta, warnings
    found = find_receipt_perception(render, render_hash)
    if found:
        return judge.load_perception(found), {
            "mode": "receipts", "path": os.path.abspath(found),
            "render_hash": render_hash, "hash_match": True}, warnings
    out_path = os.path.join(bundle, "render.perception.json")
    generate_render_perception(render, render_hash, out_path)
    return judge.load_perception(out_path), {
        "mode": "generated", "path": os.path.abspath(out_path),
        "render_hash": render_hash, "hash_match": True}, warnings

# ---------------------------------------------------------------------------
# Post-filter patterns (docs/public/JUDGE_REVIEW.md §7.1) — applied to model output, not input.
# ---------------------------------------------------------------------------

# Claims of having HEARD something. A vision-only (listened=false) judge
# asserting these is the exact failure observed live with minicpm-v4.5 —
# strip, never trust (docs/public/JUDGE_REVIEW.md §7.1.1).
#
# The pattern must cover whole classes of audio assertion: a
# visual_artifact issue with evidence "the audio track has a gap and the music
# drops out here" survived UNREDACTED. We broaden to the audio LEXICON a deaf
# judge must never invoke: named sound sources (music, voice, dialogue/dialog,
# audio track), heard absence (quiet/silence/silent as a property), and audio
# discontinuity verbs (drops out / cuts off / fades out / goes quiet). Care is
# taken NOT to over-redact a genuine visual finding about ON-SCREEN TEXT: we do
# NOT match bare "text"/"caption"/"subtitle" (those are visual), only
# audio-domain phrasing. "silent"/"silence" are audio here because a vision-only
# judge has no business asserting them; an on-screen "[silent]" caption would be
# quoted as caption text, not asserted as a heard property.
_AUDIO_CLAIM_RE = re.compile(
    r"(?:"
    r"\bi\s+(?:can\s+)?hear(?:d)?\b"
    r"|\baudibl\w+"
    r"|\bsound(?:s|ed)?\s+(?:like|of|is|was)\b"
    r"|\bclick(?:s|ing)?\b|\bpop(?:s|ping)?\b|\bhiss(?:es|ing)?"
    r"|\blisten(?:ed|ing)?\b"
    # Named audio sources a vision-only judge cannot perceive.
    r"|\bmusic\b|\bvoice(?:s|over)?\b|\bdialou?g(?:ue)?\b|\baudio\s+track\b"
    # Heard absence — quiet/silence/silent asserted as a property.
    r"|\bquiet\b|\bsilen(?:ce|t)\b"
    # Audio-discontinuity verbs (gap/dropout/fade described as heard).
    r"|\bdrops?\s+out\b|\bcuts?\s+off\b|\bfades?\s+out\b|\bgoes?\s+quiet\b"
    r")",
    re.IGNORECASE)

# Measurement-class numeric claims the judge must not make: loudness numbers
# and sub-second ms precision (its visual granularity is >= +/-500 ms at 1 fps).
_MEASUREMENT_RE = re.compile(
    r"-?\d+(?:\.\d+)?\s?(?:LUFS|dBTP|dB\b)", re.IGNORECASE)
_SUBSECOND_RE = re.compile(r"\b\d{1,3}\s?ms\b")

_REDACTION_TOKEN = "[measurement-class claim removed — instruments own this]"


def post_filter_review(review: dict, fps: float, duration_ms: int,
                       listened: bool) -> tuple[dict, dict]:
    """Consumer-side defense per docs/public/JUDGE_REVIEW.md §7.1. Returns (filtered, report).

    - listened=false: DROP issues with kind audio_artifact, and DROP issues
      whose evidence asserts hearing (dropped issues are preserved in the
      report for audit — never silently lost).
    - Redact measurement-class numbers (LUFS/dB, sub-second ms) from evidence
      and summary text — the judge quotes instruments, it never measures.
    - Quantize at_ms/end_ms to the sampling grid and clamp into the render;
      tag each issue with granularity_ms so consumers know the uncertainty.
    - NEVER flips the verdict (instruments and the receipt consumer own
      verdict arbitration); if filtering removed issues, that fact is in the
      report and the consumer can downgrade to needs_review.
    """
    report: dict = {"removed_issues": [], "redactions": 0,
                    "timestamps_quantized": 0}
    out = json.loads(json.dumps(review))  # deep copy; original kept raw
    grid_ms = 1000.0 / fps
    granularity_ms = int(grid_ms / 2)

    def scrub(text: str) -> str:
        nonlocal_report = []  # noqa: F841 — readability only
        n0 = len(_MEASUREMENT_RE.findall(text)) + len(_SUBSECOND_RE.findall(text))
        if n0:
            report["redactions"] += n0
            text = _MEASUREMENT_RE.sub(_REDACTION_TOKEN, text)
            text = _SUBSECOND_RE.sub(_REDACTION_TOKEN, text)
        return text

    kept = []
    for issue in out.get("issues", []):
        evidence = issue.get("evidence", "")
        if not listened and (issue.get("kind") == "audio_artifact"
                             or _AUDIO_CLAIM_RE.search(evidence)):
            issue["_removed_reason"] = ("listened=false — audio-perception "
                                        "claim from a deaf judge")
            report["removed_issues"].append(issue)
            continue
        issue["evidence"] = scrub(evidence)
        if issue.get("suggested_fix"):
            issue["suggested_fix"] = scrub(issue["suggested_fix"])
        # Timestamp honesty: snap to the frame grid, clamp into the render.
        for key in ("at_ms", "end_ms"):
            if isinstance(issue.get(key), (int, float)):
                q = int(round(issue[key] / grid_ms) * grid_ms)
                q = max(0, min(q, duration_ms))
                if q != issue[key]:
                    report["timestamps_quantized"] += 1
                issue[key] = q
        issue["granularity_ms"] = granularity_ms
        kept.append(issue)
    out["issues"] = kept
    out["summary"] = scrub(out.get("summary", ""))
    if not listened and _AUDIO_CLAIM_RE.search(out["summary"]):
        # Summary is prose, not droppable — flag it instead.
        report["summary_audio_claim"] = True
        out["summary"] += " [post-filter: summary contains an audio-perception" \
                          " claim from a vision-only judge — distrust it]"
    return out, report


# ---------------------------------------------------------------------------
# Frame-read-failure detector. A one-frame preflight cannot prove that every
# frame remains readable through a longer review. The post-run guard therefore
# reclassifies a model-reported all-frame EACCES/read failure as infrastructure,
# allowing auto mode to step down instead of recording a false completion.
# ---------------------------------------------------------------------------

# Read-failure signature: an EXPLICIT OS/tool error token, NOT generic prose.
# Examples: "ALL 20 frame files returned EACCES", "permission denied", and
# "Read tool failed". The pattern deliberately excludes generic "could not
# read" prose because a legitimate all-black `fail` verdict can use it.
# The old pattern also matched generic "could not read ... the file"
# and "frames unreadable" — but a LEGITIMATE all-black `fail` verdict says
# exactly that ("every frame solid black", "could not read the file name burned
# into any frame"), so it was wrongly reclassified completed->error and the
# ladder needlessly stepped down / hit all_rungs_failed. We now require a hard
# OS/tool error token (EACCES, permission denied, Read tool failure, cannot
# open, no such file) that a content finding about black/garbled pixels never
# contains. This is the LEXICAL half of the guard; the structural half below
# (empty issues + low confidence) is what makes a populated-issues fail verdict
# impossible to reclassify, regardless of phrasing.
_READ_BLOCKED_RE = re.compile(
    r"("
    r"\beacces\b"                              # the errno token itself
    r"|\bpermission denied\b"
    r"|\bread\s+tool\s+(?:returned|failed|denied|error)"  # the CLI tool by name
    r"|\bcannot\s+open\b"                      # OS open() failure phrasing
    r"|\bno\s+such\s+file\b"                   # ENOENT phrasing
    r"|\benoent\b"
    r")",
    re.IGNORECASE)

# "Did it fail on (nearly) ALL frames, not just one?" — the critical-fraction
# guard so a one-off unreadable frame inside an otherwise-fine review is NOT
# escalated to an infra failure. "all N frames", "every frame", "zero ... were
# readable", "no frames ... readable" all mean the whole sample was lost.
_ALL_FRAMES_RE = re.compile(
    r"\b(all\s+\d+\s+frame|all\s+(?:the\s+)?frames|every\s+frame"
    r"|zero\s+frames?\b|no\s+frames?\s+(?:were\s+)?readable"
    r"|none\s+of\s+the\s+frames?)\b",
    re.IGNORECASE)

# Structural read-failure signal: a real Read-blocked
# review raised NO visual issues (it saw nothing to report) and is honestly
# low-confidence. The expected shape is issues:[] with confidence around 0.1. The
# threshold is generous (≤0.15) but a populated issues[] — i.e. the judge DID
# raise a visual finding, like the black-frame `fail` — can NEVER satisfy this,
# so a genuine fail verdict is structurally immune to read-failure reclassification.
_STRUCTURAL_CONF_MAX = 0.15


def detect_frame_read_failure(review: dict, n_frames: int) -> tuple[bool, str | None]:
    """Post-run check: did the judge report it could not read the frames?

    Returns (is_infra_failure, cause). Reclassifies completed->error ONLY when
    BOTH of these hold:
      1. LEXICAL: an explicit OS/tool error token (EACCES / permission denied /
         "Read tool failed" / cannot open / no such file) appears in the review
         text — a hard infra signal, not generic "unreadable"/"could not read"
         prose that an all-black `fail` verdict legitimately uses.
      2. STRUCTURAL: the judge raised NO visual issues (issues == []) AND the
         verdict is low-confidence (confidence <= 0.15) — the "saw zero frames"
         shape. A populated issues[] fail verdict (the judge
         DID see and report pixels) cannot satisfy this, so it is never
         reclassified.
    Requiring BOTH makes escalation robust in both directions: a real EACCES
    sweep is caught (it has the token AND the empty-issues/low-conf shape), while
    a legitimate fail about black/garbled frames is immune (populated issues,
    high confidence, no OS token). Escalating a real review to infra-error would
    itself be a lie, so the guard stays conservative on purpose.

    Args:
        review: the model's (pre-filter) review dict.
        n_frames: how many frames the bundle actually sent (for the message).
    """
    if not review:
        return False, None
    # Gather every text surface the judge could have voiced the failure in.
    texts: list[str] = []
    summary = review.get("summary") or ""
    texts.append(summary)
    texts.extend(review.get("cannot_assess") or [])
    for iss in review.get("issues") or []:
        texts.append(iss.get("evidence") or "")
    blob = "\n".join(texts)
    # (1) LEXICAL: a hard OS/tool error token must be present.
    if not _READ_BLOCKED_RE.search(blob):
        return False, None
    # (2) STRUCTURAL: no visual issues raised AND low confidence — the "saw zero
    # frames" shape. A populated issues[] (a real visual finding) is structurally
    # immune; a high-confidence verdict is too. This is the guard that makes a
    # genuine `fail` on an all-black render impossible to reclassify.
    issues = review.get("issues") or []
    conf = review.get("confidence")
    structural = (not issues and isinstance(conf, (int, float))
                  and not isinstance(conf, bool)
                  and conf <= _STRUCTURAL_CONF_MAX)
    if not structural:
        return False, None
    # Read-blocked token + structural shape confirmed; also require the explicit
    # whole-sample qualifier ("all N frames" / "zero readable") so the message is
    # accurate (the structural empty-issues check already implies it, but the
    # quote below reads better when the text names the scope).
    if not _ALL_FRAMES_RE.search(blob):
        return False, None
    # Extract a short quote (the sentence carrying the signal) for the cause.
    m = _READ_BLOCKED_RE.search(summary) or _READ_BLOCKED_RE.search(blob)
    quote = ""
    if m:
        src = summary if _READ_BLOCKED_RE.search(summary) else blob
        start = src.rfind(".", 0, m.start()) + 1
        end = src.find(".", m.end())
        end = end if end != -1 else min(len(src), m.end() + 120)
        quote = src[start:end].strip()[:240]
    cause = (
        f"frame Read tool was BLOCKED on all/critical fraction of the "
        f"{n_frames} sampled frames — the judge saw zero (or near-zero) frames, "
        f"so this is an INFRASTRUCTURE failure, not a completed review. The "
        f"judge's own words: \"{quote}\". This is the CLI frame-read probe/the judge-status contract transient: "
        f"the claude CLI Read tool returns EACCES under parallel/nested claude "
        f"sessions mid-run, AFTER the 1-frame pre-flight probe passed. "
        f"Reclassified completed->error so the ladder can step down to a "
        f"working rung (or report an honest all-rungs-failed envelope).")
    return True, cause


# ---------------------------------------------------------------------------
# Prompt assembly — judge.py templates + a CLI-specific frame-file manifest.
# judge.build_prompts formats its own "frame N=ms" map (no paths); the CLI
# judge needs actual file paths it can Read, so we assemble here from the
# same imported templates. Templates stay single-sourced in judge.py.
# ---------------------------------------------------------------------------


def build_cli_prompts(mode: str, perception: dict, duration_s: float,
                      fps: float, intent: str, frames: list[dict],
                      width: int, window: tuple[int, int] | None,
                      window_reason: str) -> tuple[str, str]:
    """Returns (system_prompt, user_prompt) for the CLI judge call."""
    granularity_ms = int(1000.0 / fps / 2)
    sys_p = judge.SYSTEM_PROMPT.format(
        fps=fps, granularity_ms=granularity_ms,
        watched="true", listened="false")
    # CLI-context addendum: tells the model HOW it perceives in this harness.
    sys_p += (
        "\nCLI HARNESS CONTEXT: you run as a non-interactive subprocess with a"
        " Read tool. The frame JPEG files listed in the user message are your"
        " ONLY view of the video — Read EVERY one of them (batch multiple Read"
        " calls per message) before judging. You received NO audio stream;"
        " the transcript inside the instrument facts is your only knowledge"
        " of what was said. Do not read any other files.\n")

    rows = "\n".join(f"  {os.path.relpath(m['path'])} = {m['at_ms']} ms"
                     for m in frames)
    fm = (f"FRAME FILES ({len(frames)} frames sampled at {fps:.3f} fps, "
          f"{width}px wide, relative to your working directory):\n{rows}\n"
          "Read ALL of these frame files now, then judge.\n\n")

    if mode == "global":
        user_p = judge.GLOBAL_REVIEW_PROMPT.format(
            duration_s=duration_s,
            intent=intent,
            digest=judge.digest_perception(perception),
            frame_manifest=fm,
            audio_cut_clause="",  # deaf: no audio clauses, ever
            audio_question=("Audio: you did NOT receive audio. Add unanswered"
                            " audio questions to cannot_assess."),
        )
    else:
        assert window is not None
        user_p = judge.WINDOW_REVIEW_PROMPT.format(
            fps=fps, start_ms=window[0], end_ms=window[1],
            reason=window_reason or "(no reason recorded)",
            intent=intent,
            digest=judge.digest_perception(perception, window=window),
            frame_manifest=fm,
            listen_clause="",
            audio_artifact_clause="",
        )
    return sys_p, user_p


# ---------------------------------------------------------------------------
# Provider invocation — Claude. Other provider adapters keep the same envelope
# contract with provider-specific argv and output parsing.
# ---------------------------------------------------------------------------


def detect_providers() -> dict:
    """Detect the Claude CLI without a model call or quota use."""
    path = shutil.which("claude")
    entry: dict = {
        "binary": "claude",
        "found": bool(path),
        "path": path,
        "adapter": "implemented",
    }
    if path:
        try:
            cp = subprocess.run(
                [path, "--version"], capture_output=True, text=True, timeout=15
            )
            entry["version"] = cp.stdout.strip() or cp.stderr.strip()
        except (subprocess.TimeoutExpired, OSError) as e:
            entry["version_error"] = str(e)
    return {"claude": entry}


def preflight_read_probe(claude_bin: str, model: str, bundle: str,
                         frame_relpath: str, timeout_s: int = 120
                         ) -> tuple[bool, str]:
    """One cheap CLI call proving the Read tool can see a frame (the CLI frame-read probe).

    Same flag set, cwd and relative-path shape the full review will use —
    that is the point: it fails the way the review would fail, for one
    frame's cost instead of twenty. Returns (ok, reason). The caller treats
    a missing binary as "skip the probe" (invoke_claude owns that not_run).
    """
    path = shutil.which(claude_bin)
    if not path:
        return False, f"claude CLI not found ({claude_bin!r})"
    prompt = (
        f"Use the Read tool to read the file {frame_relpath} (an image). "
        "If the Read succeeds, reply with exactly READ_OK and nothing else. "
        "If the Read fails, reply with READ_FAIL: followed by the exact "
        "error message you received.")
    cmd = [
        path, "--safe-mode", "-p",
        "--output-format", "json",
        "--model", model,
        "--tools", "Read",
        "--no-session-persistence",
    ]
    try:
        cp = subprocess.run(cmd, input=prompt, capture_output=True, text=True,
                            cwd=bundle, timeout=timeout_s)
    except subprocess.TimeoutExpired:
        return False, f"probe exceeded {timeout_s}s timeout"
    if cp.returncode != 0:
        return False, f"probe CLI exit {cp.returncode}: {cp.stderr[-300:]}"
    try:
        env = json.loads(cp.stdout)
    except json.JSONDecodeError:
        return False, f"probe emitted non-JSON: {cp.stdout[-200:]}"
    if env.get("is_error"):
        return False, f"probe is_error: {str(env.get('result'))[:300]}"
    result = str(env.get("result") or "")
    if "READ_OK" in result:
        return True, "probe read one frame successfully"
    return False, f"probe could not read the frame: {result[:400]}"


def invoke_claude(claude_bin: str, sys_p: str, user_p: str, model: str,
                  cwd: str, timeout_s: int) -> tuple[dict | None, dict, str | None]:
    """Run one judge review through the claude CLI.

    Returns (review|None, cli_meta, not_run_or_error_reason|None).
    review None + reason => caller decides not_run vs error from cli_meta.

    Non-interactive Claude argv contract:
      --safe-mode            clean judge context (no CLAUDE.md/skills/hooks),
                             subscription OAuth intact
      --tools Read           the judge may ONLY read files (the frames)
      --json-schema          CLI-side structured-output enforcement; verdict
                             arrives in envelope.structured_output
      --no-session-persistence  judge calls never pollute resumable history
    Prompt goes via stdin (long prompts; argv stays clean).
    """
    path = shutil.which(claude_bin)
    if not path:
        return None, {"available": False}, (
            f"claude CLI not found ({claude_bin!r}) — honest not_run")
    cmd = [
        path, "--safe-mode", "-p",
        "--output-format", "json",
        "--json-schema", json.dumps(judge.REVIEW_SCHEMA),
        "--model", model,
        "--tools", "Read",
        "--system-prompt", sys_p,
        "--no-session-persistence",
    ]
    try:
        cp = subprocess.run(cmd, input=user_p, capture_output=True, text=True,
                            cwd=cwd, timeout=timeout_s)
    except subprocess.TimeoutExpired:
        return None, {"available": True, "timed_out": True}, (
            f"claude CLI exceeded {timeout_s}s timeout")
    if cp.returncode != 0:
        return None, {"available": True, "exit_code": cp.returncode}, (
            f"claude CLI exit {cp.returncode}: {cp.stderr[-800:]}")
    try:
        env = json.loads(cp.stdout)
    except json.JSONDecodeError:
        return None, {"available": True}, (
            f"claude CLI emitted non-JSON: {cp.stdout[-400:]}")
    meta = {
        "available": True,
        "envelope_type": env.get("type"),
        "is_error": env.get("is_error"),
        "duration_ms": env.get("duration_ms"),
        "duration_api_ms": env.get("duration_api_ms"),
        "num_turns": env.get("num_turns"),
        "usage": env.get("usage") and {
            k: env["usage"].get(k) for k in (
                "input_tokens", "cache_creation_input_tokens",
                "cache_read_input_tokens", "output_tokens")},
        # Subscription note: the CLI reports dollar accounting even on OAuth
        # quota — record it as accounting, not as money actually spent.
        "accounting_cost_usd": env.get("total_cost_usd"),
        "session_id": env.get("session_id"),
        "model": model,
    }
    if env.get("is_error"):
        return None, meta, f"claude CLI is_error: {str(env.get('result'))[:400]}"
    structured = env.get("structured_output")
    if structured is None:
        # Fallback: some failure modes leave the verdict in result text.
        try:
            structured = json.loads(env.get("result") or "")
        except (json.JSONDecodeError, TypeError):
            return None, meta, ("no structured_output in CLI envelope and "
                                "result text is not verdict JSON")
        meta["structured_output_fallback"] = True
    try:
        review = judge.validate_review(structured)
    except ValueError as e:
        return None, meta, f"verdict failed schema validation: {e}"
    return review, meta, None


# ---------------------------------------------------------------------------
# CLI entry
# ---------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["review", "detect"])
    ap.add_argument("--render")
    ap.add_argument("--perception",
                    help="instrument facts for THE RENDER (default: "
                         "auto-resolve by content hash from the receipts "
                         "dir, else generate via the app/perception sidecar)")
    ap.add_argument("--intent", default="(no edit intent provided)")
    ap.add_argument("--provider", default=DEFAULT_PROVIDER,
                    choices=["claude"],
                    help="which subscription CLI judges")
    ap.add_argument("--cli-model", default=DEFAULT_CLI_MODEL,
                    help="model alias passed to the CLI (default: sonnet)")
    ap.add_argument("--claude-bin", default="claude")
    ap.add_argument("--mode", default="global", choices=["global", "window"])
    ap.add_argument("--windows", help="start:end ms span (window mode)")
    ap.add_argument("--window-reason", default="flagged by instrument checks")
    ap.add_argument("--fps", type=float, default=None,
                    help="requested sampling fps (default 1 global, 5 window);"
                         " auto-reduced if it would exceed --max-frames")
    ap.add_argument("--max-frames", type=int, default=DEFAULT_MAX_FRAMES)
    ap.add_argument("--width", type=int, default=DEFAULT_FRAME_WIDTH)
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_S)
    ap.add_argument("--out", help="write envelope JSON here")
    ap.add_argument("--bundle-dir", help="workspace for frames (default: tmp)")
    ap.add_argument("--keep-bundle", action="store_true",
                    help="keep the frames workspace after the run")
    args = ap.parse_args()

    if args.command == "detect":
        print(json.dumps(detect_providers(), indent=2))
        return 0

    if not args.render:
        print("review requires --render", file=sys.stderr)
        return 2
    if not os.path.exists(args.render):
        print(f"render not found: {args.render}", file=sys.stderr)
        return 2
    duration_s = judge.probe_duration_s(args.render)
    duration_ms = int(duration_s * 1000)

    # Bundle = the CLI subprocess's whole world: cwd containing only frames
    # (+ generated perception). Created BEFORE perception resolution because
    # sidecar-generated reports land inside it. Anchored under the CALLER's
    # cwd, not /tmp: the claude CLI sandbox denies /tmp reads even when the
    # subprocess cwd IS the bundle (all 20 frames EACCES, observed live
    # Keep the bundle project-local so sandboxed CLIs can read the frames.
    bundle = args.bundle_dir or tempfile.mkdtemp(prefix="cli_judge_", dir=os.getcwd())
    os.makedirs(bundle, exist_ok=True)

    # Perception facts for THE RENDER (the coordinate-space guard): resolve, then refuse any
    # coordinate-space mismatch LOUDLY before frames/quota are spent.
    try:
        perception, perception_source, warnings = resolve_perception(
            args.render, args.perception, bundle)
    except (ValueError, RuntimeError, OSError) as e:
        print(f"perception resolution failed: {e}", file=sys.stderr)
        return 2
    try:
        judge.sanity_check_perception(perception, duration_ms)
    except ValueError as e:
        print(f"perception sanity check FAILED "
              f"(source: {perception_source['mode']} "
              f"{perception_source['path']}): {e}", file=sys.stderr)
        return 2

    window: tuple[int, int] | None = None
    if args.mode == "window":
        if not args.windows:
            print("--mode window requires --windows start:end", file=sys.stderr)
            return 2
        a, b = args.windows.split(":")
        window = (int(a), int(b))
    span_s = ((window[1] - window[0]) / 1000.0) if window else duration_s

    # Effective fps: spread max_frames EVENLY across the span rather than
    # sampling dense and dropping the tail (a blind tail is worse than a
    # slightly coarser grid). Honest limit recorded in the envelope.
    fps_req = args.fps or (DEFAULT_WINDOW_FPS if window else DEFAULT_GLOBAL_FPS)
    fps_eff = fps_req
    if math.ceil(span_s * fps_req) > args.max_frames:
        fps_eff = args.max_frames / span_s
        # Confidence cap: the cap is a real perceptual limit — surface
        # it in-band, not just as a backend.fps number nobody compares.
        warnings.append(
            f"requested {fps_req} fps exceeds the {args.max_frames}-frame cap "
            f"over {span_s:.1f}s — sampling degraded to {fps_eff:.3f} fps "
            f"(±{int(1000.0 / fps_eff / 2)} ms visual granularity); brief "
            "glitches between samples are unobservable at this rate")

    frames_dir = os.path.join(bundle, "frames")
    frames = judge.extract_frames(
        args.render, frames_dir, fps_eff,
        start_ms=window[0] if window else None,
        end_ms=window[1] if window else None,
        max_frames=args.max_frames, width=args.width)
    # Manifest paths must be relative to the bundle (the subprocess cwd).
    for m in frames:
        m["path"] = os.path.relpath(m["path"], bundle)

    # Bundles WE created (cleaned at the end unless kept). A caller-supplied
    # --bundle-dir is the caller's to keep; retry bundles are always ours.
    cleanup_bundles = [] if args.bundle_dir else [bundle]

    # Pre-flight Read probe (the CLI frame-read probe — see module docstring): prove the CLI
    # can Read ONE frame with the exact flags/cwd/relative path the review
    # will use, retry once in a fresh bundle, give up honestly otherwise —
    # all BEFORE the 20-frame prompt burns quota. Skipped when the binary is
    # missing (invoke_claude owns that not_run path).
    preflight: dict | None = None
    skip_reason: str | None = None
    probe_timeout = min(120, args.timeout)
    if frames and shutil.which(args.claude_bin):
        ok1, why1 = preflight_read_probe(
            args.claude_bin, args.cli_model, bundle, frames[0]["path"],
            probe_timeout)
        attempts = [{"bundle": bundle, "ok": ok1, "reason": why1}]
        if not ok1:
            print(f"[cli_judge] pre-flight Read probe failed ({why1}) — "
                  "retrying once in a fresh bundle dir", file=sys.stderr)
            retry_bundle = tempfile.mkdtemp(
                prefix="cli_judge_retry_",
                dir=os.path.dirname(os.path.abspath(bundle)),
            )
            cleanup_bundles.append(retry_bundle)
            shutil.copytree(os.path.join(bundle, "frames"),
                            os.path.join(retry_bundle, "frames"))
            ok2, why2 = preflight_read_probe(
                args.claude_bin, args.cli_model, retry_bundle,
                frames[0]["path"], probe_timeout)
            attempts.append({"bundle": retry_bundle, "ok": ok2, "reason": why2})
            if ok2:
                bundle = retry_bundle  # review where Reads provably work
            else:
                skip_reason = (
                    "pre-flight Read probe failed twice (original + fresh "
                    "bundle dir) — full review NOT attempted, 20-frame "
                    f"prompt quota not burned. [1] {why1} [2] {why2}. "
                    "Known cause: transient EACCES under parallel claude "
                    "sessions (config-lock contention) — retry "
                    "when other sessions are idle.")
        preflight = {"attempts": attempts}

    prev_cwd = os.getcwd()
    os.chdir(bundle)  # so relpath in prompt rows == what the model Reads
    try:
        sys_p, user_p = build_cli_prompts(
            args.mode, perception, duration_s, fps_eff, args.intent,
            frames, args.width, window, args.window_reason)
    finally:
        os.chdir(prev_cwd)

    if skip_reason is not None:
        review_raw, cli_meta, reason = None, {
            "available": True, "preflight_failed": True}, skip_reason
    else:
        review_raw, cli_meta, reason = invoke_claude(
            args.claude_bin, sys_p, user_p, args.cli_model, bundle,
            args.timeout)

    # error_class distinguishes INFRASTRUCTURE failures (adapter crash, CLI
    # absent at run-time, blocked file reads — the environment failed) from any
    # future model/content error. The ladder (ladder_judge.py) steps down ONLY
    # on infrastructure-class errors. Today every status="error" this adapter
    # emits IS infrastructure-class (a model verdict is always "completed"), so
    # we tag it explicitly rather than leaving the ladder to infer it.
    error_class: str | None = None
    if review_raw is not None:
        review, pf_report = post_filter_review(
            review_raw, fps_eff, duration_ms, listened=False)
        # the judge-status contract RESULT-CLASS honesty: the CLI returned a schema-valid verdict,
        # but if the judge's own review text says it could not read all/critical
        # frames (EACCES under nested claude sessions, AFTER the probe passed),
        # this was NOT a completed review — it failed on infrastructure. Run the
        # check on the PRE-filter review (the post-filter redacts measurement
        # numbers but leaves the read-failure language intact).
        infra, infra_cause = detect_frame_read_failure(review_raw, len(frames))
        if infra:
            status = "error"
            error_class = "infrastructure"
            # Preserve the (honest, low-confidence) review for audit, but the
            # STATUS now tells the truth: the engine treats error as job-fail and
            # the ladder steps down. Naming the probe history in the reason.
            probe_trail = ""
            if preflight and preflight.get("attempts"):
                probe_trail = " Pre-flight probe history: " + "; ".join(
                    f"[{i + 1}] ok={a['ok']} {a['reason']}"
                    for i, a in enumerate(preflight["attempts"]))
            reason = infra_cause + probe_trail
        else:
            status = "completed"
    else:
        # A missing binary is an honest not_run: this rung was unavailable.
        # Once a present CLI has been invoked, however, any failure is an
        # infrastructure error. That includes the two-attempt Read preflight:
        # auto mode must be able to step down to Codex/Antigravity/Grok instead
        # of terminating on a detected-but-unusable Claude installation.
        if not cli_meta.get("available"):
            status = "not_run"
        else:
            status = "error"
            error_class = "infrastructure"  # probe/run failure, timeout, output
        review, pf_report = None, None

    envelope = {
        "schema": judge.SCHEMA,
        "ts": judge.now_iso(),
        "render": os.path.abspath(args.render),
        "mode": args.mode,
        "backend": {
            "name": ADAPTER_NAME,
            "provider": args.provider,
            "model": f"{args.provider}/{args.cli_model}",
            "fps": round(fps_eff, 4),
            "fps_requested": fps_req,
            "resolution": f"frames {args.width}px JPEG",
            "watched": True,
            "listened": False,   # invariant of this adapter class
            "frames_sent": len(frames),
        },
        "window": ({"start_ms": window[0], "end_ms": window[1],
                    "reason": args.window_reason} if window else None),
        "status": status,
        # When status=="error": "infrastructure" (adapter/CLI/blocked-reads) —
        # the ladder steps down on this class. None otherwise. (the judge-status contract)
        "error_class": error_class,
        "not_run_reason": reason,
        "review": review,             # post-filtered — the consumable verdict
        "review_raw": review_raw,     # pre-filter, for audit
        "post_filter": pf_report,
        # Provenance of the instrument facts the judge reasoned from
        # (the coordinate-space guard): explicit | receipts (hash-matched) | generated.
        "perception_source": perception_source,
        "warnings": warnings,
        "cli": cli_meta,
        # Pre-flight Read probe attempts (the CLI frame-read probe); None = probe skipped
        # (no frames, or binary missing — invoke owns that not_run).
        "preflight": preflight,
        "prompt_chars": {"system": len(sys_p), "user": len(user_p)},
        "bundle_dir": bundle if args.keep_bundle else None,
    }
    text = json.dumps(envelope, indent=2)
    if args.out:
        with open(args.out, "w") as f:
            f.write(text + "\n")
    print(text)
    kept_bundle = os.path.abspath(bundle) if args.keep_bundle else None
    for b in cleanup_bundles:
        if kept_bundle and os.path.abspath(b) == kept_bundle:
            continue
        shutil.rmtree(b, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
