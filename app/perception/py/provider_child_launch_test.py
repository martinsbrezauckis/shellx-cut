"""Inert unit coverage for the Rust-to-Python provider child handoff."""

from __future__ import annotations

import importlib.util
import hashlib
import ntpath
import os
import subprocess
import sys
import unittest
from unittest.mock import mock_open, patch


HERE = os.path.dirname(os.path.abspath(__file__))
ADAPTERS = os.path.join(HERE, "judge", "adapters")
LADDER = os.path.join(ADAPTERS, "ladder_judge.py")
sys.path.insert(0, HERE)
sys.path.insert(0, ADAPTERS)

import generate_prompt_adapter
import generate_storyboard_adapter
import grok_tool_policy
import provider_child_launch
import restricted_claude


def _import_ladder_with_its_schema():
    # Generators import the judge.adapters namespace package; the separately
    # launched ladder imports judge/judge.py as a module. Keep both real routes
    # isolated in this combined unit-test process without altering product code.
    schema_path = os.path.join(HERE, "judge", "judge.py")
    spec = importlib.util.spec_from_file_location("judge", schema_path)
    schema = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(schema)
    with patch.dict(sys.modules, {"judge": schema}):
        import ladder_judge
    return ladder_judge


ladder_judge = _import_ladder_with_its_schema()


LAUNCH = {
    "executable": "/admitted/bin/provider-runtime",
    "entrypoint": "/admitted/lib/provider-entry.mjs",
    "environment": {"HOME": "/admitted/home", "PATH": "/admitted/bin"},
}
RUNNER_TEMP = "/runner/attempt/tmp"
WINDOWS_SHIM = r"C:\Users\tester\AppData\Roaming\npm\codex.cmd"
WINDOWS_ENTRY = r"C:\Users\tester\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.js"
WINDOWS_NODE = r"C:\Program Files\nodejs\node.exe"
WINDOWS_LAUNCH = {
    "executable": WINDOWS_SHIM,
    "entrypoint": None,
    "environment": {"PATH": r"C:\Users\tester\AppData\Roaming\npm;C:\Program Files\nodejs"},
}
NATIVE_WRAPPERS = (
    ("claude", r"C:\Users\tester\.local\bin\claude.exe",
     "0087ba3b221d4e55186063ac044f6dba0e7540f7482198ee10a5b9d58a2ed851"),
    ("grok", r"C:\Users\tester\.grok\bin\grok.exe",
     "b5f2ba9aa4eae7fd2f97449f90ab736fad7d00162865abc75216cbe912604e25"),
)


def npm_shim(target=r"node_modules\@openai\codex\bin\codex.js", modern=False):
    head = ("@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\n"
            "EXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n"
            'IF EXIST "%dp0%\\node.exe" (\r\n'
            '  SET "_prog=%dp0%\\node.exe"\r\n'
            ') ELSE (\r\n  SET "_prog=node"\r\n')
    if modern:
        tail = (')\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & '
                'set PATHEXT=%PATHEXT:;.JS;=;% & "%_prog%"  ')
    else:
        tail = ('  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\n'
                'endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  ')
    return (head + tail + '"%dp0%\\' + target + '" %*\r\n').encode()


class Completed:
    returncode = 0
    stdout = b'{"result":"{\\"template_id\\":\\"builtin.title.simple\\",\\"params\\":{},\\"at_ms\\":null}"}'
    stderr = b""


class ProviderChildLaunchTests(unittest.TestCase):
    def test_three_line_native_wrappers_launch_exe_directly(self):
        arguments = ['--version', 'literal & echo injected', '%PATH%']
        for name, target, expected_sha in NATIVE_WRAPPERS:
            raw = ('@echo off\r\n"' + target + '" %*\r\n'
                   'exit /b %errorlevel%\r\n').encode('ascii')
            self.assertEqual(hashlib.sha256(raw).hexdigest(), expected_sha)
            launch = {**WINDOWS_LAUNCH,
                      'executable': ntpath.join(ntpath.dirname(WINDOWS_SHIM), name + '.cmd')}
            with self.subTest(name=name), patch.object(
                provider_child_launch.sys, 'platform', 'win32'
            ), patch('builtins.open', mock_open(read_data=raw)), patch.object(
                provider_child_launch.os.path, 'isfile', side_effect=lambda path: path == target
            ):
                self.assertEqual(provider_child_launch.command(launch, arguments),
                                 [target, *arguments])

    def test_native_wrapper_rejects_other_commands_and_targets(self):
        target = r'C:\Users\tester\.local\bin\claude.exe'
        base = ('@echo off\r\n"' + target + '" %*\r\n'
                'exit /b %errorlevel%\r\n').encode('ascii')
        variants = (base + b'echo extra\r\n',
                    base.replace(b'claude.exe', b'calc.exe'),
                    base.replace(b'" %*', b'" & calc & %*'),
                    base.replace(b'" %*', b'" --unsafe %*'),
                    base.replace(b'C:\\Users', b'%USERPROFILE%'))
        launch = {**WINDOWS_LAUNCH,
                  'executable': ntpath.join(ntpath.dirname(WINDOWS_SHIM), 'claude.cmd')}
        for raw in variants:
            with self.subTest(raw=raw), patch.object(
                provider_child_launch.sys, 'platform', 'win32'
            ), patch('builtins.open', mock_open(read_data=raw)), patch.object(
                provider_child_launch.os.path, 'isfile', return_value=True
            ), self.assertRaises(ValueError):
                provider_child_launch.command(launch, ['--version'])

    def test_windows_npm_shim_uses_selected_node_and_preserves_literal_arguments(self):
        arguments = ['exec', '--json', 'literal & echo injected', '%PATH%', 'a"b']
        for modern in (False, True):
            with self.subTest(modern=modern), patch.object(
                provider_child_launch.sys, 'platform', 'win32'
            ), patch('builtins.open', mock_open(read_data=npm_shim(modern=modern))), patch.object(
                provider_child_launch.os.path, 'isfile',
                side_effect=lambda path: path in (WINDOWS_ENTRY, WINDOWS_NODE)
            ), patch.object(
                provider_child_launch.os.path, 'realpath', side_effect=lambda path: path
            ):
                command = provider_child_launch.command(WINDOWS_LAUNCH, arguments)
            self.assertEqual(command, [WINDOWS_NODE, WINDOWS_ENTRY, *arguments])

    def test_windows_npm_shim_rejects_ambiguous_or_escaping_content(self):
        variants = [
            npm_shim() + b'echo extra\r\n',
            npm_shim(target=r'..\outside.js'),
            npm_shim(target=r'node_modules\@openai\codex\bin\codex.js" & calc & "'),
            npm_shim().replace(b'"%_prog%"', b'"other.exe"'),
            npm_shim() + b'x' * 16384,
        ]
        for raw in variants:
            with self.subTest(raw=raw[:35]), patch.object(
                provider_child_launch.sys, 'platform', 'win32'
            ), patch('builtins.open', mock_open(read_data=raw)), patch.object(
                provider_child_launch.os.path, 'isfile', return_value=True
            ), patch.object(
                provider_child_launch.os.path, 'realpath', side_effect=lambda path: path
            ), self.assertRaises(ValueError):
                provider_child_launch.command(WINDOWS_LAUNCH, ['--help'])

    def test_windows_npm_shim_rejects_linked_entrypoint_outside_shim_directory(self):
        with patch.object(provider_child_launch.sys, 'platform', 'win32'), patch(
            'builtins.open', mock_open(read_data=npm_shim())
        ), patch.object(provider_child_launch.os.path, 'isfile', return_value=True), patch.object(
            provider_child_launch.os.path, 'realpath',
            side_effect=lambda path: r'C:\other\codex.js' if path == WINDOWS_ENTRY else path
        ), self.assertRaisesRegex(ValueError, 'escapes shim directory'):
            provider_child_launch.command(WINDOWS_LAUNCH, ['--help'])

    def test_windows_npm_shim_rejects_missing_interpreter_and_invalid_selected_path(self):
        with patch.object(provider_child_launch.sys, 'platform', 'win32'), patch(
            'builtins.open', mock_open(read_data=npm_shim())
        ), patch.object(provider_child_launch.os.path, 'isfile',
                        side_effect=lambda path: path == WINDOWS_ENTRY), patch.object(
                            provider_child_launch.os.path, 'realpath', side_effect=lambda path: path
                        ):
            with self.assertRaisesRegex(ValueError, 'node.exe is missing'):
                provider_child_launch.command(WINDOWS_LAUNCH, [])
        invalid = {**WINDOWS_LAUNCH, 'executable': r'C:\npm&echo\codex.cmd'}
        with patch.object(provider_child_launch.sys, 'platform', 'win32'):
            with self.assertRaisesRegex(ValueError, 'invalid admitted'):
                provider_child_launch.command(invalid, [])

    def test_combined_unit_imports_preserve_both_real_judge_routes(self):
        self.assertEqual(
            os.path.realpath(ladder_judge.judge.__file__),
            os.path.join(HERE, "judge", "judge.py"),
        )
        self.assertTrue(ladder_judge.judge.ISSUE_KINDS)
        self.assertTrue(hasattr(sys.modules["judge"], "__path__"))
        self.assertIs(
            generate_prompt_adapter.grok_tool_policy,
            generate_storyboard_adapter.grok_tool_policy,
        )

    def test_isolated_ladder_imports_the_packaged_provider_launch_helper(self):
        result = subprocess.run(
            [sys.executable, "-B", "-I", LADDER, "--help"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Bundled ShellX Cut judge access ladder", result.stdout)

    def test_launches_reject_unknown_provider_and_copy_child_environment(self):
        handoff = {
            "schema": provider_child_launch.SCHEMA,
            "launches": {"claude": LAUNCH},
        }
        launches = provider_child_launch.launches(handoff, {"claude"})
        self.assertEqual(
            provider_child_launch.command(launches["claude"], ["--version"]),
            [LAUNCH["executable"], LAUNCH["entrypoint"], "--version"],
        )
        with patch.dict(
            provider_child_launch.os.environ,
            {"TMPDIR": RUNNER_TEMP, "UNRELATED_PARENT_KEY": "must-not-reach-child"},
            clear=True,
        ):
            environment = provider_child_launch.environment(launches["claude"])
        environment["HOME"] = "mutated-only-in-test"
        self.assertEqual(launches["claude"]["environment"]["HOME"], "/admitted/home")
        self.assertEqual(
            {key: environment[key] for key in ("TMPDIR", "TEMP", "TMP")},
            {"TMPDIR": RUNNER_TEMP, "TEMP": RUNNER_TEMP, "TMP": RUNNER_TEMP},
        )
        self.assertNotIn("UNRELATED_PARENT_KEY", environment)
        handoff["launches"]["grok"] = LAUNCH
        with self.assertRaisesRegex(ValueError, "unsupported provider"):
            provider_child_launch.launches(handoff, {"claude"})

    def test_planner_children_use_exact_prefix_and_child_environment(self):
        with patch.dict(provider_child_launch.os.environ, {"TEMP": RUNNER_TEMP}, clear=True):
            for adapter in (generate_prompt_adapter, generate_storyboard_adapter):
                with self.subTest(adapter=adapter.__name__), patch.object(
                    adapter.subprocess, "run", return_value=Completed()
                ) as run:
                    answer, _model, error = adapter.spawn_cli("claude", LAUNCH, "plan this", 10)
                self.assertIsNone(error)
                self.assertIn("template_id", answer)
                command = run.call_args.args[0]
                self.assertEqual(command[:2], [LAUNCH["executable"], LAUNCH["entrypoint"]])
                self.assertEqual(command[2:], ["-p", "--output-format", "json", "--tools", ""])
                self.assertEqual(
                    run.call_args.kwargs["env"],
                    {**LAUNCH["environment"], "TMPDIR": RUNNER_TEMP,
                     "TEMP": RUNNER_TEMP, "TMP": RUNNER_TEMP},
                )

    def test_unset_context_keeps_the_existing_planner_child_route(self):
        with patch.object(
            generate_prompt_adapter.subprocess, "run", return_value=Completed()
        ) as run:
            _answer, _model, error = generate_prompt_adapter.spawn_cli(
                "claude", "/ordinary/claude", "plan this", 10
            )
        self.assertIsNone(error)
        self.assertEqual(
            run.call_args.args[0],
            ["/ordinary/claude", "-p", "--output-format", "json", "--tools", ""],
        )
        self.assertIsNone(run.call_args.kwargs["env"])

    def test_judge_policy_prefixes_keep_provider_policy_after_entrypoint(self):
        cli = {"path": LAUNCH["executable"], "launch": LAUNCH, "version": "2.1.248"}
        with patch.dict(provider_child_launch.os.environ, {"TMP": RUNNER_TEMP}, clear=True):
            claude = restricted_claude.restricted_claude_argv(cli, "sonnet", json_schema=None)
            self.assertEqual(claude[:2], [LAUNCH["executable"], LAUNCH["entrypoint"]])
            self.assertIn("--restricted", claude)
            self.assertEqual(
                restricted_claude.child_environment(cli),
                {**LAUNCH["environment"], "TMPDIR": RUNNER_TEMP,
                 "TEMP": RUNNER_TEMP, "TMP": RUNNER_TEMP},
            )

            grok = grok_tool_policy.grok_judge_argv(cli, "/tmp/prompt.json", "/tmp/bundle", "")
            self.assertEqual(grok[:2], [LAUNCH["executable"], LAUNCH["entrypoint"]])
            self.assertIn("--no-subagents", grok)

    def test_admitted_child_rejects_missing_runner_temporary_locator(self):
        with patch.dict(provider_child_launch.os.environ, {}, clear=True):
            with self.assertRaisesRegex(ValueError, "temporary locator"):
                provider_child_launch.environment(LAUNCH)
        with patch.dict(provider_child_launch.os.environ, {"TEMP": "relative/tmp"}, clear=True):
            with self.assertRaisesRegex(ValueError, "non-absolute"):
                provider_child_launch.environment(LAUNCH)

    def test_admitted_judge_ladder_never_detects_unadmitted_path_providers(self):
        with patch.object(
            ladder_judge.cli_judge, "detect_providers", return_value={
                "claude": {"found": True, "restricted_read_capable": True}
            }
        ) as claude, patch.object(ladder_judge.codex_judge, "detect") as codex, patch.object(
            ladder_judge.antigravity_judge, "detect"
        ) as antigravity, patch.object(ladder_judge.grok_judge, "detect") as grok:
            ladder = ladder_judge.detect_ladder({"claude": LAUNCH})
        self.assertEqual(ladder["auto_selected"], "claude")
        claude.assert_called_once_with(LAUNCH)
        codex.assert_not_called()
        antigravity.assert_not_called()
        grok.assert_not_called()

    def test_unsupported_admitted_wrapper_only_marks_its_provider_unavailable(self):
        codex_launch = {**LAUNCH, "executable": "/admitted/codex.cmd"}
        with patch.object(
            ladder_judge.cli_judge, "detect_providers", side_effect=ValueError("shim contents")
        ), patch.object(ladder_judge.codex_judge, "detect", return_value={
            "provider": "codex", "found": True, "judge_ready": True
        }) as codex, patch.object(ladder_judge.antigravity_judge, "detect") as agy, patch.object(
            ladder_judge.grok_judge, "detect"
        ) as grok:
            ladder = ladder_judge.detect_ladder({"claude": LAUNCH, "codex": codex_launch})
        self.assertEqual(ladder["auto_selected"], "codex")
        self.assertEqual(ladder["rungs"][0]["availability_reason"],
                         "provider launch wrapper is unsupported")
        self.assertNotIn("shim contents", str(ladder))
        codex.assert_called_once_with(codex_launch)
        agy.assert_not_called()
        grok.assert_not_called()


if __name__ == "__main__":
    unittest.main()
