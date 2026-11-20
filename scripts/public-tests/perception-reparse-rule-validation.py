#!/usr/bin/env python3
"""The perception runners refuse only links and junctions, not every reparse point.

OneDrive keeps files and folders as cloud-file reparse points, and Windows can
compress a file into a reparse point; both are the file or folder itself. The
engine refuses only name-surrogate reparse points (symbolic links, junctions),
and judge/project_folders.py holds that one rule for the Python side. These
checks prove that the visual-index writer and the restricted Claude frame
copier follow it:

  - classification by reparse tag (every platform);
  - on Windows, with real folders: a junction and a folder symlink at the
    index folder are refused, and a compressed (WOF) index file is replaced
    like an ordinary file.

Run: python3 scripts/public-tests/perception-reparse-rule-validation.py
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
import types

_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_PY = os.path.join(_ROOT, "app", "perception", "py")
sys.path.insert(0, _PY)
sys.path.insert(0, os.path.join(_PY, "judge", "adapters"))

import restricted_claude_windows  # noqa: E402
import siglip_index  # noqa: E402

_FAILS: list[str] = []
# Tags from [MS-FSCC] 2.1.2.1 "Reparse Tags".
SURROGATES = {"symbolic link": 0xA000000C, "junction": 0xA0000003, "WSL symbolic link": 0xA000001D}
ORDINARY = {"cloud placeholder": 0x9000001A, "cloud placeholder F": 0x9000F01A,
            "compressed (WOF)": 0x80000017, "deduplicated": 0x80000013}


def check(name: str, cond: bool, detail: str = "") -> None:
    print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        _FAILS.append(f"{name}: {detail}")


def entry(tag: int, mode: int) -> types.SimpleNamespace:
    return types.SimpleNamespace(st_mode=mode, st_file_attributes=0x400, st_reparse_tag=tag,
                                 st_nlink=1)


def _copier_names_another_entry(attributes: int, tag: int) -> bool:
    folders = getattr(restricted_claude_windows, "project_folders", None)
    if folders is None:
        # Before the shared rule the copier refused every reparse point.
        return bool(attributes & 0x400)
    return folders.attributes_name_another_entry(attributes, tag)


def test_classification() -> None:
    for kind, mode in (("folder", stat.S_IFDIR | 0o755), ("file", stat.S_IFREG | 0o644)):
        for name, tag in SURROGATES.items():
            check(f"visual index: a {name} {kind} is refused", siglip_index.is_reparse(entry(tag, mode)))
            check(f"Claude frame copier: a {name} handle is refused",
                  _copier_names_another_entry(0x400, tag))
        for name, tag in ORDINARY.items():
            check(f"visual index: a {name} {kind} is ordinary",
                  not siglip_index.is_reparse(entry(tag, mode)))
            check(f"Claude frame copier: a {name} handle is ordinary",
                  not _copier_names_another_entry(0x400, tag))
    check("a reparse point whose tag could not be read is refused",
          siglip_index.is_reparse(entry(0, stat.S_IFDIR | 0o755)))
    check("an entry without a reparse point is ordinary",
          not _copier_names_another_entry(0x10, 0))


def _junction(target: str, path: str) -> bool:
    import _winapi  # noqa: PLC0415
    try:
        _winapi.CreateJunction(target, path)
        return True
    except OSError:
        return False


def test_real_windows_folders() -> None:
    if os.name != "nt":
        print("  [SKIP] real junction, symlink and compressed-file checks need Windows")
        return
    index = {"schema": "fixture", "frames": [{"ms": 0, "v": [0.1]}]}
    with tempfile.TemporaryDirectory() as root:
        outside = os.path.join(root, "outside")
        os.mkdir(outside)
        sentinel = os.path.join(outside, "a1.json")
        with open(sentinel, "w", encoding="utf-8") as output:
            output.write("outside-sentinel")
        for kind in ("junction", "folder symlink"):
            embeddings = os.path.join(root, f"embeddings-{kind.replace(' ', '-')}")
            if kind == "junction":
                made = _junction(outside, embeddings)
            else:
                try:
                    os.symlink(outside, embeddings, target_is_directory=True)
                    made = True
                except OSError:
                    made = False
            if not made:
                print(f"  [SKIP] could not create a {kind} here")
                continue
            try:
                siglip_index.write_index_bounded(index, os.path.join(embeddings, "a1.json"), 1 << 20)
                refused = False
            except ValueError:
                refused = True
            check(f"visual index: a {kind} at the index folder is refused", refused)
            with open(sentinel, encoding="utf-8") as source:
                check(f"visual index: the file behind the {kind} is unchanged",
                      source.read() == "outside-sentinel")

        embeddings = os.path.join(root, "embeddings")
        os.mkdir(embeddings)
        leaf = os.path.join(embeddings, "a1.json")
        with open(leaf, "w", encoding="utf-8") as output:
            output.write(json.dumps({"old": True}) + " " * 65536)
        compact = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "compact.exe")
        subprocess.run([compact, "/c", "/exe:xpress4k", leaf], capture_output=True, check=False)
        info = os.lstat(leaf)
        tag = getattr(info, "st_reparse_tag", 0)
        if not getattr(info, "st_file_attributes", 0) & 0x400:
            print("  [SKIP] compact.exe did not make the file a reparse point on this volume")
            return
        check("the compressed file carries a non-link reparse tag", tag == ORDINARY["compressed (WOF)"],
              hex(tag))
        try:
            siglip_index.write_index_bounded(index, leaf, 1 << 20)
            replaced = True
        except ValueError as error:
            replaced = False
            print(f"    {error}")
        check("visual index: a compressed (WOF) index file is replaced like an ordinary file", replaced)
        with open(leaf, encoding="utf-8") as source:
            check("visual index: the new index was written", json.loads(source.read()) == index)


def main() -> int:
    for test in (test_classification, test_real_windows_folders):
        print(test.__name__)
        test()
    if _FAILS:
        print(f"\n{len(_FAILS)} REPARSE RULE CHECK(S) FAILED")
        return 1
    print("\nALL REPARSE RULE CHECKS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
