"""Typed provider-child launch handoff from the Cut Rust process.

This is intentionally not a Runner admission parser. Rust validates the
immutable Runner context and sends this small, already-selected mapping only to
the Cut-owned Python adapter. The adapter uses it only when it creates the
actual provider subprocess; it never installs the mapping into its own process
environment or resolves a provider through its ambient PATH. Runner's
host-managed context selects paths and a child environment, not immutable Node
or JavaScript bytes; native qualification must check the installed launch.
"""

from __future__ import annotations

import json
import ntpath
import os
import re
import sys


SCHEMA = "shellx-cut/provider-child-launches/1"
_MAX_STDIN_BYTES = 64 * 1024
_TEMPORARY_KEYS = ("TMPDIR", "TEMP", "TMP")
_MAX_SHIM_BYTES = 16 * 1024
_WINDOWS_UNSAFE_PATH = re.compile(r'[%!&|<>^"?*\r\n]')
_NPM_TARGET = re.compile(r'"%dp0%\\(node_modules\\[A-Za-z0-9_@.\\-]+\.js)"', re.I)
_NATIVE_EXE_LINE = re.compile(r'"([A-Za-z]:\\[^"\r\n]+\.exe)" %\*', re.I)


def _windows_absolute_file(path):
    drive, remainder = ntpath.splitdrive(path)
    return (bool(re.fullmatch(r'[A-Za-z]:', drive)) and remainder.startswith('\\')
            and '/' not in path and ':' not in remainder
            and not _WINDOWS_UNSAFE_PATH.search(path)
            and all(part not in ('', '.', '..') for part in path[3:].split('\\')))


def _npm_shim_target(shim):
    """Accept only the Node cmd-shim forms whose target can be reconstructed."""
    lines = [line.strip() for line in shim.splitlines() if line.strip()]
    head = ['@ECHO off', 'GOTO start', ':find_dp0', 'SET dp0=%~dp0',
            'EXIT /b', ':start', 'SETLOCAL', 'CALL :find_dp0',
            'IF EXIST "%dp0%\\node.exe" (', 'SET "_prog=%dp0%\\node.exe"',
            ') ELSE (', 'SET "_prog=node"']
    if lines[:len(head)] != head:
        raise ValueError('unsupported Windows npm provider shim')
    tail = lines[len(head):]
    old_tail = ['SET PATHEXT=%PATHEXT:;.JS;=;%', ')']
    new_tail = [')']
    if tail[:2] == old_tail:
        tail = tail[2:]
        invocation = 'endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  '
    elif tail[:1] == new_tail:
        tail = tail[1:]
        invocation = ('endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & '
                      'set PATHEXT=%PATHEXT:;.JS;=;% & "%_prog%"  ')
    else:
        raise ValueError('unsupported Windows npm provider shim')
    if len(tail) != 1 or not tail[0].startswith(invocation) or not tail[0].endswith(' %*'):
        raise ValueError('unsupported Windows npm provider shim')
    target = tail[0][len(invocation):-3]
    match = _NPM_TARGET.fullmatch(target)
    if not match or any(part in ('', '.', '..') for part in match[1].split('\\')):
        raise ValueError('unsafe Windows npm provider target')
    return match[1]


def _native_exe_shim_target(shim, lines):
    """Resolve the installed three-line native wrapper without running cmd.exe."""
    if len(lines) != 3 or lines[0].lower() != '@echo off' or lines[2].lower() != 'exit /b %errorlevel%':
        return None
    match = _NATIVE_EXE_LINE.fullmatch(lines[1])
    if not match:
        return None
    target = match[1]
    if (not _windows_absolute_file(target) or
            ntpath.splitext(ntpath.basename(target))[0].lower() !=
            ntpath.splitext(ntpath.basename(shim))[0].lower() or
            not os.path.isfile(target)):
        raise ValueError('unsafe Windows native provider shim target')
    return target


def _windows_shim_command(launch):
    shim = launch['executable']
    if launch['entrypoint'] is not None or not _windows_absolute_file(shim):
        raise ValueError('invalid admitted Windows npm provider shim')
    with open(shim, 'rb') as source:
        raw = source.read(_MAX_SHIM_BYTES + 1)
    if len(raw) > _MAX_SHIM_BYTES:
        raise ValueError('Windows npm provider shim exceeds size bound')
    try:
        script = raw.decode('utf-8-sig')
    except UnicodeDecodeError as error:
        raise ValueError('Windows npm provider shim is not UTF-8') from error
    lines = [line.strip() for line in script.splitlines() if line.strip()]
    native_target = _native_exe_shim_target(shim, lines)
    if native_target is not None:
        return [native_target]
    target = _npm_shim_target(script)
    directory = ntpath.dirname(shim)
    entrypoint = ntpath.join(directory, target)
    if not os.path.isfile(entrypoint):
        raise ValueError('Windows npm provider entrypoint is missing')
    # A lexical node_modules path may still traverse an NTFS junction or link.
    # Compare physical paths after opening the shim and before launching Node.
    physical_directory = ntpath.normcase(os.path.realpath(directory))
    physical_entrypoint = ntpath.normcase(os.path.realpath(entrypoint))
    try:
        contained = ntpath.commonpath((physical_directory, physical_entrypoint)) == physical_directory
    except ValueError:
        contained = False
    if (not _windows_absolute_file(physical_directory)
            or not _windows_absolute_file(physical_entrypoint)
            or not contained
            or physical_entrypoint == physical_directory):
        raise ValueError('Windows npm provider entrypoint escapes shim directory')
    # Use only Runner's selected child PATH. Its directories are admitted
    # locators; in host-managed mode they do not pin Node's current bytes.
    adjacent = ntpath.join(directory, 'node.exe')
    if os.path.isfile(adjacent):
        return [adjacent, entrypoint]
    path = launch['environment'].get('PATH')
    if not path:
        raise ValueError('admitted Windows npm provider has no interpreter PATH')
    for component in path.split(';'):
        if not _windows_absolute_file(component):
            raise ValueError('admitted Windows npm provider PATH is invalid')
        candidate = ntpath.join(component, 'node.exe')
        if os.path.isfile(candidate):
            return [candidate, entrypoint]
    raise ValueError('admitted Windows npm provider node.exe is missing')


def _text(value, field):
    if not isinstance(value, str) or not value:
        raise ValueError(f"provider child {field} must be a non-empty string")
    return value


def _launch(value):
    if not isinstance(value, dict) or set(value) != {"executable", "entrypoint", "environment"}:
        raise ValueError("provider child launch has an invalid shape")
    executable = _text(value.get("executable"), "executable")
    entrypoint = value.get("entrypoint")
    if entrypoint is not None:
        entrypoint = _text(entrypoint, "entrypoint")
    environment = value.get("environment")
    if not isinstance(environment, dict) or not environment:
        raise ValueError("provider child environment must be a non-empty object")
    if any(not isinstance(key, str) or not key or not isinstance(item, str) or not item
           for key, item in environment.items()):
        raise ValueError("provider child environment must contain non-empty text entries")
    return {
        "executable": executable,
        "entrypoint": entrypoint,
        "environment": dict(environment),
    }


def launches(value, allowed):
    """Validate the Rust handoff and retain only this adapter's providers."""
    if not isinstance(value, dict) or set(value) != {"schema", "launches"}:
        raise ValueError("provider child handoff has an invalid shape")
    if value.get("schema") != SCHEMA:
        raise ValueError("provider child handoff schema is invalid")
    raw_launches = value.get("launches")
    if not isinstance(raw_launches, dict):
        raise ValueError("provider child handoff launches must be an object")
    unknown = set(raw_launches).difference(allowed)
    if unknown:
        raise ValueError("provider child handoff includes an unsupported provider")
    return {provider: _launch(launch) for provider, launch in raw_launches.items()}


def launches_from_stdin(allowed):
    """Read exactly one bounded Rust-to-Python handoff from standard input."""
    raw = sys.stdin.buffer.read(_MAX_STDIN_BYTES + 1)
    if len(raw) > _MAX_STDIN_BYTES:
        raise ValueError("provider child handoff exceeds 64 KiB")
    try:
        value = json.loads(raw)
    except (TypeError, ValueError, json.JSONDecodeError) as error:
        raise ValueError(f"provider child handoff is not JSON: {error}") from error
    return launches(value, allowed)


def command(launch, arguments):
    """Return the exact admitted prefix followed by existing policy arguments."""
    if sys.platform == 'win32' and launch['executable'].lower().endswith('.cmd'):
        prefix = _windows_shim_command(launch)
    else:
        prefix = [launch["executable"]]
        if launch["entrypoint"] is not None:
            prefix.append(launch["entrypoint"])
    return prefix + list(arguments)


def environment(launch):
    """The admitted child environment plus Runner's owned temporary locator.

    Python's ``env=`` replaces the provider child's inherited environment. Keep
    that boundary, while carrying the one job-owned temporary directory that
    Runner deliberately gave this adapter. Do not use a host default: a
    missing Runner locator is an invalid selected-provider route.
    """
    temporary = next(
        (os.environ.get(key) for key in _TEMPORARY_KEYS if os.environ.get(key)),
        None,
    )
    if not temporary or not os.path.isabs(temporary):
        raise ValueError("Runner-owned provider temporary locator is missing or non-absolute")
    child = dict(launch["environment"])
    child.update({key: temporary for key in _TEMPORARY_KEYS})
    return child
