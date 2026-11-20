#!/usr/bin/env python3
"""A perception runner never starts a program planted in its current folder.

The engine starts its runners with its own current folder, which can be a
project received from someone else (an agent CLI running cutd inside it). On
Windows both shutil.which and a bare program name look in the current folder
first. This check plants an "ffmpeg" in a fresh current folder (on Windows a
copy of hostname.exe, elsewhere a script) and adds a relative PATH entry,
then proves that the shared media helper neither returns it nor lets a bare
"ffmpeg" start it. No real media tool is run.

Run: python3 scripts/public-tests/perception-current-folder-validation.py
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile

_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_PY = os.path.join(_ROOT, "app", "perception", "py")
MARKER = "PLANTED-FFMPEG-RAN"

PROBE = r"""
import os, subprocess, sys
sys.path.insert(0, sys.argv[1])
import shutil
import local_media_io
# How the runners name ffmpeg (before the shared helper they used shutil.which).
lookup = getattr(local_media_io, "media_tool", None)
tool = lookup("ffmpeg") if lookup else (shutil.which("ffmpeg") or "ffmpeg")
print("media_tool=" + tool)
try:
    out = subprocess.run([tool], capture_output=True, text=True, timeout=20)
    print("bare=" + out.stdout.strip())
except OSError as error:
    print("bare-error=" + type(error).__name__)
"""


def main() -> int:
    failures = []
    with tempfile.TemporaryDirectory() as root:
        planted_dir = os.path.join(root, "received-project")
        os.mkdir(planted_dir)
        if os.name == "nt":
            planted = os.path.join(planted_dir, "ffmpeg.exe")
            shutil.copyfile(os.path.join(os.environ.get("SystemRoot", r"C:\Windows"),
                                         "System32", "hostname.exe"), planted)
            marker = subprocess.run([planted], capture_output=True, text=True).stdout.strip()
        else:
            planted = os.path.join(planted_dir, "ffmpeg")
            with open(planted, "w", encoding="utf-8") as output:
                output.write(f"#!/bin/sh\necho {MARKER}\n")
            os.chmod(planted, 0o755)
            marker = MARKER
        empty_path = os.path.join(root, "empty-path")
        os.mkdir(empty_path)
        env = dict(os.environ)
        for name in ("SHELLX_CUT_FFMPEG", "SHELLX_CUT_FFPROBE", "SHELLX_CUT_FFMPEG_DIR",
                     "NoDefaultCurrentDirectoryInExePath"):
            env.pop(name, None)
        # A relative entry and a folder without ffmpeg; the system folder stays
        # so Windows can start Python itself.
        env["PATH"] = os.pathsep.join([".", empty_path] + (
            [os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32")]
            if os.name == "nt" else []))
        result = subprocess.run([sys.executable, "-c", PROBE, _PY], cwd=planted_dir, env=env,
                                capture_output=True, text=True, timeout=60)
        lines = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
        checks = [
            ("the media helper does not return the planted ffmpeg",
             lines.get("media_tool") == "ffmpeg", lines.get("media_tool")),
            ("a bare ffmpeg does not start the planted one",
             marker not in lines.get("bare", ""), lines.get("bare") or lines.get("bare-error")),
        ]
        for name, ok, detail in checks:
            print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + ("" if ok else f" — {detail}"))
            if not ok:
                failures.append(name)
        if result.returncode != 0:
            print(result.stderr[-800:])
            failures.append("probe exited with an error")
    if failures:
        print(f"\n{len(failures)} CURRENT FOLDER CHECK(S) FAILED")
        return 1
    print("\nALL CURRENT FOLDER CHECKS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
