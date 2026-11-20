#!/usr/bin/env python3
"""No-quota checks that a linked folder inside a project never redirects a runner.

A project received from someone else may hold a link (on Windows a junction)
at a FOLDER, for example ``.scratch/redirect`` pointing outside the project.
These checks run the direct entry points of the review runners and of the other
perception runners, with every provider and media tool replaced by stand-ins
(no provider process, no network, no ffmpeg), and prove that:

  - a review result (``--out``) two folders below such a link is refused
    before any review work and the file outside the project is unchanged;
  - a ``--bundle-dir`` or ``--frames-dir`` below such a link is refused and
    nothing is created outside the project;
  - an analysis file read through such a link is refused;
  - the analysis, visual-index, dub and matte runners refuse an output below
    such a link before they start;
  - links ABOVE the project folder (macOS /tmp, a linked Videos folder) are
    still followed, and a folder holding ops.jsonl counts as a project.

Run: python3 scripts/public-tests/judge-ancestor-links-validation.py
"""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import sys
import tempfile
from unittest import mock

_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_PY = os.path.join(_ROOT, "app", "perception", "py")
_JUDGE = os.path.join(_PY, "judge")
sys.path.insert(0, os.path.join(_JUDGE, "adapters"))
sys.path.insert(0, _JUDGE)
sys.path.append(_PY)

import antigravity_judge  # noqa: E402
import cli_judge  # noqa: E402
import codex_judge  # noqa: E402
import grok_judge  # noqa: E402
import judge  # noqa: E402
import ladder_judge  # noqa: E402

SENTINEL = b"outside-project-sentinel\n"
_FAILS: list[str] = []
# Which folder link the checks plant: a symbolic link, or on Windows also a
# junction (main runs every check once per kind).
_KIND = "symlink"


def check(name: str, cond: bool, detail: str = "") -> None:
    print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        _FAILS.append(f"{name}: {detail}")


def _folder_link(target: str, path: str) -> str | None:
    """Make ``path`` a folder link to ``target``: a symbolic link, or on
    Windows a junction when that kind is selected or symlinks need a
    privilege this account lacks."""
    if _KIND == "symlink":
        try:
            os.symlink(target, path, target_is_directory=True)
            return "folder symlink"
        except (OSError, NotImplementedError):
            pass
    if os.name == "nt":
        import _winapi  # noqa: PLC0415
        try:
            _winapi.CreateJunction(target, path)
            return "junction"
        except OSError:
            pass
    return None


def _tree(path: str) -> list[str]:
    return sorted(os.path.relpath(os.path.join(base, name), path)
                  for base, dirs, files in os.walk(path) for name in dirs + files)


class Layout:
    """``root/untrusted.cutproj`` with ``.scratch/redirect`` linked to
    ``root/outside``, which holds ``ordinary/review.json`` (the sentinel)."""

    def __init__(self, root: str, project_name: str = "untrusted.cutproj"):
        self.project = os.path.join(root, project_name)
        os.makedirs(os.path.join(self.project, ".scratch"))
        os.makedirs(os.path.join(self.project, "receipts"))
        with open(os.path.join(self.project, "ops.jsonl"), "wb"):
            pass
        self.outside = os.path.join(root, "outside")
        os.makedirs(os.path.join(self.outside, "ordinary"))
        self.sentinel = os.path.join(self.outside, "ordinary", "review.json")
        with open(self.sentinel, "wb") as output:
            output.write(SENTINEL)
        self.kind = _folder_link(self.outside, os.path.join(self.project, ".scratch", "redirect"))
        self.planted = os.path.join(self.project, ".scratch", "redirect", "ordinary")
        self.render = os.path.join(self.project, "render.mp4")
        data = b"synthetic rendered media bytes"
        with open(self.render, "wb") as output:
            output.write(data)
        self.render_hash = "sha256:" + hashlib.sha256(data).hexdigest()
        self.report = os.path.join(self.project, "receipts", "render.perception.json")
        self.write_report(self.report)
        self.before = _tree(self.outside)

    def write_report(self, path: str) -> None:
        with open(path, "w", encoding="utf-8") as output:
            json.dump({"schema": "shellx-cut/perception/1", "asset_hash": self.render_hash,
                       "words": {"words": [{"word": "ordinary", "start_ms": 0, "end_ms": 100}]}},
                      output)

    def outside_unchanged(self) -> bool:
        with open(self.sentinel, "rb") as source:
            return source.read() == SENTINEL and _tree(self.outside) == self.before


def _run(main, argv: list[str]) -> tuple[int, str]:
    stderr = io.StringIO()
    with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()), \
            contextlib.redirect_stderr(stderr):
        try:
            code = main()
        except SystemExit as error:
            code = error.code if isinstance(error.code, int) else 1
        except Exception as error:  # noqa: BLE001 - an escape is a failed check
            return -1, f"{type(error).__name__}: {error}"
    return code, stderr.getvalue()


def _layout(root: str, **kwargs) -> Layout | None:
    layout = Layout(tempfile.mkdtemp(dir=root), **kwargs)
    if layout.kind is None:
        print("  [SKIP] this system can create neither a folder link nor a junction")
        return None
    return layout


def _refused(name: str, layout: Layout, code: int, stderr: str, word: str = "refused") -> None:
    check(f"{name} ({layout.kind}): refused before any work", code == 2 and word in stderr,
          f"{code} {stderr[-300:]}")
    check(f"{name} ({layout.kind}): nothing outside the project changed",
          layout.outside_unchanged(), repr(_tree(layout.outside)))


def test_review_result_below_a_linked_folder() -> None:
    unready = {"rungs": [], "order": [], "auto_selected": None}
    with tempfile.TemporaryDirectory() as root:
        for name, main, argv in (
                ("judge.py --out", judge.main,
                 ["judge.py", "review", "--backend", "dry-run"]),
                ("ladder --out", ladder_judge.main,
                 ["ladder_judge.py", "review", "--provider", "auto"]),
                ("antigravity --out", antigravity_judge.main,
                 ["antigravity_judge.py", "review"])):
            layout = _layout(root)
            if layout is None:
                return
            out = os.path.join(layout.planted, "review.json")
            with mock.patch.object(ladder_judge, "detect_ladder", return_value=unready), \
                    mock.patch.object(judge, "probe_duration_s", return_value=1.0), \
                    mock.patch.object(judge, "extract_frames", return_value=[]):
                code, stderr = _run(main, argv + ["--render", layout.render, "--perception",
                                                  layout.report, "--out", out])
            _refused(name, layout, code, stderr)


def test_bundle_and_frames_folders_below_a_linked_folder() -> None:
    with tempfile.TemporaryDirectory() as root:
        for name in ("claude --bundle-dir", "grok --bundle-dir", "codex --bundle-dir",
                     "judge.py --frames-dir"):
            layout = _layout(root)
            if layout is None:
                return
            bundle = os.path.join(layout.planted, "bundle")
            out = os.path.join(layout.project, ".scratch", "judge", "envelope.json")
            os.makedirs(os.path.dirname(out))
            common = ["review", "--render", layout.render, "--perception", layout.report]
            stand_in = mock.Mock(return_value=(None, {"available": True}, "stand-in"))
            with mock.patch.object(judge, "probe_duration_s", return_value=1.0), \
                    mock.patch.object(judge, "run", side_effect=AssertionError("ffmpeg")), \
                    mock.patch.object(cli_judge, "invoke_claude", stand_in), \
                    mock.patch.object(grok_judge, "invoke_grok", stand_in), \
                    mock.patch.object(codex_judge, "invoke_codex", stand_in), \
                    mock.patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", True), \
                    mock.patch.object(cli_judge.restricted_claude, "resolve_restricted_claude",
                                      return_value=({"path": "claude", "version": "2.1.248",
                                                     "launch": None}, None)):
                if name.startswith("claude"):
                    code, stderr = _run(cli_judge.main, ["cli_judge.py", *common,
                                                         "--bundle-dir", bundle, "--out", out])
                elif name.startswith("grok"):
                    code, stderr = _run(grok_judge.main, ["grok_judge.py", *common,
                                                          "--bundle-dir", bundle, "--out", out])
                elif name.startswith("codex"):
                    code, stderr = _run(codex_judge.main, ["codex_judge.py", *common,
                                                           "--bundle-dir", bundle, "--out", out])
                else:
                    code, stderr = _run(judge.main, ["judge.py", *common, "--backend", "dry-run",
                                                     "--frames-dir", bundle, "--out", out])
            _refused(name, layout, code, stderr)
            check(f"{name} ({layout.kind}): the review wrote no result",
                  not os.path.exists(out), out)
            check(f"{name} ({layout.kind}): no provider was asked", not stand_in.called)


def test_analysis_file_read_through_a_linked_folder() -> None:
    with tempfile.TemporaryDirectory() as root:
        layout = _layout(root)
        if layout is None:
            return
        outside_report = os.path.join(layout.outside, "ordinary", "report.json")
        layout.write_report(outside_report)
        layout.before = _tree(layout.outside)
        with mock.patch.object(judge, "probe_duration_s", return_value=1.0), \
                mock.patch.object(judge, "extract_frames", return_value=[]):
            code, stderr = _run(judge.main, [
                "judge.py", "review", "--backend", "dry-run", "--render", layout.render,
                "--perception", os.path.join(layout.planted, "report.json")])
        _refused("judge.py --perception", layout, code, stderr)


def test_other_runners_refuse_an_output_below_a_linked_folder() -> None:
    with tempfile.TemporaryDirectory() as root:
        layout = _layout(root)
        if layout is None:
            return
        out = layout.sentinel.replace(layout.outside, os.path.join(
            layout.project, ".scratch", "redirect"))
        import instruments  # noqa: PLC0415
        report = io.StringIO()
        with mock.patch.object(instruments, "REAL_STDOUT", report):
            code, stderr = _run(instruments.main, ["instruments.py", layout.render, "--out", out])
        check(f"instruments --out ({layout.kind}): refused before analysis",
              code == 1 and "refused" in report.getvalue(), f"{code} {report.getvalue()[-300:]}")
        check(f"instruments --out ({layout.kind}): outside unchanged", layout.outside_unchanged())

        import siglip_index  # noqa: PLC0415
        code, stderr = _run(siglip_index.main, ["siglip_index.py", "--model", "stand-in",
                                                 layout.render, out])
        _refused("siglip_index out", layout, code, stderr, "siglip_index:")

        import dub_runner  # noqa: PLC0415
        code, stderr = _run(lambda: dub_runner.resolve_output_path(out) and 0, ["dub_runner.py"])
        check(f"dub_runner out_wav ({layout.kind}): refused", code not in (0, -1),
              f"{code} {stderr[-300:]}")
        check(f"dub_runner out_wav ({layout.kind}): outside unchanged", layout.outside_unchanged())

        model = os.path.join(layout.project, "model.bin")
        with open(model, "wb") as output:
            output.write(b"stand-in model")
        for module_name, argv in (
                ("sam2_runner", [layout.render, out, "--frame", "0", "--point", "1,1"]),
                ("matte_runner", [layout.render, out, "--model", model]),
                ("matanyone_runner", [layout.render, out, "--mask", model, "--model", model])):
            try:
                module = __import__(module_name)
            except ImportError as error:
                print(f"  [SKIP] {module_name}: its runtime is not installed here ({error.name})")
                continue
            code, stderr = _run(module.main, [f"{module_name}.py", *argv])
            _refused(f"{module_name} output", layout, code, stderr, f"{module_name}:")


def test_links_above_the_project_are_followed() -> None:
    with tempfile.TemporaryDirectory() as root:
        real_home = os.path.join(root, "real-home")
        project = os.path.join(real_home, "kept.cutproj")
        os.makedirs(os.path.join(project, ".scratch", "judge", "render_001"))
        linked_home = os.path.join(root, "linked-home")
        kind = _folder_link(real_home, linked_home)
        if kind is None:
            print("  [SKIP] this system can create neither a folder link nor a junction")
            return
        render = os.path.join(project, "render.mp4")
        with open(render, "wb") as output:
            output.write(b"render")
        report = os.path.join(project, "render.perception.json")
        with open(report, "w", encoding="utf-8") as output:
            json.dump({"schema": "shellx-cut/perception/1"}, output)
        out = os.path.join(linked_home, "kept.cutproj", ".scratch", "judge", "render_001",
                           "review.json")
        with mock.patch.object(judge, "probe_duration_s", return_value=1.0), \
                mock.patch.object(judge, "extract_frames", return_value=[]):
            code, stderr = _run(judge.main, ["judge.py", "review", "--backend", "dry-run",
                                             "--render", render, "--perception", report,
                                             "--out", out])
        written = os.path.join(project, ".scratch", "judge", "render_001", "review.json")
        check(f"a {kind} above the project is followed", code == 0 and os.path.isfile(written),
              f"{code} {stderr[-300:]}")


def test_a_folder_with_ops_jsonl_is_a_project() -> None:
    with tempfile.TemporaryDirectory() as root:
        layout = _layout(root, project_name="no-suffix")
        if layout is None:
            return
        with mock.patch.object(judge, "probe_duration_s", return_value=1.0), \
                mock.patch.object(judge, "extract_frames", return_value=[]):
            code, stderr = _run(judge.main, [
                "judge.py", "review", "--backend", "dry-run", "--render", layout.render,
                "--perception", layout.report, "--out",
                os.path.join(layout.planted, "review.json")])
        _refused("project named by ops.jsonl", layout, code, stderr)


def main() -> int:
    global _KIND
    for _KIND in ["symlink"] + (["junction"] if os.name == "nt" else []):
        _run_all()
    if _FAILS:
        print(f"\n{len(_FAILS)} ANCESTOR LINK CHECK(S) FAILED")
        return 1
    print("\nALL ANCESTOR LINK CHECKS PASSED")
    return 0


def _run_all() -> None:
    for test in (test_review_result_below_a_linked_folder,
                 test_bundle_and_frames_folders_below_a_linked_folder,
                 test_analysis_file_read_through_a_linked_folder,
                 test_other_runners_refuse_an_output_below_a_linked_folder,
                 test_links_above_the_project_are_followed,
                 test_a_folder_with_ops_jsonl_is_a_project):
        print(f"{test.__name__} ({_KIND})")
        test()


if __name__ == "__main__":
    sys.exit(main())
