"""Capability-gated, no-tool policy for a Grok Build render judge.

The judge receives visual evidence as ACP input-image content blocks.  It must
not gain a second information or action channel through local built-ins, shell,
web, or MCP tools.  The policy is deliberately constructed with Grok's
documented allow-then-remove ordering instead of relying on undocumented empty
``--tools`` list semantics.

Evidence basis: Grok Build 1.0.21's shipped CLI reference documents that
``--tools`` disables default built-in injection, ``--disallowed-tools`` runs
after it, bare ``MCPTool`` matches every MCP tool invocation, and deny wins.
The resolver checks both the minimum release and the advertised flags before a
judge can start.  A missing or opaque capability is an honest not-run result;
the adapter never retries with wider permissions.

This is intentionally narrower than configured-MCP lifecycle management.  It
denies model-originated MCP *tool invocations*.  It does not assert that a
trusted configured MCP server cannot be discovered or started before a model
tries to call it; that is a separate provider/configuration lifecycle concern.
"""

from __future__ import annotations

import re
import os
import shutil
import subprocess
import sys

_PY_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, _PY_ROOT)
import provider_child_launch

MIN_GROK_TOOL_POLICY_VERSION = (1, 0, 21)
_VERSION_RE = re.compile(r"(?<![0-9.])(\d+)\.(\d+)\.(\d+)(?![0-9.])")

# A nonempty allowlist is required because empty --tools semantics are not
# documented. Grok documents that --disallowed-tools runs after --tools, so
# removing all three leaves no built-in judge tool available. The former
# per-tool `vision_describe` exclusion is intentionally absent: if it is a
# built-in it is outside this allowlist; if it remains the native MCP route the
# bare MCPTool deny matches it regardless of its server-qualified name.
TOOL_ALLOW_THEN_REMOVE = ("read_file", "grep", "list_dir")
TOOL_REMOVALS = ("read_file", "grep", "list_dir", "Agent")
MCP_DENY_RULE = "MCPTool"
REQUIRED_FLAGS = (
    "--tools",
    "--disallowed-tools",
    "--deny",
    "--no-subagents",
    "--disable-web-search",
    "--sandbox",
    "--permission-mode",
)
MCP_STARTUP_LIMITATION = (
    "denies model-originated MCP tool invocations only; configured MCP server "
    "discovery or startup is not asserted by this per-invocation policy")


def _parse_version(text: str) -> tuple[int, int, int] | None:
    match = _VERSION_RE.search(text)
    if match is None:
        return None
    return tuple(int(part) for part in match.groups())


def _minimum_version_text() -> str:
    return ".".join(str(part) for part in MIN_GROK_TOOL_POLICY_VERSION)


def resolve_grok_tool_policy(grok_bin: str, launch: dict | None = None) -> tuple[dict | None, str | None]:
    """Resolve a Grok CLI whose documented no-tool policy can be enforced.

    This admission performs only ``--version`` and ``--help`` subprocesses. It
    never starts a model session and never reads provider-owned auth or config
    files. Unknown, older, failed, or flag-incomplete wrappers are rejected
    before prompt-file creation and cannot fall back to a broader command.
    """
    path = launch["executable"] if launch else shutil.which(grok_bin)
    if not path:
        return None, f"grok CLI not found ({grok_bin!r}) — honest not_run"
    try:
        version = subprocess.run(
            provider_child_launch.command(launch, ["--version"]) if launch else [path, "--version"],
            capture_output=True, text=True, encoding="utf-8", timeout=15,
            env=provider_child_launch.environment(launch) if launch else None)
    except (subprocess.TimeoutExpired, OSError) as exc:
        return None, f"could not verify Grok no-tool policy version: {exc}"
    version_text = (version.stdout.strip() or version.stderr.strip())[:240]
    if version.returncode != 0:
        return None, (
            "could not verify Grok no-tool policy version "
            f"(exit {version.returncode}): {version_text or '(no version output)'}")
    parsed = _parse_version(version_text)
    if parsed is None:
        return None, (
            "Grok version is not parseable; no-tool judge policy requires Grok "
            f"Build >= {_minimum_version_text()}")
    if parsed < MIN_GROK_TOOL_POLICY_VERSION:
        return None, (
            f"Grok Build {'.'.join(str(part) for part in parsed)} lacks the "
            "documented no-tool judge policy; requires >= "
            f"{_minimum_version_text()}")

    try:
        help_result = subprocess.run(
            provider_child_launch.command(launch, ["--help"]) if launch else [path, "--help"],
            capture_output=True, text=True, encoding="utf-8", timeout=15,
            env=provider_child_launch.environment(launch) if launch else None)
    except (subprocess.TimeoutExpired, OSError) as exc:
        return None, f"could not verify Grok no-tool policy flags: {exc}"
    help_text = (help_result.stdout or "") + "\n" + (help_result.stderr or "")
    missing = tuple(flag for flag in REQUIRED_FLAGS if flag not in help_text)
    if help_result.returncode != 0 or missing:
        detail = ", ".join(missing) if missing else f"exit {help_result.returncode}"
        return None, (
            "Grok CLI does not advertise the complete no-tool judge policy "
            f"({detail}); refusing a broader invocation")
    return {
        "path": path,
        "version": version_text,
        "version_tuple": parsed,
        "launch": launch,
    }, None


def grok_judge_argv(cli: dict, prompt_file: str, cwd: str,
                    model: str) -> list[str]:
    """Build the fixed no-tool headless argv after capability admission."""
    argv = [
        "--prompt-file", prompt_file,
        "--output-format", "json",
        "--no-memory",
        "--tools", ",".join(TOOL_ALLOW_THEN_REMOVE),
        "--disallowed-tools", ",".join(TOOL_REMOVALS),
        "--deny", MCP_DENY_RULE,
        "--no-subagents",
        "--disable-web-search",
        "--sandbox", "read-only",
        "--permission-mode", "dontAsk",
        "--cwd", cwd,
    ]
    if model:
        argv += ["--model", model]
    launch = cli.get("launch")
    return provider_child_launch.command(launch, argv) if launch else [cli["path"]] + argv


def child_environment(cli: dict) -> dict | None:
    launch = cli.get("launch")
    return provider_child_launch.environment(launch) if launch else None


def policy_metadata(cli: dict) -> dict:
    """Receipt-visible policy scope, including the explicit MCP limitation."""
    return {
        "mode": "allow_then_remove_builtins",
        "minimum_version": _minimum_version_text(),
        "version": cli["version"],
        "builtins_removed": list(TOOL_REMOVALS[:-1]),
        "mcp_tool_invocations": "denied",
        "mcp_server_startup": MCP_STARTUP_LIMITATION,
        "sandbox": "read-only",
        "permission_mode": "dontAsk",
    }
