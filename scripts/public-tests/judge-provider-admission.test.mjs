#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const pythonProbe = spawnSync(
  process.env.CUTD_ADAPTER_PYTHON || (process.platform === "win32" ? "python" : "python3"),
  ["-c", "import sys; print(sys.executable)"],
  { encoding: "utf8" },
);
const python = pythonProbe.status === 0 ? pythonProbe.stdout.trim() : "";
const REASON = "render judge unavailable until restricted tool/file access is verified";

const source = String.raw`
import contextlib
import io
import json
import os
import sys
import types
from unittest import mock

root = sys.argv[1]
adapters = os.path.join(root, "app", "perception", "py", "judge", "adapters")
judge_root = os.path.dirname(adapters)
sys.path[:0] = [adapters, judge_root]

import antigravity_judge
import codex_judge
import grok_judge
import ladder_judge

REASON = "render judge unavailable until restricted tool/file access is verified"


def expect(condition, detail):
    if not condition:
        raise AssertionError(detail)


@contextlib.contextmanager
def argv(values):
    old = sys.argv
    sys.argv = values
    try:
        yield
    finally:
        sys.argv = old


def verdict():
    return {
        "verdict": "pass",
        "issues": [],
        "cannot_assess": [],
        "confidence": 0.9,
        "summary": "fixture verdict",
    }


# The pure Codex helpers survive the render-judge boundary and remain shared by
# both legacy adapter modules. They do no binary/model work.
schema = {"type": "object", "properties": {"x": {"type": "string"}}}
strict = codex_judge.to_strict_schema(schema)
expect(schema.get("required") is None, repr(schema))
expect(strict["required"] == ["x"] and strict["additionalProperties"] is False, repr(strict))
encoded = json.dumps(verdict())
expect(codex_judge._extract_json("preamble {bad} " + encoded) == verdict(), "Codex decoder")
expect(antigravity_judge._extract_json is codex_judge._extract_json, "Antigravity decoder sharing")
expect(grok_judge.codex_judge._extract_json is codex_judge._extract_json, "Grok decoder sharing")


for adapter, binary, provider in (
    (codex_judge, "codex", "codex"),
    (antigravity_judge, "agy", "antigravity"),
):
    fixture_path = "/fixture/bin/" + binary
    commands = []

    def version_only(command, **_kwargs):
        commands.append(command)
        expect(command == [fixture_path, "--version"], repr(command))
        return types.SimpleNamespace(stdout=provider + " fixture 1.2.3\n", stderr="", returncode=0)

    with (
        mock.patch.object(adapter.shutil, "which", return_value=fixture_path),
        mock.patch.object(adapter.subprocess, "run", side_effect=version_only),
        mock.patch("builtins.open", side_effect=AssertionError("detect opened provider state")),
    ):
        detected = adapter.detect()
    expect(detected["found"] is True and detected["path"] == fixture_path, repr(detected))
    expect(detected["judge_ready"] is False, repr(detected))
    expect(detected["availability_reason"] == REASON, repr(detected))
    expect(detected["logged_in"] is None and detected["version"] == provider + " fixture 1.2.3", repr(detected))
    expect(commands == [[fixture_path, "--version"]], repr(commands))

    with (
        mock.patch.object(adapter.shutil, "which", return_value=None),
        mock.patch.object(adapter.subprocess, "run", side_effect=AssertionError("missing detect ran a process")),
    ):
        missing = adapter.detect()
    expect(missing["found"] is False and missing["path"] is None, repr(missing))
    expect(missing["judge_ready"] is False and missing["availability_reason"] == REASON, repr(missing))

    # An explicit review remains an honest terminal receipt even when --render
    # names a non-existent/unreadable file. Every render/protocol hook raises if
    # reached; no provider or model subprocess may occur.
    blocked_stdout = io.StringIO()
    with (
        mock.patch.object(adapter.subprocess, "run", side_effect=AssertionError("review started a subprocess")),
        mock.patch.object(adapter.judge, "probe_duration_s", side_effect=AssertionError("review probed a render")),
        mock.patch.object(adapter.judge, "extract_frames", side_effect=AssertionError("review extracted frames")),
        contextlib.redirect_stdout(blocked_stdout),
        argv([adapter.__file__, "review", "--render", "/fixture/missing-render.mp4", "--mode", "window"]),
    ):
        code = adapter.main()
    receipt = json.loads(blocked_stdout.getvalue())
    expect(code == 0 and receipt["status"] == "not_run", repr(receipt))
    expect(receipt["not_run_reason"] == REASON, repr(receipt))
    expect(receipt["backend"]["provider"] == provider, repr(receipt))
    expect(receipt["backend"]["watched"] is False and receipt["backend"]["frames_sent"] == 0, repr(receipt))
    expect(receipt["prompt_chars"] == {"system": 0, "user": 0}, repr(receipt))
    expect(receipt["bundle_dir"] is None and receipt["review"] is None, repr(receipt))

expect(not hasattr(codex_judge, "invoke_codex") and not hasattr(codex_judge, "build_codex_prompts"),
       "Codex unsupported invocation remains reachable")
expect(not hasattr(antigravity_judge, "invoke_antigravity") and not hasattr(antigravity_judge, "build_antigravity_prompts"),
       "Antigravity unsupported invocation remains reachable")

# Auto selection skips present-but-unready Codex/Antigravity and can still use
# a qualified Claude or Grok entry. This is detection-only: no adapter runs.
with (
    mock.patch.object(ladder_judge.cli_judge, "detect_providers", return_value={"claude": {"found": True, "restricted_read_capable": True}}),
    mock.patch.object(ladder_judge.codex_judge, "detect", return_value={"provider": "codex", "found": True, "judge_ready": False, "availability_reason": REASON}),
    mock.patch.object(ladder_judge.antigravity_judge, "detect", return_value={"provider": "antigravity", "found": True, "judge_ready": False, "availability_reason": REASON}),
    mock.patch.object(ladder_judge.grok_judge, "detect", return_value={"provider": "grok", "found": True, "judge_ready": True}),
):
    claude_ladder = ladder_judge.detect_ladder()
expect(claude_ladder["auto_selected"] == "claude", repr(claude_ladder))

with (
    mock.patch.object(ladder_judge.cli_judge, "detect_providers", return_value={"claude": {"found": False}}),
    mock.patch.object(ladder_judge.codex_judge, "detect", return_value={"provider": "codex", "found": True, "judge_ready": False, "availability_reason": REASON}),
    mock.patch.object(ladder_judge.antigravity_judge, "detect", return_value={"provider": "antigravity", "found": True, "judge_ready": False, "availability_reason": REASON}),
    mock.patch.object(ladder_judge.grok_judge, "detect", return_value={"provider": "grok", "found": True, "judge_ready": True}),
):
    grok_ladder = ladder_judge.detect_ladder()
expect(grok_ladder["auto_selected"] == "grok", repr(grok_ladder))

# A forced unsupported provider retains the explicit selection. It may emit its
# own not_run receipt but cannot fall through to Grok/Claude.
selected = []
def explicit_adapter(provider, passthrough, launches=None):
    selected.append((provider, passthrough, launches))
    return 0, {"status": "not_run", "not_run_reason": REASON}
with (
    mock.patch.object(ladder_judge, "detect_ladder", return_value=grok_ladder),
    mock.patch.object(ladder_judge, "run_adapter", side_effect=explicit_adapter),
    contextlib.redirect_stdout(io.StringIO()),
    argv(["ladder_judge.py", "review", "--provider", "codex", "--render", "/fixture/missing-render.mp4"]),
):
    code = ladder_judge.main()
expect(code == 0 and [provider for provider, _, _ in selected] == ["codex"], repr(selected))
expect(selected[0][2] is None, repr(selected))

print("PASS render-judge provider admission boundary")
`;

if (!python) {
  test("Codex and Antigravity render-judge admission", { skip: "Python is not available" }, () => {});
} else {
  test("Codex and Antigravity render-judge admission", () => {
    const result = spawnSync(python, ["-c", source, ROOT], {
      cwd: ROOT,
      encoding: "utf8",
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /PASS render-judge provider admission boundary/);
  });
}
