#!/usr/bin/env python3
import json
import sys

_mode = sys.argv[1] if len(sys.argv) > 1 else "plan"
req = json.loads(sys.stdin.read() or "{}")
mode = req.get("mode") if req.get("mode") != "auto" else "quick_prompt"
input_text = str(req.get("input") or "")
answers = req.get("answers") or {}

if mode == "director_brief" and not answers.get("audience"):
    print(json.dumps({
        "schema": "shellx-cut/generate-storyboard-result/1",
        "status": "needs_input",
        "backend": {"provider": "fixture", "model": "fixture/generate-storyboard"},
        "questions": [{
            "id": "audience",
            "field": "audience",
            "prompt": "Who is the main audience for this video?",
            "choices": ["internal team", "new customers", "existing users"]
        }],
        "storyboard": {
            "schema": "shellx-cut/generate-storyboard/1",
            "storyboard_id": "gsb_fixture_needs_input",
            "mode": "director_brief",
            "status": "needs_input",
            "brief": {"purpose": "launch_video"},
            "brief_meta": {"stated": ["purpose"], "inferred": [], "missing": ["audience"]},
            "scenes": [],
            "validation": {"result": "warn", "warnings": ["audience missing"], "errors": [], "missing_inputs": ["audience"]}
        },
        "warnings": ["audience missing"]
    }))
    raise SystemExit(0)

print(json.dumps({
    "schema": "shellx-cut/generate-storyboard-result/1",
    "status": "completed",
    "backend": {"provider": "fixture", "model": "fixture/generate-storyboard"},
    "questions": [],
    "storyboard": {
        "schema": "shellx-cut/generate-storyboard/1",
        "storyboard_id": "gsb_fixture_launch",
        "mode": mode,
        "status": "valid",
        "brief": {
            "purpose": "product_intro",
            "audience": answers.get("audience", "product teams"),
            "platform": "youtube",
            "duration_ms": 12000,
            "aspect": "16:9",
            "fps": 30,
            "core_message": input_text[:120],
            "tone": "clear, operational, proof-first",
            "asset_strategy": "generated"
        },
        "brief_meta": {
            "stated": ["purpose", "core_message"],
            "inferred": ["platform", "duration_ms", "tone"],
            "missing": []
        },
        "scenes": [
            {
                "scene_id": "s01",
                "index": 1,
                "role": "hook",
                "range_ms": [0, 4000],
                "source": "generate_template",
                "template_id": "builtin.title-card.episode",
                "params": {"title": "Launch Notes"},
                "screen_text": "Launch Notes",
                "narration": "",
                "asset_refs": [],
                "missing_assets": [],
                "motion": "title card reveal",
                "transition_in": "none",
                "transition_out": "hard_cut",
                "evidence": {"needs_preview": True, "needs_state_check": True, "needs_contact_sheet": False}
            },
            {
                "scene_id": "s02",
                "index": 2,
                "role": "proof",
                "range_ms": [4000, 8000],
                "source": "generate_template",
                "template_id": "builtin.lower-third.clean",
                "params": {"name": "Marta", "accent": "#33CC99"},
                "screen_text": "Marta",
                "narration": "",
                "asset_refs": [],
                "missing_assets": [],
                "motion": "lower third reveal",
                "transition_in": "hard_cut",
                "transition_out": "hard_cut",
                "evidence": {"needs_preview": True, "needs_state_check": True, "needs_contact_sheet": False}
            },
            {
                "scene_id": "s03",
                "index": 3,
                "role": "cta",
                "range_ms": [8000, 12000],
                "source": "generate_template",
                "template_id": "builtin.callout.arrow-label",
                "params": {"label": "Review the edit", "color": "#FFD24A"},
                "screen_text": "Review the edit",
                "narration": "",
                "asset_refs": [],
                "missing_assets": [],
                "motion": "callout pop",
                "transition_in": "hard_cut",
                "transition_out": "none",
                "evidence": {"needs_preview": True, "needs_state_check": True, "needs_contact_sheet": False}
            }
        ],
        "validation": {"result": "pass", "warnings": [], "errors": [], "missing_inputs": []},
        "next": {"recommended_policy": "preview", "actions": []}
    },
    "warnings": []
}))
