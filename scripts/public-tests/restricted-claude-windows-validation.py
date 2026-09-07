#!/usr/bin/env python3
"""Native, no-provider checks for the Windows restricted Claude frame copier."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from unittest import mock

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ADAPTERS = os.path.join(ROOT, "app", "perception", "py", "judge", "adapters")
sys.path.insert(0, ADAPTERS)

import restricted_claude_windows as restricted  # noqa: E402


FAILURES: list[str] = []


def check(name: str, condition: bool, detail: str = "") -> None:
    print(f"  [{'PASS' if condition else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def rejects(name: str, staging: str, relative: str) -> None:
    try:
        restricted.create_frame_only_bundle(staging, [relative])
    except (OSError, ValueError):
        check(name, True)
    else:
        check(name, False, "copy unexpectedly succeeded")


def junction(link: str, target: str) -> None:
    result = subprocess.run(["cmd", "/d", "/c", "mklink", "/J", link, target],
                            capture_output=True, text=True, encoding="utf-8")
    check("test fixture creates directory junction", result.returncode == 0,
          (result.stdout + result.stderr).strip())
    if result.returncode != 0:
        raise RuntimeError("cannot test Windows junction rejection")


def main() -> int:
    if os.name != "nt":
        print("SKIP restricted Claude Windows validation: non-Windows host")
        return 0
    supported, reason = restricted.available()
    check("typed Win32 handle API is available", supported, str(reason))
    if not supported:
        return 1
    api = restricted._api()
    check("CreateFileW returns pointer-width HANDLE", api[0].CreateFileW.restype is restricted.wintypes.HANDLE,
          repr(api[0].CreateFileW.restype))
    with tempfile.TemporaryDirectory(prefix="restricted_claude_windows_") as parent:
        staging, frames = os.path.join(parent, "staging"), os.path.join(parent, "staging", "frames")
        outside = os.path.join(parent, "outside")
        os.makedirs(frames)
        os.makedirs(outside)
        frame, secret = os.path.join(frames, "frame.jpg"), os.path.join(outside, "secret.jpg")
        with open(frame, "wb") as output:
            output.write(b"trusted-frame")
        with open(secret, "wb") as output:
            output.write(b"outside-secret")
        execution = restricted.create_frame_only_bundle(staging, ["frames/frame.jpg"])
        try:
            copied = open(os.path.join(execution, "frames", "frame.jpg"), "rb").read()
            check("validated regular frame copies exact bytes", copied == b"trusted-frame", repr(copied))
            check("execution root excludes staging-only canary",
                  not os.path.exists(os.path.join(execution, "outside", "secret.jpg")))
            handle = restricted._open(execution, restricted.FILE_READ_ATTRIBUTES, directory=True)
            try:
                final, identity = restricted._validate(handle, directory=True)
                check("private execution root remains a same-volume disk directory",
                      identity[0] != 0 and os.path.basename(final) == os.path.basename(execution), final)
            finally:
                restricted._close(handle)
        finally:
            shutil.rmtree(execution, ignore_errors=True)
        redirected_parent = os.path.join(parent, "redirected-parent")
        redirected_staging = os.path.join(redirected_parent, "staging")
        os.makedirs(os.path.join(redirected_staging, "frames"))
        with open(os.path.join(redirected_staging, "frames", "frame.jpg"), "wb") as output:
            output.write(b"redirected-frame")
        parent_link = os.path.join(parent, "staging-parent-link")
        roots_before = {
            name for name in os.listdir(redirected_parent)
            if name.startswith("cli_judge_restricted_")
        }
        junction(parent_link, redirected_parent)
        rejects("caller staging ancestor junction is rejected before output root",
                os.path.join(parent_link, "staging"), "frames/frame.jpg")
        roots_after = {
            name for name in os.listdir(redirected_parent)
            if name.startswith("cli_judge_restricted_")
        }
        check("ancestor-junction rejection creates no execution root or copy",
              roots_after == roots_before, repr((roots_before, roots_after)))
        link = os.path.join(frames, "outside-link.jpg")
        os.symlink(secret, link)
        rejects("leaf symbolic link is rejected before copy", staging, "frames/outside-link.jpg")
        junction_path = os.path.join(frames, "outside-junction")
        junction(junction_path, outside)
        rejects("junction ancestor is rejected before copy", staging, "frames/outside-junction/secret.jpg")
        redirected_root = os.path.join(parent, "cli_judge_restricted_" + "d" * 32)
        junction(redirected_root, outside)
        with mock.patch.object(restricted.secrets, "token_hex", return_value="d" * 32):
            rejects("preexisting destination junction is rejected before any frame copy", staging,
                    "frames/frame.jpg")
        check("destination junction receives no copied frame",
              not os.path.exists(os.path.join(outside, "frame.jpg")))
        hard_link = os.path.join(frames, "hard-link.jpg")
        os.link(frame, hard_link)
        rejects("hard-linked frame is rejected before copy", staging, "frames/hard-link.jpg")
        os.unlink(hard_link)
        replacement = os.path.join(outside, "replacement.jpg")
        with open(replacement, "wb") as output:
            output.write(b"replacement-secret")
        original_validate, attempted, replaced, denied = restricted._validate, False, False, False

        def swap_after_source_open(handle, **kwargs):
            nonlocal attempted, replaced, denied
            result = original_validate(handle, **kwargs)
            if not attempted and not kwargs["directory"]:
                attempted = True
                try:
                    os.replace(replacement, frame)
                    replaced = True
                except PermissionError:
                    denied = True
            return result

        with mock.patch.object(restricted, "_validate", side_effect=swap_after_source_open):
            execution = restricted.create_frame_only_bundle(staging, ["frames/frame.jpg"])
        try:
            copied = open(os.path.join(execution, "frames", "frame.jpg"), "rb").read()
            check("post-open source replacement cannot change copied handle bytes",
                  attempted and (replaced or denied) and copied == b"trusted-frame",
                  repr((attempted, replaced, denied, copied)))
            check("replacement secret was never copied", copied != b"replacement-secret", repr(copied))
        finally:
            shutil.rmtree(execution, ignore_errors=True)
    if FAILURES:
        print("FAILURES:\n  " + "\n  ".join(FAILURES))
        return 1
    print("PASS restricted Claude Windows validation")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
