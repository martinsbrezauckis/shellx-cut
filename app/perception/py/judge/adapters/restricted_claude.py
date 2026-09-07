"""File-rooted Claude Read capability for untrusted render-judge prompts.

Claude's working directory is not a security boundary by itself. This helper
requires the CLI release which documents ``--restricted`` file confinement,
builds a new frame-only working root, and supplies the non-interactive argv
used by both the one-frame probe and the real review.

The boundary protects untrusted media, transcripts, intent, and caller-provided
staging contents/links. The installed CLI, Cut code, and other same-user host
processes remain trusted components for this release.
"""

from __future__ import annotations

import os
import re
import secrets
import shutil
import stat
import subprocess

MIN_RESTRICTED_VERSION = (2, 1, 248)
_VERSION_RE = re.compile(r"(?<![0-9.])(\d+)\.(\d+)\.(\d+)(?![0-9.])")


def _parse_version(text: str) -> tuple[int, int, int] | None:
    match = _VERSION_RE.search(text)
    if match is None:
        return None
    return tuple(int(part) for part in match.groups())


def _openat_available() -> bool:
    return (os.name != "nt" and hasattr(os, "O_NOFOLLOW")
            and hasattr(os, "O_DIRECTORY") and os.open in os.supports_dir_fd)


def _unsupported_descriptor_reason() -> str:
    return ("host lacks the descriptor-based no-follow traversal required for "
            "restricted Claude frame reads")


def restricted_frame_bundle_available() -> tuple[bool, str | None]:
    """Report whether this host can construct a confined Claude Read root."""
    if os.name == "nt":
        import restricted_claude_windows
        return restricted_claude_windows.available()
    if _openat_available():
        return True, None
    return False, _unsupported_descriptor_reason()


def resolve_restricted_claude(claude_bin: str) -> tuple[dict | None, str | None]:
    """Resolve and version-check a Claude executable without starting a model.

    Wrapper binaries are supported as long as their normal ``--version`` output
    contains a semantic Claude Code version. An unknown version is rejected:
    an older or opaque wrapper must not silently lose the restricted-read
    guarantee.
    """
    bundle_ready, bundle_reason = restricted_frame_bundle_available()
    if not bundle_ready:
        return None, bundle_reason
    path = shutil.which(claude_bin)
    if not path:
        return None, f"claude CLI not found ({claude_bin!r}) — honest not_run"
    try:
        cp = subprocess.run(
            [path, "--version"], capture_output=True, text=True,
            encoding="utf-8", timeout=15)
    except (subprocess.TimeoutExpired, OSError) as exc:
        return None, f"could not verify Claude restricted-mode version: {exc}"
    version_text = (cp.stdout.strip() or cp.stderr.strip())[:240]
    if cp.returncode != 0:
        return None, (
            "could not verify Claude restricted-mode version "
            f"(exit {cp.returncode}): {version_text or '(no version output)'}")
    version = _parse_version(version_text)
    if version is None:
        return None, (
            "Claude version is not parseable; restricted file confinement "
            f"requires Claude Code >= {'.'.join(map(str, MIN_RESTRICTED_VERSION))}")
    if version < MIN_RESTRICTED_VERSION:
        return None, (
            f"Claude Code {'.'.join(map(str, version))} lacks the required "
            "restricted file-confinement contract; requires >= "
            f"{'.'.join(map(str, MIN_RESTRICTED_VERSION))}")
    return {"path": path, "version": version_text, "version_tuple": version}, None


def restricted_claude_argv(cli: dict, model: str, *, json_schema: str | None) -> list[str]:
    """Return the fixed capability boundary for a Claude judge invocation."""
    argv = [
        cli["path"], "--safe-mode", "--restricted", "--strict-mcp-config",
        "--disallowedTools", "mcp__*", "--permission-prompts", "none",
        "--no-chrome", "-p", "--output-format", "json", "--model", model,
        "--tools", "Read", "--no-session-persistence",
    ]
    if json_schema is not None:
        argv += ["--json-schema", json_schema]
    return argv


def _safe_relative_frame_path(value: object) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError("frame path must be a non-empty relative path")
    if os.path.isabs(value):
        raise ValueError(f"frame path is absolute: {value!r}")
    normalized = os.path.normpath(value)
    if normalized in (".", "..") or normalized.startswith(".." + os.sep):
        raise ValueError(f"frame path escapes the staging bundle: {value!r}")
    if normalized.split(os.sep, 1)[0] != "frames":
        raise ValueError(f"frame path is outside the staging frames directory: {value!r}")
    return normalized


def _path_error(kind: str, exc: OSError) -> ValueError:
    detail = exc.strerror or str(exc)
    return ValueError(f"{kind} was refused by no-follow traversal: {detail}")


def _open_root_directory(path: str) -> int:
    """Open every absolute path component without following a symlink."""
    if not _openat_available():
        raise ValueError(_unsupported_descriptor_reason())
    absolute = os.path.abspath(path)
    root_fd = os.open(os.path.sep, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in (part for part in absolute.split(os.path.sep) if part):
            try:
                child_fd = os.open(
                    component, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY,
                    dir_fd=root_fd)
            except OSError as exc:
                raise _path_error(f"directory component {component!r}", exc) from exc
            os.close(root_fd)
            root_fd = child_fd
        if not stat.S_ISDIR(os.fstat(root_fd).st_mode):
            raise ValueError("restricted frame root is not a directory")
        return root_fd
    except Exception:
        os.close(root_fd)
        raise


def _open_regular_frame(root_fd: int, relative: str) -> int:
    """Open a singly-linked frame through descriptor-bound no-follow traversal."""
    components = relative.split(os.sep)
    parent_fd = os.dup(root_fd)
    try:
        for component in components[:-1]:
            child_fd = os.open(
                component, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY,
                dir_fd=parent_fd)
            os.close(parent_fd)
            parent_fd = child_fd
        # FIFOs and other nonregular nodes must be rejected after fstat, but
        # opening a FIFO for read first would block the judge forever.  The
        # nonblocking flag preserves regular-file reads and lets validation
        # fail closed for every other node type.
        frame_fd = os.open(components[-1], os.O_RDONLY | os.O_NOFOLLOW
                           | getattr(os, "O_NONBLOCK", 0),
                           dir_fd=parent_fd)
    except OSError as exc:
        raise _path_error(f"frame {relative!r}", exc) from exc
    finally:
        os.close(parent_fd)
    try:
        info = os.fstat(frame_fd)
    except OSError as exc:
        os.close(frame_fd)
        raise _path_error(f"frame {relative!r}", exc) from exc
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        os.close(frame_fd)
        raise ValueError("frame must be a singly-linked regular file")
    return frame_fd


def _create_execution_root(parent_fd: int, parent_path: str) -> tuple[str, str, int]:
    """Create an exclusive sibling directory without reopening its parent path."""
    for _ in range(64):
        name = f".cli_judge_restricted_{secrets.token_hex(16)}"
        try:
            os.mkdir(name, 0o700, dir_fd=parent_fd)
        except FileExistsError:
            continue
        try:
            descriptor = os.open(
                name, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY,
                dir_fd=parent_fd)
        except OSError:
            try:
                os.rmdir(name, dir_fd=parent_fd)
            except OSError:
                pass
            raise
        return os.path.join(parent_path, name), name, descriptor
    raise ValueError("could not allocate an exclusive restricted frame root")


def _create_execution_target(root_fd: int, relative: str) -> int:
    """Create a frame destination beneath the bound execution-root descriptor."""
    components = relative.split(os.sep)
    parent_fd = os.dup(root_fd)
    try:
        for component in components[:-1]:
            try:
                os.mkdir(component, 0o700, dir_fd=parent_fd)
            except FileExistsError:
                pass
            child_fd = os.open(
                component, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY,
                dir_fd=parent_fd)
            os.close(parent_fd)
            parent_fd = child_fd
        return os.open(
            components[-1], os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600, dir_fd=parent_fd)
    except OSError as exc:
        raise _path_error(f"execution frame {relative!r}", exc) from exc
    finally:
        os.close(parent_fd)


def _copy_open_regular_file(source_fd: int, destination_fd: int) -> None:
    """Copy between already-bound descriptors without reopening either path."""
    while True:
        chunk = os.read(source_fd, 1 << 20)
        if not chunk:
            return
        view = memoryview(chunk)
        while view:
            view = view[os.write(destination_fd, view):]


def _remove_failed_execution_root(parent_fd: int, name: str) -> None:
    """Use Python's fd-safe rmtree only where it is documented as available."""
    if getattr(shutil.rmtree, "avoids_symlink_attacks", False):
        try:
            shutil.rmtree(name, dir_fd=parent_fd)
        except OSError:
            pass


def _create_posix_frame_only_bundle(staging_bundle: str,
                                    frame_relpaths: list[object]) -> str:
    if not _openat_available():
        raise ValueError(_unsupported_descriptor_reason())
    staging_path = os.path.abspath(staging_bundle)
    parent_path, staging_name = os.path.split(staging_path)
    if not staging_name:
        raise ValueError("staging bundle must not be the filesystem root")
    parent_fd = _open_root_directory(parent_path)
    staging_fd = -1
    execution_fd = -1
    execution_name: str | None = None
    try:
        try:
            staging_fd = os.open(
                staging_name, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY,
                dir_fd=parent_fd)
        except OSError as exc:
            raise _path_error("staging bundle", exc) from exc
        execution_root, execution_name, execution_fd = _create_execution_root(
            parent_fd, parent_path)
        for value in frame_relpaths:
            relative = _safe_relative_frame_path(value)
            source_fd = _open_regular_frame(staging_fd, relative)
            try:
                destination_fd = _create_execution_target(execution_fd, relative)
                try:
                    _copy_open_regular_file(source_fd, destination_fd)
                finally:
                    os.close(destination_fd)
            finally:
                os.close(source_fd)
        return execution_root
    except Exception:
        if execution_fd >= 0:
            os.close(execution_fd)
            execution_fd = -1
        if execution_name is not None:
            _remove_failed_execution_root(parent_fd, execution_name)
        raise
    finally:
        if execution_fd >= 0:
            os.close(execution_fd)
        if staging_fd >= 0:
            os.close(staging_fd)
        os.close(parent_fd)


def create_frame_only_bundle(staging_bundle: str,
                             frame_relpaths: list[object]) -> str:
    """Copy selected frames into the platform's confined Claude Read root."""
    if os.name == "nt":
        import restricted_claude_windows
        return restricted_claude_windows.create_frame_only_bundle(
            staging_bundle, frame_relpaths)
    return _create_posix_frame_only_bundle(staging_bundle, frame_relpaths)
