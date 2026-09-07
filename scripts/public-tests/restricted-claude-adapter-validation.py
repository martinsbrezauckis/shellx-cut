#!/usr/bin/env python3
"""No-provider contract tests for Claude's restricted judge Read boundary."""

from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import types
from unittest import mock

_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_ADAPTERS = os.path.join(_ROOT, "app", "perception", "py", "judge", "adapters")
sys.path.insert(0, _ADAPTERS)

import cli_judge  # noqa: E402
import ladder_judge  # noqa: E402
import restricted_claude  # noqa: E402

_FAILS: list[str] = []


def check(name: str, condition: bool, detail: str = "") -> None:
    status = "PASS" if condition else "FAIL"
    print(f"  [{status}] {name}" + (f" — {detail}" if detail and not condition else ""))
    if not condition:
        _FAILS.append(f"{name}: {detail}")


def _value_after(argv: list[str], flag: str) -> str | None:
    try:
        return argv[argv.index(flag) + 1]
    except (ValueError, IndexError):
        return None


def test_capability_argv_and_frame_root() -> None:
    print("test_capability_argv_and_frame_root")

    def version_run(command, **_kwargs):
        check("Claude capability probe is version-only",
              command == ["/fixture/claude", "--version"], repr(command))
        return types.SimpleNamespace(returncode=0,
                                     stdout="2.1.248 (Claude Code)\n", stderr="")

    with (
        mock.patch.object(restricted_claude.shutil, "which", return_value="/fixture/claude"),
        mock.patch.object(restricted_claude.subprocess, "run", side_effect=version_run),
    ):
        cli, reason = restricted_claude.resolve_restricted_claude("claude")
    check("minimum restricted Claude release is accepted", cli is not None and reason is None,
          repr(reason))
    if cli is not None:
        argv = restricted_claude.restricted_claude_argv(
            cli, "sonnet", json_schema='{"type":"object"}')
        check("Claude command enables documented restricted file root",
              "--restricted" in argv and "--safe-mode" in argv
              and "--strict-mcp-config" in argv and "--no-chrome" in argv,
              repr(argv))
        check("Claude command permits only Read and no interactive approval",
              _value_after(argv, "--tools") == "Read"
              and _value_after(argv, "--permission-prompts") == "none"
              and _value_after(argv, "--disallowedTools") == "mcp__*",
              repr(argv))
        check("Claude command does not widen the restricted root",
              "--add-dir" not in argv, repr(argv))

    for version, marker in (("2.1.247 (Claude Code)", "too old"),
                            ("wrapper release unknown", "unparseable")):
        with (
            mock.patch.object(restricted_claude.shutil, "which", return_value="/fixture/claude"),
            mock.patch.object(
                restricted_claude.subprocess, "run",
                return_value=types.SimpleNamespace(returncode=0, stdout=version, stderr="")),
        ):
            unsupported, why = restricted_claude.resolve_restricted_claude("claude")
        check(f"{marker} Claude version is fail-closed", unsupported is None and bool(why), repr(why))

    with mock.patch.object(
            cli_judge.restricted_claude, "resolve_restricted_claude",
            return_value=(None, "restricted version unavailable")) as resolve, \
            mock.patch.object(cli_judge.subprocess, "run") as no_model_call:
        review, meta, reason = cli_judge.invoke_claude(
            "claude", "system", "untrusted text", "sonnet", "/fixture", 10)
    check("unsupported Claude does not spawn a model invocation",
          review is None and meta.get("available") is False
          and reason == "restricted version unavailable"
          and resolve.called and not no_model_call.called,
          repr((meta, reason)))

    with tempfile.TemporaryDirectory() as root:
        staging = os.path.join(root, "staging")
        frames = os.path.join(staging, "frames")
        os.makedirs(frames)
        frame = os.path.join(frames, "frame-000001.jpg")
        with open(frame, "wb") as output:
            output.write(b"frame-pixels")
        with open(os.path.join(staging, "unrelated-secret.txt"), "wb") as output:
            output.write(b"not a frame")
        if not restricted_claude._openat_available():
            try:
                restricted_claude.create_frame_only_bundle(
                    staging, ["frames/frame-000001.jpg"])
            except ValueError:
                unavailable_is_closed = True
            else:
                unavailable_is_closed = False
            check("hosts without descriptor nofollow support fail closed", unavailable_is_closed)
            return
        execution = restricted_claude.create_frame_only_bundle(
            staging, ["frames/frame-000001.jpg"])
        try:
            with open(os.path.join(execution, "frames", "frame-000001.jpg"), "rb") as source:
                copied = source.read()
            check("restricted execution root contains copied frame bytes",
                  copied == b"frame-pixels", repr(copied))
            check("restricted execution root excludes staging-only files",
                  not os.path.exists(os.path.join(execution, "unrelated-secret.txt")))
        finally:
            shutil.rmtree(execution, ignore_errors=True)

        for candidate, name in (("../outside.jpg", "escaping"),
                                ("unrelated-secret.txt", "outside frames")):
            try:
                restricted_claude.create_frame_only_bundle(staging, [candidate])
            except ValueError:
                rejected = True
            else:
                rejected = False
            check(f"{name} frame path is rejected before copy", rejected)

        link = os.path.join(frames, "outside-link.jpg")
        outside = os.path.join(root, "outside.jpg")
        with open(outside, "wb") as output:
            output.write(b"outside")
        try:
            os.symlink(outside, link)
        except (NotImplementedError, OSError):
            symlink_rejected = True
        else:
            try:
                restricted_claude.create_frame_only_bundle(staging, ["frames/outside-link.jpg"])
            except ValueError:
                symlink_rejected = True
            else:
                symlink_rejected = False
        check("symlinked frames are rejected before copy", symlink_rejected)

        hard_link = os.path.join(frames, "hard-link.jpg")
        try:
            os.link(frame, hard_link)
        except OSError:
            hard_link_rejected = True
        else:
            try:
                restricted_claude.create_frame_only_bundle(staging, ["frames/hard-link.jpg"])
            except ValueError:
                hard_link_rejected = True
            else:
                hard_link_rejected = False
        check("hard-linked frames are rejected before copy", hard_link_rejected)
        if os.path.exists(hard_link):
            os.unlink(hard_link)

        if hasattr(os, "mkfifo"):
            fifo = os.path.join(frames, "frame-fifo")
            try:
                os.mkfifo(fifo)
            except OSError:
                fifo_rejected = True
            else:
                try:
                    restricted_claude.create_frame_only_bundle(
                        staging, ["frames/frame-fifo"])
                except ValueError:
                    fifo_rejected = True
                else:
                    fifo_rejected = False
            check("FIFO frames are rejected without a blocking open", fifo_rejected)

        # A metadata failure happens after the leaf is open.  The helper must
        # translate it into the same fail-closed error without leaking the
        # opened descriptor into the long-lived judge process.
        metadata_root_fd = restricted_claude._open_root_directory(staging)
        opened: list[int] = []
        closed: list[int] = []
        original_open = os.open
        original_close = os.close

        def record_open(*args, **kwargs):
            descriptor = original_open(*args, **kwargs)
            opened.append(descriptor)
            return descriptor

        def record_close(descriptor: int) -> None:
            closed.append(descriptor)
            original_close(descriptor)

        try:
            with (
                mock.patch.object(restricted_claude.os, "open", side_effect=record_open),
                mock.patch.object(restricted_claude.os, "close", side_effect=record_close),
                mock.patch.object(restricted_claude.os, "fstat",
                                  side_effect=OSError("fixture metadata fault")),
            ):
                try:
                    restricted_claude._open_regular_frame(
                        metadata_root_fd, "frames/frame-000001.jpg")
                except ValueError:
                    metadata_rejected = True
                else:
                    metadata_rejected = False
        finally:
            original_close(metadata_root_fd)
        check("frame metadata failures close the opened descriptor",
              metadata_rejected and bool(opened) and opened[-1] in closed,
              repr((opened, closed)))

        staging_fd = restricted_claude._open_root_directory(staging)
        source_fd = restricted_claude._open_regular_frame(
            staging_fd, "frames/frame-000001.jpg")
        real_frames = os.path.join(staging, "real-frames")
        outside_frames = os.path.join(root, "outside-frames")
        os.makedirs(outside_frames)
        os.rename(frames, real_frames)
        try:
            os.symlink(outside_frames, frames)
        except (NotImplementedError, OSError):
            parent_replacement_rejected = True
        else:
            parent_replacement_rejected = False
        try:
            bound_destination = os.path.join(root, "bound-copy.jpg")
            destination_fd = os.open(
                bound_destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            try:
                restricted_claude._copy_open_regular_file(source_fd, destination_fd)
            finally:
                os.close(destination_fd)
        finally:
            os.close(source_fd)
            os.close(staging_fd)
        with open(bound_destination, "rb") as source:
            bound_bytes = source.read()
        check("opened frame descriptor survives parent replacement without redirect",
              bound_bytes == b"frame-pixels", repr(bound_bytes))

        if not parent_replacement_rejected:
            try:
                restricted_claude.create_frame_only_bundle(
                    staging, ["frames/frame-000001.jpg"])
            except ValueError:
                fresh_parent_rejected = True
            else:
                fresh_parent_rejected = False
        else:
            fresh_parent_rejected = True
        check("replaced frame parent cannot redirect a fresh descriptor traversal",
              fresh_parent_rejected)


def test_owned_canonical_alias_and_process_boundary() -> None:
    print("test_owned_canonical_alias_and_process_boundary")
    if not restricted_claude._openat_available():
        return
    with tempfile.TemporaryDirectory() as root:
        real_parent = os.path.join(root, "real-parent")
        alias_parent = os.path.join(root, "alias-parent")
        staging = os.path.join(real_parent, "staging")
        os.makedirs(os.path.join(staging, "frames"))
        with open(os.path.join(staging, "frames", "frame.jpg"), "wb") as output:
            output.write(b"trusted-frame")
        try:
            os.symlink(real_parent, alias_parent)
        except (NotImplementedError, OSError):
            return
        alias_staging = os.path.join(alias_parent, "staging")
        try:
            restricted_claude.create_frame_only_bundle(
                alias_staging, ["frames/frame.jpg"])
        except ValueError:
            caller_alias_rejected = True
        else:
            caller_alias_rejected = False
        check("caller staging aliases are rejected by strict parent traversal",
              caller_alias_rejected)
        execution = restricted_claude.create_frame_only_bundle(
            os.path.realpath(alias_staging), ["frames/frame.jpg"])
        try:
            with open(os.path.join(execution, "frames", "frame.jpg"), "rb") as source:
                canonical_copy = source.read()
            check("canonical path of an owned staging root remains usable",
                  canonical_copy == b"trusted-frame", repr(canonical_copy))
        finally:
            shutil.rmtree(execution, ignore_errors=True)

    cli = {
        "path": "/fixture/claude",
        "version": "2.1.248 (Claude Code)",
        "version_tuple": (2, 1, 248),
    }
    calls: list[tuple[list[str], dict]] = []

    def fixture_run(command, **kwargs):
        calls.append((command, kwargs))
        if "--json-schema" not in command:
            envelope = {"is_error": False, "result": "READ_OK"}
        else:
            envelope = {
                "is_error": False,
                "structured_output": {
                    "verdict": "pass", "issues": [], "cannot_assess": [],
                    "confidence": 0.9, "summary": "fixture review",
                },
            }
        return types.SimpleNamespace(returncode=0, stdout=json.dumps(envelope), stderr="")

    with mock.patch.object(cli_judge.subprocess, "run", side_effect=fixture_run):
        probe_ok, probe_reason = cli_judge.preflight_read_probe(
            cli, "sonnet", "/frame-only-root", "frames/frame.jpg", 10)
        review, meta, review_reason = cli_judge.invoke_claude(
            "claude", "system", "untrusted user text", "sonnet",
            "/frame-only-root", 10, cli)
    check("probe and review use the same confined frame-only cwd",
          probe_ok and review is not None and review_reason is None
          and len(calls) == 2 and all(call[1]["cwd"] == "/frame-only-root" for call in calls),
          repr((probe_reason, meta, calls)))
    check("probe and review retain exact restricted argv controls",
          all("--restricted" in call[0] and _value_after(call[0], "--tools") == "Read"
              and "--strict-mcp-config" in call[0] and "--add-dir" not in call[0]
              for call in calls), repr(calls))

    with mock.patch.object(cli_judge.subprocess, "run", side_effect=OSError("fixture launch lost")):
        probe_ok, probe_reason = cli_judge.preflight_read_probe(
            cli, "sonnet", "/frame-only-root", "frames/frame.jpg", 10)
        review, meta, review_reason = cli_judge.invoke_claude(
            "claude", "system", "untrusted user text", "sonnet",
            "/frame-only-root", 10, cli)
    check("launch failures stay classified as probe or review infrastructure errors",
          not probe_ok and "could not launch restricted Claude" in probe_reason
          and review is None and meta.get("launch_error") is True
          and review_reason and "could not launch restricted Claude" in review_reason,
          repr((probe_reason, meta, review_reason)))

    windows_calls: list[tuple[str, list[object]]] = []
    windows = types.SimpleNamespace(
        available=lambda: (True, None),
        create_frame_only_bundle=lambda staging, paths: (
            windows_calls.append((staging, paths)) or "C:\\\\restricted-root"),
    )
    with (
        mock.patch.object(restricted_claude.os, "name", "nt"),
        mock.patch.dict(sys.modules, {"restricted_claude_windows": windows}),
    ):
        windows_ready, windows_reason = restricted_claude.restricted_frame_bundle_available()
        windows_bundle = restricted_claude.create_frame_only_bundle(
            "C:\\\\staging", ["frames\\\\frame.jpg"])
    check("Windows dispatch uses the platform confined-copy helper",
          windows_ready and windows_reason is None
          and windows_bundle == "C:\\\\restricted-root"
          and windows_calls == [("C:\\\\staging", ["frames\\\\frame.jpg"])],
          repr((windows_ready, windows_reason, windows_calls)))


def test_auto_ladder_skips_unsupported_claude() -> None:
    print("test_auto_ladder_skips_unsupported_claude")
    with (
        mock.patch.object(ladder_judge.cli_judge, "detect_providers", return_value={
            "claude": {
                "found": True,
                "path": "/fixture/claude",
                "restricted_read_capable": False,
                "restricted_read_reason": "requires Claude Code >= 2.1.248",
            },
        }),
        mock.patch.object(ladder_judge.codex_judge, "detect", return_value={
            "provider": "codex", "found": True, "judge_ready": False,
            "availability_reason": "restricted judge capability is unverified",
            "path": "/fixture/codex"}),
        mock.patch.object(ladder_judge.antigravity_judge, "detect", return_value={
            "provider": "antigravity", "found": False}),
        mock.patch.object(ladder_judge.grok_judge, "detect", return_value={
            "provider": "grok", "found": True, "judge_ready": True,
            "path": "/fixture/grok"}),
    ):
        ladder = ladder_judge.detect_ladder()
    claude = next(rung for rung in ladder["rungs"] if rung["provider"] == "claude")
    codex = next(rung for rung in ladder["rungs"] if rung["provider"] == "codex")
    check("auto ladder skips unready Claude and Codex for admitted Grok",
          ladder["auto_selected"] == "grok" and claude["judge_ready"] is False
          and codex["judge_ready"] is False,
          repr(ladder))
    reason = ladder_judge.skip_reason_block({"rungs": [{
        "provider": "grok", "found": False,
        "availability_reason": "no-tool policy flags are incomplete",
    }]})
    check("ladder preserves an unavailable provider's capability reason",
          "no-tool policy flags are incomplete" in reason, reason)

    with (
        mock.patch.object(ladder_judge.cli_judge, "detect_providers", return_value={
            "claude": {"found": False}}),
        mock.patch.object(ladder_judge.codex_judge, "detect", return_value={
            "provider": "codex", "found": False}),
        mock.patch.object(ladder_judge.antigravity_judge, "detect", return_value={
            "provider": "antigravity", "found": False}),
        mock.patch.object(ladder_judge.grok_judge, "detect", return_value={
            "provider": "grok", "found": True, "path": "/fixture/future"}),
    ):
        unadmitted_ladder = ladder_judge.detect_ladder()
    check("found adapters require an explicit judge-ready declaration",
          unadmitted_ladder["auto_selected"] is None,
          repr(unadmitted_ladder))


def test_staging_symlinks_fail_before_provider_pipeline() -> None:
    print("test_staging_symlinks_fail_before_provider_pipeline")
    with tempfile.TemporaryDirectory() as root:
        render = os.path.join(root, "render.mp4")
        with open(render, "wb") as output:
            output.write(b"render")
        real_bundle = os.path.join(root, "real-bundle")
        linked_bundle = os.path.join(root, "linked-bundle")
        os.makedirs(real_bundle)
        try:
            os.symlink(real_bundle, linked_bundle)
        except (NotImplementedError, OSError):
            return
        with mock.patch.object(sys, "argv", [
                "cli_judge.py", "review", "--render", render,
                "--bundle-dir", linked_bundle]), \
                mock.patch.object(cli_judge.judge, "probe_duration_s", return_value=1.0):
            linked_bundle_result = cli_judge.main()
        check("symlinked staging bundle is refused before perception or provider work",
              linked_bundle_result == 2, repr(linked_bundle_result))

        bundle = os.path.join(root, "bundle")
        frames_target = os.path.join(root, "frames-target")
        os.makedirs(bundle)
        os.makedirs(frames_target)
        os.symlink(frames_target, os.path.join(bundle, "frames"))
        with (
            mock.patch.object(sys, "argv", [
                "cli_judge.py", "review", "--render", render,
                "--bundle-dir", bundle]),
            mock.patch.object(cli_judge.judge, "probe_duration_s", return_value=1.0),
            mock.patch.object(cli_judge, "resolve_perception",
                              return_value=({}, {"mode": "fixture"}, [])),
            mock.patch.object(cli_judge.judge, "sanity_check_perception"),
        ):
            linked_frames_result = cli_judge.main()
        check("symlinked staging frames directory is refused before extraction",
              linked_frames_result == 2, repr(linked_frames_result))


def main() -> int:
    test_capability_argv_and_frame_root()
    test_owned_canonical_alias_and_process_boundary()
    test_auto_ladder_skips_unsupported_claude()
    test_staging_symlinks_fail_before_provider_pipeline()
    if _FAILS:
        print("FAILED:")
        for failure in _FAILS:
            print("  - " + failure)
        return 1
    print("ALL RESTRICTED CLAUDE VALIDATION TESTS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
