#!/usr/bin/env python3
"""Grok Build CLI render judge adapter.

Sends sampled frames as ACP image content blocks through Grok Build, disables
web search and unrelated tools, validates the returned visual verdict, applies
the shared honesty filter, and emits shellx-cut/judge-review/1.
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

# Single source of truth for prompts/schema/digest lives in ../judge.py.
_JUDGE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _JUDGE_DIR)
import judge  # noqa: E402

# Shared pipeline (perception resolution by content hash, the post-filter,
# frame-extraction defaults) lives in cli_judge.py; codex_judge.py owns the
# fence/prose-tolerant JSON recovery. Import both so all adapters share ONE
# implementation of the contract — the only per-provider code is invoke_* +
# the content-block assembly.
_ADAPTERS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _ADAPTERS_DIR)
import cli_judge  # noqa: E402
import codex_judge  # noqa: E402  (_extract_json — one shared implementation)

ADAPTER_NAME = "cli"                 # same adapter CLASS as claude/codex/agy (CLI judge)
DEFAULT_PROVIDER = "grok"
# Omit --model by default so Grok uses the account's current vision-capable
# default. An explicit --cli-model passes a model id through verbatim.
DEFAULT_CLI_MODEL = ""
DEFAULT_GLOBAL_FPS = cli_judge.DEFAULT_GLOBAL_FPS
DEFAULT_WINDOW_FPS = cli_judge.DEFAULT_WINDOW_FPS
DEFAULT_MAX_FRAMES = cli_judge.DEFAULT_MAX_FRAMES
DEFAULT_FRAME_WIDTH = cli_judge.DEFAULT_FRAME_WIDTH
DEFAULT_TIMEOUT_S = cli_judge.DEFAULT_TIMEOUT_S

# Tool grok must NOT attempt: its native vision MCP tool, which is rejected in
# the stdio-standalone subprocess (no shellX HTTP MCP transport). Disabling it
# routes grok straight to reading the inline --prompt-json image block (which
# works), avoiding an unsupported tool round-trip.
_DISALLOWED_TOOLS = "vision_describe"


# ---------------------------------------------------------------------------
# Explicit in-prompt schema — grok has NO model-side schema enforcement
# (--output-format json shapes only the OUTER envelope, not the answer; there is
# no --output-schema like codex, no response_schema like gemini-api). So the
# verdict structure must be spelled out IN THE PROMPT or the model invents its
# own field names. We DERIVE the block from judge.REVIEW_SCHEMA / judge.ISSUE_KINDS
# so it can never drift from the single source of truth (the schema). This
# matches the defensive posture of the other plain-text adapters.
# ---------------------------------------------------------------------------


def _build_schema_block() -> str:
    """Render judge.REVIEW_SCHEMA into an explicit, copy-this prompt block.

    grok (plain-text answer inside .text, no schema flag) needs the literal
    field names and enum values; we generate them from the schema so a future
    schema change updates this block automatically. Returns a string appended to
    the user prompt.
    """
    kinds = " | ".join(judge.ISSUE_KINDS)
    return (
        "\nRespond with ONLY ONE JSON OBJECT — no prose, no markdown fences, no "
        "text before or after it. Do not run shell commands, browse the web, or "
        "call tools.\n"
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
# Detection — grok binary presence (the ladder's skip gate). Detection-only, no
# model call. A present-but-logged-out grok reports found=true;
# invoke time turns a login/auth failure into an honest error.
# ---------------------------------------------------------------------------


def detect() -> dict:
    """Is the `grok` CLI present? `found` gates the ladder.

    Records found + version. `logged_in` is a BEST-EFFORT, read-only signal:
    grok caches credentials under ~/.grok/ after `grok login`, so we report
    whether that dir holds any credential-looking file — WITHOUT reading or
    echoing token material. It is left None when we cannot tell; the invoke path
    decides honestly (a failed/empty turn becomes status error). `logged_in` is
    advisory only; `found` is what the ladder selects on. Detection stays cheap
    and side-effect free.
    """
    path = shutil.which("grok")
    entry: dict = {"provider": "grok", "binary": "grok",
                   "found": bool(path), "path": path,
                   "adapter": "implemented (vision)"}
    if path:
        try:
            cp = subprocess.run([path, "--version"], capture_output=True,
                                text=True, encoding="utf-8", timeout=15)
            entry["version"] = cp.stdout.strip() or cp.stderr.strip()
        except (subprocess.TimeoutExpired, OSError) as e:
            entry["version_error"] = str(e)
        # Best-effort, READ-ONLY login signal — never echo token material. grok
        # caches creds under ~/.grok/ after `grok login`; presence of a
        # credential-named file is a weak "logged in" hint. We do NOT open or
        # parse it (no secrets in output). None => unknown; invoke decides.
        grok_dir = os.path.expanduser("~/.grok")
        try:
            if os.path.isdir(grok_dir):
                names = os.listdir(grok_dir)
                entry["logged_in"] = any(
                    "cred" in n or "auth" in n or "token" in n or "session" in n
                    for n in names) or None
            else:
                entry["logged_in"] = None
        except OSError:
            entry["logged_in"] = None
    return entry


# ---------------------------------------------------------------------------
# Prompt assembly — judge.py templates + Grok execution context. Grok reads frames
# as ACP IMAGE CONTENT BLOCKS carried inline in --prompt-json (no -i flag, no
# --add-dir path refs, no @ syntax). The manifest is the same "frame N = ms"
# time map judge.build_prompts emits for frame backends.
# ---------------------------------------------------------------------------


def build_grok_prompts(mode: str, perception: dict, duration_s: float,
                       fps: float, intent: str, frames: list[dict],
                       width: int, window: tuple[int, int] | None,
                       window_reason: str) -> tuple[str, str]:
    """Returns (system_prompt, user_prompt) for the grok judge call.

    Reuses judge.build_prompts (single-sourced templates, frame-time-map form)
    and appends execution context so the model knows how input is presented:
    the attached image content blocks are its only view of the video, it
    received NO audio, and it must answer with ONLY the verdict JSON (grok emits
    a plain-text answer inside .text — there is no schema flag, so the JSON-only
    instruction is the ONLY enforcement, recovered defensively after). The
    EXPLICIT_SCHEMA_BLOCK is appended so grok uses the exact field names.
    """
    sys_p, user_p = judge.build_prompts(
        mode=mode, perception=perception, duration_s=duration_s, fps=fps,
        intent=intent, listened=False, watched=True,
        window=window, window_reason=window_reason or "(no reason recorded)",
        frame_manifest=frames)
    user_p += (
        "\n\nThe frame images are ATTACHED to this message as image content "
        "blocks IN ORDER (the order matches the FRAME TIME MAP above). Examine "
        "every one before judging.\n"
        + EXPLICIT_SCHEMA_BLOCK)
    sys_p += (
        "\nGROK EXECUTION CONTEXT: you run as a non-interactive `grok "
        f"--prompt-json` subprocess. The {len(frames)} JPEG frames ATTACHED as "
        f"image content blocks (sampled at {fps:.3f} fps, {width}px wide, in "
        "FRAME TIME MAP order) are your ONLY view of the video — look at the "
        "pixels. You received NO audio stream; the transcript inside the "
        "instrument facts is your only knowledge of what was said. Do not run "
        "shell commands, edit files, browse the web, or call tools. Output ONLY "
        "the verdict JSON object.\n")
    return sys_p, user_p


# ---------------------------------------------------------------------------
# ACP content-block assembly — grok --prompt-json takes a JSON ARRAY of ACP
# content blocks. The text block carries the system rules + user prompt; each
# frame is a FLAT image block {type:"image", data:<b64>, mimeType:...}.
# (NB: flat data/mimeType — NOT a nested source object, which omits the
# required top-level `data` field.)
# ---------------------------------------------------------------------------


def build_content_blocks(full_prompt: str,
                         frame_paths: list[str]) -> tuple[list[dict], int]:
    """Build the ACP content-block array for --prompt-json.

    Returns (blocks, total_image_bytes). The first block is the text prompt
    (system rules + user prompt, folded together exactly like codex/agy/gemini);
    each subsequent block is a base64 ACP image block in FRAME TIME MAP order.
    total_image_bytes is recorded in cli_meta for footprint accounting.
    """
    blocks: list[dict] = [{"type": "text", "text": full_prompt}]
    total_bytes = 0
    for fp in frame_paths:
        with open(fp, "rb") as f:
            raw = f.read()
        total_bytes += len(raw)
        blocks.append({
            "type": "image",
            "data": base64.b64encode(raw).decode("ascii"),
            "mimeType": "image/jpeg",
        })
    return blocks, total_bytes


# ---------------------------------------------------------------------------
# Invocation — Grok Build CLI `grok --prompt-json` (clean machine JSON outer
# envelope; verdict recovered from .text).
# ---------------------------------------------------------------------------


def invoke_grok(grok_bin: str, sys_p: str, user_p: str, model: str,
                frame_paths: list[str], cwd: str, timeout_s: int
                ) -> tuple[dict | None, dict, str | None]:
    """Run one judge review through the `grok` CLI (single-turn --prompt-json).

    Returns (review|None, cli_meta, error_or_not_run_reason|None) — the SAME
    triple shape as cli_judge.invoke_claude / codex_judge.invoke_codex /
    antigravity_judge.invoke_antigravity, so the envelope assembly is shared.

    Non-interactive Grok Build argv contract:
      --prompt-file <path>     single-turn; reads the prompt from a FILE and
                               auto-detects a JSON array of ACP content blocks
                               ([text, image*]). We use the FILE channel (not the
                               inline --prompt-json) because base64 frames exceed
                               the Linux per-arg limit (Errno 7) — see module doc.
      --output-format json     ONE machine object on stdout {text, stopReason,
                               sessionId, requestId, thought}; verdict is in .text.
      --no-memory              throwaway call; never read/write session memory.
      --disable-web-search     no web tools (judge reasons from frames + facts).
      --disallowed-tools vision_describe   skip grok's native vision MCP tool,
                               which is rejected in stdio-standalone (no shellX
                               HTTP MCP transport) — routes grok straight to the
                               inline image block. The frame is STILL seen.
      --model <id>             OPTIONAL model id (omitted by default -> account
                               default, which is vision-capable).
    There is NO reliable single-turn --system-prompt channel we use here; like
    codex/agy/gemini, the system rules are PREPENDED to the user prompt (clearly
    delimited) inside the text content block, so all rungs share one prompt path.
    """
    path = shutil.which(grok_bin)
    if not path:
        return None, {"available": False}, (
            f"grok CLI not found ({grok_bin!r}) — honest not_run")

    # Fold the system rules into the prompt (single-turn, one prompt path across
    # all rungs), then carry it + the frames as ACP content blocks.
    full_prompt = (
        "SYSTEM INSTRUCTIONS (follow exactly):\n" + sys_p +
        "\n\n----- TASK -----\n" + user_p)
    blocks, image_bytes = build_content_blocks(full_prompt, frame_paths)

    cwd_abs = os.path.abspath(cwd)
    # Write the content-block array to a file and pass it via --prompt-file. The
    # inline --prompt-json would put ~130 KB of base64 frames into a single argv
    # value and can exceed Linux MAX_ARG_STRLEN. The file channel has no
    # such limit. Written under the bundle (cleaned with it; never world-readable
    # /tmp). If the write fails, that is an honest error (no fabricated verdict).
    prompt_file = os.path.join(cwd_abs, "_grok_prompt.json")
    try:
        with open(prompt_file, "w") as f:
            json.dump(blocks, f)
    except OSError as e:
        return None, {"available": True}, (
            f"grok adapter could not write the prompt-file {prompt_file}: {e}")

    cmd = [
        path,
        "--prompt-file", prompt_file,        # JSON content blocks from a FILE (ARG_MAX-safe)
        "--output-format", "json",
        "--no-memory",                       # throwaway; never touch session memory
        "--disable-web-search",              # judge reasons from frames + facts only
        "--disallowed-tools", _DISALLOWED_TOOLS,  # skip the rejected native vision MCP tool
        "--cwd", cwd_abs,                    # subprocess root = clean bundle (avoids
                                             # grok's recursive watcher hitting /tmp
                                             # permission-denied noise)
    ]
    if model:
        cmd += ["--model", model]            # model id, e.g. "grok-build"

    try:
        cp = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", cwd=cwd_abs,
                            timeout=timeout_s)
    except subprocess.TimeoutExpired:
        return None, {"available": True, "timed_out": True}, (
            f"grok CLI exceeded {timeout_s}s timeout")

    meta = {
        "available": True,
        "exit_code": cp.returncode,
        "model": model or "(grok account default)",
        "schema_enforced": False,            # grok has no --output-schema flag
        "image_bytes": image_bytes,
        "frames_attached": len(frame_paths),
    }

    # Non-zero exit is an honest error (a hard auth/login failure surfaces here).
    if cp.returncode != 0:
        return None, meta, (
            f"grok CLI exit {cp.returncode}: "
            f"{cp.stderr.strip()[-600:] or '(no stderr)'}")

    # --output-format json => ONE object {text, stopReason, sessionId...} on
    # stdout. grok's recursive file-watcher can emit a few tracing/error lines
    # before the JSON; the object is the LAST balanced {...} on stdout, so we
    # recover it defensively (the same _extract_json that recovers the verdict).
    raw_stdout = (cp.stdout or "").strip()
    if not raw_stdout:
        return None, meta, (
            "grok returned EMPTY stdout (exit 0) — likely an auth/login failure "
            "(run `grok login --oauth` or `--device-auth`) or a silenced turn. "
            "Cannot fabricate a verdict. stderr tail: "
            f"{cp.stderr.strip()[-300:] or '(none)'}")
    outer = _extract_outer_json(raw_stdout)
    if outer is None:
        return None, meta, (
            f"grok --output-format json emitted no parseable object: "
            f"{raw_stdout[:400]}")
    meta["stop_reason"] = outer.get("stopReason")
    meta["session_id"] = outer.get("sessionId")
    meta["request_id"] = outer.get("requestId")

    # The verdict lives in .text. We IGNORE .thought (chain-of-thought noise).
    response_text = outer.get("text")
    if not isinstance(response_text, str) or not response_text.strip():
        return None, meta, (
            "grok response object has empty/absent .text — no verdict produced "
            f"(stopReason={outer.get('stopReason')!r})")

    # grok has no schema flag; the prompt demands a bare JSON object. Recover it
    # defensively (tolerating stray ``` fences / a prose preamble) via
    # codex_judge._extract_json — ONE shared implementation across rungs.
    structured = codex_judge._extract_json(response_text)
    if structured is None:
        return None, meta, f"grok .text is not verdict JSON: {response_text[:400]}"
    try:
        review = judge.validate_review(structured)
    except ValueError as e:
        return None, meta, f"grok verdict failed schema validation: {e}"
    return review, meta, None


def _extract_outer_json(text: str) -> dict | None:
    """Recover grok's --output-format json envelope object from stdout.

    grok normally prints exactly one JSON object, but its recursive file-watcher
    can prepend tracing/error lines (e.g. permission-denied on a sibling dir)
    before the object. We try a straight parse first, then fall back to the LAST
    balanced {...} span on stdout (the response object is emitted last). Returns
    the parsed dict, or None if no object is recoverable.
    """
    text = text.strip()
    try:
        obj = json.loads(text)
        return obj if isinstance(obj, dict) else None
    except json.JSONDecodeError:
        pass
    # Fallback: scan for the LAST balanced top-level {...} object on stdout.
    last: dict | None = None
    depth = 0
    start = -1
    for i, ch in enumerate(text):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}":
            if depth > 0:
                depth -= 1
                if depth == 0 and start >= 0:
                    try:
                        obj = json.loads(text[start:i + 1])
                        if isinstance(obj, dict):
                            last = obj
                    except json.JSONDecodeError:
                        pass
                    start = -1
    return last


# ---------------------------------------------------------------------------
# CLI entry — mirrors codex_judge.py / antigravity_judge.py's structure (shared
# resolution + filter + envelope), differing only in the provider invocation
# (frames via inline ACP content blocks, verdict recovered from the .text field).
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
                    help="grok model id (e.g. 'grok-build'); default: omit "
                         "--model, use the grok account default")
    ap.add_argument("--grok-bin", default="grok")
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

    # Bundle = the grok subprocess's working root (--cwd). Project-local, not
    # /tmp (same reasoning as the other adapters: keep ONE bundle convention so
    # the Rust wire-in passes the same directory shape — and grok's recursive
    # file-watcher hits permission-denied noise under /tmp's systemd-private dirs).
    bundle = args.bundle_dir or tempfile.mkdtemp(prefix="grok_judge_",
                                                 dir=os.getcwd())
    os.makedirs(bundle, exist_ok=True)

    # Perception facts for THE RENDER (the coordinate-space guard) — shared resolver, then the
    # coordinate-space sanity gate, before frames are extracted or a provider request is sent.
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
    # grok reads frames from inline --prompt-json image blocks (base64). We pass
    # absolute paths to build_content_blocks, which reads + encodes them.
    frame_paths = [os.path.abspath(m["path"]) for m in frames]

    sys_p, user_p = build_grok_prompts(
        args.mode, perception, duration_s, fps_eff, args.intent,
        frames, args.width, window, args.window_reason)

    if not frames:
        # No frames extracted (e.g. zero-duration render) — honest not_run, no
        # call made (consistent with the ladder's "nothing to judge" path).
        review_raw, cli_meta, reason = None, {"available": True}, (
            "no frames could be extracted from the render — nothing to judge")
    else:
        review_raw, cli_meta, reason = invoke_grok(
            args.grok_bin, sys_p, user_p, args.cli_model, frame_paths,
            bundle, args.timeout)

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
            "resolution": f"frames {args.width}px JPEG (--prompt-file ACP blocks)",
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
