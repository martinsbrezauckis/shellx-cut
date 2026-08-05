#!/usr/bin/env python3
"""Deterministic comment.draft fixture for the full-coverage release gate."""

from __future__ import annotations

import json
import sys
from datetime import datetime, timezone


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def main() -> int:
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    if command == "detect":
        print(json.dumps({
            "fixture": {"found": True, "adapter": "implemented", "path": __file__},
        }))
        return 0
    if command != "draft":
        print("usage: comment-draft-adapter.py draft|detect", file=sys.stderr)
        return 2

    req = json.load(sys.stdin)
    comment = req.get("comment") or {}
    at_ms = int(comment.get("at_ms") or 500)
    comment_id = str(comment.get("id") or "comment")
    label = f"FCV applied {comment_id}"
    envelope = {
        "schema": "shellx-cut/comment-draft/1",
        "ts": now_iso(),
        "status": "completed",
        "backend": {"provider": "fixture", "model": "release-gate/deterministic"},
        "draft": {
            "verbs": [
                {"verb": "edit.add_marker", "args": {"at_ms": at_ms, "label": label}},
            ],
            "rationale": "Deterministic release-gate draft: add a timeline marker so comment.apply has a real edit to execute.",
            "confidence": 1.0,
        },
        "reason": None,
    }
    print(json.dumps(envelope))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
