#!/usr/bin/env python3
"""Antigravity CLI render judge adapter.

Runs the agy subscription CLI against sampled render frames and measured facts,
recovers a structured visual verdict, applies the shared honesty filter, and
emits shellx-cut/judge-review/1. No API key is required.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile

# Single source of truth for prompts/schema/digest lives in ../judge.py.
_JUDGE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _JUDGE_DIR)
import judge  # noqa: E402

# Shared pipeline (perception resolution by content hash, the post-filter,
# frame-extraction defaults) lives in cli_judge.py; codex_judge.py owns the
# fence/prose-tolerant JSON recovery. Import both so all adapters share ONE
# implementation of the contract — the only per-provider code is invoke_* +
# frame manifest.
_ADAPTERS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _ADAPTERS_DIR)
import cli_judge  # noqa: E402
import codex_judge  # noqa: E402  (_extract_json — one shared implementation)

ADAPTER_NAME = "cli"                 # same adapter CLASS as claude/codex (CLI judge)
DEFAULT_PROVIDER = "antigravity"
# Omit --model by default so agy uses the account's current vision-capable
# default. An explicit --cli-model passes a provider label through verbatim.
DEFAULT_CLI_MODEL = ""
DEFAULT_GLOBAL_FPS = cli_judge.DEFAULT_GLOBAL_FPS
DEFAULT_WINDOW_FPS = cli_judge.DEFAULT_WINDOW_FPS
DEFAULT_MAX_FRAMES = cli_judge.DEFAULT_MAX_FRAMES
DEFAULT_FRAME_WIDTH = cli_judge.DEFAULT_FRAME_WIDTH
DEFAULT_TIMEOUT_S = cli_judge.DEFAULT_TIMEOUT_S


# ---------------------------------------------------------------------------
# Explicit in-prompt schema — agy has NO model-side schema enforcement
# (no --output-schema like codex, no response_schema like gemini-api, no
# Ollama `format`), so the verdict structure must be spelled out IN THE PROMPT
# or the model may invent its own field names. We derive the block from
# judge.REVIEW_SCHEMA / judge.ISSUE_KINDS so it cannot drift from
# the single source of truth (the schema). Field names + enum values are
# stated EXACTLY because that is the only enforcement this rung has.
# ---------------------------------------------------------------------------


def _build_schema_block() -> str:
    """Render judge.REVIEW_SCHEMA into an explicit, copy-this prompt block.

    agy (plain-text, no schema flag) needs the literal field names and enum
    values; we generate them from the schema so a future schema change updates
    this block automatically. Returns a string appended to the user prompt.
    """
    kinds = " | ".join(judge.ISSUE_KINDS)
    return (
        "\nRespond with ONLY ONE JSON OBJECT — no prose, no markdown fences, no "
        "text before or after it. Do not run shell commands or edit files.\n"
        "The JSON MUST use these EXACT field names and allowed values "
        "(do NOT rename fields, do NOT invent values):\n"
        "{\n"
        '  "verdict": "pass" | "fail" | "needs_review",\n'
        '  "issues": [                       // [] if the render is clean\n'
        '    {\n'
        '      "at_ms": <integer ms>,        // REQUIRED — absolute ms from render start\n'
        '      "end_ms": <integer ms>,       // optional\n'
        f'      "kind": "{kinds}",  // REQUIRED — pick EXACTLY one of these\n'
        '      "severity": "blocker" | "major" | "minor",  // REQUIRED — exactly one\n'
        '      "evidence": "<what you actually SAW in the frames at this point>", // REQUIRED\n'
        '      "suggested_fix": "<optional>"\n'
        '    }\n'
        '  ],\n'
        '  "cannot_assess": ["<things you could not judge, e.g. audio>"],\n'
        '  "confidence": <number 0.0-1.0>,   // REQUIRED\n'
        '  "summary": "<one paragraph>"      // REQUIRED\n'
        "}\n"
        "Use the field name \"kind\" (NOT \"type\"), \"evidence\" (NOT "
        "\"description\"), and a severity of exactly blocker/major/minor "
        "(NOT \"error\"/\"warning\").\n")


EXPLICIT_SCHEMA_BLOCK = _build_schema_block()


# ---------------------------------------------------------------------------
# Detection — agy binary presence (the ladder's skip gate). Detection-only, no
# model call, no quota burned. A present-but-logged-out agy reports found=true;
# invoke time turns a login failure (empty/failed run) into an honest error.
# ---------------------------------------------------------------------------


def detect() -> dict:
    """Is the `agy` CLI present? `found` gates the ladder.

    Antigravity exposes no machine-readable auth-status command we can trust;
    a 401-style failure surfaces as a failed launch instead. So we
    record found + version only; `logged_in` is left None (unknown) — the
    invoke path decides honestly (a failed/empty run becomes status error). This
    keeps detection cheap and side-effect free, exactly what the ladder needs.
    """
    path = shutil.which("agy")
    entry: dict = {"provider": "antigravity", "binary": "agy",
                   "found": bool(path), "path": path,
                   "adapter": "implemented (vision)"}
    if path:
        try:
            cp = subprocess.run([path, "--version"], capture_output=True,
                                text=True, encoding="utf-8", timeout=15)
            entry["version"] = cp.stdout.strip() or cp.stderr.strip()
        except (subprocess.TimeoutExpired, OSError) as e:
            entry["version_error"] = str(e)
        # No trustworthy auth-status command (antigravity.rs lesson) — auth is
        # unknown until a real run. The ladder only needs `found` to select;
        # invoke turns any auth failure into an honest error, never a fake pass.
        entry["logged_in"] = None
    return entry


# ---------------------------------------------------------------------------
# Prompt assembly — judge.py templates + an Antigravity frame manifest. agy
# reads the frames as FILES from its workspace (--add-dir) referenced by
# ABSOLUTE PATH inside the prompt text (no -i flag like codex, no @ syntax like
# gemini — plain absolute paths, which the probe proved agy's file tools open
# and pass to vision). The manifest is the same "frame N = ms" time map
# judge.build_prompts emits for frame backends.
# ---------------------------------------------------------------------------


def build_antigravity_prompts(mode: str, perception: dict, duration_s: float,
                              fps: float, intent: str, frames: list[dict],
                              width: int, window: tuple[int, int] | None,
                              window_reason: str,
                              frame_abs_paths: list[str]) -> tuple[str, str]:
    """Returns (system_prompt, user_prompt) for the agy judge call.

    Reuses judge.build_prompts (single-sourced templates, frame-time-map form)
    and appends an Antigravity-harness note so the model knows HOW it perceives
    here: it must OPEN and VIEW each listed image file (its only view of the
    video), it received NO audio, and it must answer with ONLY the verdict JSON
    (agy emits a final plain-text message — there is no schema flag, so the
    JSON-only instruction is the ONLY enforcement, recovered defensively after).
    """
    sys_p, user_p = judge.build_prompts(
        mode=mode, perception=perception, duration_s=duration_s, fps=fps,
        intent=intent, listened=False, watched=True,
        window=window, window_reason=window_reason or "(no reason recorded)",
        frame_manifest=frames)
    # Explicit absolute-path list (matching the FRAME TIME MAP order) so agy's
    # file tools open each frame. The probe proved agy reads images referenced
    # by absolute path when the dir is in --add-dir scope.
    refs = "\n".join(f"  frame {i + 1} ({frames[i]['at_ms']}ms): {p}"
                     for i, p in enumerate(frame_abs_paths))
    user_p += (
        "\n\nThe following frame images are in your workspace (added via "
        "--add-dir). OPEN AND VIEW every one IN ORDER (the order matches the "
        f"FRAME TIME MAP above) before judging:\n{refs}\n"
        + EXPLICIT_SCHEMA_BLOCK)
    sys_p += (
        "\nANTIGRAVITY HARNESS CONTEXT: you run as a non-interactive `agy "
        f"--print` subprocess. The {len(frames)} JPEG frames listed in the task "
        f"(sampled at {fps:.3f} fps, {width}px wide, in FRAME TIME MAP order) "
        "are your ONLY view of the video — open each file and look at the "
        "pixels. You received NO audio stream; the transcript inside the "
        "instrument facts is your only knowledge of what was said. Do not run "
        "shell commands, edit files, or browse. Output ONLY the verdict JSON "
        "object.\n")
    return sys_p, user_p


# ---------------------------------------------------------------------------
# Invocation — Antigravity `agy --print` (plain-text response).
# ---------------------------------------------------------------------------


def invoke_antigravity(agy_bin: str, sys_p: str, user_p: str, model: str,
                       cwd: str, timeout_s: int
                       ) -> tuple[dict | None, dict, str | None]:
    """Run one judge review through the `agy` CLI (non-interactive print mode).

    Returns (review|None, cli_meta, error_or_not_run_reason|None) — the SAME
    triple shape as cli_judge.invoke_claude / codex_judge.invoke_codex, so the
    envelope assembly is shared.

    Non-interactive Antigravity argv contract:
      --sandbox            terminal-restricted context (read-only-ish); does NOT
                           break image reads (verified). We use it INSTEAD of
                           --dangerously-skip-permissions: the judge only reads
                           frames and emits text, so the auto-approve flag is
                           unnecessary and is intentionally NOT passed.
      --add-dir <bundle>   put the frame files in the agent's workspace so its
                           file tools can open them (the frames live under the
                           bundle; we pass the bundle root).
      --log-file <path>    redirect agy's noisy glog server diagnostics to a
                           throwaway file inside the bundle so stdout stays the
                           CLEAN response channel (the log has NO response text —
                           it is a cleanliness measure, not a recovery fallback).
      --model <label>      OPTIONAL human model label (omitted by default ->
                           account default, which is vision-capable).
      --print "<prompt>"   non-interactive; the prompt is the flag's VALUE and
                           MUST come LAST (a trailing flag is swallowed as the
                           prompt — antigravity.rs lesson).
    There is NO --system-prompt flag, so the system rules are PREPENDED to the
    user prompt (clearly delimited), as in the other CLI adapters.

    Non-TTY mitigation: `agy --print` may drop stdout under a
    non-TTY. It did NOT reproduce on 1.0.5 here, and the log file cannot recover
    a dropped response, so if stdout is EMPTY we return an honest error naming
    the issue — never a fabricated verdict.
    """
    path = shutil.which(agy_bin)
    if not path:
        return None, {"available": False}, (
            f"agy CLI not found ({agy_bin!r}) — honest not_run")

    # agy has no --system-prompt; fold the system rules into the prompt value.
    full_prompt = (
        "SYSTEM INSTRUCTIONS (follow exactly):\n" + sys_p +
        "\n\n----- TASK -----\n" + user_p)

    cwd_abs = os.path.abspath(cwd)
    log_path = os.path.join(cwd_abs, "_agy.log")
    cmd = [
        path,
        "--sandbox",                 # read-restricted; does NOT block image reads
        "--add-dir", cwd_abs,        # frames in workspace scope
        "--log-file", log_path,      # noisy glog -> file, keep stdout clean
    ]
    if model:
        cmd += ["--model", model]    # human label, e.g. "Gemini 3.5 Flash (Low)"
    # --print MUST be last and its prompt is the flag VALUE (antigravity.rs):
    cmd += ["--print", full_prompt]

    try:
        cp = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", cwd=cwd_abs,
                            timeout=timeout_s)
    except subprocess.TimeoutExpired:
        return None, {"available": True, "timed_out": True}, (
            f"agy CLI exceeded {timeout_s}s timeout")

    meta = {
        "available": True,
        "exit_code": cp.returncode,
        "model": model or "(agy account default)",
        "schema_enforced": False,    # agy has no --output-schema/-format flag
    }

    # Non-zero exit is an honest error (auth failure surfaces here — agy has no
    # auth-status command, so a logged-out run fails the launch / prints nothing).
    if cp.returncode != 0:
        return None, meta, (
            f"agy CLI exit {cp.returncode}: {cp.stderr.strip()[-600:] or '(no stderr)'}")

    raw = (cp.stdout or "").strip()
    if not raw:
        # Empty stdout under non-TTY is the known failure mode
        # (or a silent auth/login failure). We cannot recover the response from
        # the log file, so this is an honest error — NEVER a fabricated verdict.
        return None, meta, (
            "agy --print returned EMPTY stdout (exit 0). Likely Antigravity CLI "
            "stdout was silently dropped under a non-TTY pipe/subprocess "
            "OR a silent auth failure. Cannot recover from the log file (it "
            "carries no response text). stderr tail: "
            f"{cp.stderr.strip()[-300:] or '(none)'}")

    # agy returns PLAIN TEXT — no schema flag. The prompt demands a bare JSON
    # object; recover it defensively (tolerating stray ``` fences / a prose
    # preamble) via codex_judge._extract_json — ONE shared implementation.
    structured = codex_judge._extract_json(raw)
    if structured is None:
        return None, meta, f"agy response is not verdict JSON: {raw[:400]}"
    try:
        review = judge.validate_review(structured)
    except ValueError as e:
        return None, meta, f"agy verdict failed schema validation: {e}"
    return review, meta, None


# ---------------------------------------------------------------------------
# CLI entry — mirrors codex_judge.py's structure (shared resolution + filter +
# envelope), differing only in the provider invocation (frames via --add-dir +
# absolute-path refs in the prompt, plain-text response).
# ---------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["review", "detect"])
    ap.add_argument("--render")
    ap.add_argument("--perception",
                    help="instrument facts for THE RENDER (default: "
                         "auto-resolve by content hash from the receipts dir, "
                         "else generate via the app/perception sidecar)")
    ap.add_argument("--intent", default="(no edit intent provided)")
    ap.add_argument("--cli-model", default=DEFAULT_CLI_MODEL,
                    help="agy model LABEL (e.g. 'Gemini 3.5 Flash (Low)'); "
                         "default: omit --model, use the agy account default")
    ap.add_argument("--agy-bin", default="agy")
    ap.add_argument("--mode", default="global", choices=["global", "window"])
    ap.add_argument("--windows", help="start:end ms span (window mode)")
    ap.add_argument("--window-reason", default="flagged by instrument checks")
    ap.add_argument("--fps", type=float, default=None,
                    help="requested sampling fps (default 1 global, 5 window); "
                         "auto-reduced if it would exceed --max-frames")
    ap.add_argument("--max-frames", type=int, default=DEFAULT_MAX_FRAMES)
    ap.add_argument("--width", type=int, default=DEFAULT_FRAME_WIDTH)
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_S)
    ap.add_argument("--out", help="write envelope JSON here")
    ap.add_argument("--bundle-dir", help="workspace for frames (default: tmp)")
    ap.add_argument("--keep-bundle", action="store_true",
                    help="keep the frames workspace after the run")
    args = ap.parse_args()

    if args.command == "detect":
        print(json.dumps(detect(), indent=2))
        return 0

    if not args.render:
        print("review requires --render", file=sys.stderr)
        return 2
    if not os.path.exists(args.render):
        print(f"render not found: {args.render}", file=sys.stderr)
        return 2
    duration_s = judge.probe_duration_s(args.render)
    duration_ms = int(duration_s * 1000)

    # Bundle = the agy subprocess's working root AND --add-dir scope. Project-
    # local, not /tmp (same reasoning as the other adapters: keep ONE bundle
    # convention so the Rust wire-in passes the same directory shape).
    bundle = args.bundle_dir or tempfile.mkdtemp(prefix="antigravity_judge_",
                                                 dir=os.getcwd())
    os.makedirs(bundle, exist_ok=True)

    # Perception facts for THE RENDER (the coordinate-space guard) — shared resolver, then the
    # coordinate-space sanity gate, BEFORE any frames or quota are spent.
    try:
        perception, perception_source, warnings = cli_judge.resolve_perception(
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

    # Effective fps: spread max_frames EVENLY over the span (no blind tail) —
    # identical policy to the other adapters so reviews are comparable.
    fps_req = args.fps or (DEFAULT_WINDOW_FPS if window else DEFAULT_GLOBAL_FPS)
    fps_eff = fps_req
    if math.ceil(span_s * fps_req) > args.max_frames:
        fps_eff = args.max_frames / span_s
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
    # agy opens frames from its --add-dir workspace via ABSOLUTE paths in the
    # prompt (the probe proved absolute-path file refs are read by its vision).
    frame_abs_paths = [os.path.abspath(m["path"]) for m in frames]

    sys_p, user_p = build_antigravity_prompts(
        args.mode, perception, duration_s, fps_eff, args.intent,
        frames, args.width, window, args.window_reason, frame_abs_paths)

    if not frames:
        # No frames extracted (e.g. zero-duration render) — honest not_run, no
        # call made (consistent with the ladder's "nothing to judge" path).
        review_raw, cli_meta, reason = None, {"available": True}, (
            "no frames could be extracted from the render — nothing to judge")
    else:
        review_raw, cli_meta, reason = invoke_antigravity(
            args.agy_bin, sys_p, user_p, args.cli_model, bundle, args.timeout)

    if review_raw is not None:
        status = "completed"
        review, pf_report = cli_judge.post_filter_review(
            review_raw, fps_eff, duration_ms, listened=False)
    else:
        # binary missing / no frames => not_run; model attempted+failed => error.
        status = "not_run" if not cli_meta.get("available") or not frames \
            else "error"
        review, pf_report = None, None

    envelope = {
        "schema": judge.SCHEMA,
        "ts": judge.now_iso(),
        "render": os.path.abspath(args.render),
        "mode": args.mode,
        "backend": {
            "name": ADAPTER_NAME,
            "provider": DEFAULT_PROVIDER,
            "model": f"{DEFAULT_PROVIDER}/{args.cli_model or 'default'}",
            "fps": round(fps_eff, 4),
            "fps_requested": fps_req,
            "resolution": f"frames {args.width}px JPEG (--add-dir + path refs)",
            "watched": True,
            "listened": False,    # invariant of this adapter class
            "frames_sent": len(frames),
        },
        "window": ({"start_ms": window[0], "end_ms": window[1],
                    "reason": args.window_reason} if window else None),
        "status": status,
        "not_run_reason": reason,
        "review": review,            # post-filtered — the consumable verdict
        "review_raw": review_raw,    # pre-filter, for audit
        "post_filter": pf_report,
        "perception_source": perception_source,
        "warnings": warnings,
        "cli": cli_meta,
        "prompt_chars": {"system": len(sys_p), "user": len(user_p)},
        "bundle_dir": bundle if args.keep_bundle else None,
    }
    text = json.dumps(envelope, indent=2)
    if args.out:
        with open(args.out, "w") as f:
            f.write(text + "\n")
    print(text)
    if not args.keep_bundle and not args.bundle_dir:
        shutil.rmtree(bundle, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
