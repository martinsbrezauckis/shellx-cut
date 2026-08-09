"""Bounded, source-labelled diagnostics for judge subprocess failures.

Judge CLIs do not agree on an error stream.  In particular, Claude may return
its structured error envelope on stdout while stderr is empty.  Keep both
streams visible instead of choosing one and accidentally hiding the only
actionable evidence.
"""

from __future__ import annotations


def _tail(text: str | None, limit: int) -> str:
    """Return a non-empty stream's bounded trailing text."""
    return (text or "").strip()[-limit:]


def process_failure_detail(stdout: str | None, stderr: str | None,
                           limit: int) -> str:
    """Describe every non-empty process stream without inferring a cause.

    ``limit`` applies to each stream.  The labels make it explicit whether a
    displayed JSON object is the CLI's stdout envelope or a stderr diagnostic.
    Empty output is reported explicitly so a bare exit code is never mistaken
    for evidence of a particular root cause.
    """
    parts = []
    for name, text in (("stdout", stdout), ("stderr", stderr)):
        detail = _tail(text, limit)
        if detail:
            parts.append(f"{name}: {detail}")
    return " | ".join(parts) or "(no output on stdout or stderr)"


def failed_preflight_reason(first: str, retry: str) -> str:
    """State a failed Read preflight without speculating about its cause."""
    return (
        "pre-flight Read probe failed twice (original + fresh bundle dir) — "
        "full review NOT attempted. "
        f"[1] {first} [2] {retry}. "
        "Root cause is not established by these diagnostics; inspect the "
        "reported stream/envelope and correct the environment before retrying."
    )
