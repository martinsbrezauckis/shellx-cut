#!/usr/bin/env python3
"""Deterministic verify.judge fixture for the full-coverage release gate."""

from __future__ import annotations

import json
import sys
from datetime import datetime, timezone
from pathlib import Path


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def arg_value(flag: str) -> str | None:
    try:
        return sys.argv[sys.argv.index(flag) + 1]
    except (ValueError, IndexError):
        return None


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1] != "review":
        print("usage: judge-adapter.py review --render <path> --out <path>", file=sys.stderr)
        return 2

    out = arg_value("--out")
    render = arg_value("--render")
    perception = arg_value("--perception")
    provider = arg_value("--provider") or "auto"
    envelope = {
        "schema": "shellx-cut/judge-review/1",
        "ts": now_iso(),
        "backend": {
            "name": "cli",
            "provider": "fixture",
            "requested_provider": provider,
            "watched": True,
            "listened": False,
        },
        "status": "completed",
        "not_run_reason": None,
        "review": {
            "verdict": "pass",
            "confidence": 0.99,
            "summary": "Deterministic release-gate judge review.",
            "issues": [],
            "cannot_assess": ["audio quality - fixture is vision-only"],
        },
        "stub_args": {
            "command": sys.argv[1],
            "render": render,
            "perception": perception,
            "bundle_dir": arg_value("--bundle-dir"),
            "provider": provider,
        },
    }
    text = json.dumps(envelope)
    if out:
        Path(out).parent.mkdir(parents=True, exist_ok=True)
        Path(out).write_text(text, encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
