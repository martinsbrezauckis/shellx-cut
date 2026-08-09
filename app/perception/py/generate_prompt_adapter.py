#!/usr/bin/env python3
"""generate_prompt_adapter.py - ShellX Cut Generate PROMPT planner adapter.

Called by cutd's `generate.from_prompt` verb as:

    python generate_prompt_adapter.py plan     (request JSON on stdin)

The request (schema shellx-cut/generate-prompt-request/1) carries the user's
prompt, the FULL Generate template catalog, project geometry, an `agent`
preference (auto|claude|codex|grok), an `agents` map of RESOLVED CLI paths
(cutd resolves those via its gen.rs ladder - this script never guesses
platform install dirs), and the verb's `timeout_ms` budget.

The shim routes the planning job to the user's OWN local CLI subscription
agent (Claude Code / Codex / Grok) - no hosted API, no key, no extra cost -
and prints ONE envelope JSON on stdout:

    {"schema": "shellx-cut/generate-plan/1",
     "status": "completed" | "not_run" | "error",
     "backend": {"provider": "...", "model": "..."} | null,
     "plan": {"template_id": "...", "params": {...}, "at_ms": ...} | null,
     "reason": "..." (only when not completed),
     "warnings": [...]}

Honest-degradation contract (mirrors diarize/dub): no CLI agent available ->
status:not_run with a reason; agent output that fails local validation gets
ONE retry carrying the validation errors, then status:error. The shim NEVER
fabricates a plan. Stdlib-only on purpose: it must run under the bundled
perception venv OR any system Python >= 3.8 (CUTD_ADAPTER_PYTHON).

Security posture: claude runs with --tools "" (no tool use), codex runs in the
read-only sandbox with --ephemeral, grok runs single-turn -p. The planning
call is pure text -> JSON; the agent gets no filesystem or shell authority
from us beyond what the CLI itself grants a plain prompt.
"""

import json
import os
import subprocess
import sys
import tempfile
from contextlib import suppress

SCHEMA = "shellx-cut/generate-plan/1"
AUTO_ORDER = ["claude", "codex", "grok"]
DEFAULT_TIMEOUT_MS = 120_000
# Windows CreateProcess command lines cap at ~32k chars; grok takes the prompt
# as an argv item, so keep headroom for the path + flags.
GROK_ARGV_PROMPT_LIMIT = 26_000


def emit(status, plan=None, backend=None, reason=None, warnings=None):
    out = {
        "schema": SCHEMA,
        "status": status,
        "backend": backend,
        "plan": plan,
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
    """Return the ordered [(name, path)] candidates, or emit not_run.

    agent:"auto" returns EVERY installed CLI in preference order so the caller
    can fall through when one hard-fails (installed-but-unauthenticated claude
    must not kill the feature for a user with a working codex). An explicit
    agent choice returns exactly that one - no silent substitution.
    """
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
            "no local CLI agent available for Generate planning "
            f"(probed: {', '.join(probed)}). Install Claude Code, Codex, or "
            "Grok CLI, or pass agent:\"claude|codex|grok\" for one that is "
            "installed - honest not_run, no fabricated plan"
        ),
    )


def compact_catalog(templates, include_params=True):
    """One small dict per template: enough for the model to choose + fill."""
    out = []
    for t in templates or []:
        entry = {
            "id": t.get("id"),
            "kind": t.get("kind"),
            "title": t.get("title"),
            "summary": t.get("summary"),
        }
        if include_params:
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
                    # First sentence is enough for planning.
                    p["hint"] = str(spec["description"]).split(". ")[0][:160]
                params[name] = p
            entry["params"] = params
        if t.get("available") is False:
            entry["available"] = False
        out.append(entry)
    return out


def build_prompt(req, errors=None):
    geometry = req.get("geometry") or {}
    catalog = compact_catalog(req.get("templates"))
    hint = req.get("template_id")
    context = req.get("context") or {}
    lines = [
        "You are the ShellX Cut Generate planner. ShellX Cut is a video",
        "editor; Generate templates are typed, editable visual elements",
        "(titles, lower thirds, cards, shapes, motion templates).",
        "",
        "Task: pick exactly ONE template from the catalog below and fill its",
        "params so it satisfies the user's request. Do not use any tools.",
        "Output ONLY a strict JSON object - no prose, no markdown fences:",
        '{"template_id": "<catalog id>", "params": {<param>: <value>, ...},'
        ' "at_ms": null}',
        "",
        "Rules:",
        "- template_id MUST be one of the catalog ids, matched to the",
        "  request's intent (kind + summary).",
        "- Templates marked \"available\": false cannot render on THIS",
        "  machine (they need the separate ShellX Motion app) - NEVER choose",
        "  them.",
        "- params must satisfy each param spec (type, enum, ranges). Omit a",
        "  param to accept its default; set it only when the request implies",
        "  a concrete value. Keep user-provided text verbatim (any language).",
        "- at_ms stays null unless the request names an explicit time.",
    ]
    if hint:
        lines.append(f"- The caller REQUIRES template_id '{hint}'; use it.")
    if errors:
        lines += [
            "",
            "Your previous answer failed validation with these errors - fix",
            "them and return the corrected JSON object only:",
        ] + [f"  - {e}" for e in errors]
    lines += [
        "",
        f"Project geometry: {geometry.get('width')}x{geometry.get('height')}",
        "",
        "Template catalog (JSON):",
        json.dumps(catalog, ensure_ascii=False),
        "",
        "User request:",
        str(req.get("prompt") or ""),
    ]
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
                # Shrink the catalog rather than overflow argv on Windows.
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
                # The envelope may already BE the plan object.
                if "template_id" in envelope:
                    return json.dumps(envelope), None, None
            return raw, None, None
        return None, None, f"unknown agent '{name}'"
    except subprocess.TimeoutExpired:
        return None, None, f"{name} timed out after {int(timeout_s)}s"
    except OSError as e:
        return None, None, f"{name} spawn failed: {e}"


def validate_plan(plan, req):
    errors = []
    if not isinstance(plan, dict):
        return ["plan must be a JSON object"]
    template_id = plan.get("template_id")
    ids = {t.get("id") for t in (req.get("templates") or [])}
    unavailable = {t.get("id") for t in (req.get("templates") or []) if t.get("available") is False}
    if not isinstance(template_id, str) or not template_id.strip():
        errors.append("plan.template_id is required")
    elif template_id not in ids:
        errors.append(f"unknown generate template '{template_id}'")
    elif template_id in unavailable:
        errors.append(
            f"template '{template_id}' requires the separate ShellX Motion app, "
            "which is not installed on this machine - choose an available template"
        )
    hint = req.get("template_id")
    if hint and isinstance(template_id, str) and template_id != hint:
        errors.append(f"plan.template_id must be the requested '{hint}'")
    params = plan.get("params")
    if params is not None and not isinstance(params, dict):
        errors.append("plan.params must be an object")
    if isinstance(params, dict) and isinstance(template_id, str):
        spec = {}
        for t in req.get("templates") or []:
            if t.get("id") == template_id:
                spec = t.get("params") or {}
                break
        for name in params:
            if spec and name not in spec:
                errors.append(f"plan.params.{name} is not a param of '{template_id}'")
    at_ms = plan.get("at_ms")
    if at_ms is not None and (not isinstance(at_ms, int) or at_ms < 0):
        errors.append("plan.at_ms must be a non-negative integer or null")
    return errors


def main():
    if len(sys.argv) < 2 or sys.argv[1] != "plan":
        emit("error", reason="usage: generate_prompt_adapter.py plan (request JSON on stdin)")
    req = read_request()
    candidates = pick_agents(req)

    budget_ms = req.get("timeout_ms")
    if not isinstance(budget_ms, int) or budget_ms <= 0:
        budget_ms = DEFAULT_TIMEOUT_MS
    # Leave cutd's outer timeout a 5s margin so IT never has to kill us.
    total_s = max(10.0, budget_ms / 1000.0 - 5.0)
    # Each CANDIDATE gets an equal slice; each slice funds one attempt + one
    # validation-feedback retry.
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
            plan = extract_json(answer)
            if plan is None:
                errors = ["answer contained no parseable JSON object"]
            else:
                errors = validate_plan(plan, req)
            if not errors:
                if attempt == 2:
                    warnings.append("plan accepted on retry after validation feedback")
                emit("completed", plan=plan, backend=backend, warnings=warnings)
        else:
            emit(
                "error",
                backend=backend,
                reason="agent plan failed validation after retry: " + "; ".join(errors or []),
            )
    emit(
        "error",
        backend=backend,
        reason="every available CLI agent failed: " + " | ".join(fails),
    )


if __name__ == "__main__":
    main()
