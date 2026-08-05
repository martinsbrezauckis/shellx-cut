#!/usr/bin/env python3
"""Codex subscription-CLI render judge adapter.

Uses codex exec in a clean, ephemeral, read-only working root, attaches sampled
frames with -i, validates the final structured message, and returns the shared
shellx-cut/judge-review/1 envelope without using an API key.
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

# The shared judge pipeline (perception resolution by content hash, the
# post-filter, frame extraction defaults) lives in cli_judge.py as clean
# module-level functions. Import them so all adapters share ONE implementation
# of the contract — the only per-provider code is invoke_* + frame manifest.
_ADAPTERS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _ADAPTERS_DIR)
import cli_judge  # noqa: E402

ADAPTER_NAME = "cli"                 # same adapter CLASS as claude (CLI judge)
DEFAULT_PROVIDER = "codex"
# Omit -m by default so Codex uses the current ChatGPT-account model that
# supports image input. An explicit --cli-model passes through for operators
# who know a valid account model alias.
DEFAULT_CLI_MODEL = ""               # "" => omit -m, use codex account default
DEFAULT_GLOBAL_FPS = cli_judge.DEFAULT_GLOBAL_FPS
DEFAULT_WINDOW_FPS = cli_judge.DEFAULT_WINDOW_FPS
DEFAULT_MAX_FRAMES = cli_judge.DEFAULT_MAX_FRAMES
DEFAULT_FRAME_WIDTH = cli_judge.DEFAULT_FRAME_WIDTH
DEFAULT_TIMEOUT_S = cli_judge.DEFAULT_TIMEOUT_S


# ---------------------------------------------------------------------------
# OpenAI strict-schema transform — codex --output-schema rejects a schema
# unless EVERY object sets additionalProperties:false and lists ALL its
# properties in `required` (verified live: a non-strict schema returned
# invalid_json_schema / "additionalProperties is required to be ... false").
# We keep judge.REVIEW_SCHEMA as the single semantic source and derive the
# strict variant here; validate_review still runs on the result, so an
# over-tight schema can never let a malformed verdict through.
# ---------------------------------------------------------------------------


def to_strict_schema(schema: dict) -> dict:
    """Deep-copy `schema` into OpenAI strict-mode form for codex.

    Recursively: on every object node set additionalProperties=false and force
    `required` to list ALL declared property keys (strict mode forbids optional
    keys). This makes previously-optional fields (end_ms, suggested_fix,
    cannot_assess) structurally required in the codex response — acceptable
    because the model can still emit empty strings / arrays for them, and our
    own validate_review only enforces the genuinely-required core. Returns a
    new dict; the input is untouched.
    """
    node = json.loads(json.dumps(schema))  # deep copy; never mutate the shared schema

    def walk(n: dict) -> None:
        if not isinstance(n, dict):
            return
        if n.get("type") == "object":
            props = n.get("properties") or {}
            n["additionalProperties"] = False
            n["required"] = list(props.keys())  # strict: all keys required
            for v in props.values():
                walk(v)
        if n.get("type") == "array" and isinstance(n.get("items"), dict):
            walk(n["items"])

    walk(node)
    return node


# ---------------------------------------------------------------------------
# Prompt assembly — judge.py templates + a codex frame manifest. codex sees
# the frames as ATTACHED IMAGES (-i), in the order passed, so the manifest is
# the same "frame N = ms" time map judge.build_prompts emits for native-image
# backends (NOT file paths — codex reads the pixels, it does not Read files).
# ---------------------------------------------------------------------------


def build_codex_prompts(mode: str, perception: dict, duration_s: float,
                        fps: float, intent: str, frames: list[dict],
                        width: int, window: tuple[int, int] | None,
                        window_reason: str) -> tuple[str, str]:
    """Returns (system_prompt, user_prompt) for the codex judge call.

    Reuses judge.build_prompts (single-sourced templates, image-time-map form)
    and appends a codex-harness note so the model knows HOW it perceives here:
    the attached images are its only view, it received no audio, and it must
    answer with ONLY the verdict JSON (codex emits a final agent message, not a
    tool call, so the JSON-only instruction matters even with --output-schema).
    """
    sys_p, user_p = judge.build_prompts(
        mode=mode, perception=perception, duration_s=duration_s, fps=fps,
        intent=intent, listened=False, watched=True,
        window=window, window_reason=window_reason or "(no reason recorded)",
        frame_manifest=frames)
    sys_p += (
        "\nCODEX HARNESS CONTEXT: you run as a non-interactive subprocess. The "
        f"{len(frames)} JPEG frames ATTACHED to this message (sampled at "
        f"{fps:.3f} fps, {width}px wide, in the order of the FRAME TIME MAP) are"
        " your ONLY view of the video. You received NO audio stream; the "
        "transcript inside the instrument facts is your only knowledge of what "
        "was said. Do not run shell commands or read other files. Respond with "
        "ONLY the verdict JSON object — no prose, no markdown fences.\n")
    return sys_p, user_p


# ---------------------------------------------------------------------------
# Provider detection + invocation — codex.
# ---------------------------------------------------------------------------


def detect() -> dict:
    """Is the codex CLI present and (best-effort) logged in?

    Detection only — no model call, no quota burned. `found` gates the ladder;
    `logged_in` is a best-effort read of ~/.codex/auth.json (auth_mode +
    presence of OAuth tokens, NEVER the token values). A present-but-logged-out
    codex still reports found=true; the ladder treats login failures at
    invoke time as not_run (honest), so detection stays cheap and side-effect
    free.
    """
    path = shutil.which("codex")
    entry: dict = {"provider": "codex", "binary": "codex",
                   "found": bool(path), "path": path,
                   "adapter": "implemented"}
    if path:
        try:
            cp = subprocess.run([path, "--version"], capture_output=True,
                                text=True, timeout=15)
            entry["version"] = cp.stdout.strip() or cp.stderr.strip()
        except (subprocess.TimeoutExpired, OSError) as e:
            entry["version_error"] = str(e)
        # Best-effort, READ-ONLY login signal (never echo token material).
        auth_path = os.path.expanduser("~/.codex/auth.json")
        try:
            with open(auth_path) as f:
                auth = json.load(f)
            entry["logged_in"] = bool(auth.get("tokens")) or bool(
                auth.get("OPENAI_API_KEY"))
            entry["auth_mode"] = auth.get("auth_mode")
        except (OSError, json.JSONDecodeError):
            entry["logged_in"] = None  # unknown — invoke decides honestly
    return entry


def invoke_codex(codex_bin: str, sys_p: str, user_p: str, model: str,
                 frame_paths: list[str], cwd: str, timeout_s: int
                 ) -> tuple[dict | None, dict, str | None]:
    """Run one judge review through the codex CLI.

    Returns (review|None, cli_meta, not_run_or_error_reason|None) — the SAME
    triple shape as cli_judge.invoke_claude, so the envelope assembly is shared.

    Non-interactive Codex argv contract:
      exec                      non-interactive run
      --skip-git-repo-check     the bundle dir is not a git repo
      --ephemeral               no session files persisted (judge calls are
                                throwaway; never pollute resumable history)
      --ignore-user-config      drop ~/.codex/config.toml (skills/AGENTS.md/
                                hooks) for a clean judge context; OAuth intact
      -s read-only              the judge never writes/executes (sandbox)
      -m <model>                model alias (falls back to config default if
                                the alias is unknown — we do not hard-fail)
      -C <cwd>                  working root = the throwaway bundle
      -o <lastmsg>              final agent message -> file (the verdict JSON)
      --json                    JSONL events on stdout (failure + usage signal)
      --output-schema <strict>  strict-mode JSON Schema enforcement (best
                                effort; prompt also demands the JSON)
      -i <frame> ...            attach each sampled frame as an image
      -                         read the prompt (instructions) from stdin
    The system prompt is PREPENDED to the user prompt on stdin: codex exec has
    no separate --system-prompt flag, so the harness rules ride in the same
    instruction block (clearly delimited).
    """
    path = shutil.which(codex_bin)
    if not path:
        return None, {"available": False}, (
            f"codex CLI not found ({codex_bin!r}) — honest not_run")

    # Strict schema to a temp file inside the bundle (codex --output-schema
    # takes a PATH). ABSOLUTE path — codex resolves --output-schema AFTER the
    # -C chdir, so a relative path would be looked up under the bundle twice
    # ("No such file or directory", observed live). If writing fails we proceed
    # WITHOUT the flag (prompt-only enforcement) — the verdict is still
    # validated below, so the schema flag is an enforcement bonus, not required.
    cwd_abs0 = os.path.abspath(cwd)
    schema_path = os.path.join(cwd_abs0, "_codex_verdict_schema.json")
    use_schema = True
    try:
        with open(schema_path, "w") as f:
            json.dump(to_strict_schema(judge.REVIEW_SCHEMA), f)
    except OSError:
        use_schema = False

    # Absolute working root: codex -C chdirs there; pairing it with a relative
    # subprocess cwd risks a double-relative resolution (observed as a cryptic
    # "No such file or directory" startup error). Keep -C and -o absolute.
    cwd_abs = os.path.abspath(cwd)
    cmd = [
        path, "exec",
        "--skip-git-repo-check",
        "--ephemeral",
        "--ignore-user-config",
        "-s", "read-only",
        "-C", cwd_abs,
        "-o", os.path.join(cwd_abs, "_codex_last_message.json"),
        "--json",
    ]
    # Omit -m entirely when no explicit model is requested — codex then uses its
    # built-in ChatGPT-account default (which supports images). An invalid
    # alias (e.g. gpt-5-codex on a ChatGPT account) would fail the turn.
    if model:
        cmd += ["-m", model]
    if use_schema:
        cmd += ["--output-schema", schema_path]
    for fp in frame_paths:
        cmd += ["-i", fp]
    cmd += ["-"]  # prompt from stdin

    # codex exec has no --system-prompt; fold the system rules into the prompt.
    full_prompt = (
        "SYSTEM INSTRUCTIONS (follow exactly):\n" + sys_p +
        "\n\n----- TASK -----\n" + user_p)

    try:
        cp = subprocess.run(cmd, input=full_prompt, capture_output=True,
                            text=True, cwd=cwd, timeout=timeout_s)
    except subprocess.TimeoutExpired:
        return None, {"available": True, "timed_out": True}, (
            f"codex CLI exceeded {timeout_s}s timeout")

    # Parse the JSONL event stream for usage + a model-side failure signal
    # (turn.failed / error). The verdict itself comes from the -o last-message
    # file (cleaner than reconstructing it from item.completed events).
    usage: dict | None = None
    model_error: str | None = None
    thread_id: str | None = None
    for line in cp.stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        et = ev.get("type")
        if et == "thread.started":
            thread_id = ev.get("thread_id")
        elif et == "turn.completed":
            usage = ev.get("usage")
        elif et in ("turn.failed", "error"):
            # codex nests the API error JSON as a string in .error.message /
            # .message — surface a trimmed version as the failure reason.
            msg = (ev.get("error") or {}).get("message") if et == "turn.failed" \
                else ev.get("message")
            model_error = str(msg)[:600] if msg else f"codex emitted {et}"

    meta = {
        "available": True,
        "exit_code": cp.returncode,
        "thread_id": thread_id,
        "usage": usage,
        "model": model or "(codex account default)",
        "schema_enforced": use_schema,
    }

    if model_error is not None:
        return None, meta, f"codex turn failed: {model_error}"
    if cp.returncode != 0:
        return None, meta, (
            f"codex CLI exit {cp.returncode}: {cp.stderr.strip()[-600:]}")

    # Read the final agent message (the verdict JSON). Written under cwd_abs.
    last_path = os.path.join(cwd_abs, "_codex_last_message.json")
    try:
        with open(last_path) as f:
            raw = f.read().strip()
    except OSError:
        return None, meta, (
            "codex produced no last-message file; stdout tail: "
            f"{cp.stdout.strip()[-300:]}")
    if not raw:
        return None, meta, "codex last-message file is empty (no verdict)"

    # The model may wrap JSON in ```fences``` despite instructions — strip them.
    structured = _extract_json(raw)
    if structured is None:
        return None, meta, f"codex last message is not verdict JSON: {raw[:400]}"
    try:
        review = judge.validate_review(structured)
    except ValueError as e:
        return None, meta, f"codex verdict failed schema validation: {e}"
    return review, meta, None


def _extract_json(text: str) -> dict | None:
    """Best-effort: recover the verdict JSON object from `text`, tolerating ```
    fences and a prose preamble that itself contains braces.

    codex/grok/agy with the schema/prompt enforcement usually return bare JSON,
    but a fenced or prose-prefixed answer is a known LLM failure mode. The
    earlier version returned the FIRST balanced {...} span — a model preamble
    like "Here is my analysis {note: incomplete}. Final verdict:\\n{<real
    verdict>}" made it return the wrong object, so the real verdict was discarded
    Scan every top-level balanced {...} span and
    return the first that BOTH parses as JSON AND passes judge.validate_review —
    a stray brace blob in prose fails validation and is skipped, the actual
    verdict is found. Behavior is unchanged when the response is already a single
    clean verdict object (that one span parses + validates first).
    """
    text = text.strip()
    # Strip a leading ```json / ``` fence if present.
    if text.startswith("```"):
        text = text.split("\n", 1)[-1]
        if text.rstrip().endswith("```"):
            text = text.rsplit("```", 1)[0]
        text = text.strip()

    # Fast path: the whole (de-fenced) text is a single JSON object. Validate it
    # so the clean-input path returns identically to the span scan below.
    try:
        obj = json.loads(text)
        if isinstance(obj, dict):
            try:
                judge.validate_review(obj)
                return obj
            except ValueError:
                # Parsed but is not a valid verdict — fall through to the span
                # scan (it may be a non-verdict wrapper; unlikely but harmless).
                pass
    except json.JSONDecodeError:
        pass

    # Scan ALL top-level balanced {...} spans; return the FIRST that parses AND
    # validates as a verdict. Skips prose-embedded brace blobs that aren't the
    # verdict (e.g. "{note: incomplete}" before the real object).
    first_parseable: dict | None = None  # fallback if nothing validates
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
                    span = text[start:i + 1]
                    start = -1
                    try:
                        obj = json.loads(span)
                    except json.JSONDecodeError:
                        continue
                    if not isinstance(obj, dict):
                        continue
                    try:
                        judge.validate_review(obj)
                        return obj  # the real verdict
                    except ValueError:
                        # Remember the first JSON-parseable dict as a last-resort
                        # return so a verdict the caller's validate_review will
                        # reject still surfaces a clear schema error there (not a
                        # silent None) when NO span validates.
                        if first_parseable is None:
                            first_parseable = obj
    return first_parseable


# ---------------------------------------------------------------------------
# CLI entry — mirrors cli_judge.py's structure (shared resolution + filter +
# envelope), differing only in the provider invocation (images via -i).
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
                    help="optional model alias passed to codex; omitted by default "
                         "so the account's configured model is used")
    ap.add_argument("--codex-bin", default="codex")
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

    # Bundle = the codex subprocess's working root (-C). Project-local, not
    # /tmp (same reasoning as cli_judge.py: the claude sandbox denies /tmp; we
    # keep codex consistent so the wire-in passes ONE bundle convention).
    bundle = args.bundle_dir or tempfile.mkdtemp(prefix="codex_judge_",
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
    # identical policy to cli_judge.py so reviews are comparable across rungs.
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
    # codex -i takes ABSOLUTE paths (it does not chdir into the bundle the way
    # the claude Read tool does); keep them absolute for the -i flags.
    frame_paths = [os.path.abspath(m["path"]) for m in frames]

    sys_p, user_p = build_codex_prompts(
        args.mode, perception, duration_s, fps_eff, args.intent,
        frames, args.width, window, args.window_reason)

    if not frames:
        # No frames extracted (e.g. zero-duration render) — honest not_run, no
        # call made (consistent with the ladder's "nothing to judge" path).
        review_raw, cli_meta, reason = None, {"available": True}, (
            "no frames could be extracted from the render — nothing to judge")
    else:
        review_raw, cli_meta, reason = invoke_codex(
            args.codex_bin, sys_p, user_p, args.cli_model,
            frame_paths, bundle, args.timeout)

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
            "model": f"{DEFAULT_PROVIDER}/{args.cli_model}",
            "fps": round(fps_eff, 4),
            "fps_requested": fps_req,
            "resolution": f"frames {args.width}px JPEG (attached images)",
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
