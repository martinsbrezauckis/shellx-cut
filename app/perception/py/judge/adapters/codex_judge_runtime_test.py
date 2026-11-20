"""Pure protocol and argv checks; no installed provider is launched."""

import io
import json
import os
import queue
import sys
import tempfile
import time
import unittest
from collections import UserDict
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import codex_judge_runtime as runtime


class _CaptureInput(io.BytesIO):
    def close(self):
        self.value = self.getvalue()
        super().close()


class _DoneProcess:
    def __init__(self):
        self.stdin = _CaptureInput()

    def poll(self):
        return 0

    def communicate(self, timeout=None):
        return b"", b""

    def wait(self, timeout=None):
        return 0


class CodexRuntimeTests(unittest.TestCase):
    def test_default_local_environment_gate_reads_only_literal_file_metadata(self):
        with tempfile.TemporaryDirectory() as account:
            home = os.path.join(account, ".codex")
            os.mkdir(home)
            env = {"CODEX_HOME": home, "HOME": account}
            self.assertTrue(runtime.verify_default_local_environment(env))
            with patch.object(runtime.os, "environ", UserDict(env)):
                self.assertTrue(runtime.verify_default_local_environment(None))
            with patch.object(runtime.os, "environ", UserDict(
                    {**env, "CODEX_EXEC_SERVER_URL": "unread"})):
                with self.assertRaisesRegex(runtime.ProbeError,
                                            "remote_environment_selector_present"):
                    runtime.verify_default_local_environment(None)
            with open(os.path.join(home, "environments.toml"), "wb") as output:
                output.write(b"not read")
            with self.assertRaisesRegex(runtime.ProbeError, "environment_file_present"):
                runtime.verify_default_local_environment(env)
            os.unlink(os.path.join(home, "environments.toml"))
            with patch.object(runtime.os, "lstat", side_effect=PermissionError("private")):
                with self.assertRaisesRegex(runtime.ProbeError, "environment_file_unresolved"):
                    runtime.verify_default_local_environment(env)
            with self.assertRaisesRegex(runtime.ProbeError, "remote_environment_selector_present"):
                runtime.verify_default_local_environment({**env, "CODEX_EXEC_SERVER_URL": ""})
            with self.assertRaisesRegex(runtime.ProbeError, "remote_environment_selector_present"):
                runtime.verify_default_local_environment({**env,
                    "CODEX_EXEC_SERVER_NOISE_AUTH_TOKEN": "unread"})
            with patch.object(runtime.sys, "platform", "win32"):
                self.assertTrue(runtime.verify_default_local_environment(env))
                self.assertTrue(runtime.verify_default_local_environment(
                    {"codex_home": home}))
                with self.assertRaisesRegex(runtime.ProbeError, "remote_environment_selector_present"):
                    runtime.verify_default_local_environment(
                        {"codex_home": home, "codex_exec_server_url": "unread"})
                with self.assertRaisesRegex(runtime.ProbeError, "codex_home_unresolved"):
                    runtime.verify_default_local_environment(
                        {"codex_home": home, "CODEX_HOME": home})
                with patch.object(runtime, "_windows_profile_home", return_value=account):
                    self.assertTrue(runtime.verify_default_local_environment(
                        {"USERPROFILE": "/unrelated"}))
                with patch.object(runtime, "_windows_profile_home",
                                  side_effect=runtime.ProbeError("codex_home_unresolved")):
                    with self.assertRaisesRegex(runtime.ProbeError, "codex_home_unresolved"):
                        runtime.verify_default_local_environment({"USERPROFILE": account})

    def test_default_home_follows_posix_home_then_account_fallback(self):
        if sys.platform == "win32":
            self.skipTest("POSIX account-home route")
        with tempfile.TemporaryDirectory() as account:
            with patch("pwd.getpwuid", return_value=SimpleNamespace(pw_dir=account)):
                self.assertTrue(runtime.verify_default_local_environment({"HOME": account}))
                self.assertTrue(runtime.verify_default_local_environment({"HOME": ""}))
                with self.assertRaisesRegex(runtime.ProbeError, "codex_home_unresolved"):
                    runtime.verify_default_local_environment({"HOME": "relative"})

    def test_neutral_frames_copy_only_selected_and_reject_symlink(self):
        with tempfile.TemporaryDirectory() as staging, tempfile.TemporaryDirectory() as temporary:
            os.mkdir(os.path.join(staging, "frames"))
            selected = os.path.join(staging, "frames", "one.jpg")
            with open(selected, "wb") as output:
                output.write(b"selected")
            with open(os.path.join(staging, "private.txt"), "wb") as output:
                output.write(b"unselected")
            root, paths = runtime.prepare_neutral_frames(staging, ["frames/one.jpg"], temporary)
            try:
                with open(paths[0], "rb") as source:
                    self.assertEqual(source.read(), b"selected")
                self.assertFalse(os.path.exists(os.path.join(root, "private.txt")))
            finally:
                runtime.shutil.rmtree(root)
            os.symlink(os.path.join(staging, "private.txt"), os.path.join(staging, "frames", "link.jpg"))
            with self.assertRaises(runtime.ProbeError):
                runtime.prepare_neutral_frames(staging, ["frames/link.jpg"], temporary)

    def test_neutral_execution_refuses_project_local_temp(self):
        with tempfile.TemporaryDirectory() as project:
            staging = os.path.join(project, "bundle")
            temporary = os.path.join(staging, "tmp")
            os.makedirs(temporary)
            with self.assertRaisesRegex(runtime.ProbeError, "inside_project"):
                runtime.prepare_neutral_frames(staging, ["frames/one.jpg"], temporary)
            sibling_temp = os.path.join(project, "tmp")
            os.mkdir(sibling_temp)
            with self.assertRaisesRegex(runtime.ProbeError, "inside_project"):
                runtime.prepare_neutral_frames(
                    staging, ["frames/one.jpg"], sibling_temp,
                    forbidden_roots=[project])

    def test_different_windows_drives_are_disjoint(self):
        with patch.object(runtime.os.path, "commonpath", side_effect=ValueError("different drives")):
            self.assertFalse(runtime._within("C:\\Temp", "D:\\Project"))

    def test_windows_job_cleanup_failures_are_reported(self):
        class Kernel:
            def __init__(self, terminate=True, close=True):
                self.terminate, self.close = terminate, close
            def TerminateJobObject(self, *_args):
                return self.terminate
            def CloseHandle(self, *_args):
                return self.close
        job = runtime._WindowsChildJob.__new__(runtime._WindowsChildJob)
        job.handle = 123
        job._kernel = Kernel(terminate=False)
        with self.assertRaisesRegex(runtime.ProbeError, "terminate_failed"):
            job.stop()
        self.assertEqual(job.handle, 123)
        job._kernel = Kernel(close=False)
        with self.assertRaisesRegex(runtime.ProbeError, "close_failed"):
            job.stop()
        self.assertEqual(job.handle, 123)

    def test_stop_reports_owned_group_failure_even_after_leader_reaps(self):
        if sys.platform == "win32":
            self.skipTest("POSIX group cleanup")
        proc = SimpleNamespace(pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(),
                               stderr=io.BytesIO(), poll=lambda: 0, wait=lambda timeout: 0)
        with patch.object(runtime.os, "killpg", side_effect=PermissionError("private")):
            with self.assertRaisesRegex(runtime.ProbeError,
                                        "codex_child_group_terminate_failed"):
                runtime._stop(proc)
        self.assertTrue(proc.stdin.closed)

    def test_stop_accepts_absent_owned_group(self):
        if sys.platform == "win32":
            self.skipTest("POSIX group cleanup")
        proc = SimpleNamespace(pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(),
                               stderr=io.BytesIO(), poll=lambda: 0, wait=lambda timeout: 0)
        with patch.object(runtime.os, "killpg", side_effect=ProcessLookupError()):
            runtime._stop(proc)
        self.assertTrue(proc.stdout.closed)

    def test_stop_reports_unreaped_child_after_fallback_kill(self):
        proc = SimpleNamespace(pid=None, stdin=io.BytesIO(), stdout=io.BytesIO(),
                               stderr=io.BytesIO(), poll=lambda: None,
                               kill=lambda: None,
                               wait=lambda timeout: (_ for _ in ()).throw(
                                   runtime.subprocess.TimeoutExpired("child", timeout)))
        with self.assertRaisesRegex(runtime.ProbeError, "codex_child_reap_failed"):
            runtime._stop(proc)
        self.assertTrue(proc.stderr.closed)

    def test_stop_reports_failed_kill_and_reap_without_os_detail(self):
        proc = SimpleNamespace(pid=None, stdin=io.BytesIO(), stdout=None, stderr=None,
                               poll=lambda: None,
                               kill=lambda: (_ for _ in ()).throw(PermissionError("private")),
                               wait=lambda timeout: (_ for _ in ()).throw(
                                   runtime.subprocess.TimeoutExpired("child", timeout)))
        with self.assertRaisesRegex(runtime.ProbeError,
                                    "^codex_child_terminate_failed$"):
            runtime._stop(proc)
        self.assertTrue(proc.stdin.closed)

    def test_stop_preserves_windows_job_failure_and_closes_streams(self):
        class FailedJob:
            def stop(self):
                raise runtime.ProbeError("codex_child_job_terminate_failed")
        proc = SimpleNamespace(pid=123, _shellx_child_job=FailedJob(),
                               stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=io.BytesIO(),
                               wait=lambda timeout: 0)
        with self.assertRaisesRegex(runtime.ProbeError,
                                    "codex_child_job_terminate_failed"):
            runtime._stop(proc)
        self.assertTrue(proc.stdin.closed)

    def test_stop_reports_stream_close_error_after_reap(self):
        class FailedClose:
            def close(self):
                raise OSError("private descriptor detail")
        proc = SimpleNamespace(pid=None, stdin=FailedClose(), stdout=None, stderr=None,
                               poll=lambda: 0, wait=lambda timeout: 0)
        with self.assertRaisesRegex(runtime.ProbeError,
                                    "^codex_child_stream_close_failed$"):
            runtime._stop(proc)

    def test_stalled_stdin_writer_obeys_deadline(self):
        with tempfile.TemporaryDirectory() as job:
            started = time.monotonic()
            with self.assertRaisesRegex(runtime.ProbeError, "codex_child_timeout"):
                runtime.run_bounded(
                    [sys.executable, "-c", "import time; time.sleep(30)"],
                    job, os.environ.copy(), 0.2, b"x" * (4 * 1024 * 1024))
            self.assertLess(time.monotonic() - started, 3)

    @unittest.skipIf(sys.platform == "win32", "POSIX process-group fixture")
    def test_timeout_cleans_owned_descendant(self):
        with tempfile.TemporaryDirectory() as job:
            marker = os.path.join(job, "leaked-child.txt")
            child = ("import pathlib,time,sys; time.sleep(0.7); "
                     "pathlib.Path(sys.argv[1]).write_text('leaked')")
            parent = ("import subprocess,sys,time; "
                      "subprocess.Popen([sys.executable,'-c',sys.argv[1],sys.argv[2]]); "
                      "time.sleep(30)")
            with self.assertRaisesRegex(runtime.ProbeError, "codex_child_timeout"):
                runtime.run_bounded(
                    [sys.executable, "-c", parent, child, marker],
                    job, os.environ.copy(), 0.2)
            time.sleep(0.9)
            self.assertFalse(os.path.exists(marker))

    def test_child_error_classification_never_returns_provider_text(self):
        secret = b"TOKEN=private-value; unknown feature: shell_tool"
        code = runtime._child_exit_code(secret)
        self.assertEqual(code, "codex_config_unsupported_key")
        self.assertNotIn("private-value", code)

    def test_config_read_sends_initialized_sequence_and_keeps_values_in_memory(self):
        events = queue.Queue()
        events.put(("stdout", b'{"id":1,"result":{}}\n'))
        events.put(("stdout", b'{"id":2,"result":{"config":{"mcp_servers":{"private":{"env":{"TOKEN":"secret"}}}}}}\n'))
        proc = _DoneProcess()
        with patch.object(runtime, "_spawn", return_value=(proc, events)):
            response = runtime.config_read(["codex", "app-server"], "/tmp/job", {}, 1)
        sent = [json.loads(line) for line in proc.stdin.value.splitlines()]
        self.assertEqual([message["method"] for message in sent],
                         ["initialize", "initialized", "config/read"])
        self.assertFalse(sent[-1]["params"]["includeLayers"])
        self.assertEqual(response["config"]["mcp_servers"]["private"]["env"]["TOKEN"], "secret")

    def test_config_read_pages_all_effective_features_in_same_child(self):
        events = queue.Queue()
        for response in ({"id": 1, "result": {}},
                         {"id": 2, "result": {"config": {"features": {"shell_tool": False}}}},
                         {"id": 3, "result": {"data": [{"name": "shell_tool",
                                                          "enabled": False, "defaultEnabled": True}],
                                               "nextCursor": "1"}},
                         {"id": 4, "result": {"data": [{"name": "view_image",
                                                          "enabled": False, "defaultEnabled": True}],
                                               "nextCursor": None}},
                         {"id": 5, "result": {"data": [{"cwd": "/tmp/job", "hooks": [],
                                                          "warnings": [], "errors": []}]}}):
            events.put(("stdout", json.dumps(response).encode()))
        proc = _DoneProcess()
        diagnostic = {}
        with patch.object(runtime, "_spawn", return_value=(proc, events)):
            response, features, hooks = runtime.config_read(
                ["codex", "app-server"], "/tmp/job", {}, 1,
                include_features=True, diagnostics=diagnostic)
        self.assertEqual(len(features), 2)
        self.assertEqual(hooks["data"][0]["hooks"], [])
        self.assertEqual(response["config"]["features"], {"shell_tool": False})
        self.assertEqual(diagnostic["explicit_unadmitted_feature_names"], ())
        sent = [json.loads(line) for line in proc.stdin.value.splitlines()]
        self.assertEqual([item["method"] for item in sent],
                         ["initialize", "initialized", "config/read",
                          "experimentalFeature/list", "experimentalFeature/list", "hooks/list"])
        self.assertEqual(sent[-2]["params"]["cursor"], "1")
        self.assertEqual(sent[-1]["params"]["cwds"], ["/tmp/job"])

    def test_premodel_probe_pins_catalog_and_disables_all_effective_mcp(self):
        bundled = {"models": [{"slug": "vision-fixture", "supported_in_api": True,
                               "input_modalities": ["text", "image"],
                               "shell_type": "unified_exec",
                               "apply_patch_tool_type": "freeform"}]}
        with tempfile.TemporaryDirectory() as job:
            responses = [
                {"config": {"mcp_servers": {"docs": {"env": {"SECRET": "value"}}}}},
                ({"origins": {leaf: {"name": {"type": "sessionFlags"}} for leaf in (
                    ["model_catalog_json", "web_search", "cloud.skills.enabled",
                     "tools.update_plan.enabled",
                     "tools.experimental_request_user_input.enabled",
                     "mcp_servers.docs.enabled"]
                    + [f"features.{name}" + (".enabled" if name in runtime.policy._STRUCTURED_FEATURES else "")
                       for name in runtime.policy._DISABLED_FEATURES])},
                 "config": {"model_catalog_json": os.path.join(job, "model-catalog.json"),
                            "web_search": "disabled", "cloud": {"skills": {"enabled": False}},
                            "hooks": None, "notify": None,
                            "features": {name: False for name in runtime.policy._DISABLED_FEATURES},
                            "mcp_servers": {"docs": {"enabled": False}}}},
                 [{"name": name, "enabled": False, "defaultEnabled": True}
                  for name in runtime.policy._DISABLED_FEATURES],
                 {"data": [{"cwd": job, "hooks": [], "warnings": [], "errors": []}]}),
            ]
            with patch.object(runtime, "run_bounded", return_value=json.dumps(bundled).encode()), \
                    patch.object(runtime, "config_read", side_effect=responses) as read, \
                    patch.object(runtime, "verify_default_local_environment", return_value=True):
                result = runtime.premodel_probe(
                    ["codex"], {}, job, os.path.join(job, "model-catalog.json"), 1,
                    "vision-fixture")
            self.assertEqual(result["model"], "vision-fixture")
            self.assertEqual(result["mcp_server_count"], 1)
            self.assertIn("mcp_servers.docs.enabled=false", result["overrides"])
            self.assertIn("features.code_mode_only=false", result["overrides"])
            self.assertEqual(read.call_count, 2)
            with open(os.path.join(job, "model-catalog.json"), encoding="utf-8") as source:
                self.assertEqual(json.load(source)["models"][0]["tool_mode"], "direct")

    def test_bundled_catalog_refusal_categories_are_fixed(self):
        cases = [
            (b"not json", "codex_bundled_catalog_json_invalid"),
            (b'{"future":true}', "codex_bundled_catalog_top_shape"),
            (b'{"models":[]}', "codex_bundled_catalog_selected_absent"),
            (b'{"models":[{"slug":"vision","supported_in_api":true,'
             b'"input_modalities":["text","image"],"future_file_tool":true}]}',
             "codex_bundled_catalog_unknown_field"),
        ]
        for raw, expected in cases:
            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as job, \
                    patch.object(runtime, "run_bounded", return_value=raw), \
                    patch.object(runtime, "verify_default_local_environment", return_value=True):
                diagnostic = {}
                with self.assertRaisesRegex(runtime.ProbeError, "^" + expected + "$"):
                    runtime.premodel_probe(["codex"], {}, job,
                                           os.path.join(job, "model-catalog.json"), 1,
                                           "vision", diagnostics=diagnostic)
                self.assertEqual(diagnostic["bundled_bytes"], len(raw))
                if expected.endswith("unknown_field"):
                    self.assertEqual(diagnostic["json_shape"], "object")
                    self.assertEqual(diagnostic["models_shape"], "array")
                    self.assertEqual(diagnostic["selected_matches"], 1)
                    self.assertEqual(diagnostic["failure_category"], "unknown_field")
                    self.assertEqual(diagnostic["model_field_path"], "future_file_tool")
                self.assertNotIn("models", diagnostic)

    def test_exec_review_attaches_each_image_and_schema_with_no_ignore_flag(self):
        with tempfile.TemporaryDirectory() as job:
            catalog = b'{}'
            with open(os.path.join(job, "model-catalog.json"), "wb") as output:
                output.write(catalog)
            frame = os.path.join(job, "one.jpg")
            with open(frame, "wb") as output:
                output.write(b"jpeg")
            review = {"verdict": "pass", "issues": [], "cannot_assess": [],
                      "confidence": 0.9, "summary": "Visible frames are clean."}
            probe = {"model": "gpt-6.1-sol", "effective_policy": True,
                     "local_environment_verified": True,
                     "catalog_sha256": runtime.hashlib.sha256(catalog).hexdigest(),
                     "mcp_server_count": 0, "overrides": ["features.shell_tool=false"]}
            observed = []

            def fake_run(argv, cwd, env, timeout, input_bytes=b""):
                observed.extend(argv)
                result = argv[argv.index("--output-last-message") + 1]
                with open(result, "w", encoding="utf-8") as output:
                    json.dump(review, output)
                self.assertIn(b"review this frame", input_bytes)
                return b""

            with patch.object(runtime, "run_bounded", side_effect=fake_run), \
                    patch.object(runtime, "verify_default_local_environment", return_value=True):
                parsed, meta = runtime.exec_review(
                    ["codex"], {}, job, [frame], {"type": "object"},
                    "review this frame", probe, 1)
            self.assertEqual(parsed, review)
            self.assertEqual(meta["frames_attached"], 1)
            self.assertEqual(observed[observed.index("--image") + 1], frame)
            self.assertIn("--output-schema", observed)
            self.assertNotIn("--ignore-user-config", observed)


if __name__ == "__main__":
    unittest.main()
