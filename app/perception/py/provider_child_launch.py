"""Typed provider-child launch handoff from the Cut Rust process.

This is intentionally not a Runner admission parser. Rust validates the
immutable Runner context and sends this small, already-selected mapping only to
the Cut-owned Python adapter.  The adapter uses it only when it creates the
actual provider subprocess; it never installs the mapping into its own process
environment or resolves a provider through PATH.
"""

from __future__ import annotations

import json
import os
import sys


SCHEMA = "shellx-cut/provider-child-launches/1"
_MAX_STDIN_BYTES = 64 * 1024
_TEMPORARY_KEYS = ("TMPDIR", "TEMP", "TMP")


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
