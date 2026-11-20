#!/usr/bin/env python3
"""No-quota checks that the judge adapters keep to ordinary project files.

A project received from someone else may place links or files at the paths a
review reads or writes. These checks run every adapter entry point with all
providers replaced by local stand-ins (no provider process is started, no
network is used) and prove that:

  - a linked or shared (hard-linked) review result path never redirects the
    write to a file outside the project (ladder, Antigravity, Grok prompt file);
  - a render analysis file that is a link, is shared with another location, or
    describes a different video never reaches a review prompt (Claude, Codex,
    Grok), and the run ends with a plain refusal;
  - an ordinary analysis file for this render still reaches the prompt;
  - analysis generated during a review is written to a fresh directory;
  - the render hash matches the engine's full SHA-256 and refuses a link;
  - only Windows name-surrogate reparse points (links, junctions) are refused;
    cloud placeholders and compressed files are ordinary files.

Run: python3 scripts/public-tests/judge-project-files-validation.py
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
_JUDGE = os.path.join(_ROOT, "app", "perception", "py", "judge")
sys.path.insert(0, os.path.join(_JUDGE, "adapters"))
sys.path.insert(0, _JUDGE)

import antigravity_judge  # noqa: E402
import cli_judge  # noqa: E402
import codex_judge  # noqa: E402
import grok_judge  # noqa: E402
import ladder_judge  # noqa: E402

SENTINEL = b"outside-project-sentinel\n"
CANARY = "SYNTHETIC_OTHER_PROJECT_CANARY"
ORDINARY = "ordinarytranscriptword"
_FAILS: list[str] = []


def check(name: str, cond: bool, detail: str = "") -> None:
    status = "PASS" if cond else "FAIL"
    print(f"  [{status}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        _FAILS.append(f"{name}: {detail}")


def _can_symlink(root: str) -> bool:
    probe = os.path.join(root, "symlink-probe")
    try:
        os.symlink(root, probe)
    except (NotImplementedError, OSError):
        return False
    os.unlink(probe)
    return True


def _link(kind: str, target: str, path: str) -> None:
    if kind == "symlink":
        os.symlink(target, path)
    else:
        os.link(target, path)


def _sentinel(root: str) -> str:
    path = os.path.join(root, "outside-sentinel.txt")
    with open(path, "wb") as output:
        output.write(SENTINEL)
    return path


def _read(path: str) -> bytes:
    with open(path, "rb") as source:
        return source.read()


def _report(path: str, asset_hash: str, word: str) -> None:
    with open(path, "w", encoding="utf-8") as output:
        json.dump({"schema": "shellx-cut/perception/1", "asset_hash": asset_hash,
                   "words": {"words": [{"word": word, "start_ms": 0, "end_ms": 100}]}},
                  output)


def _render(root: str) -> tuple[str, str]:
    path = os.path.join(root, "render.mp4")
    data = b"synthetic rendered media bytes"
    with open(path, "wb") as output:
        output.write(data)
    return path, "sha256:" + hashlib.sha256(data).hexdigest()


def _run(main, argv: list[str]) -> tuple[int, str]:
    stderr = io.StringIO()
    with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()), \
            contextlib.redirect_stderr(stderr):
        try:
            code = main()
        except Exception as error:  # noqa: BLE001 - an escape is a failed check
            return -1, f"{type(error).__name__}: {error}"
    return code, stderr.getvalue()


def _kinds(root: str) -> list[str]:
    return (["symlink"] if _can_symlink(root) else []) + ["hardlink"]


def test_review_result_links_never_redirect_the_write() -> None:
    with tempfile.TemporaryDirectory() as root:
        unready = {"rungs": [], "order": [], "auto_selected": None}
        for kind in _kinds(root):
            for name, main, argv in (
                    ("ladder", ladder_judge.main,
                     ["ladder_judge.py", "review", "--provider", "auto"]),
                    ("antigravity", antigravity_judge.main,
                     ["antigravity_judge.py", "review"])):
                work = tempfile.mkdtemp(dir=root)
                outside = _sentinel(work)
                bundle = os.path.join(work, "project", ".scratch", "judge", "render_001")
                os.makedirs(bundle)
                out = os.path.join(bundle, "envelope.json")
                _link(kind, outside, out)
                with mock.patch.object(ladder_judge, "detect_ladder", return_value=unready):
                    code, stderr = _run(main, argv + ["--out", out])
                check(f"{name}: {kind} at the result path leaves the outside file unchanged",
                      _read(outside) == SENTINEL, repr(_read(outside)[:80]))
                check(f"{name}: {kind} at the result path is refused before review work",
                      code == 2 and "review refused" in stderr, f"{code} {stderr[-300:]}")


def test_grok_prompt_file_link_never_redirects_the_write() -> None:
    with tempfile.TemporaryDirectory() as root:
        for kind in _kinds(root):
            work = tempfile.mkdtemp(dir=root)
            outside = _sentinel(work)
            bundle = os.path.join(work, "bundle")
            os.makedirs(bundle)
            _link(kind, outside, os.path.join(bundle, "_grok_prompt.json"))
            cli = {"path": "grok", "version": "grok 1.0.21", "launch": None}
            with mock.patch.object(grok_judge.grok_tool_policy, "resolve_grok_tool_policy",
                                   return_value=(cli, None)), \
                    mock.patch.object(grok_judge.grok_tool_policy, "grok_judge_argv",
                                      return_value=["grok-stand-in"]), \
                    mock.patch.object(grok_judge.subprocess, "run") as provider:
                provider.return_value = mock.Mock(returncode=1, stdout="", stderr="stand-in")
                review, _meta, reason = grok_judge.invoke_grok(
                    "grok", "system", "user", "", [], bundle, 5)
            check(f"grok: {kind} at the prompt file leaves the outside file unchanged",
                  _read(outside) == SENTINEL, repr(_read(outside)[:80]))
            check(f"grok: {kind} at the prompt file stops before the provider starts",
                  review is None and not provider.called and "prompt" in (reason or ""),
                  f"{provider.called} {reason!r}")


def _report_cases(root: str, render_hash: str):
    """Yield (label, report path, should_refuse, transcript word)."""
    if _can_symlink(root):
        other = os.path.join(root, "other-linked.json")
        _report(other, "sha256:other-video", CANARY)
        linked = os.path.join(root, "receipts-linked.json")
        os.symlink(other, linked)
        yield "linked report", linked, True, CANARY
    shared_target = os.path.join(root, "other-shared.json")
    _report(shared_target, render_hash, CANARY)
    shared = os.path.join(root, "receipts-shared.json")
    os.link(shared_target, shared)
    yield "hard-linked report", shared, True, CANARY
    mismatched = os.path.join(root, "receipts-mismatched.json")
    _report(mismatched, "sha256:different-video", CANARY)
    yield "report for a different video", mismatched, True, CANARY
    matching = os.path.join(root, "receipts-matching.json")
    _report(matching, render_hash, ORDINARY)
    yield "ordinary report for this render", matching, False, ORDINARY


def test_claude_prompt_receives_only_this_renders_report() -> None:
    with tempfile.TemporaryDirectory() as root:
        render, render_hash = _render(root)
        for label, report, refuse, word in _report_cases(root, render_hash):
            prompts: list[str] = []

            def invoke(_bin, system, user, *_args, **_kwargs):
                prompts.append(system + user)
                return None, {"available": True}, "stand-in reviewer"

            bundle = tempfile.mkdtemp(dir=root)
            cli = {"path": "claude", "version": "2.1.248", "launch": None}
            with mock.patch.object(cli_judge.judge, "probe_duration_s", return_value=1.0), \
                    mock.patch.object(cli_judge.judge, "extract_frames", return_value=[]), \
                    mock.patch.object(cli_judge.restricted_claude, "resolve_restricted_claude",
                                      return_value=(cli, None)), \
                    mock.patch.object(cli_judge.restricted_claude, "create_frame_only_bundle",
                                      return_value=tempfile.mkdtemp(dir=root)), \
                    mock.patch.object(cli_judge, "invoke_claude", side_effect=invoke):
                code, stderr = _run(cli_judge.main, [
                    "cli_judge.py", "review", "--render", render, "--perception", report,
                    "--bundle-dir", bundle, "--out", os.path.join(bundle, "envelope.json")])
            _expect(f"claude: {label}", refuse, code, stderr, prompts, word)


def test_codex_prompt_receives_only_this_renders_report() -> None:
    with tempfile.TemporaryDirectory() as root:
        render, render_hash = _render(root)
        frame = os.path.join(root, "frame.jpg")
        with open(frame, "wb") as output:
            output.write(b"jpeg")
        for label, report, refuse, word in _report_cases(root, render_hash):
            prompts: list[str] = []

            def invoke(_launch, system, user, *_args, **_kwargs):
                prompts.append(system + user)
                return None, {"available": True}, "stand-in reviewer"

            bundle = tempfile.mkdtemp(dir=root)
            with mock.patch.object(codex_judge.judge, "probe_duration_s", return_value=1.0), \
                    mock.patch.object(codex_judge.judge, "extract_frames",
                                      return_value=[{"path": frame, "at_ms": 0}]), \
                    mock.patch.object(codex_judge.codex_judge_runtime, "prepare_neutral_frames",
                                      return_value=(tempfile.mkdtemp(dir=root), [frame])), \
                    mock.patch.object(codex_judge, "invoke_codex", side_effect=invoke):
                code, stderr = _run(codex_judge.main, [
                    "codex_judge.py", "review", "--render", render, "--perception", report,
                    "--bundle-dir", bundle, "--out", os.path.join(bundle, "envelope.json")])
            _expect(f"codex: {label}", refuse, code, stderr, prompts, word)


def test_grok_prompt_receives_only_this_renders_report() -> None:
    with tempfile.TemporaryDirectory() as root:
        render, render_hash = _render(root)
        frame = os.path.join(root, "frame.jpg")
        with open(frame, "wb") as output:
            output.write(b"jpeg")
        for label, report, refuse, word in _report_cases(root, render_hash):
            prompts: list[str] = []

            def invoke(_bin, system, user, *_args, **_kwargs):
                prompts.append(system + user)
                return None, {"available": True}, "stand-in reviewer"

            bundle = tempfile.mkdtemp(dir=root)
            with mock.patch.object(grok_judge.judge, "probe_duration_s", return_value=1.0), \
                    mock.patch.object(grok_judge.judge, "extract_frames",
                                      return_value=[{"path": frame, "at_ms": 0}]), \
                    mock.patch.object(grok_judge, "invoke_grok", side_effect=invoke):
                code, stderr = _run(grok_judge.main, [
                    "grok_judge.py", "review", "--render", render, "--perception", report,
                    "--bundle-dir", bundle, "--out", os.path.join(bundle, "envelope.json")])
            _expect(f"grok: {label}", refuse, code, stderr, prompts, word)


def _expect(label: str, refuse: bool, code: int, stderr: str, prompts: list[str],
            word: str) -> None:
    carried = any(word in prompt for prompt in prompts)
    if refuse:
        check(f"{label} never reaches a prompt", not carried, f"{len(prompts)} prompt(s)")
        check(f"{label} ends with a plain refusal",
              code == 2 and "review refused" in stderr, f"{code} {stderr[-300:]}")
    else:
        check(f"{label} reaches the prompt", code == 0 and carried,
              f"{code} {len(prompts)} prompt(s) {stderr[-300:]}")


def test_generated_report_is_written_to_a_fresh_directory() -> None:
    with tempfile.TemporaryDirectory() as root:
        render, render_hash = _render(tempfile.mkdtemp(dir=root))
        if not _can_symlink(root):
            print("  [SKIP] generated report link check needs symlink support")
            return
        outside = _sentinel(root)
        bundle = os.path.join(root, "bundle")
        os.makedirs(bundle)
        os.symlink(outside, os.path.join(bundle, "render.perception.json"))

        def instruments(argv, **_kwargs):
            out = argv[argv.index("--out") + 1]
            _report(out, render_hash, ORDINARY)  # a plain write, as instruments.py does
            return mock.Mock(returncode=0, stdout="", stderr="")

        with mock.patch.object(cli_judge.subprocess, "run", side_effect=instruments), \
                contextlib.redirect_stderr(io.StringIO()):
            try:
                facts, source, _warnings = cli_judge.resolve_perception(render, None, bundle)
            except Exception as error:  # noqa: BLE001
                facts, source = {}, {"mode": f"{type(error).__name__}: {error}"}
        check("generated analysis leaves a planted link's target unchanged",
              _read(outside) == SENTINEL, repr(_read(outside)[:80]))
        check("generated analysis is still used for the review",
              source.get("mode") == "generated" and ORDINARY in json.dumps(facts), repr(source))


def test_content_hash_matches_the_engine() -> None:
    try:
        import judge_files
    except ImportError as error:
        check("content hash helper is available", False, repr(error))
        return
    with tempfile.TemporaryDirectory() as root:
        path = os.path.join(root, "media.bin")
        data = bytes(range(256)) * 4099  # spans several read chunks
        with open(path, "wb") as output:
            output.write(data)
        check("render hash is the full sha256 the engine records",
              judge_files.content_hash(path) == "sha256:" + hashlib.sha256(data).hexdigest())
        if _can_symlink(root):
            link = os.path.join(root, "linked.bin")
            os.symlink(path, link)
            try:
                judge_files.content_hash(link)
                refused = False
            except judge_files.RefusedProjectFile:
                refused = True
            check("a link at the render path is refused", refused)


def test_only_name_surrogate_reparse_points_name_another_entry() -> None:
    import stat as stat_module
    import types
    import judge_files

    def entry(tag: int, attributes: int = 0x400, mode: int = stat_module.S_IFREG | 0o644):
        return types.SimpleNamespace(st_mode=mode, st_file_attributes=attributes,
                                     st_reparse_tag=tag, st_nlink=1)

    # Tag values from [MS-FSCC] 2.1.2.1 "Reparse Tags".
    for name, tag in (("symbolic link", 0xA000000C), ("junction", 0xA0000003),
                      ("WSL symbolic link", 0xA000001D)):
        check(f"a {name} reparse point names another entry",
              judge_files.names_another_entry(entry(tag)))
    for name, tag in (("cloud placeholder", 0x9000001A), ("cloud placeholder F", 0x9000F01A),
                      ("compressed (WOF)", 0x80000017), ("deduplicated", 0x80000013)):
        check(f"a {name} reparse point is the file itself",
              not judge_files.names_another_entry(entry(tag)))
    check("a reparse point whose tag could not be read is refused",
          judge_files.names_another_entry(entry(0)))
    check("an entry without a reparse point is the file itself",
          not judge_files.names_another_entry(entry(0, attributes=0x20)))
    check("a POSIX symbolic link names another entry",
          judge_files.names_another_entry(entry(0, attributes=0,
                                                mode=stat_module.S_IFLNK | 0o777)))


def main() -> int:
    tests = [
        test_review_result_links_never_redirect_the_write,
        test_grok_prompt_file_link_never_redirects_the_write,
        test_claude_prompt_receives_only_this_renders_report,
        test_codex_prompt_receives_only_this_renders_report,
        test_grok_prompt_receives_only_this_renders_report,
        test_generated_report_is_written_to_a_fresh_directory,
        test_content_hash_matches_the_engine,
        test_only_name_surrogate_reparse_points_name_another_entry,
    ]
    for test in tests:
        test()
    print()
    if _FAILS:
        print(f"FAILED ({len(_FAILS)}):")
        for failure in _FAILS:
            print("  - " + failure)
        return 1
    print("ALL JUDGE PROJECT FILE CHECKS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
