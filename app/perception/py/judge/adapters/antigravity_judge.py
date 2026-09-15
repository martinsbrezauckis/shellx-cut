#!/usr/bin/env python3
"""Antigravity render-judge admission boundary.

The ``agy`` binary can be discovered without accessing provider credentials,
but this release has not verified a restricted tool/file boundary for using it
as a render judge. A review request is therefore recorded as ``not_run``
before it reads a render, extracts frames, builds a prompt, or starts a model.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys

_JUDGE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _JUDGE_DIR)
_PY_ROOT = os.path.dirname(_JUDGE_DIR)
sys.path.insert(0, _PY_ROOT)
import judge  # noqa: E402
import provider_child_launch  # noqa: E402

_ADAPTERS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _ADAPTERS_DIR)
import codex_judge  # noqa: E402  (shared pure structured-output decoder)

ADAPTER_NAME = "cli"
DEFAULT_PROVIDER = "antigravity"
DEFAULT_CLI_MODEL = ""
AVAILABILITY_REASON = (
    "render judge unavailable until restricted tool/file access is verified")

# Preserve the established shared pure decoder import for consumers that use
# the adapter module; render-judge admission does not invoke it.
_extract_json = codex_judge._extract_json


def detect(launch: dict | None = None) -> dict:
    """Report ``agy`` binary presence separately from render-judge admission.

    The only subprocess is ``agy --version``. Authentication remains
    provider-owned and is not inspected by this adapter.
    """
    path = launch["executable"] if launch else shutil.which("agy")
    entry: dict = {
        "provider": DEFAULT_PROVIDER,
        "binary": "agy",
        "found": bool(path),
        "judge_ready": False,
        "availability_reason": AVAILABILITY_REASON,
        "path": path,
        "adapter": "implemented (render-judge admission pending)",
    }
    if path:
        try:
            completed = subprocess.run(
                provider_child_launch.command(launch, ["--version"]) if launch else [path, "--version"],
                capture_output=True, text=True, encoding="utf-8", timeout=15,
                env=provider_child_launch.environment(launch) if launch else None)
            entry["version"] = completed.stdout.strip() or completed.stderr.strip()
        except (subprocess.TimeoutExpired, OSError) as error:
            entry["version_error"] = str(error)
        entry["logged_in"] = None
    return entry


def _not_run_envelope(args: argparse.Namespace) -> dict:
    """Create a terminal admission receipt without inspecting a render."""
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


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["review", "detect"])
    ap.add_argument("--render")
    ap.add_argument("--perception")
    ap.add_argument("--intent")
    ap.add_argument("--cli-model", default=DEFAULT_CLI_MODEL)
    ap.add_argument("--agy-bin", default="agy")
    ap.add_argument("--mode", default="global", choices=["global", "window"])
    ap.add_argument("--windows")
    ap.add_argument("--window-reason")
    ap.add_argument("--fps", type=float)
    ap.add_argument("--max-frames", type=int)
    ap.add_argument("--width", type=int)
    ap.add_argument("--timeout", type=int)
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
        print(f"invalid admitted Antigravity provider handoff: {error}", file=sys.stderr)
        return 2
    if args.provider_child_stdin and launch is None:
        print("admitted Antigravity provider handoff is missing Antigravity", file=sys.stderr)
        return 2

    if args.command == "detect":
        print(json.dumps(detect(launch), indent=2))
        return 0

    envelope = _not_run_envelope(args)
    text = json.dumps(envelope, indent=2)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as output:
            output.write(text + "\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
