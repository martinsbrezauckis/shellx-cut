#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const pythonProbe = spawnSync(
  process.env.CUTD_ADAPTER_PYTHON || (process.platform === "win32" ? "python" : "python3"),
  ["-c", "import sys; print(sys.executable)"],
  { encoding: "utf8" },
);

if (pythonProbe.status !== 0 || !pythonProbe.stdout.trim()) {
  console.log("SKIP grok-judge-tool-policy.test.mjs: Python is not available");
} else {
  const python = pythonProbe.stdout.trim();

  test("Grok judge admits only the documented no-tool policy", () => {
    const source = String.raw`
import base64
import json
import os
import subprocess
import sys
import tempfile
import types
from unittest import mock

root = sys.argv[1]
judge_root = os.path.join(root, "app", "perception", "py", "judge")
sys.path.insert(0, os.path.join(judge_root, "adapters"))
sys.path.insert(0, judge_root)
import grok_judge
import grok_tool_policy


def expect(condition, message):
    if not condition:
        raise AssertionError(message)


def result(stdout="", stderr="", returncode=0):
    return types.SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)


# The version/help admission does not open auth/config or contact a model. It
# accepts the exact release whose embedded reference documents allow-then-remove
# and the MCPTool denial rule.
observed_probes = []
def supported_probe(command, **kwargs):
    observed_probes.append(command)
    expect(kwargs.get("encoding") == "utf-8", repr(kwargs))
    if command == ["/fixture/grok", "--version"]:
        return result("grok 1.0.21 (fixture)\\n")
    if command == ["/fixture/grok", "--help"]:
        return result("\\n".join(grok_tool_policy.REQUIRED_FLAGS))
    raise AssertionError(f"unexpected capability probe: {command!r}")

with (
    mock.patch.object(grok_tool_policy.shutil, "which", return_value="/fixture/grok"),
    mock.patch.object(grok_tool_policy.subprocess, "run", side_effect=supported_probe),
):
    cli, reason = grok_tool_policy.resolve_grok_tool_policy("grok")
expect(reason is None and cli is not None, repr(reason))
expect(observed_probes == [["/fixture/grok", "--version"], ["/fixture/grok", "--help"]],
       repr(observed_probes))
expect(cli["version_tuple"] == (1, 0, 21), repr(cli))

# An old or flag-incomplete wrapper is refused before prompt creation/model run.
def old_probe(command, **_kwargs):
    expect(command == ["/fixture/grok", "--version"], repr(command))
    return result("grok 1.0.20 (fixture)\\n")
with (
    mock.patch.object(grok_tool_policy.shutil, "which", return_value="/fixture/grok"),
    mock.patch.object(grok_tool_policy.subprocess, "run", side_effect=old_probe),
):
    old_cli, old_reason = grok_tool_policy.resolve_grok_tool_policy("grok")
expect(old_cli is None and "requires >= 1.0.21" in (old_reason or ""), repr(old_reason))

# Every failed capability probe is terminal. These branches must neither accept
# an opaque wrapper nor advance to --help/model execution after version failure.
for label, failed_result, expected in (
    ("version error", result(stderr="fixture version error", returncode=7), "exit 7"),
    ("unknown version", result("fixture release unknown\\n"), "not parseable"),
):
    observed = []
    def failed_version(command, **_kwargs):
        observed.append(command)
        return failed_result
    with (
        mock.patch.object(grok_tool_policy.shutil, "which", return_value="/fixture/grok"),
        mock.patch.object(grok_tool_policy.subprocess, "run", side_effect=failed_version),
    ):
        failed_cli, failed_reason = grok_tool_policy.resolve_grok_tool_policy("grok")
    expect(failed_cli is None and expected in (failed_reason or ""),
           f"{label}: {failed_reason!r}")
    expect(observed == [["/fixture/grok", "--version"]],
           f"{label} advanced after version failure: {observed!r}")

for label, failed_exception in (
    ("version timeout", subprocess.TimeoutExpired(["/fixture/grok", "--version"], 15)),
    ("version OSError", OSError("fixture version unavailable")),
):
    with (
        mock.patch.object(grok_tool_policy.shutil, "which", return_value="/fixture/grok"),
        mock.patch.object(grok_tool_policy.subprocess, "run", side_effect=failed_exception),
    ):
        failed_cli, failed_reason = grok_tool_policy.resolve_grok_tool_policy("grok")
    expect(failed_cli is None and "could not verify Grok no-tool policy version" in (failed_reason or ""),
           f"{label}: {failed_reason!r}")

for label, failed_help in (
    ("help error", result(stderr="fixture help error", returncode=9)),
    ("help timeout", subprocess.TimeoutExpired(["/fixture/grok", "--help"], 15)),
    ("help OSError", OSError("fixture help unavailable")),
):
    with (
        mock.patch.object(grok_tool_policy.shutil, "which", return_value="/fixture/grok"),
        mock.patch.object(grok_tool_policy.subprocess, "run", side_effect=[
            result("grok 1.0.21 (fixture)\\n"), failed_help,
        ]),
    ):
        failed_cli, failed_reason = grok_tool_policy.resolve_grok_tool_policy("grok")
    expected = ("could not verify Grok no-tool policy flags"
                if isinstance(failed_help, BaseException)
                else "complete no-tool judge policy")
    expect(failed_cli is None and expected in (failed_reason or ""),
           f"{label}: {failed_reason!r}")

with (
    mock.patch.object(grok_tool_policy.shutil, "which", return_value="/fixture/grok"),
    mock.patch.object(grok_tool_policy.subprocess, "run", side_effect=[
        result("grok 1.0.21 (fixture)\\n"), result("--tools\\n--deny\\n"),
    ]),
):
    missing_cli, missing_reason = grok_tool_policy.resolve_grok_tool_policy("grok")
expect(missing_cli is None and "complete no-tool judge policy" in (missing_reason or ""),
       repr(missing_reason))

# detect preserves canonical auth ownership: it has no ~/.grok directory probe.
with (
    mock.patch.object(grok_judge.shutil, "which", return_value="/fixture/grok"),
    mock.patch.object(grok_judge.grok_tool_policy, "resolve_grok_tool_policy", return_value=(cli, None)),
    mock.patch.object(grok_judge.os, "listdir", side_effect=AssertionError("must not inspect auth")),
):
    detected = grok_judge.detect()
expect(detected["found"] is True and detected["logged_in"] is None, repr(detected))
expect(detected["tool_policy"]["mcp_tool_invocations"] == "denied", repr(detected))
expect("not asserted" in detected["tool_policy"]["mcp_server_startup"], repr(detected))

# The ladder preserves a present but unsupported Grok wrapper as found while
# refusing auto admission and reporting its capability reason.
import ladder_judge
with (
    mock.patch.object(ladder_judge.cli_judge, "detect_providers", return_value={"claude": {"found": False}}),
    mock.patch.object(ladder_judge.codex_judge, "detect", return_value={"provider": "codex", "found": False}),
    mock.patch.object(ladder_judge.antigravity_judge, "detect", return_value={"provider": "antigravity", "found": False}),
    mock.patch.object(ladder_judge.grok_judge, "detect", return_value={
        "provider": "grok", "found": True, "judge_ready": False,
        "availability_reason": "policy missing",
    }),
):
    unsupported_ladder = ladder_judge.detect_ladder()
unsupported_grok = next(entry for entry in unsupported_ladder["rungs"] if entry["provider"] == "grok")
expect(unsupported_ladder["auto_selected"] is None and unsupported_grok["judge_ready"] is False,
       repr(unsupported_ladder))
expect("grok: found but unavailable (policy missing)" in ladder_judge.skip_reason_block(unsupported_ladder),
       ladder_judge.skip_reason_block(unsupported_ladder))

# Unsupported admission must not create an image prompt or invoke a model under
# weaker flags. This is the no-broader-retry boundary.
with (
    mock.patch.object(grok_judge.grok_tool_policy, "resolve_grok_tool_policy", return_value=(None, "policy missing")),
    mock.patch.object(grok_judge, "build_content_blocks") as no_blocks,
    mock.patch.object(grok_judge.subprocess, "run") as no_model,
):
    review, meta, error = grok_judge.invoke_grok(
        "grok", "system", "user", "", ["/unused.jpg"], "/fixture", 10)
expect(review is None and meta["available"] is False and error == "policy missing", repr((meta, error)))
expect(not no_blocks.called and not no_model.called, "unsupported policy reached prompt/model path")

# Exercise the real old-version resolver through invoke_grok. It must stop at
# the version probe, create no prompt-file, and make no model-shaped command.
with tempfile.TemporaryDirectory() as unsupported_workspace:
    old_probe_calls = []
    def old_invoke_probe(command, **_kwargs):
        old_probe_calls.append(command)
        return result("grok 1.0.20 (fixture)\\n")
    with (
        mock.patch.object(grok_tool_policy.shutil, "which", return_value="/fixture/grok"),
        mock.patch.object(grok_tool_policy.subprocess, "run", side_effect=old_invoke_probe),
    ):
        review, meta, error = grok_judge.invoke_grok(
            "grok", "system", "user", "", ["/unused.jpg"], unsupported_workspace, 10)
    expect(review is None and meta["available"] is False and "requires >= 1.0.21" in (error or ""),
           repr((meta, error)))
    expect(old_probe_calls == [["/fixture/grok", "--version"]], repr(old_probe_calls))
    expect(not os.path.exists(os.path.join(unsupported_workspace, "_grok_prompt.json")),
           "unsupported version created a prompt-file")

# Mock protocol completion verifies the production argv and keeps the image as
# an inline ACP block; no live provider, auth, or configured MCP is contacted.
with tempfile.TemporaryDirectory() as workspace:
    frame = os.path.join(workspace, "frame.jpg")
    with open(frame, "wb") as output:
        output.write(b"fixture-frame-pixels")

    observed_command = []
    def model_protocol(command, **kwargs):
        observed_command[:] = command
        expect(kwargs.get("cwd") == workspace, repr(kwargs))
        expect(command[0] == "/fixture/grok", repr(command))
        def value_after(flag):
            return command[command.index(flag) + 1]
        expect(value_after("--tools") == "read_file,grep,list_dir", repr(command))
        expect(value_after("--disallowed-tools") == "read_file,grep,list_dir,Agent", repr(command))
        expect(command.index("--tools") < command.index("--disallowed-tools"), repr(command))
        expect(value_after("--deny") == "MCPTool", repr(command))
        expect("--no-subagents" in command and "--disable-web-search" in command, repr(command))
        expect(value_after("--sandbox") == "read-only", repr(command))
        expect(value_after("--permission-mode") == "dontAsk", repr(command))
        expect("vision_describe" not in command, repr(command))
        expect("Agent" in value_after("--disallowed-tools") and "--no-subagents" in command,
               repr(command))
        with open(value_after("--prompt-file"), "r", encoding="utf-8") as prompt:
            blocks = json.load(prompt)
        expect([block["type"] for block in blocks] == ["text", "image"], repr(blocks))
        expect(blocks[1]["mimeType"] == "image/jpeg", repr(blocks[1]))
        expect(base64.b64decode(blocks[1]["data"]) == b"fixture-frame-pixels", repr(blocks[1]))
        verdict = {
            "verdict": "pass", "issues": [], "cannot_assess": [],
            "confidence": 0.91, "summary": "fixture reviewed inline frame",
        }
        return result(json.dumps({
            "text": json.dumps(verdict), "stopReason": "end_turn",
            "sessionId": "fixture", "requestId": "fixture-request",
        }))

    with (
        mock.patch.object(grok_judge.grok_tool_policy, "resolve_grok_tool_policy", return_value=(cli, None)),
        mock.patch.object(grok_judge.subprocess, "run", side_effect=model_protocol),
    ):
        review, meta, error = grok_judge.invoke_grok(
            "grok", "system", "user", "grok-build", [frame], workspace, 10)
    expect(error is None and review is not None and review["verdict"] == "pass", repr((review, error)))
    expect(meta["tool_policy"]["version"].startswith("grok 1.0.21 (fixture)"), repr(meta))
    expect(meta["tool_policy"]["mcp_tool_invocations"] == "denied", repr(meta))
    expect(observed_command.count("--deny") == 1, repr(observed_command))

print("PASS grok judge no-tool capability and inline-image protocol")
`;
    const result = spawnSync(python, ["-c", source, ROOT], {
      cwd: ROOT,
      encoding: "utf8",
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /PASS grok judge no-tool capability and inline-image protocol/);
  });
}
