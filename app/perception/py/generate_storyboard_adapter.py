#!/usr/bin/env python3
"""generate_storyboard_adapter.py - ShellX Cut Generate STORYBOARD planner.

Called by cutd's `generate.storyboard` verb as:

    python generate_storyboard_adapter.py plan   (request JSON on stdin)

The request (schema shellx-cut/generate-storyboard-request/1) carries the
user's input (prompt / director brief / script / media context), mode, prior
director `answers`, the FULL Generate template catalog, an `agent` preference
(auto|claude|codex|grok), an `agents` map of RESOLVED CLI paths (cutd resolves
those via its gen.rs ladder), and the verb's `timeout_ms` budget.

The shim routes the multi-scene planning job to the user's OWN local CLI
subscription agent (Claude Code / Codex / Grok) and prints ONE envelope JSON
on stdout:

    {"schema": "shellx-cut/generate-storyboard-result/1",
     "status": "completed" | "needs_input" | "not_run" | "error",
     "backend": {"provider": "...", "model": "..."} | null,
     "storyboard": {schema shellx-cut/generate-storyboard/1 ...} | null,
     "questions": [ {field, question, choices?} at most one ],
     "reason": "..." (only when not completed/needs_input),
     "warnings": [...]}

The IR contract and the one-question director protocol live in the shipped
agent skills (request.skill_path):
    skill/shellx-cut/craft/generate-storyboard-planning.md
    skill/shellx-cut/craft/generate-director-questioning.md
This shim embeds the same rules in its instruction so the planning CLI needs
no repo access. cutd re-validates the IR authoritatively after us.

Honest-degradation contract (mirrors diarize/dub): no CLI agent available ->
status:not_run with a reason; output that fails local validation gets ONE
retry carrying the errors, then status:error. NEVER a fabricated storyboard.
Stdlib-only: runs under the bundled perception venv or any Python >= 3.8.

Security posture: claude runs with --tools "" (no tool use), codex runs in the
read-only sandbox with --ephemeral, grok runs single-turn -p.
"""

import json
import os
import subprocess
import sys
import tempfile
from contextlib import suppress

SCHEMA = "shellx-cut/generate-storyboard-result/1"
STORYBOARD_SCHEMA = "shellx-cut/generate-storyboard/1"
AUTO_ORDER = ["claude", "codex", "grok"]
DEFAULT_TIMEOUT_MS = 120_000
GROK_ARGV_PROMPT_LIMIT = 26_000
MODES = ("quick_prompt", "director_brief", "script", "existing_media")
SOURCES = (
    "generate_template",
    "existing_media",
    "assemble_slot",
    "generated_asset",
    "caption",
    "audio",
)
BRIEF_FIELDS = (
    "purpose",
    "audience",
    "platform",
    "duration",
    "core message",
    "asset strategy",
    "tone",
    "constraints",
)


def emit(status, storyboard=None, questions=None, backend=None, reason=None, warnings=None):
    out = {
        "schema": SCHEMA,
        "status": status,
        "backend": backend,
        "storyboard": storyboard,
        "questions": questions or [],
        "warnings": warnings or [],
    }
    if reason is not None:
        out["reason"] = reason
    sys.stdout.write(json.dumps(out))
    sys.stdout.flush()
    sys.exit(0)


def read_request():
    try:
        raw = sys.stdin.read()
        req = json.loads(raw)
        if not isinstance(req, dict):
            raise ValueError("request is not an object")
        return req
    except Exception as e:  # noqa: BLE001 - single honest error surface
        emit("error", reason=f"adapter could not parse the request JSON: {e}")


def pick_agents(req):
    """Ordered [(name, path)] candidates, or emit not_run. agent:"auto"
    returns EVERY installed CLI so hard failures fall through; an explicit
    choice returns exactly that one - no silent substitution."""
    pref = (req.get("agent") or "auto").strip() or "auto"
    agents = req.get("agents") or {}
    if not isinstance(agents, dict):
        agents = {}
    order = AUTO_ORDER if pref == "auto" else [pref]
    found = []
    probed = []
    for name in order:
        path = agents.get(name)
        if isinstance(path, str) and path:
            found.append((name, path))
        else:
            probed.append(name)
    if found:
        return found
    emit(
        "not_run",
        reason=(
            "no local CLI agent available for storyboard planning "
            f"(probed: {', '.join(probed)}). Install Claude Code, Codex, or "
            "Grok CLI, or pass agent:\"claude|codex|grok\" for one that is "
            "installed - honest not_run, no fabricated storyboard"
        ),
    )


def compact_catalog(templates):
    out = []
    for t in templates or []:
        entry = {
            "id": t.get("id"),
            "kind": t.get("kind"),
            "title": t.get("title"),
            "summary": t.get("summary"),
        }
        params = {}
        for name, spec in (t.get("params") or {}).items():
            if not isinstance(spec, dict):
                continue
            p = {"type": spec.get("type")}
            if spec.get("required"):
                p["required"] = True
            if spec.get("default") is not None:
                p["default"] = spec.get("default")
            if spec.get("enum"):
                p["enum"] = spec.get("enum")
            if spec.get("description"):
                p["hint"] = str(spec["description"]).split(". ")[0][:160]
            params[name] = p
        entry["params"] = params
        if t.get("available") is False:
            entry["available"] = False
        out.append(entry)
    return out


def build_prompt(req, errors=None):
    mode = req.get("mode") or "auto"
    answers = req.get("answers") or {}
    context = req.get("context") or {}
    catalog = compact_catalog(req.get("templates"))
    lines = [
        "You are the ShellX Cut storyboard DIRECTOR-PLANNER. ShellX Cut is a",
        "video editor; a Generate Storyboard is a typed multi-scene plan the",
        "editor can validate, preview per scene, and insert as real timeline",
        "operations. Do not use any tools.",
        "",
        "Output ONLY one strict JSON object - no prose, no markdown fences:",
        '{"storyboard": {...IR below...}, "questions": []}',
        "",
        "Storyboard IR (every rule is machine-validated):",
        f'- "schema": "{STORYBOARD_SCHEMA}" (exactly).',
        '- "storyboard_id": short kebab-case slug for this plan.',
        f'- "mode": one of {list(MODES)}. Echo the request mode; when the',
        '  request mode is "auto", pick the best fit.',
        '- "status": "valid" for a complete plan, "needs_input" when you must',
        "  ask the director question described below.",
        '- "scenes": ordered array; each scene has:',
        '    "scene_id" (unique slug), "index" (1-based, ascending),',
        '    "role" (short label: hook, intro, lower-third, point-1, cta,',
        "    outro, ...),",
        f'    "source" (one of {list(SOURCES)}),',
        '    "range_ms" ([start_ms, end_ms], end > start, scenes contiguous',
        "    from 0, typical scene 2000-8000 ms; honor a stated duration,",
        "    else target 15000-30000 ms total),",
        '    plus for source "generate_template": "template_id" (a REAL id',
        '    from the catalog) and "params" per that template\'s spec;',
        '    for source "assemble_slot": "query" describing the existing',
        "    media needed - describe the need, never claim a match exists.",
        '- Templates marked "available": false cannot render on THIS machine',
        "  (they need the separate ShellX Motion app) - NEVER use them in a scene.",
        '- Prefer "generate_template" scenes (they materialize); use',
        '  "assemble_slot" for footage the user must already have.',
        '- "brief_meta": {"stated": [], "inferred": [], "missing": []}',
        f"  classifying these fields: {list(BRIEF_FIELDS)}.",
        '- "missing_assets": [] (names of assets the plan needs but lacks).',
        '- "validation": {"missing_inputs": []} (same strings as',
        "  brief_meta.missing).",
        "- Never invent preview or insert evidence; you only PLAN.",
        "",
        "Director-question protocol:",
        "- Ask at most ONE question, and only when the highest-value missing",
        f"  brief field (priority order: {', '.join(BRIEF_FIELDS)}) would",
        "  materially change the plan.",
        '- To ask: storyboard.status = "needs_input" AND top-level',
        '  "questions": [{"field": "...", "question": "...",',
        '  "choices": ["...", ...]?}] with exactly one entry.',
        '- NEVER ask in quick_prompt mode: plan with inferred values and list',
        "  them in brief_meta.inferred / warnings instead.",
        "- Never ask about a field already covered by the request or the",
        "  answers object below.",
    ]
    if errors:
        lines += [
            "",
            "Your previous answer failed validation with these errors - fix",
            "them and return the corrected JSON object only:",
        ] + [f"  - {e}" for e in errors]
    lines += [
        "",
        f"Request mode: {mode}",
        "",
        "Template catalog (JSON):",
        json.dumps(catalog, ensure_ascii=False),
        "",
        "Director input:",
        str(req.get("input") or ""),
    ]
    if answers:
        lines += ["", "Director answers so far (JSON):", json.dumps(answers, ensure_ascii=False)]
    if context:
        lines += ["", "Caller context (JSON):", json.dumps(context, ensure_ascii=False)]
    return "\n".join(lines)


def extract_json(text):
    """First balanced {...} object in `text` (fences and prose tolerated)."""
    if not isinstance(text, str):
        return None
    start = text.find("{")
    while start != -1:
        depth = 0
        in_str = False
        esc = False
        for i in range(start, len(text)):
            c = text[i]
            if in_str:
                if esc:
                    esc = False
                elif c == "\\":
                    esc = True
                elif c == '"':
                    in_str = False
                continue
            if c == '"':
                in_str = True
            elif c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    try:
                        return json.loads(text[start : i + 1])
                    except Exception:  # noqa: BLE001
                        break
        start = text.find("{", start + 1)
    return None


def spawn_cli(name, path, prompt, timeout_s):
    """Run the CLI; return (answer_text, model_hint, error_reason)."""
    try:
        if name == "claude":
            proc = subprocess.run(
                [path, "-p", "--output-format", "json", "--tools", ""],
                input=prompt.encode("utf-8"),
                capture_output=True,
                timeout=timeout_s,
            )
            raw = proc.stdout.decode("utf-8", "replace")
            if proc.returncode != 0:
                tail = raw.strip()[-400:] or proc.stderr.decode("utf-8", "replace").strip()[-400:]
                return None, None, f"claude exited {proc.returncode}: {tail}"
            envelope = extract_json(raw)
            if isinstance(envelope, dict) and isinstance(envelope.get("result"), str):
                return envelope["result"], envelope.get("model"), None
            return raw, None, None
        if name == "codex":
            with tempfile.NamedTemporaryFile(
                mode="r", suffix=".txt", delete=False, encoding="utf-8"
            ) as last:
                last_path = last.name
            try:
                proc = subprocess.run(
                    [
                        path,
                        "exec",
                        "-",
                        "--skip-git-repo-check",
                        "--ephemeral",
                        "-s",
                        "read-only",
                        "--color",
                        "never",
                        "-o",
                        last_path,
                    ],
                    input=prompt.encode("utf-8"),
                    capture_output=True,
                    timeout=timeout_s,
                )
                if proc.returncode != 0:
                    tail = proc.stderr.decode("utf-8", "replace").strip()[-400:]
                    return None, None, f"codex exited {proc.returncode}: {tail}"
                with open(last_path, "r", encoding="utf-8", errors="replace") as f:
                    return f.read(), None, None
            finally:
                with suppress(FileNotFoundError):
                    os.unlink(last_path)
        if name == "grok":
            if len(prompt) > GROK_ARGV_PROMPT_LIMIT:
                return None, None, "prompt too large for grok argv transport"
            proc = subprocess.run(
                [path, "-p", prompt, "--output-format", "json"],
                capture_output=True,
                timeout=timeout_s,
            )
            raw = proc.stdout.decode("utf-8", "replace")
            if proc.returncode != 0:
                tail = raw.strip()[-400:] or proc.stderr.decode("utf-8", "replace").strip()[-400:]
                return None, None, f"grok exited {proc.returncode}: {tail}"
            envelope = extract_json(raw)
            if isinstance(envelope, dict):
                for key in ("response", "result", "text", "content"):
                    if isinstance(envelope.get(key), str):
                        return envelope[key], envelope.get("model"), None
                if "storyboard" in envelope:
                    return json.dumps(envelope), None, None
            return raw, None, None
        return None, None, f"unknown agent '{name}'"
    except subprocess.TimeoutExpired:
        return None, None, f"{name} timed out after {int(timeout_s)}s"
    except OSError as e:
        return None, None, f"{name} spawn failed: {e}"


def validate_storyboard(sb, req, questions):
    """Mirror cutd's validate_generate_storyboard closely enough that a pass
    here almost always passes there; cutd stays authoritative."""
    errors = []
    if not isinstance(sb, dict):
        return ["storyboard must be a JSON object"]
    if sb.get("schema") != STORYBOARD_SCHEMA:
        errors.append(f"storyboard.schema must be {STORYBOARD_SCHEMA}")
    if sb.get("mode") not in MODES:
        errors.append(f"storyboard.mode must be one of {list(MODES)}")
    status = sb.get("status")
    if status not in ("draft", "needs_input", "valid", "previewed", "inserted"):
        errors.append("storyboard.status must be draft, needs_input, valid, previewed, or inserted")
    if status == "needs_input" and not questions:
        errors.append('status "needs_input" requires exactly one questions[] entry')
    if questions and len(questions) > 1:
        errors.append("questions must contain at most one entry")
    scenes = sb.get("scenes")
    if not isinstance(scenes, list):
        return errors + ["storyboard.scenes must be an array"]
    ids = {t.get("id") for t in (req.get("templates") or [])}
    unavailable = {t.get("id") for t in (req.get("templates") or []) if t.get("available") is False}
    seen_ids = set()
    for idx, scene in enumerate(scenes):
        label = f"scene[{idx}]"
        if not isinstance(scene, dict):
            errors.append(f"{label} must be an object")
            continue
        sid = scene.get("scene_id")
        if not isinstance(sid, str) or not sid.strip():
            errors.append(f"{label}.scene_id is required")
        elif sid in seen_ids:
            errors.append(f"{label}.scene_id '{sid}' is duplicated")
        else:
            seen_ids.add(sid)
        index = scene.get("index")
        if not isinstance(index, int) or index <= 0:
            errors.append(f"{label}.index must be a positive integer")
        role = scene.get("role")
        if not isinstance(role, str) or not role.strip():
            errors.append(f"{label}.role is required")
        source = scene.get("source")
        if source not in SOURCES:
            errors.append(f"{label}.source has unsupported value '{source}'")
        rng = scene.get("range_ms")
        if (
            not isinstance(rng, list)
            or len(rng) != 2
            or not all(isinstance(v, int) and v >= 0 for v in rng)
            or rng[1] <= rng[0]
        ):
            errors.append(f"{label}.range_ms must be [start,end] with end > start")
        if source == "generate_template":
            tid = scene.get("template_id")
            if not isinstance(tid, str) or not tid.strip():
                errors.append(f"{label}.template_id is required for generate_template scenes")
            elif tid not in ids:
                errors.append(f"unknown generate template '{tid}'")
            elif tid in unavailable:
                errors.append(
                    f"{label}: template '{tid}' requires the separate ShellX Motion app, "
                    "which is not installed on this machine - use an available template"
                )
        if source == "assemble_slot":
            q = scene.get("query")
            if not isinstance(q, str) or not q.strip():
                errors.append(f"{label}.query is required for assemble_slot scenes")
    return errors


def main():
    if len(sys.argv) < 2 or sys.argv[1] != "plan":
        emit("error", reason="usage: generate_storyboard_adapter.py plan (request JSON on stdin)")
    req = read_request()
    candidates = pick_agents(req)

    budget_ms = req.get("timeout_ms")
    if not isinstance(budget_ms, int) or budget_ms <= 0:
        budget_ms = DEFAULT_TIMEOUT_MS
    total_s = max(10.0, budget_ms / 1000.0 - 5.0)
    slice_s = max(10.0, total_s / len(candidates))

    warnings = []
    backend = None
    fails = []
    for name, path in candidates:
        backend = {"provider": name, "model": None}
        errors = None
        for attempt, share in ((1, 0.6), (2, 0.4)):
            prompt = build_prompt(req, errors=errors)
            answer, model, fail = spawn_cli(name, path, prompt, slice_s * share)
            if fail:
                # HARD failure (spawn/auth/exit/timeout): fall through to the
                # next installed CLI instead of dying on the first.
                fails.append(f"{name}: {fail}")
                warnings.append(f"agent {name} failed, trying next: {fail}"[:200])
                break
            if model:
                backend["model"] = model
            wrapper = extract_json(answer)
            storyboard = None
            questions = []
            if isinstance(wrapper, dict):
                storyboard = wrapper.get("storyboard")
                if storyboard is None and wrapper.get("schema") == STORYBOARD_SCHEMA:
                    storyboard = wrapper  # model returned the IR bare - accept it
                raw_q = wrapper.get("questions")
                if isinstance(raw_q, list):
                    questions = [q for q in raw_q if isinstance(q, dict)]
            if storyboard is None:
                errors = ["answer contained no storyboard object"]
            else:
                errors = validate_storyboard(storyboard, req, questions)
            if not errors:
                if attempt == 2:
                    warnings.append("storyboard accepted on retry after validation feedback")
                status = (
                    "needs_input"
                    if (storyboard.get("status") == "needs_input" or questions)
                    else "completed"
                )
                emit(
                    status,
                    storyboard=storyboard,
                    questions=questions[:1],
                    backend=backend,
                    warnings=warnings,
                )
        else:
            emit(
                "error",
                backend=backend,
                reason="agent storyboard failed validation after retry: "
                + "; ".join(errors or []),
            )
    emit(
        "error",
        backend=backend,
        reason="every available CLI agent failed: " + " | ".join(fails),
    )


if __name__ == "__main__":
    main()
