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
from unittest import mock

root = sys.argv[1]
adapters = os.path.join(root, "app", "perception", "py", "judge", "adapters")
judge_root = os.path.dirname(adapters)
sys.path[:0] = [adapters, judge_root]

import antigravity_judge
import codex_judge
import grok_judge
import ladder_judge

REASON = "native render-judge admission is pending"


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


help_text = " ".join(codex_judge.codex_judge_policy.TRANSPORT_FLAGS).encode()
outputs = [b"codex-cli 0.160.1\n", help_text]
with (mock.patch.object(codex_judge.shutil, "which", return_value="/fixture/codex"),
      mock.patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", False),
      mock.patch.object(codex_judge, "_child_route", return_value=(["/fixture/codex"], None)),
      mock.patch.object(codex_judge.codex_judge_runtime, "run_bounded", side_effect=outputs) as run,
      mock.patch.object(codex_judge.codex_judge_runtime, "premodel_probe",
                        side_effect=AssertionError("unadmitted detect probed policy"))):
    installed = codex_judge.detect()
expect(installed["found"] and not installed["judge_ready"], repr(installed))
expect(installed["capabilities"]["direct_image_and_schema"] is True, repr(installed))
expect(installed["availability_reason"], repr(installed))
expect([call.args[0] for call in run.call_args_list] ==
       [["/fixture/codex", "--version"], ["/fixture/codex", "exec", "--help"]], repr(run.call_args_list))

# Synthetic effective-policy results check the admission contract without a
# provider process or configuration read.
with (mock.patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", True),
      mock.patch.object(codex_judge.shutil, "which", return_value="/fixture/codex"),
      mock.patch.object(codex_judge, "_child_route", return_value=(["/fixture/codex"], None)),
      mock.patch.object(codex_judge.codex_judge_runtime, "run_bounded", side_effect=outputs),
      mock.patch.object(codex_judge.codex_judge_runtime, "premodel_probe",
                        return_value={"effective_policy": True}) as probe):
    capable = codex_judge.detect()
expect(capable["judge_ready"] is True and capable["availability_reason"] is None, repr(capable))
expect(capable["capabilities"]["native_tool_free_admission"] is True, repr(capable))
expect(probe.call_count == 1 and probe.call_args.args[0] == ["/fixture/codex"], repr(probe.call_args))

with (mock.patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", True),
      mock.patch.object(codex_judge.shutil, "which", return_value="/fixture/codex"),
      mock.patch.object(codex_judge, "_child_route", return_value=(["/fixture/codex"], None)),
      mock.patch.object(codex_judge.codex_judge_runtime, "run_bounded", side_effect=outputs),
      mock.patch.object(codex_judge.codex_judge_runtime, "premodel_probe",
                        return_value={"effective_policy": False}) as rejected_probe):
    rejected = codex_judge.detect()
expect(rejected_probe.call_count == 1 and rejected["found"], repr(rejected))
expect(rejected["judge_ready"] is False and rejected["availability_reason"], repr(rejected))
expect(rejected["capabilities"]["native_tool_free_admission"] is False, repr(rejected))

with (mock.patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", True),
      mock.patch.object(codex_judge.shutil, "which", return_value="/fixture/codex"),
      mock.patch.object(codex_judge, "_child_route", return_value=(["/fixture/codex"], None)),
      mock.patch.object(codex_judge.codex_judge_runtime, "run_bounded", side_effect=outputs),
      mock.patch.object(codex_judge.codex_judge_runtime, "premodel_probe",
                        side_effect=codex_judge.codex_judge_runtime.ProbeError("codex_effective_policy_tools"))):
    refused = codex_judge.detect()
expect(refused["found"] and refused["judge_ready"] is False, repr(refused))
expect(refused["availability_reason"] == "codex_effective_policy_tools", repr(refused))

with (mock.patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", True),
      mock.patch.object(codex_judge.shutil, "which", return_value="/fixture/codex"),
      mock.patch.object(codex_judge, "_child_route", return_value=(["/fixture/codex"], None)),
      mock.patch.object(codex_judge.codex_judge_runtime, "run_bounded",
                        side_effect=[outputs[0], b"--image --ephemeral"]),
      mock.patch.object(codex_judge.codex_judge_runtime, "premodel_probe",
                        side_effect=AssertionError("transport-negative detect probed policy"))):
    incomplete = codex_judge.detect()
expect(incomplete["found"] and not incomplete["judge_ready"], repr(incomplete))

with mock.patch.object(codex_judge.shutil, "which", return_value=None):
    missing = codex_judge.detect()
expect(not missing["found"] and not missing["judge_ready"], repr(missing))

# An explicit false native gate must stop review before render/model even if the
# shipping gate changes later.
blocked_stdout = io.StringIO()
with (mock.patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", False),
      mock.patch.object(codex_judge.judge, "probe_duration_s", side_effect=AssertionError("render probed")),
      mock.patch.object(codex_judge.codex_judge_runtime, "run_bounded", side_effect=AssertionError("provider started")),
      contextlib.redirect_stdout(blocked_stdout),
      argv([codex_judge.__file__, "review", "--render", "/fixture/missing-render.mp4"])):
    code = codex_judge.main()
receipt = json.loads(blocked_stdout.getvalue())
expect(code == 0 and receipt["status"] == "not_run", repr(receipt))
expect(receipt["backend"]["watched"] is False and receipt["backend"]["frames_sent"] == 0, repr(receipt))

# Antigravity remains unready when its native broker route is unavailable.
with mock.patch.object(antigravity_judge.shutil, "which", return_value=None):
    agy_missing = antigravity_judge.detect()
expect(not agy_missing["found"] and not agy_missing["judge_ready"], repr(agy_missing))
agy_stdout = io.StringIO()
with (mock.patch.object(antigravity_judge.judge, "probe_duration_s", side_effect=AssertionError("render probed")),
      mock.patch.object(antigravity_judge.judge, "extract_frames", side_effect=AssertionError("frames extracted")),
      contextlib.redirect_stdout(agy_stdout),
      argv([antigravity_judge.__file__, "review", "--render", "/fixture/missing-render.mp4"])):
    agy_code = antigravity_judge.main()
agy_receipt = json.loads(agy_stdout.getvalue())
expect(agy_code == 0 and agy_receipt["status"] == "not_run", repr(agy_receipt))
expect(agy_receipt["backend"]["frames_sent"] == 0, repr(agy_receipt))

# Auto selection uses affirmative capability and retains the configured order.
# All entries are mocked; no adapter or provider runs.
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

with (
    mock.patch.object(ladder_judge.cli_judge, "detect_providers", return_value={"claude": {"found": False}}),
    mock.patch.object(ladder_judge.codex_judge, "detect", return_value={"provider": "codex", "found": True, "judge_ready": True, "availability_reason": None}),
    mock.patch.object(ladder_judge.antigravity_judge, "detect", return_value={"provider": "antigravity", "found": True, "judge_ready": False, "availability_reason": REASON}),
    mock.patch.object(ladder_judge.grok_judge, "detect", return_value={"provider": "grok", "found": True, "judge_ready": True}),
):
    capable_ladder = ladder_judge.detect_ladder()
expect(capable_ladder["auto_selected"] == "codex", repr(capable_ladder))
expect(capable_ladder["rungs"][2]["judge_ready"] is False, repr(capable_ladder))

# A forced unready provider retains the explicit selection and cannot fall through.
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

print("PASS capability-based render-judge admission boundary")
`;

if (!python) {
  test("capability-based render-judge admission", { skip: "Python is not available" }, () => {});
} else {
  test("capability-based render-judge admission", () => {
    const result = spawnSync(python, ["-c", source, ROOT], {
      cwd: ROOT,
      encoding: "utf8",
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /PASS capability-based render-judge admission boundary/);
  });
}
