#!/usr/bin/env python3
import json
import sys

_mode = sys.argv[1] if len(sys.argv) > 1 else "plan"
req = json.loads(sys.stdin.read() or "{}")
prompt = str(req.get("prompt") or "")
name = "Prompt UI"
if "marta" in prompt.lower():
    name = "Marta Prompt"

print(json.dumps({
    "schema": "shellx-cut/generate-plan/1",
    "status": "completed",
    "backend": {"provider": "fixture", "model": "fixture/generate-plan"},
    "plan": {
        "template_id": req.get("template_id") or "builtin.lower-third.clean",
        "params": {"name": name, "accent": "#33CC99", "duration_ms": 4000},
        "at_ms": req.get("at_ms", 1000),
        "rationale": "fixture lower third",
        "confidence": 0.91,
        "alternatives": []
    },
    "warnings": []
}))
