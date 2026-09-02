#!/usr/bin/env python3
"""No-quota correctness tests for the bundled ShellX Cut judge ladder.

Deterministic, no-quota, no-CLI regression tests for the judge adapters. They
pin the honesty guarantees that must not regress: a real verdict still
validates and a real failure is never reclassified.

Covered:
  - cli_judge failure diagnostics retain labelled stdout/stderr evidence and
        do not infer a root cause from an error-envelope keyword.
  - judge.validate_review enforces REVIEW_SCHEMA value constraints
        (kind/severity enums, confidence in [0,1], at_ms/end_ms integer coercion)
        and the enums are DERIVED from REVIEW_SCHEMA (stay in sync).
  - codex_judge._extract_json recovers the real verdict from a prose
        preamble that itself contains braces (returns the validating object,
        not the first {...}); unchanged on already-clean JSON.
  - codex_judge.detect is limited to binary/version detection and never opens
        or JSON-parses Codex's canonical authentication content.
  - cli_judge.detect_frame_read_failure does not reclassify a legitimate all-black
        `fail` verdict (populated issues / high confidence / no OS token) to
        infrastructure, while still catching a real EACCES sweep.
  - cli_judge post-filter redacts or strips audio claims (music, audio track,
        drops out, ...) from a vision-only judge, without over-redacting a
        genuine visual finding about on-screen text.
  - judge.digest_perception skips a malformed-but-tagged span instead of
        raising an uncaught KeyError.

Run: python3 scripts/public-tests/judge-adapter-validation.py
"""

from __future__ import annotations

import ast
import os
import sys
import types
from unittest import mock

_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_JUDGE = os.path.join(_ROOT, "app", "perception", "py", "judge")
sys.path.insert(0, os.path.join(_JUDGE, "adapters"))
sys.path.insert(0, _JUDGE)

import judge          # noqa: E402
import codex_judge    # noqa: E402
import cli_judge      # noqa: E402
import diagnostics    # noqa: E402

_FAILS: list[str] = []


def check(name: str, cond: bool, detail: str = "") -> None:
    """Record a single assertion; print PASS/FAIL inline (mirrors test_ladder)."""
    status = "PASS" if cond else "FAIL"
    print(f"  [{status}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        _FAILS.append(f"{name}: {detail}")


def _good_verdict() -> dict:
    """A schema-valid baseline verdict to mutate per-case."""
    return {
        "verdict": "fail",
        "issues": [{
            "at_ms": 9000, "kind": "visual_artifact", "severity": "major",
            "evidence": "Frame at 9000 ms is completely black.",
        }],
        "confidence": 0.9,
        "summary": "One black frame at ~9s.",
    }


def _raises_valueerror(obj: dict) -> bool:
    """True iff judge.validate_review(obj) raises ValueError (deep-copied input)."""
    import copy
    try:
        judge.validate_review(copy.deepcopy(obj))
        return False
    except ValueError:
        return True


# ---------------------------------------------------------------------------
# Windows prompt stdin is always explicit UTF-8
# ---------------------------------------------------------------------------


def test_text_prompt_subprocesses_pin_utf8():
    print("test_text_prompt_subprocesses_pin_utf8")
    prompt_calls: list[str] = []
    violations: list[str] = []
    for root, _, files in os.walk(_JUDGE):
        for filename in files:
            if not filename.endswith(".py"):
                continue
            path = os.path.join(root, filename)
            with open(path, "r", encoding="utf-8") as source:
                tree = ast.parse(source.read(), filename=path)
            for node in ast.walk(tree):
                if not isinstance(node, ast.Call):
                    continue
                func = node.func
                if not (isinstance(func, ast.Attribute)
                        and isinstance(func.value, ast.Name)
                        and func.value.id == "subprocess"
                        and func.attr == "run"):
                    continue
                keywords = {keyword.arg: keyword.value for keyword in node.keywords
                            if keyword.arg is not None}
                if "input" not in keywords:
                    continue
                text = keywords.get("text")
                if not (isinstance(text, ast.Constant) and text.value is True):
                    # Byte-oriented stdin (for example prompt.encode("utf-8"))
                    # is already locale-independent and outside this contract.
                    continue
                relative = os.path.relpath(path, _ROOT)
                location = f"{relative}:{node.lineno}"
                prompt_calls.append(location)
                encoding = keywords.get("encoding")
                if not (isinstance(encoding, ast.Constant)
                        and str(encoding.value).lower().replace("_", "-") == "utf-8"):
                    violations.append(location)
    check("all text-mode judge prompt calls declare encoding=utf-8",
          not violations, ", ".join(violations))
    check("Claude and Codex prompt stdin sites remain covered",
          len(prompt_calls) >= 3, repr(prompt_calls))


# ---------------------------------------------------------------------------
# CLI failure diagnostics preserve their actual stream/envelope
# ---------------------------------------------------------------------------


def test_failure_diagnostics_are_labelled_and_non_speculative():
    print("test_failure_diagnostics_are_labelled_and_non_speculative")
    stdout = ('{"is_error":true,"result":"OAuth session expired and could '
              'not be refreshed"}')
    stderr = "launcher warning: stale config lock"
    detail = diagnostics.process_failure_detail(stdout, stderr, 400)
    check("stdout error envelope retained with its source label",
          detail.startswith("stdout: {") and "OAuth session expired" in detail,
          detail)
    check("stderr retained alongside stdout envelope",
          "stderr: launcher warning" in detail, detail)
    empty = diagnostics.process_failure_detail("", "", 20)
    check("empty streams are explicit", empty == "(no output on stdout or stderr)", empty)

    reason = diagnostics.failed_preflight_reason(detail, detail)
    check("failed preflight does not assert a root cause",
          "Root cause is not established" in reason
          and "Known cause:" not in reason
          and "re-authenticate" not in reason,
          reason)


# ---------------------------------------------------------------------------
# validate_review enforces the schema's value constraints
# ---------------------------------------------------------------------------


def test_validate_review_enforces_enums():
    print("test_validate_review_enforces_enums")
    # A clean valid verdict still passes (no regression).
    check("valid verdict accepted", not _raises_valueerror(_good_verdict()))

    # Each malformed value must be rejected.
    bad_kind = _good_verdict()
    bad_kind["issues"][0]["kind"] = "made_up_kind"
    check("out-of-enum kind rejected", _raises_valueerror(bad_kind))

    bad_sev = _good_verdict()
    bad_sev["issues"][0]["severity"] = "catastrophic"
    check("out-of-enum severity rejected", _raises_valueerror(bad_sev))

    conf_str = _good_verdict()
    conf_str["confidence"] = "high"
    check("non-numeric confidence rejected", _raises_valueerror(conf_str))

    conf_oor = _good_verdict()
    conf_oor["confidence"] = 7.5
    check("confidence out of [0,1] rejected", _raises_valueerror(conf_oor))

    at_str = _good_verdict()
    at_str["issues"][0]["at_ms"] = "around 9000"
    check("non-numeric at_ms rejected", _raises_valueerror(at_str))

    # Float that loses precision -> reject; integral float -> coerce to int.
    at_frac = _good_verdict()
    at_frac["issues"][0]["at_ms"] = 9000.5
    check("fractional float at_ms rejected", _raises_valueerror(at_frac))

    at_intfloat = _good_verdict()
    at_intfloat["issues"][0]["at_ms"] = 9000.0
    coerced = judge.validate_review(at_intfloat)
    check("integral float at_ms coerced to int",
          coerced["issues"][0]["at_ms"] == 9000
          and isinstance(coerced["issues"][0]["at_ms"], int),
          repr(coerced["issues"][0]["at_ms"]))

    end_frac = _good_verdict()
    end_frac["issues"][0]["end_ms"] = 9500.25
    check("fractional float end_ms rejected", _raises_valueerror(end_frac))

    # confidence bounds: 0.0 and 1.0 are inclusive-valid.
    for c in (0.0, 1.0, 0.6):
        v = _good_verdict()
        v["confidence"] = c
        check(f"confidence {c} accepted (inclusive bounds)",
              not _raises_valueerror(v))

    # bool is not a valid number / int (Python bool is an int subtype).
    conf_bool = _good_verdict()
    conf_bool["confidence"] = True
    check("bool confidence rejected", _raises_valueerror(conf_bool))


def test_enums_derived_from_schema():
    print("test_enums_derived_from_schema")
    # The validator's allowed values must BE the schema's, not a parallel list.
    schema_issue = judge.REVIEW_SCHEMA["properties"]["issues"]["items"]["properties"]
    check("kind enum == schema issue kind enum",
          judge._KIND_ENUM == schema_issue["kind"]["enum"]
          == judge.ISSUE_KINDS)
    check("severity enum == schema severity enum",
          judge._SEVERITY_ENUM == schema_issue["severity"]["enum"])
    check("verdict enum == schema verdict enum",
          judge._VERDICT_ENUM
          == judge.REVIEW_SCHEMA["properties"]["verdict"]["enum"])
    # Every schema-declared kind is actually accepted (round-trip).
    all_kinds_ok = True
    for k in judge.ISSUE_KINDS:
        v = _good_verdict()
        v["issues"][0]["kind"] = k
        if _raises_valueerror(v):
            all_kinds_ok = False
    check("every schema kind value is accepted", all_kinds_ok)


# ---------------------------------------------------------------------------
# _extract_json recovers the real verdict past a brace-bearing preamble
# ---------------------------------------------------------------------------


def test_prose_preamble_verdict_recovered():
    print("test_prose_preamble_verdict_recovered")
    import json
    verdict = _good_verdict()
    vjson = json.dumps(verdict)
    # A prose preamble carries its own {...} blob before the real verdict object.
    # Returning {note: incomplete} would make the
    # caller's validate_review then rejected it -> status:error, real verdict lost.
    resp = (f"Here is my analysis {{note: incomplete}}. Final verdict:\n{vjson}")
    got = codex_judge._extract_json(resp)
    check("verdict recovered (not None)", got is not None, repr(resp[:80]))
    check("recovered object is the real verdict",
          got is not None and got.get("verdict") == "fail"
          and got.get("issues") and got["issues"][0]["kind"] == "visual_artifact",
          repr(got))

    # Clean bare JSON is unchanged.
    clean = codex_judge._extract_json(vjson)
    check("clean JSON returns the same verdict",
          clean is not None and clean.get("verdict") == "fail", repr(clean))

    # Fenced JSON still recovers.
    fenced = codex_judge._extract_json("```json\n" + vjson + "\n```")
    check("fenced JSON recovers the verdict",
          fenced is not None and fenced.get("verdict") == "fail", repr(fenced))

    # A preamble blob that is itself VALID JSON but NOT a verdict is skipped.
    decoy = ('{"status": "thinking", "step": 1} '
             'then the verdict: ' + vjson)
    got2 = codex_judge._extract_json(decoy)
    check("valid-but-non-verdict preamble blob skipped",
          got2 is not None and got2.get("verdict") == "fail", repr(got2))

    # No verdict anywhere -> None (honest, no fabrication).
    check("no verdict object -> None",
          codex_judge._extract_json("just prose, no json at all") is None)


# ---------------------------------------------------------------------------
# Codex detection must leave canonical provider auth content to the CLI
# ---------------------------------------------------------------------------


def test_codex_detection_never_reads_canonical_auth_content():
    print("test_codex_detection_never_reads_canonical_auth_content")
    source_path = os.path.join(_JUDGE, "adapters", "codex_judge.py")
    with open(source_path, "r", encoding="utf-8") as source:
        tree = ast.parse(source.read(), filename=source_path)
    detect_fn = next(
        (node for node in tree.body
         if isinstance(node, ast.FunctionDef) and node.name == "detect"),
        None)
    check("Codex detect function remains present", detect_fn is not None)
    if detect_fn is None:
        return

    calls: list[str] = []
    literals: list[str] = []
    for node in ast.walk(detect_fn):
        if isinstance(node, ast.Call):
            if isinstance(node.func, ast.Name):
                calls.append(node.func.id)
            elif isinstance(node.func, ast.Attribute):
                owner = node.func.value
                if isinstance(owner, ast.Name):
                    calls.append(f"{owner.id}.{node.func.attr}")
        elif isinstance(node, ast.Constant) and isinstance(node.value, str):
            literals.append(node.value)

    check("Codex detect does not open authentication content", "open" not in calls,
          repr(calls))
    check("Codex detect does not JSON-load authentication content", "json.load" not in calls,
          repr(calls))
    forbidden = [literal for literal in literals
                 if any(marker in literal for marker in (
                     ".codex/auth.json", "OPENAI_API_KEY", "auth_mode"))]
    check("Codex detect contains no canonical-auth content references", not forbidden,
          repr(forbidden))

    codex_path = "/fixture/bin/codex"
    commands: list[list[str]] = []

    def fake_run(command, **_kwargs):
        commands.append(command)
        return types.SimpleNamespace(stdout="codex fixture 1.2.3\n", stderr="")

    with (
        mock.patch.object(codex_judge.shutil, "which", return_value=codex_path),
        mock.patch.object(codex_judge.subprocess, "run", side_effect=fake_run),
        mock.patch("builtins.open", side_effect=AssertionError("detect opened auth content"))
                as forbidden_open,
        mock.patch.object(codex_judge.json, "load",
                          side_effect=AssertionError("detect JSON-loaded auth content"))
                as forbidden_json_load,
    ):
        detected = codex_judge.detect()

    check("Codex detect reports the resolved binary", detected.get("found") is True
          and detected.get("path") == codex_path, repr(detected))
    check("Codex detect reports its version", detected.get("version") == "codex fixture 1.2.3",
          repr(detected))
    check("Codex detect leaves authentication unknown", detected.get("logged_in") is None
          and "auth_mode" not in detected, repr(detected))
    check("Codex detect runs only --version", commands == [[codex_path, "--version"]],
          repr(commands))
    check("Codex detect did not open canonical auth content", not forbidden_open.called)
    check("Codex detect did not JSON-load canonical auth content",
          not forbidden_json_load.called)

    with mock.patch.object(codex_judge.shutil, "which", return_value=None):
        missing = codex_judge.detect()
    check("missing Codex remains an honest binary-missing detection",
          missing.get("found") is False and missing.get("path") is None
          and "version" not in missing and "logged_in" not in missing,
          repr(missing))


# ---------------------------------------------------------------------------
# The read-failure detector does not reclassify a real all-black failure
# ---------------------------------------------------------------------------


def test_all_black_fail_not_reclassified():
    print("test_all_black_fail_not_reclassified")
    # The reproduced false positive: a LEGIT all-black `fail` whose prose trips
    # the old READ_BLOCKED + ALL_FRAMES heuristics. It has populated issues[] and
    # high confidence — must NOT be reclassified to infrastructure.
    all_black_fail = {
        "verdict": "fail",
        "issues": [{"at_ms": 0, "kind": "visual_artifact", "severity": "blocker",
                    "evidence": "Every frame is solid black."}],
        "confidence": 0.95,
        "summary": ("Every frame is solid black across all 20 frames; I could "
                    "not read the file name burned into any frame."),
        "cannot_assess": [],
    }
    infra, _ = cli_judge.detect_frame_read_failure(all_black_fail, 20)
    check("all-black fail not reclassified to infrastructure", infra is False,
          str(infra))

    # The REAL EACCES sweep is still caught (OS token + empty issues + low conf).
    eacces = {
        "verdict": "needs_review", "issues": [], "confidence": 0.1,
        "summary": "All 20 frame files returned EACCES (permission denied) — "
                   "zero frames were readable.",
        "cannot_assess": ["Visual integrity — all frames unreadable (EACCES)"],
    }
    infra2, cause = cli_judge.detect_frame_read_failure(eacces, 20)
    check("real EACCES sweep still caught -> infra=True", infra2 is True,
          str(infra2))
    check("infra cause names BLOCKED reads", infra2 and "BLOCKED" in (cause or ""))

    # A read-blocked OS token but with a populated issues[] (not the zero-frame
    # shape) -> structural guard blocks reclassification.
    token_but_issues = {
        "verdict": "fail",
        "issues": [{"at_ms": 1000, "kind": "visual_artifact", "severity": "major",
                    "evidence": "garbled pixels"}],
        "confidence": 0.8,
        "summary": "EACCES appeared once but I still reviewed every frame.",
        "cannot_assess": [],
    }
    infra3, _ = cli_judge.detect_frame_read_failure(token_but_issues, 20)
    check("OS token + populated issues -> NOT infra (structural guard)",
          infra3 is False, str(infra3))


# ---------------------------------------------------------------------------
# Audio-claim redaction from a vision-only judge
# ---------------------------------------------------------------------------


def test_audio_claims_stripped_from_vision_only_judge():
    print("test_audio_claims_stripped_from_vision_only_judge")
    # The reproduced miss: a visual_artifact issue whose evidence asserts audio.
    review = {
        "verdict": "needs_review",
        "issues": [{
            "at_ms": 9000, "kind": "visual_artifact", "severity": "major",
            "evidence": "The audio track has a noticeable gap and the music "
                        "drops out here.",
        }],
        "confidence": 0.6,
        "summary": "Possible issue around 9s.",
    }
    filtered, report = cli_judge.post_filter_review(
        review, fps=1.0, duration_ms=60000, listened=False)
    check("audio-asserting issue removed from a deaf judge",
          len(filtered["issues"]) == 0
          and len(report["removed_issues"]) == 1,
          f"issues={filtered['issues']} removed={report['removed_issues']}")

    # Each broadened token triggers removal.
    for phrase in ("the music swells", "a voiceover narrates", "the dialogue is",
                   "it goes quiet here", "the sound cuts off",
                   "the track fades out", "a long silence", "it is silent"):
        r = {
            "verdict": "needs_review",
            "issues": [{"at_ms": 1000, "kind": "visual_artifact",
                        "severity": "minor", "evidence": phrase}],
            "confidence": 0.5, "summary": "x",
        }
        f, rep = cli_judge.post_filter_review(r, 1.0, 60000, listened=False)
        check(f"audio phrase removed: {phrase!r}",
              len(f["issues"]) == 0, f"survived: {f['issues']}")

    # A GENUINE visual finding about on-screen TEXT must NOT be over-redacted.
    visual_text = {
        "verdict": "fail",
        "issues": [{"at_ms": 2000, "kind": "caption_error", "severity": "major",
                    "evidence": "The on-screen caption text reads 'Helo' "
                                "(misspelled) in the lower third."}],
        "confidence": 0.7, "summary": "Caption typo on screen.",
    }
    f2, _ = cli_judge.post_filter_review(visual_text, 1.0, 60000, listened=False)
    check("on-screen text finding NOT over-redacted (kept)",
          len(f2["issues"]) == 1, f"dropped: {f2}")


# ---------------------------------------------------------------------------
# digest_perception is robust to a malformed-but-tagged span
# ---------------------------------------------------------------------------


def test_digest_skips_malformed_span():
    print("test_digest_skips_malformed_span")
    # The reproduced KeyError: a silence span missing end_ms. Must NOT raise.
    p = {"schema": judge.PERCEPTION_SCHEMA,
         "silences": [{"start_ms": 10}]}            # no end_ms
    try:
        out = judge.digest_perception(p)
        raised = False
    except Exception as e:  # noqa: BLE001 — any exception is the failure here
        out, raised = f"{type(e).__name__}: {e}", True
    check("malformed silence span does not raise", not raised, str(out))
    check("malformed span is skipped (not rendered)",
          isinstance(out, str) and "silence spans" not in out, str(out)[:120])

    # A well-formed span alongside a malformed one: the good one still renders.
    p2 = {"schema": judge.PERCEPTION_SCHEMA,
          "silences": [{"start_ms": 10}, {"start_ms": 100, "end_ms": 200}]}
    out2 = judge.digest_perception(p2)
    check("good span among malformed ones still rendered",
          "100-200ms" in out2, out2[:160])

    # A word missing end_ms must not crash the speech-span line either.
    p3 = {"schema": judge.PERCEPTION_SCHEMA,
          "words": {"words": [{"word": "hi", "start_ms": 0}]}}  # no end_ms
    try:
        judge.digest_perception(p3)
        raised3 = False
    except Exception as e:  # noqa: BLE001
        raised3 = True
    check("malformed word span does not raise", not raised3)


def main() -> int:
    tests = [
        test_text_prompt_subprocesses_pin_utf8,
        test_failure_diagnostics_are_labelled_and_non_speculative,
        test_validate_review_enforces_enums,
        test_enums_derived_from_schema,
        test_prose_preamble_verdict_recovered,
        test_codex_detection_never_reads_canonical_auth_content,
        test_all_black_fail_not_reclassified,
        test_audio_claims_stripped_from_vision_only_judge,
        test_digest_skips_malformed_span,
    ]
    for t in tests:
        t()
    print()
    if _FAILS:
        print(f"FAILED ({len(_FAILS)}):")
        for f in _FAILS:
            print("  - " + f)
        return 1
    print("ALL VALIDATION TESTS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
