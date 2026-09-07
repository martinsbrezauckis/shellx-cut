#!/usr/bin/env python3
"""Codex render-judge admission boundary.

Codex can be detected without touching provider-owned authentication state, but
this release does not have verified restricted tool/file access for a render
judge. Review requests therefore produce a terminal, honest ``not_run``
envelope before render inspection or a model subprocess.

The strict-schema and structured-output helpers remain here because other
judge adapters use them as their pure shared implementation.
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
import judge  # noqa: E402

ADAPTER_NAME = "cli"
DEFAULT_PROVIDER = "codex"
DEFAULT_CLI_MODEL = ""
AVAILABILITY_REASON = (
    "render judge unavailable until restricted tool/file access is verified")


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


def detect() -> dict:
    """Report Codex binary presence separately from render-judge admission.

    Detection runs only ``codex --version``. It never reads provider-owned
    authentication or configuration state and never starts a model turn.
    """
    path = shutil.which("codex")
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
            completed = subprocess.run(
                [path, "--version"], capture_output=True, text=True,
                encoding="utf-8", timeout=15)
            entry["version"] = completed.stdout.strip() or completed.stderr.strip()
        except (subprocess.TimeoutExpired, OSError) as error:
            entry["version_error"] = str(error)
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


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["review", "detect"])
    ap.add_argument("--render")
    ap.add_argument("--perception")
    ap.add_argument("--intent")
    ap.add_argument("--cli-model", default=DEFAULT_CLI_MODEL)
    ap.add_argument("--codex-bin", default="codex")
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
    args = ap.parse_args()

    if args.command == "detect":
        print(json.dumps(detect(), indent=2))
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
