"""Bounded Codex child protocol for the render judge.

The only provider process identity comes from provider_child_launch. Raw
config/read responses and provider stderr never leave this module or enter a
receipt. No account or configuration files are opened by this adapter.
"""

from __future__ import annotations

import hashlib
import json
import os
import queue
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Mapping
from pathlib import Path

import codex_judge_policy as policy

_MAX_OUTPUT = 8 * 1024 * 1024
_MAX_LINE = 2 * 1024 * 1024

def _within(child: str, parent: str) -> bool:
    """Different Windows drives are disjoint absolute roots."""
    try:
        return os.path.commonpath((child, parent)) == parent
    except ValueError:
        return False

def prepare_neutral_frames(staging: str, relatives: list[str], temporary: str,
                           forbidden_roots: list[str] | None = None) -> tuple[str, list[str]]:
    """Copy only selected regular frames to a private OS/Runner temporary root."""
    if not relatives or not os.path.isdir(temporary):
        raise ProbeError("codex_frame_staging_invalid")
    temp_root = os.path.realpath(temporary)
    source_root = os.path.realpath(staging)
    for forbidden in [source_root, *(forbidden_roots or [])]:
        forbidden = os.path.realpath(forbidden)
        if _within(temp_root, forbidden):
            raise ProbeError("codex_execution_root_inside_project")
    root = tempfile.mkdtemp(prefix="codex_judge_exec_", dir=temporary)
    paths = []
    try:
        for relative in relatives:
            parts = relative.replace("\\", "/").split("/") if isinstance(relative, str) else []
            if (not parts or any(part in ("", ".", "..") for part in parts)
                    or os.path.isabs(relative)):
                raise ProbeError("codex_frame_path_invalid")
            source = os.path.join(staging, *parts)
            if (os.path.commonpath((source_root, os.path.realpath(source))) != source_root
                    or os.path.islink(source)):
                raise ProbeError("codex_frame_outside_staging")
            destination = os.path.join(root, *parts)
            os.makedirs(os.path.dirname(destination), mode=0o700, exist_ok=True)
            flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
            with os.fdopen(os.open(source, flags), "rb") as src:
                if not stat.S_ISREG(os.fstat(src.fileno()).st_mode):
                    raise ProbeError("codex_frame_not_regular")
                with open(destination, "xb") as dst:
                    shutil.copyfileobj(src, dst, 1 << 20)
            os.chmod(destination, 0o600)
            paths.append(destination)
        return root, paths
    except Exception as error:
        original = ProbeError("codex_frame_copy_failed") if isinstance(error, OSError) else error
        try:
            shutil.rmtree(root)
        except FileNotFoundError:
            pass
        except OSError:
            raise FrameCleanupError(root, original) from error
        if os.path.lexists(root):
            raise FrameCleanupError(root, original) from error
        if original is not error:
            raise original from error
        raise


class ProbeError(RuntimeError):
    """A fixed receipt-safe failure code, never provider-supplied text."""


class FrameCleanupError(ProbeError):
    """Frame preparation failed and its owned temporary directory remains."""

    def __init__(self, retained_directory: str, original: Exception):
        super().__init__("codex_frame_cleanup_failed")
        self.retained_directory = retained_directory
        self.original = original


def _windows_profile_home() -> str:
    """Mirror dirs-sys 0.5.0 FOLDERID_Profile for the current user."""
    import ctypes
    import uuid

    identifier = ctypes.create_string_buffer(
        uuid.UUID("5e6c858f-0e22-4760-9afe-ea3317b67173").bytes_le)
    result = ctypes.c_void_p()
    shell = ctypes.WinDLL("shell32", use_last_error=True)
    ole = ctypes.WinDLL("ole32", use_last_error=True)
    shell.SHGetKnownFolderPath.argtypes = [ctypes.c_void_p, ctypes.c_uint32,
                                           ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    shell.SHGetKnownFolderPath.restype = ctypes.c_long
    ole.CoTaskMemFree.argtypes = [ctypes.c_void_p]
    ole.CoTaskMemFree.restype = None
    try:
        if shell.SHGetKnownFolderPath(identifier, 0, None, ctypes.byref(result)) != 0:
            raise ProbeError("codex_home_unresolved")
        home = ctypes.wstring_at(result.value) if result.value else ""
        if not home or not os.path.isabs(home):
            raise ProbeError("codex_home_unresolved")
        return home
    finally:
        if result.value:
            ole.CoTaskMemFree(result)


def verify_default_local_environment(env: dict | None) -> bool:
    """Admit only the tagged CLI's unconfigured default local environment.

    `codex exec` does not select a thread environment. Codex can otherwise
    select remote executors (and their MCP bindings) from environments.toml or
    CODEX_EXEC_SERVER_* selectors. Check names and one literal file's existence;
    never read provider configuration or authentication contents.
    """
    child_env = os.environ if env is None else env
    if not isinstance(child_env, Mapping):
        raise ProbeError("codex_child_environment_invalid")
    windows = sys.platform == "win32"
    if any(isinstance(key, str) and (key.upper() if windows else key).startswith(
            "CODEX_EXEC_SERVER_") for key in child_env):
        raise ProbeError("codex_remote_environment_selector_present")
    home_keys = [key for key in child_env if isinstance(key, str)
                 and (key.upper() if windows else key) == "CODEX_HOME"]
    if len(home_keys) > 1:
        raise ProbeError("codex_home_unresolved")
    configured_home = child_env.get(home_keys[0]) if home_keys else None
    if configured_home is not None and configured_home != "":
        if not isinstance(configured_home, str) or not os.path.isabs(configured_home):
            raise ProbeError("codex_home_unresolved")
        home = os.path.realpath(configured_home)
        if not os.path.isdir(home):
            raise ProbeError("codex_home_unresolved")
    else:
        if windows:
            account_home = _windows_profile_home()
        else:
            account_home = child_env.get("HOME")
            if not account_home:
                # dirs-sys consults the account database only when HOME is
                # absent or empty on POSIX.
                try:
                    import pwd
                    account_home = pwd.getpwuid(os.getuid()).pw_dir
                except (ImportError, KeyError, OSError) as error:
                    raise ProbeError("codex_home_unresolved") from error
        if not isinstance(account_home, str) or not os.path.isabs(account_home):
            raise ProbeError("codex_home_unresolved")
        home = os.path.join(account_home, ".codex")
    try:
        os.lstat(os.path.join(home, "environments.toml"))
    except FileNotFoundError:
        return True
    except OSError as error:
        raise ProbeError("codex_environment_file_unresolved") from error
    raise ProbeError("codex_environment_file_present")


def _startup() -> dict:
    if sys.platform == "win32":
        # Popen closes the primary thread handle before returning. Keep the
        # process suspended until its exact owned Job is attached below.
        return {"creationflags": subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP
                | 0x00000004}  # CREATE_SUSPENDED (Win32)
    return {"start_new_session": True}


class _WindowsChildJob:
    """Terminate only this spawned CLI tree, nested under Runner's owning Job."""

    def __init__(self, proc: subprocess.Popen):
        import ctypes

        self._ctypes = ctypes
        self._kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self._kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
        self._kernel.CreateJobObjectW.restype = ctypes.c_void_p
        self._kernel.AssignProcessToJobObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        self._kernel.AssignProcessToJobObject.restype = ctypes.c_int
        self._kernel.TerminateJobObject.argtypes = [ctypes.c_void_p, ctypes.c_uint]
        self._kernel.TerminateJobObject.restype = ctypes.c_int
        self._kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        self._kernel.CloseHandle.restype = ctypes.c_int
        self.handle = self._kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ProbeError("codex_child_job_create_failed")
        # CREATE_NEW_PROCESS_GROUP does not break away from Runner's inherited
        # Job. Nested association gives local timeout cleanup an exact child
        # tree while Runner still owns cancellation of the enclosing job.
        if not self._kernel.AssignProcessToJobObject(self.handle, proc._handle):
            self._kernel.CloseHandle(self.handle)
            self.handle = None
            raise ProbeError("codex_child_job_assign_failed")

    def stop(self) -> None:
        if self.handle:
            if not self._kernel.TerminateJobObject(self.handle, 1):
                raise ProbeError("codex_child_job_terminate_failed")
            if not self._kernel.CloseHandle(self.handle):
                raise ProbeError("codex_child_job_close_failed")
            self.handle = None

    def resume_primary_thread(self, pid: int) -> None:
        """Resume the single initial thread only after Job assignment.

        Popen closes hThread; Toolhelp's documented thread snapshot recovers
        the suspended child's sole thread. Any ambiguity refuses before resume.
        """
        import ctypes
        from ctypes import wintypes

        class THREADENTRY32(ctypes.Structure):
            _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD),
                        ("th32ThreadID", wintypes.DWORD), ("th32OwnerProcessID", wintypes.DWORD),
                        ("tpBasePri", wintypes.LONG), ("tpDeltaPri", wintypes.LONG),
                        ("dwFlags", wintypes.DWORD)]

        k = self._kernel
        k.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
        k.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
        k.Thread32First.argtypes = [wintypes.HANDLE, ctypes.POINTER(THREADENTRY32)]
        k.Thread32First.restype = wintypes.BOOL
        k.Thread32Next.argtypes = [wintypes.HANDLE, ctypes.POINTER(THREADENTRY32)]
        k.Thread32Next.restype = wintypes.BOOL
        k.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        k.OpenThread.restype = wintypes.HANDLE
        k.ResumeThread.argtypes = [wintypes.HANDLE]
        k.ResumeThread.restype = wintypes.DWORD
        snapshot = k.CreateToolhelp32Snapshot(0x00000004, 0)
        if snapshot in (None, ctypes.c_void_p(-1).value):
            raise ProbeError("codex_child_thread_snapshot_failed")
        try:
            entry = THREADENTRY32()
            entry.dwSize = ctypes.sizeof(entry)
            thread_ids = []
            found = k.Thread32First(snapshot, ctypes.byref(entry))
            while found:
                if entry.th32OwnerProcessID == pid:
                    thread_ids.append(entry.th32ThreadID)
                found = k.Thread32Next(snapshot, ctypes.byref(entry))
            if len(thread_ids) != 1:
                raise ProbeError("codex_child_primary_thread_ambiguous")
            thread = k.OpenThread(0x0002, False, thread_ids[0])
            if not thread:
                raise ProbeError("codex_child_primary_thread_unavailable")
            try:
                if k.ResumeThread(thread) != 1:
                    raise ProbeError("codex_child_resume_refused")
            finally:
                k.CloseHandle(thread)
        finally:
            k.CloseHandle(snapshot)


def _reader(stream, destination: queue.Queue, label: str) -> None:
    used = 0
    try:
        while True:
            try:
                line = stream.readline(_MAX_LINE + 1)
            except (OSError, ValueError):
                break
            if not line:
                break
            used += len(line)
            if len(line) > _MAX_LINE or used > _MAX_OUTPUT:
                destination.put((label, None))
                return
            destination.put((label, line))
    finally:
        destination.put((label, b""))


def _spawn(command: list[str], cwd: str, env: dict | None) -> tuple[subprocess.Popen, queue.Queue]:
    proc = subprocess.Popen(
        command, cwd=cwd, env=env, stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, **_startup())
    if sys.platform == "win32":
        try:
            proc._shellx_child_job = _WindowsChildJob(proc)
            proc._shellx_child_job.resume_primary_thread(proc.pid)
        except ProbeError:
            _stop(proc)
            raise
    events: queue.Queue = queue.Queue()
    for label, stream in (("stdout", proc.stdout), ("stderr", proc.stderr)):
        threading.Thread(target=_reader, args=(stream, events, label), daemon=True).start()
    return proc, events


def _stop(proc: subprocess.Popen) -> None:
    job = getattr(proc, "_shellx_child_job", None)
    cleanup_error = None
    def failed(code: str) -> None:
        nonlocal cleanup_error
        if cleanup_error is None:
            cleanup_error = ProbeError(code)

    if job is not None:
        try:
            job.stop()
        except ProbeError as error:
            cleanup_error = error
    elif sys.platform != "win32" and getattr(proc, "pid", None):
        # start_new_session=True makes this PID the exact owned process group.
        # Kill it even if the leader already exited; children may remain.
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            # An absent group means there is no owned process tree left to kill.
            pass
        except OSError:
            # Killing only the leader cannot establish that its children died.
            failed("codex_child_group_terminate_failed")
            if proc.poll() is None:
                try:
                    proc.kill()
                except ProcessLookupError:
                    pass
                except OSError:
                    failed("codex_child_terminate_failed")
    elif proc.poll() is None:
        try:
            proc.kill()
        except ProcessLookupError:
            pass
        except OSError:
            failed("codex_child_terminate_failed")
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            proc.kill()
        except ProcessLookupError:
            pass
        except OSError:
            failed("codex_child_terminate_failed")
        try:
            proc.wait(timeout=3)
        except (OSError, subprocess.TimeoutExpired):
            failed("codex_child_reap_failed")
    except OSError:
        failed("codex_child_reap_failed")
    for stream in (getattr(proc, "stdin", None), getattr(proc, "stdout", None),
                   getattr(proc, "stderr", None)):
        try:
            if stream:
                stream.close()
        except ValueError:
            # Already-closed streams are harmless during repeated cleanup.
            pass
        except OSError:
            failed("codex_child_stream_close_failed")
    if cleanup_error is not None:
        raise cleanup_error


def _child_exit_code(stderr: bytes) -> str:
    """Classify known startup refusals without retaining provider text."""
    message = stderr.decode("utf-8", "replace").lower()
    if "unknown feature" in message or "unknown field" in message:
        return "codex_config_unsupported_key"
    if "unknown variant" in message or "invalid value" in message:
        return "codex_config_unsupported_value"
    if "model_catalog_json" in message or "model catalog" in message:
        return "codex_catalog_load_failed"
    if "sandbox" in message or "permission profile" in message:
        return "codex_sandbox_refused"
    if "auth" in message or "log in" in message:
        return "codex_auth_unavailable"
    return "codex_child_exit"


def run_bounded(command: list[str], cwd: str, env: dict | None,
                timeout_s: int, input_bytes: bytes = b"") -> bytes:
    """Capture a bounded stdout; stderr is consumed but never exposed."""
    proc, events = _spawn(command, cwd, env)
    deadline = time.monotonic() + timeout_s
    stdout = bytearray()
    stderr = bytearray()
    done = set()

    def writer() -> None:
        try:
            if proc.stdin:
                proc.stdin.write(input_bytes)
                proc.stdin.close()
            events.put(("stdin", b""))
        except (BrokenPipeError, OSError, ValueError):
            events.put(("stdin", None))

    threading.Thread(target=writer, daemon=True).start()
    try:
        while len(done) < 3:
            try:
                label, chunk = events.get(timeout=max(0.01, deadline - time.monotonic()))
            except queue.Empty as error:
                raise ProbeError("codex_child_timeout") from error
            if chunk is None:
                raise ProbeError("codex_child_io" if label == "stdin"
                                 else "codex_child_output_limit")
            if chunk == b"":
                done.add(label)
            elif label == "stdout":
                stdout.extend(chunk)
            elif len(stderr) < 8192:
                stderr.extend(chunk[:8192 - len(stderr)])
            if time.monotonic() > deadline:
                raise ProbeError("codex_child_timeout")
        try:
            exit_code = proc.wait(timeout=max(0.01, deadline - time.monotonic()))
        except subprocess.TimeoutExpired as error:
            raise ProbeError("codex_child_timeout") from error
        if exit_code != 0:
            raise ProbeError(_child_exit_code(stderr))
        return bytes(stdout)
    except (BrokenPipeError, OSError) as error:
        raise ProbeError("codex_child_io") from error
    finally:
        _stop(proc)


def config_read(command: list[str], cwd: str, env: dict | None,
                timeout_s: int = 20, include_features: bool = False,
                diagnostics: dict | None = None) -> dict | tuple[dict, list]:
    """Read effective config and optionally all feature defaults in one JSONL child."""
    proc, events = _spawn(command, cwd, env)
    deadline = time.monotonic() + timeout_s
    def send(message: dict) -> dict | None:
        if proc.stdin is None:
            raise ProbeError("codex_app_server_stdin")
        proc.stdin.write(json.dumps(message, separators=(",", ":")).encode() + b"\n")
        proc.stdin.flush()
        if "id" not in message:
            return None
        while True:
            try:
                label, line = events.get(timeout=max(0.01, deadline - time.monotonic()))
            except queue.Empty as error:
                raise ProbeError("codex_app_server_timeout") from error
            if line is None:
                raise ProbeError("codex_app_server_output_limit")
            if label == "stdout" and line:
                try:
                    response = json.loads(line)
                except (ValueError, UnicodeDecodeError) as error:
                    raise ProbeError("codex_app_server_protocol") from error
                if isinstance(response, dict) and response.get("id") == message["id"]:
                    if "error" in response or not isinstance(response.get("result"), dict):
                        raise ProbeError("codex_app_server_rejected")
                    return response["result"]
            if line == b"" and label == "stdout":
                raise ProbeError("codex_app_server_closed")
            if time.monotonic() > deadline:
                raise ProbeError("codex_app_server_timeout")
    try:
        send({"id": 1, "method": "initialize", "params": {
            "clientInfo": {"name": "shellx_cut_judge_probe",
                           "title": "ShellX Cut Judge Probe", "version": "0.6.121"},
            "capabilities": {"experimentalApi": True}}})
        send({"method": "initialized"})
        config = send({"id": 2, "method": "config/read", "params": {
            "cwd": cwd, "includeLayers": False}})
        if not include_features:
            return config
        if diagnostics is not None:
            diagnostics["explicit_unadmitted_feature_names"] = (
                policy.explicit_unadmitted_feature_names(config))
        features, cursor, seen_cursors = [], None, set()
        for request_id in range(3, 11):
            params = {"limit": 128}
            if cursor is not None:
                params["cursor"] = cursor
            page = send({"id": request_id, "method": "experimentalFeature/list",
                         "params": params})
            if not isinstance(page.get("data"), list) or not isinstance(page.get("nextCursor"),
                                                                          (str, type(None))):
                raise ProbeError("codex_feature_inventory_shape")
            features.extend(page["data"])
            if len(features) > 1024:
                raise ProbeError("codex_feature_inventory_limit")
            cursor = page["nextCursor"]
            if cursor is None:
                hooks = send({"id": request_id + 1, "method": "hooks/list",
                              "params": {"cwds": [cwd]}})
                return config, features, hooks
            if not re.fullmatch(r"[0-9]{1,6}", cursor) or cursor in seen_cursors:
                raise ProbeError("codex_feature_inventory_cursor")
            seen_cursors.add(cursor)
        raise ProbeError("codex_feature_inventory_limit")
    except (BrokenPipeError, OSError) as error:
        raise ProbeError("codex_app_server_io") from error
    finally:
        _stop(proc)
    raise ProbeError("codex_app_server_no_config")


def _config_argv(overrides: list[str]) -> list[str]:
    argv: list[str] = []
    for item in overrides:
        argv.extend(("-c", item))
    return argv


def _feature_only_overrides() -> list[str]:
    return policy.feature_only_overrides()


def premodel_probe(command_prefix: list[str], env: dict | None, cwd: str,
                   catalog_path: str, timeout_s: int = 20,
                   model_slug: str = "gpt-6.1-sol",
                   diagnostics: dict | None = None,
                   deadline_s: float | None = None) -> dict:
    """Return receipt-safe metadata or refuse before any model invocation."""
    if not os.path.isdir(cwd) or not os.path.isabs(cwd) or not os.path.isabs(catalog_path):
        raise ProbeError("codex_job_directory_invalid")
    if os.path.dirname(catalog_path) != os.path.abspath(cwd):
        raise ProbeError("codex_catalog_outside_job")
    verify_default_local_environment(env)
    def budget() -> float:
        remaining = timeout_s if deadline_s is None else min(timeout_s, deadline_s - time.monotonic())
        if remaining <= 0:
            raise ProbeError("codex_child_timeout")
        return remaining
    raw = run_bounded(command_prefix + ["debug", "models", "--bundled"],
                      cwd, env, budget())
    if diagnostics is not None:
        diagnostics["bundled_bytes"] = len(raw)
    try:
        bundled = json.loads(raw)
    except (ValueError, UnicodeDecodeError) as error:
        raise ProbeError("codex_bundled_catalog_json_invalid") from error
    if diagnostics is not None:
        diagnostics["json_shape"] = ("object" if isinstance(bundled, dict) else
                                     "array" if isinstance(bundled, list) else "scalar")
        models = bundled.get("models") if isinstance(bundled, dict) else None
        diagnostics["models_shape"] = ("array" if isinstance(models, list) else
                                        "other")
        diagnostics["selected_matches"] = (sum(isinstance(model, dict)
                                               and model.get("slug") == model_slug
                                               for model in models)
                                           if isinstance(models, list) else None)
    try:
        catalog = policy.build_job_catalog(bundled, model_slug)
    except ValueError as error:
        # These are policy-owned constant messages, never provider text.
        known = {
            "bundled Codex catalog has an unknown top-level field": "top_shape",
            "bundled Codex model catalog or selected model is missing": "model_list",
            "selected Codex model is absent from bundled catalog": "selected_absent",
            "selected Codex model is duplicated in bundled catalog": "selected_duplicate",
            "selected Codex model is unavailable in bundled catalog": "selected_unavailable",
            "selected Codex model does not advertise image input": "selected_no_image",
            "bundled Codex legacy instructions disagree with model messages": "legacy_instructions",
            "bundled Codex model contains an unknown object field": "unknown_field",
            "bundled Codex model contains an unknown array field": "unknown_array",
            "bundled Codex model has an invalid object value": "object_type",
            "bundled Codex model has an invalid optional object value": "optional_object_type",
            "bundled Codex model has an invalid array value": "array_type",
            "bundled Codex model has an invalid boolean value": "boolean_type",
            "bundled Codex model has an invalid integer value": "integer_type",
            "bundled Codex model has an unsupported value": "unsupported_value",
            "bundled Codex model has an invalid text value": "text_type",
        }
        category = known.get(str(error), "invalid")
        if diagnostics is not None:
            diagnostics["failure_category"] = category
            field = getattr(error, "path", None)
            if isinstance(field, str) and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_.\[\]-]{0,127}", field):
                diagnostics["model_field_path"] = field
        raise ProbeError("codex_bundled_catalog_" + category) from error
    encoded = json.dumps(catalog, ensure_ascii=False, separators=(",", ":")).encode()
    if len(encoded) > _MAX_OUTPUT:
        raise ProbeError("codex_job_catalog_too_large")
    path = Path(catalog_path)
    with path.open("xb") as output:
        output.write(encoded)
    os.chmod(path, 0o600)
    digest = hashlib.sha256(encoded).hexdigest()
    baseline = config_read(command_prefix + _config_argv(_feature_only_overrides())
                           + ["app-server"], cwd, env, budget())
    try:
        names = policy.effective_mcp_names(baseline)
    except ValueError as error:
        raise ProbeError("codex_mcp_metadata_invalid") from error
    overrides = policy.job_config_overrides(names, catalog_path)
    effective, inventory, hooks = config_read(command_prefix + _config_argv(overrides)
                                              + ["app-server"], cwd, env, budget(),
                                              include_features=True, diagnostics=diagnostics)
    if diagnostics is not None:
        diagnostics["inventory_unadmitted_feature_names"] = (
            policy.unadmitted_enabled_feature_names(inventory))
        diagnostics["still_enabled_disabled_feature_names"] = (
            policy.still_enabled_disabled_feature_names(inventory))
        diagnostics["effective_value_failure_categories"] = (
            policy.effective_value_diagnostics(effective, names, catalog_path))
        diagnostics["disabled_feature_value_shapes"] = (
            policy.disabled_feature_value_shapes(effective))
        diagnostics["feature_inventory_failure"] = (
            policy.feature_inventory_failure(inventory))
        diagnostics["feature_inventory_shape_diagnostics"] = (
            policy.feature_inventory_shape_diagnostics(inventory))
        diagnostics["hook_inventory_failure"] = (
            policy.hook_inventory_failure(hooks, cwd))
        diagnostics["enabled_unmanaged_hook_metadata"] = (
            policy.enabled_unmanaged_hook_metadata(hooks))
        hook_rows = hooks.get("data") if isinstance(hooks, dict) else None
        hook_rows = hook_rows[0].get("hooks") if isinstance(hook_rows, list) and len(hook_rows) == 1 and isinstance(hook_rows[0], dict) else None
        if isinstance(hook_rows, list):
            diagnostics["hook_counts"] = {
                "enabled_managed": sum(isinstance(row, dict) and row.get("enabled") is True
                                       and row.get("isManaged") is True for row in hook_rows),
                "enabled_unmanaged": sum(isinstance(row, dict) and row.get("enabled") is True
                                         and row.get("isManaged") is not True for row in hook_rows),
                "disabled": sum(isinstance(row, dict) and row.get("enabled") is False
                                for row in hook_rows),
            }
    failure = policy.effective_config_failure(effective, names, catalog_path)
    if failure is not None:
        raise ProbeError("codex_effective_policy_" + failure)
    feature_failure = policy.feature_inventory_failure(inventory)
    if feature_failure is not None:
        raise ProbeError("codex_feature_inventory_" + feature_failure)
    hook_failure = policy.hook_inventory_failure(hooks, cwd)
    if hook_failure is not None:
        raise ProbeError("codex_hook_inventory_" + hook_failure)
    return {"model": model_slug, "catalog_sha256": digest,
            "mcp_server_count": len(names), "effective_policy": True,
            "local_environment_verified": True,
            "overrides": overrides}


def exec_review(command_prefix: list[str], env: dict | None, cwd: str,
                frame_paths: list[str], schema: dict, prompt: str,
                probe: dict, timeout_s: int) -> tuple[dict, dict]:
    """One direct-image, strict-schema model turn after a reviewed admission."""
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import judge  # imported here to keep the premodel probe independent

    if (not probe.get("effective_policy") or probe.get("local_environment_verified") is not True
            or not isinstance(probe.get("model"), str)):
        raise ProbeError("codex_effective_policy_missing")
    catalog_path = os.path.join(cwd, "model-catalog.json")
    try:
        with open(catalog_path, "rb") as source:
            catalog_bytes = source.read(_MAX_OUTPUT + 1)
    except OSError as error:
        raise ProbeError("codex_job_catalog_missing") from error
    if (len(catalog_bytes) > _MAX_OUTPUT
            or hashlib.sha256(catalog_bytes).hexdigest() != probe.get("catalog_sha256")):
        raise ProbeError("codex_job_catalog_changed")
    if not frame_paths:
        raise ProbeError("codex_frames_missing")
    root = os.path.realpath(cwd)
    for frame in frame_paths:
        if (not os.path.isabs(frame) or os.path.commonpath((root, os.path.realpath(frame))) != root
                or not stat.S_ISREG(os.stat(frame, follow_symlinks=False).st_mode)):
            raise ProbeError("codex_frame_outside_job")
    schema_path = os.path.join(cwd, "verdict-schema.json")
    result_path = os.path.join(cwd, "verdict-result.json")
    try:
        with open(schema_path, "x", encoding="utf-8") as output:
            json.dump(schema, output, separators=(",", ":"))
        os.chmod(schema_path, 0o600)
        argv = command_prefix + _config_argv(probe["overrides"]) + [
            "exec", "--model", probe["model"], "--ephemeral", "--skip-git-repo-check",
            "--output-schema", schema_path, "--output-last-message", result_path,
        ]
        for frame in frame_paths:
            argv.extend(("--image", frame))
        argv.append("-")
        verify_default_local_environment(env)
        run_bounded(argv, cwd, env, timeout_s, prompt.encode("utf-8"))
        with open(result_path, "rb") as result:
            raw = result.read(1024 * 1024 + 1)
        if len(raw) > 1024 * 1024:
            raise ProbeError("codex_verdict_output_limit")
        review = json.loads(raw)
        if not isinstance(review, dict):
            raise ProbeError("codex_verdict_invalid")
        judge.validate_review(review)
        return review, {"model": probe["model"], "catalog_sha256": probe["catalog_sha256"],
                        "mcp_server_count": probe["mcp_server_count"],
                        "schema_enforced": True, "frames_attached": len(frame_paths),
                        "effective_policy": True, "local_environment_verified": True}
    except (OSError, ValueError, TypeError, UnicodeDecodeError) as error:
        raise ProbeError("codex_verdict_invalid") from error
