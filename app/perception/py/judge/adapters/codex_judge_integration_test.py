"""Local adapter wiring fixtures; no provider or render subprocess starts."""

import os
import sys
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(__file__))
import codex_judge as adapter


class CodexAdapterWiringTests(unittest.TestCase):
    def test_ordinary_installed_route_uses_maintained_command_consumer(self):
        with patch.object(adapter.shutil, "which", return_value="C:\\npm\\codex.cmd"), \
                patch.object(adapter.provider_child_launch, "command", return_value=["node", "codex.js"]) as command:
            prefix, env = adapter._child_route(None)
        self.assertEqual(prefix, ["node", "codex.js"])
        self.assertIsInstance(env, dict)
        self.assertEqual(command.call_args.args[0]["executable"], "C:\\npm\\codex.cmd")
        self.assertIsNone(command.call_args.args[0]["entrypoint"])

    def test_admitted_route_never_resolves_path(self):
        launch = {"executable": "/admitted/codex", "entrypoint": None,
                  "environment": {"PATH": "/admitted"}}
        with patch.object(adapter.shutil, "which", side_effect=AssertionError("PATH fallback")), \
                patch.object(adapter.provider_child_launch, "command", return_value=["/admitted/codex"]), \
                patch.object(adapter.provider_child_launch, "environment", return_value={"PATH": "/admitted"}):
            self.assertEqual(adapter._child_route(launch)[0], ["/admitted/codex"])

    def test_project_bundle_is_retained_but_model_cwd_is_neutral_and_removed(self):
        with tempfile.TemporaryDirectory() as project, tempfile.TemporaryDirectory() as temporary:
            staging = os.path.join(project, "audit-bundle")
            render = os.path.join(project, "sample.mp4")
            Path(render).write_bytes(b"synthetic")
            args = SimpleNamespace(render=render, max_frames=1, width=64,
                                   timeout=5, bundle_dir=staging, keep_bundle=True,
                                   perception=None, mode="global", windows=None,
                                   fps=None, intent="inspect", window_reason=None,
                                   cli_model="gpt-6.1-sol")
            observed = {}

            def frames(_render, directory, *_args, **_kwargs):
                os.makedirs(directory, exist_ok=True)
                path = os.path.join(directory, "f1.jpg")
                Path(path).write_bytes(b"frame")
                return [{"path": path, "time_ms": 0}]

            def invoke(_launch, _sys, _user, _model, paths, cwd, _timeout):
                observed["cwd"] = cwd
                observed["paths"] = paths
                observed["frame_bytes"] = Path(paths[0]).read_bytes()
                observed["files"] = sorted(
                    os.path.relpath(os.path.join(root, name), cwd)
                    for root, _, names in os.walk(cwd) for name in names)
                return None, {"available": False}, "native pending"

            with patch.object(adapter, "_job_temp", return_value=temporary), \
                    patch.object(adapter.judge, "probe_duration_s", return_value=1.0), \
                    patch.object(adapter.cli_judge, "resolve_perception", return_value=({}, "fixture", [])), \
                    patch.object(adapter.judge, "sanity_check_perception"), \
                    patch.object(adapter.judge, "extract_frames", side_effect=frames), \
                    patch.object(adapter.judge, "build_prompts", return_value=("system", "user")), \
                    patch.object(adapter, "invoke_codex", side_effect=invoke):
                receipt = adapter.review(args, None)
            self.assertEqual(receipt["status"], "not_run")
            self.assertEqual(receipt["bundle_dir"], staging)
            self.assertEqual(observed["files"], ["frames/f1.jpg"])
            self.assertEqual(observed["frame_bytes"], b"frame")
            self.assertTrue(os.path.commonpath((temporary, observed["cwd"])) == temporary)
            self.assertFalse(os.path.exists(observed["cwd"]))
            self.assertTrue(os.path.isfile(os.path.join(staging, "frames", "f1.jpg")))

    def _cleanup_case(self, failure=None, missing=False, exception=False, keep=False):
        with tempfile.TemporaryDirectory() as project, tempfile.TemporaryDirectory() as temporary:
            render = Path(project, "sample.mp4")
            render.write_bytes(b"synthetic")
            args = SimpleNamespace(render=str(render), max_frames=1, width=64,
                                   timeout=5, bundle_dir=None, keep_bundle=keep,
                                   perception=None, mode="global", windows=None,
                                   fps=None, intent="inspect", window_reason=None,
                                   cli_model="fixture")
            observed = {}

            def frames(_render, directory, *_args, **_kwargs):
                os.makedirs(directory, exist_ok=True)
                path = Path(directory, "f1.jpg")
                path.write_bytes(b"synthetic frame")
                observed["staging"] = str(path.parent.parent)
                return [{"path": str(path), "time_ms": 0}]

            def invoke(_launch, _sys, _user, _model, _paths, cwd, _timeout):
                observed["execution"] = cwd
                if exception:
                    raise ValueError("original fixture failure")
                return {"fixture": True}, {"available": True}, None

            real_remove = adapter.shutil.rmtree
            def remove(path, *pos, **kw):
                if path == observed.get(failure):
                    raise PermissionError("synthetic lock")
                if missing and path == observed.get("execution"):
                    real_remove(path)
                    raise FileNotFoundError(path)
                return real_remove(path, *pos, **kw)

            with ExitStack() as stack:
                for obj, name, kwargs in [
                    (adapter, "_job_temp", {"return_value": temporary}),
                    (adapter.judge, "probe_duration_s", {"return_value": 1.0}),
                    (adapter.cli_judge, "resolve_perception", {"return_value": ({}, "fixture", [])}),
                    (adapter.judge, "sanity_check_perception", {}),
                    (adapter.judge, "extract_frames", {"side_effect": frames}),
                    (adapter.judge, "build_prompts", {"return_value": ("system", "user")}),
                    (adapter, "invoke_codex", {"side_effect": invoke}),
                    (adapter.cli_judge, "post_filter_review", {"return_value": ({"verdict": "ok"}, {})}),
                    (adapter.cli_judge, "detect_frame_read_failure", {"return_value": (False, None)}),
                    (adapter.shutil, "rmtree", {"side_effect": remove}),
                ]:
                    stack.enter_context(patch.object(obj, name, **kwargs))
                if exception:
                    with self.assertRaises(adapter.CleanupError) as caught:
                        adapter.review(args, None)
                    self.assertIsInstance(caught.exception.original, ValueError)
                    self.assertIs(caught.exception.__cause__, caught.exception.original)
                    self.assertEqual(caught.exception.retained["execution_bundle_dir"], observed["execution"])
                    self.assertFalse(os.path.exists(observed["staging"]))
                    return
                receipt = adapter.review(args, None)
            self.assertEqual(receipt["status"], "completed")
            self.assertEqual(receipt["review"], {"verdict": "ok"})
            for kind, key in (("execution", "execution_bundle_dir"), ("staging", "bundle_dir")):
                retained = kind == failure or (kind == "staging" and keep)
                self.assertEqual(os.path.exists(observed[kind]), retained)
                self.assertEqual(receipt[key], observed[kind] if retained else None)
            if failure:
                self.assertTrue(any("remove" in warning for warning in receipt["warnings"]))
            else:
                self.assertEqual(receipt["warnings"], [])

    def test_owned_cleanup_success_and_already_absent(self):
        self._cleanup_case()
        self._cleanup_case(missing=True)

    def test_cleanup_failure_reports_each_retained_directory(self):
        self._cleanup_case(failure="execution")
        self._cleanup_case(failure="staging")

    def test_keep_bundle_preserves_staging_but_cleans_execution(self):
        self._cleanup_case(keep=True)

    def test_cleanup_failure_preserves_original_exception_and_other_cleanup(self):
        self._cleanup_case(failure="execution", exception=True)

    def test_frame_preparation_cleanup_failure_retains_original_and_cleans_staging(self):
        with tempfile.TemporaryDirectory() as temporary:
            render = Path(temporary, "sample.mp4")
            render.write_bytes(b"synthetic")
            args = SimpleNamespace(render=str(render), max_frames=1, width=64,
                                   timeout=5, bundle_dir=None, keep_bundle=False,
                                   perception=None, mode="global", windows=None,
                                   fps=None, intent="inspect", window_reason=None,
                                   cli_model="fixture")
            observed = {}
            def frames(_render, directory, *_args, **_kwargs):
                observed["staging"] = str(Path(directory).parent)
                return [{"path": str(Path(directory, "f.jpg")), "time_ms": 0}]
            original = ValueError("original frame failure")
            failure = adapter.codex_judge_runtime.FrameCleanupError("/retained/owned", original)
            with patch.object(adapter, "_job_temp", return_value=temporary), \
                    patch.object(adapter.judge, "probe_duration_s", return_value=1.0), \
                    patch.object(adapter.cli_judge, "resolve_perception", return_value=({}, "fixture", [])), \
                    patch.object(adapter.judge, "sanity_check_perception"), \
                    patch.object(adapter.judge, "extract_frames", side_effect=frames), \
                    patch.object(adapter.codex_judge_runtime, "prepare_neutral_frames", side_effect=failure):
                with self.assertRaises(adapter.CleanupError) as caught:
                    adapter.review(args, None)
            self.assertIs(caught.exception.original, original)
            self.assertEqual(caught.exception.retained, {"execution_bundle_dir": "/retained/owned"})
            self.assertFalse(os.path.exists(observed["staging"]))

    def test_frame_copy_failure_reports_unremoved_execution_directory(self):
        runtime = adapter.codex_judge_runtime
        with tempfile.TemporaryDirectory() as project, tempfile.TemporaryDirectory() as temporary:
            Path(project, "frame.jpg").write_bytes(b"synthetic")
            real_remove = runtime.shutil.rmtree
            def remove(path, *args, **kwargs):
                if Path(path).name.startswith("codex_judge_exec_"):
                    raise PermissionError("synthetic lock")
                return real_remove(path, *args, **kwargs)
            with patch.object(runtime.shutil, "copyfileobj", side_effect=OSError("copy failed")), \
                    patch.object(runtime.shutil, "rmtree", side_effect=remove):
                with self.assertRaises(runtime.FrameCleanupError) as caught:
                    runtime.prepare_neutral_frames(project, ["frame.jpg"], temporary)
            self.assertEqual(str(caught.exception.original), "codex_frame_copy_failed")
            self.assertTrue(os.path.isdir(caught.exception.retained_directory))

    def test_future_cli_version_does_not_fail_capability_admission(self):
        help_text = " ".join(adapter.codex_judge_policy.TRANSPORT_FLAGS)
        with patch.object(adapter, "NATIVE_ADMISSION_VERIFIED", True), \
                patch.object(adapter, "_child_route", return_value=(["codex"], None)), \
                patch.object(adapter.shutil, "which", return_value="/bin/codex"), \
                patch.object(adapter.codex_judge_runtime, "run_bounded",
                             side_effect=[b"codex-cli 0.161.0", help_text.encode()]), \
                patch.object(adapter.codex_judge_runtime, "premodel_probe",
                             return_value={"effective_policy": True}) as probe:
            result = adapter.detect(None)
        self.assertTrue(result["judge_ready"])
        self.assertEqual(result["version"], "codex-cli 0.161.0")
        self.assertFalse(os.path.exists(probe.call_args.args[2]))
        self.assertLessEqual(probe.call_args.args[4], 5.0)
        self.assertGreater(probe.call_args.kwargs["deadline_s"], adapter.time.monotonic())

    def test_detect_refuses_failed_effective_policy_and_cleans_owned_tmp(self):
        help_text = " ".join(adapter.codex_judge_policy.TRANSPORT_FLAGS)
        with patch.object(adapter, "NATIVE_ADMISSION_VERIFIED", True), \
                patch.object(adapter, "_child_route", return_value=(["codex"], None)), \
                patch.object(adapter.shutil, "which", return_value="/bin/codex"), \
                patch.object(adapter.codex_judge_runtime, "run_bounded",
                             side_effect=[b"codex-cli 0.162.0", help_text.encode()]), \
                patch.object(adapter.codex_judge_runtime, "premodel_probe",
                             side_effect=adapter.codex_judge_runtime.ProbeError(
                                 "codex_effective_policy_mcp_value")) as probe:
            result = adapter.detect(None)
        self.assertFalse(result["judge_ready"])
        self.assertEqual(result["availability_reason"], "codex_effective_policy_mcp_value")
        self.assertFalse(os.path.exists(probe.call_args.args[2]))

    def test_detect_before_native_admission_never_starts_app_server(self):
        help_text = " ".join(adapter.codex_judge_policy.TRANSPORT_FLAGS)
        with patch.object(adapter, "NATIVE_ADMISSION_VERIFIED", False), \
                patch.object(adapter, "_child_route", return_value=(["codex"], None)), \
                patch.object(adapter.shutil, "which", return_value="/bin/codex"), \
                patch.object(adapter.codex_judge_runtime, "run_bounded",
                             side_effect=[b"codex-cli 0.162.0", help_text.encode()]), \
                patch.object(adapter.codex_judge_runtime, "premodel_probe") as probe:
            result = adapter.detect(None)
        self.assertFalse(result["judge_ready"])
        probe.assert_not_called()

    def test_frame_read_failure_is_not_reported_as_watched(self):
        with tempfile.TemporaryDirectory() as project, tempfile.TemporaryDirectory() as temporary:
            render = os.path.join(project, "sample.mp4")
            Path(render).write_bytes(b"synthetic")
            args = SimpleNamespace(render=render, max_frames=1, width=64,
                                   timeout=5, bundle_dir=os.path.join(project, "bundle"),
                                   keep_bundle=False, perception=None, mode="global",
                                   windows=None, fps=None, intent="inspect", window_reason=None,
                                   cli_model="gpt-6.1-sol")
            def frames(_render, directory, *_args, **_kwargs):
                os.makedirs(directory, exist_ok=True)
                path = os.path.join(directory, "f1.jpg")
                Path(path).write_bytes(b"frame")
                return [{"path": path, "time_ms": 0}]
            with patch.object(adapter, "_job_temp", return_value=temporary), \
                    patch.object(adapter.judge, "probe_duration_s", return_value=1.0), \
                    patch.object(adapter.cli_judge, "resolve_perception", return_value=({}, "fixture", [])), \
                    patch.object(adapter.judge, "sanity_check_perception"), \
                    patch.object(adapter.judge, "extract_frames", side_effect=frames), \
                    patch.object(adapter.judge, "build_prompts", return_value=("system", "user")), \
                    patch.object(adapter, "invoke_codex", return_value=({"verdict": "pass"},
                                                                         {"available": True}, None)), \
                    patch.object(adapter.cli_judge, "post_filter_review", return_value=({}, {})), \
                    patch.object(adapter.cli_judge, "detect_frame_read_failure",
                                 return_value=(True, "frame unavailable")):
                receipt = adapter.review(args, None)
            self.assertEqual(receipt["status"], "error")
            self.assertFalse(receipt["backend"]["watched"])
            self.assertEqual(receipt["backend"]["frames_sent"], 1)

    def test_selected_image_model_flows_through_probe_and_review(self):
        with patch.object(adapter, "NATIVE_ADMISSION_VERIFIED", True), \
                patch.object(adapter, "detect", return_value={"judge_ready": True}), \
                patch.object(adapter, "_child_route", return_value=(["codex"], None)), \
                patch.object(adapter.codex_judge_runtime, "premodel_probe",
                             return_value={"model": "vision-fixture", "effective_policy": True}) as probe, \
                patch.object(adapter.codex_judge_runtime, "exec_review",
                             return_value=({"verdict": "pass"}, {})) as review:
            result, _, reason = adapter.invoke_codex(
                None, "system", "user", "vision-fixture", ["/tmp/frame.jpg"],
                "/tmp/neutral", 5)
        self.assertEqual(result, {"verdict": "pass"})
        self.assertIsNone(reason)
        self.assertEqual(probe.call_args.args[-1], "vision-fixture")
        self.assertEqual(review.call_args.args[0], ["codex"])


if __name__ == "__main__":
    unittest.main()
