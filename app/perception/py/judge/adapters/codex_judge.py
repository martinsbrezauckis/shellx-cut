#!/usr/bin/env python3
"""Codex render-judge admission boundary.

Probe direct-image and strict-output transport separately from complete
frame-only file/tool confinement. Review remains ``not_run`` until both are
verified for the installed CLI, before render inspection or a model subprocess.

The strict-schema and structured-output helpers remain here because other
judge adapters use them as their pure shared implementation.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import shutil
import sys
import tempfile
import time

_JUDGE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _JUDGE_DIR)
_PY_ROOT = os.path.dirname(_JUDGE_DIR)
sys.path.insert(0, _PY_ROOT)
import judge  # noqa: E402
import judge_files  # noqa: E402
import provider_child_launch  # noqa: E402
_ADAPTERS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _ADAPTERS_DIR)
import codex_judge_policy  # noqa: E402
import codex_judge_runtime  # noqa: E402
import cli_judge  # noqa: E402

ADAPTER_NAME = "cli"
DEFAULT_PROVIDER = "codex"
DEFAULT_CLI_MODEL = "gpt-6.1-sol"
# Root must change this only after the installed provider-child route, effective
# tool config, and direct-image sibling/network canary are qualified together.
NATIVE_ADMISSION_VERIFIED = True
AVAILABILITY_REASON = "Codex tool-free image judge awaits native admission"


class CleanupError(RuntimeError):
    """An input failure accompanied by directories that could not be removed."""

    def __init__(self, original: BaseException, retained: dict[str, str]):
        super().__init__("Codex judge cleanup failed")
        self.original = original
        self.retained = retained


def _remove_owned_directory(path: str) -> bool:
    """Remove an owned directory, treating an already absent directory as clean."""
    try:
        shutil.rmtree(path)
    except FileNotFoundError:
        pass
    except OSError:
        return False
    return not os.path.lexists(path)


def _child_route(launch: dict | None) -> tuple[list[str], dict | None]:
    """Use the selected Runner child, or the ordinary installed-user CLI."""
    if launch is not None:
        return provider_child_launch.command(launch, []), provider_child_launch.environment(launch)
    path = shutil.which("codex")
    if not path:
        raise ValueError("installed Codex CLI is missing")
    installed = {"executable": path, "entrypoint": None,
                 "environment": dict(os.environ)}
    return provider_child_launch.command(installed, []), installed["environment"]


def to_strict_schema(schema: dict) -> dict:
    """Deep-copy ``schema`` into the strict object form used by Codex.

    This is intentionally a pure helper. It does not invoke a provider or
    inspect a render, and remains available to consumers that need strict
    output-schema conversion independently of render-judge admission.
    """
    node = json.loads(json.dumps(schema))

    def walk(value: dict) -> None:
        if not isinstance(value, dict):
            return
        if value.get("type") == "object":
            properties = value.get("properties") or {}
            value["additionalProperties"] = False
            value["required"] = list(properties.keys())
            for child in properties.values():
                walk(child)
        if value.get("type") == "array" and isinstance(value.get("items"), dict):
            walk(value["items"])

    walk(node)
    return node


def _extract_json(text: str) -> dict | None:
    """Recover a valid judge verdict from bare, fenced, or prose-wrapped JSON.

    This is a pure shared decoder used by the other adapters; it deliberately
    remains independent of Codex CLI admission.
    """
    text = text.strip()
    if text.startswith("```"):
        text = text.split("\n", 1)[-1]
        if text.rstrip().endswith("```"):
            text = text.rsplit("```", 1)[0]
        text = text.strip()

    try:
        parsed = json.loads(text)
        if isinstance(parsed, dict):
            try:
                judge.validate_review(parsed)
                return parsed
            except ValueError:
                pass
    except json.JSONDecodeError:
        pass

    first_parseable: dict | None = None
    depth = 0
    start = -1
    for index, char in enumerate(text):
        if char == "{":
            if depth == 0:
                start = index
            depth += 1
        elif char == "}" and depth > 0:
            depth -= 1
            if depth == 0 and start >= 0:
                span = text[start:index + 1]
                start = -1
                try:
                    parsed = json.loads(span)
                except json.JSONDecodeError:
                    continue
                if not isinstance(parsed, dict):
                    continue
                try:
                    judge.validate_review(parsed)
                    return parsed
                except ValueError:
                    if first_parseable is None:
                        first_parseable = parsed
    return first_parseable


def detect(launch: dict | None = None) -> dict:
    """Report Codex binary presence separately from render-judge admission.

    Before native admission, detection only checks version and exec help. Once
    admitted, it also runs the bounded no-model policy probe in owned TMP.
    """
    path = launch["executable"] if launch else shutil.which("codex")
    entry: dict = {
        "provider": DEFAULT_PROVIDER,
        "binary": "codex",
        "found": bool(path),
        "judge_ready": False,
        "availability_reason": AVAILABILITY_REASON,
        "path": path,
        "adapter": "implemented (render-judge admission pending)",
    }
    if path:
        try:
            deadline = time.monotonic() + 5.0
            def budget() -> float:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise codex_judge_runtime.ProbeError("codex_child_timeout")
                return remaining
            prefix, env = _child_route(launch)
            command = lambda argv: prefix + argv
            cwd = tempfile.gettempdir()
            version = codex_judge_runtime.run_bounded(
                command(["--version"]), cwd, env, budget()).decode("utf-8", "replace")
            version_match = re.search(r"\bcodex-cli [0-9]+\.[0-9]+\.[0-9]+\b", version)
            entry["version"] = version_match.group(0) if version_match else "unrecognized"
            help_text = codex_judge_runtime.run_bounded(
                command(["exec", "--help"]), cwd, env, budget()).decode("utf-8", "replace")
            _ready, _reason, detail = codex_judge_policy.judge_admission(
                help_text)
            detail["native_tool_free_admission"] = False
            detail["frame_only_file_and_tool_confinement"] = False
            entry["capabilities"] = detail
            if NATIVE_ADMISSION_VERIFIED and detail["direct_image_and_schema"]:
                with tempfile.TemporaryDirectory(prefix="codex_judge_detect_",
                                                 dir=_job_temp(launch)) as job:
                    os.chmod(job, 0o700)
                    probe = codex_judge_runtime.premodel_probe(
                        prefix, env, job, os.path.join(job, "model-catalog.json"),
                        budget(), DEFAULT_CLI_MODEL, deadline_s=deadline)
                    entry["judge_ready"] = probe.get("effective_policy") is True
            detail["native_tool_free_admission"] = entry["judge_ready"]
            detail["frame_only_file_and_tool_confinement"] = entry["judge_ready"]
            entry["availability_reason"] = (None if entry["judge_ready"]
                                             else (_reason if NATIVE_ADMISSION_VERIFIED
                                                   else AVAILABILITY_REASON))
            entry["capabilities"] = detail
        except codex_judge_runtime.ProbeError as error:
            entry["capability_error"] = str(error)
            entry["availability_reason"] = str(error)
        except (OSError, ValueError) as error:
            entry["capability_error"] = type(error).__name__
        entry["logged_in"] = None
    return entry


def _not_run_envelope(args: argparse.Namespace) -> dict:
    """Create the terminal receipt without inspecting the requested render."""
    return {
        "schema": judge.SCHEMA,
        "ts": judge.now_iso(),
        "render": os.path.abspath(args.render) if args.render else None,
        "mode": args.mode,
        "backend": {
            "name": ADAPTER_NAME,
            "provider": DEFAULT_PROVIDER,
            "model": f"{DEFAULT_PROVIDER}/{args.cli_model or 'default'}",
            "watched": False,
            "listened": False,
            "frames_sent": 0,
        },
        "window": None,
        "status": "not_run",
        "not_run_reason": AVAILABILITY_REASON,
        "review": None,
        "review_raw": None,
        "post_filter": None,
        "perception_source": None,
        "warnings": [],
        "cli": {
            "available": False,
            "admission": "unverified_restricted_tool_file_access",
        },
        "prompt_chars": {"system": 0, "user": 0},
        "bundle_dir": None,
    }


def invoke_codex(launch: dict | None, sys_p: str, user_p: str, model: str,
                 frame_paths: list[str], cwd: str, timeout_s: int
                 ) -> tuple[dict | None, dict, str | None]:
    """Execute a previously admitted direct-image review, or refuse safely."""
    if not NATIVE_ADMISSION_VERIFIED:
        return None, {"available": False, "admission": "native_pending"}, AVAILABILITY_REASON
    detected = detect(launch)
    if not detected.get("judge_ready"):
        return None, {"available": False, "admission": "cli_capability_unavailable"}, (
            detected.get("availability_reason") or AVAILABILITY_REASON)
    try:
        command, environment = _child_route(launch)
        catalog_path = os.path.join(cwd, "model-catalog.json")
        probe = codex_judge_runtime.premodel_probe(
            command, environment, cwd, catalog_path, min(30, timeout_s), model)
        prompt = "SYSTEM INSTRUCTIONS:\n" + sys_p + "\n\nREVIEW TASK:\n" + user_p
        review, meta = codex_judge_runtime.exec_review(
            command, environment, cwd, frame_paths, to_strict_schema(judge.REVIEW_SCHEMA),
            prompt, probe, timeout_s)
        return review, {"available": True, **meta}, None
    except codex_judge_runtime.ProbeError as error:
        return None, {"available": True, "admission": "failed"}, str(error)
    except (OSError, ValueError):
        return None, {"available": True, "admission": "failed"}, "codex_judge_runtime_failed"


def _job_temp(launch: dict | None) -> str:
    temporary = next((os.environ.get(name) for name in ("TMPDIR", "TEMP", "TMP")
                      if os.environ.get(name)), None)
    if launch and (not temporary or not os.path.isabs(temporary)):
        raise ValueError("Runner-owned temporary directory is missing")
    return temporary or tempfile.gettempdir()


def review(args: argparse.Namespace, launch: dict | None) -> dict:
    """Resolve measured facts, attach only sampled frames, filter the verdict."""
    if not args.render or not os.path.isfile(args.render):
        raise ValueError("review requires an existing render")
    if args.max_frames < 1 or args.width < 2 or args.timeout < 1:
        raise ValueError("invalid frame or timeout bound")
    duration_s = judge.probe_duration_s(args.render)
    if duration_s <= 0:
        raise ValueError("render duration is empty")
    duration_ms = int(duration_s * 1000)
    temporary = os.path.realpath(_job_temp(launch))
    if args.bundle_dir:
        judge_files.require_plain_directory(args.bundle_dir, "Codex staging bundle")
        staging = os.path.realpath(args.bundle_dir)
        os.makedirs(staging, mode=0o700, exist_ok=True)
        owned_staging = False
    else:
        staging = tempfile.mkdtemp(prefix="codex_judge_", dir=temporary)
        owned_staging = True
    execution = None
    receipt = None
    try:
        perception, perception_source, warnings = cli_judge.resolve_perception(
            args.render, args.perception, staging)
        judge.sanity_check_perception(perception, duration_ms)
        window = None
        if args.mode == "window":
            if not args.windows:
                raise ValueError("window review requires start:end")
            a, b = (int(value) for value in args.windows.split(":"))
            if not 0 <= a < b <= duration_ms:
                raise ValueError("window is outside render")
            window = (a, b)
        span_s = ((window[1] - window[0]) / 1000) if window else duration_s
        requested_fps = args.fps or (cli_judge.DEFAULT_WINDOW_FPS if window
                                     else cli_judge.DEFAULT_GLOBAL_FPS)
        if not math.isfinite(requested_fps) or requested_fps <= 0:
            raise ValueError("fps must be positive and finite")
        fps = min(requested_fps, args.max_frames / span_s)
        if fps < requested_fps:
            warnings.append(f"frame cap reduced sampling to {fps:.3f} fps")
        frames = judge.extract_frames(
            args.render, os.path.join(staging, "frames"), fps,
            start_ms=window[0] if window else None,
            end_ms=window[1] if window else None,
            max_frames=args.max_frames, width=args.width)
        if not frames:
            raise ValueError("no visual frames were extracted")
        relatives = [os.path.relpath(frame["path"], staging) for frame in frames]
        forbidden_roots = []
        render_dir = os.path.dirname(os.path.realpath(args.render))
        if args.bundle_dir or render_dir != os.path.realpath(temporary):
            forbidden_roots.append(render_dir)
        execution, frame_paths = codex_judge_runtime.prepare_neutral_frames(
            staging, relatives, temporary, forbidden_roots=forbidden_roots)
        system, user = judge.build_prompts(
            args.mode, perception, duration_s, fps, args.intent,
            listened=False, watched=True, window=window,
            window_reason=args.window_reason, frame_manifest=frames)
        system += ("\nCodex receives the sampled JPEGs directly as image attachments "
                   "in frame-time-map order. No audio was supplied.\n")
        raw, metadata, reason = invoke_codex(
            launch, system, user, args.cli_model or DEFAULT_CLI_MODEL,
            frame_paths, execution, args.timeout)
        frame_failure = False
        if raw is not None:
            review_value, filtered = cli_judge.post_filter_review(
                raw, fps, duration_ms, listened=False)
            frame_failure, frame_reason = cli_judge.detect_frame_read_failure(raw, len(frames))
            status = "error" if frame_failure else "completed"
            if frame_failure:
                reason = frame_reason
        else:
            review_value, filtered = None, None
            status = "not_run" if not metadata.get("available") else "error"
        receipt = {
            "schema": judge.SCHEMA, "ts": judge.now_iso(),
            "render": os.path.abspath(args.render), "mode": args.mode,
            "backend": {"name": ADAPTER_NAME, "provider": DEFAULT_PROVIDER,
                        "model": f"codex/{args.cli_model or DEFAULT_CLI_MODEL}",
                        "fps": round(fps, 4), "fps_requested": requested_fps,
                        "resolution": f"frames {args.width}px JPEG",
                        "watched": raw is not None and not frame_failure, "listened": False,
                        "frames_sent": len(frames) if raw is not None else 0},
            "window": ({"start_ms": window[0], "end_ms": window[1],
                        "reason": args.window_reason} if window else None),
            "status": status,
            "error_class": "infrastructure" if status == "error" else None,
            "not_run_reason": reason, "review": review_value, "review_raw": raw,
            "post_filter": filtered, "perception_source": perception_source,
            "warnings": warnings, "cli": metadata,
            "prompt_chars": {"system": len(system), "user": len(user)},
            "bundle_dir": staging if args.keep_bundle else None,
            "execution_bundle_dir": None,
        }
        return receipt
    finally:
        original = sys.exception()
        retained = {}
        if isinstance(original, codex_judge_runtime.FrameCleanupError):
            retained["execution_bundle_dir"] = original.retained_directory
            original = original.original
        if execution and not _remove_owned_directory(execution):
            retained["execution_bundle_dir"] = execution
        if owned_staging and not args.keep_bundle and not _remove_owned_directory(staging):
            retained["bundle_dir"] = staging
        if retained:
            if original is not None:
                raise CleanupError(original, retained) from original
            receipt.update(retained)
            receipt["warnings"].append(
                "Temporary judge files could not be removed. Close applications using "
                "the listed directories, then remove those directories manually.")


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["review", "detect"])
    ap.add_argument("--render")
    ap.add_argument("--perception")
    ap.add_argument("--intent", default="(no edit intent provided)")
    ap.add_argument("--cli-model", default=DEFAULT_CLI_MODEL)
    ap.add_argument("--codex-bin", default="codex")
    ap.add_argument("--mode", default="global", choices=["global", "window"])
    ap.add_argument("--windows")
    ap.add_argument("--window-reason", default="flagged by instrument checks")
    ap.add_argument("--fps", type=float)
    ap.add_argument("--max-frames", type=int, default=cli_judge.DEFAULT_MAX_FRAMES)
    ap.add_argument("--width", type=int, default=cli_judge.DEFAULT_FRAME_WIDTH)
    ap.add_argument("--timeout", type=int, default=cli_judge.DEFAULT_TIMEOUT_S)
    ap.add_argument("--out")
    ap.add_argument("--bundle-dir")
    ap.add_argument("--keep-bundle", action="store_true")
    ap.add_argument("--provider-child-stdin", action="store_true",
                    help=argparse.SUPPRESS)
    args = ap.parse_args()

    try:
        launch = (provider_child_launch.launches_from_stdin({DEFAULT_PROVIDER})
                  .get(DEFAULT_PROVIDER) if args.provider_child_stdin else None)
    except ValueError as error:
        print(f"invalid admitted Codex provider handoff: {error}", file=sys.stderr)
        return 2
    if args.provider_child_stdin and launch is None:
        print("admitted Codex provider handoff is missing Codex", file=sys.stderr)
        return 2

    if args.command == "detect":
        print(json.dumps(detect(launch), indent=2))
        return 0
    if args.command == "review" and args.out:
        try:
            judge_files.require_replaceable(args.out, "review result")
        except judge_files.RefusedProjectFile as error:
            print(f"review refused: {error}", file=sys.stderr)
            return 2

    if not NATIVE_ADMISSION_VERIFIED:
        envelope = _not_run_envelope(args)
    else:
        try:
            envelope = review(args, launch)
        except CleanupError as error:
            print(f"Codex judge input/preparation failed: {type(error.original).__name__}",
                  file=sys.stderr)
            for kind, path in error.retained.items():
                print(f"Cleanup failed for {kind}: {path}. Close applications using "
                      "this directory, then remove it manually.", file=sys.stderr)
            return 2
        except judge_files.RefusedProjectFile as error:
            # Fixed adapter text naming the refused project file; no provider output.
            print(f"review refused: {error}", file=sys.stderr)
            return 2
        except (OSError, RuntimeError, ValueError) as error:
            print(f"Codex judge input/preparation failed: {type(error).__name__}",
                  file=sys.stderr)
            return 2
    text = json.dumps(envelope, indent=2)
    if args.out:
        judge_files.write_text_replacing(args.out, text + "\n", "review result")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
