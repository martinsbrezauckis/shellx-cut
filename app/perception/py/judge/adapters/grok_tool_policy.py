"""Capability-gated, no-tool policy shared by Grok judges and planners.

The judge receives visual evidence as ACP input-image content blocks.  It must
not gain a second information or action channel through local built-ins, shell,
web, or MCP tools.  The policy is deliberately constructed with Grok's
documented allow-then-remove ordering instead of relying on undocumented empty
``--tools`` list semantics.

Evidence basis: Grok Build 1.0.21's shipped CLI reference documents that
``--tools`` disables default built-in injection, ``--disallowed-tools`` runs
after it, bare ``MCPTool`` matches every MCP tool invocation, and deny wins.
The resolver checks the advertised flags before a call can start; version text
is informational only. A missing or opaque capability is an honest not-run result;
the adapter never retries with wider permissions.

This is intentionally narrower than configured-MCP lifecycle management.  It
denies model-originated MCP *tool invocations*.  It does not assert that a
trusted configured MCP server cannot be discovered or started before a model
tries to call it; that is a separate provider/configuration lifecycle concern.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time

_PY_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, _PY_ROOT)
import provider_child_launch

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


JUDGE_REQUIRED_FLAGS = REQUIRED_FLAGS + ("--prompt-file", "--output-format", "--no-memory", "--cwd")
PLANNER_REQUIRED_FLAGS = REQUIRED_FLAGS + ("-p", "--output-format")


def resolve_grok_tool_policy(grok_bin: str, launch: dict | None = None, *,
                             required_flags=REQUIRED_FLAGS, timeout_s=30.0) -> tuple[dict | None, str | None]:
    """Check actual policy/transport flags; version text is informational only.

    Only --version and --help probes run here, never a model session or an
    auth/config read. A failed help contract cannot fall back to broader flags.
    """
    path = launch["executable"] if launch else shutil.which(grok_bin)
    if not path:
        return None, f"grok CLI not found ({grok_bin!r}) — honest not_run"
    deadline = time.monotonic() + max(0.0, timeout_s)
    if timeout_s <= 0:
        return None, "no time remains for Grok capability admission"
    version_text = ""
    try:
        version = subprocess.run(
            provider_child_launch.command(launch, ["--version"]) if launch else [path, "--version"],
            capture_output=True, text=True, encoding="utf-8", timeout=min(15.0, timeout_s / 2),
            env=provider_child_launch.environment(launch) if launch else None)
        version_text = (version.stdout.strip() or version.stderr.strip())[:240]
    except (subprocess.TimeoutExpired, OSError):
        pass
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return None, "Grok capability admission exceeded its time budget"
    try:
        help_result = subprocess.run(
            provider_child_launch.command(launch, ["--help"]) if launch else [path, "--help"],
            capture_output=True, text=True, encoding="utf-8", timeout=min(15.0, remaining),
            env=provider_child_launch.environment(launch) if launch else None)
    except (subprocess.TimeoutExpired, OSError) as exc:
        return None, f"could not verify Grok no-tool policy flags: {exc}"
    help_text = (help_result.stdout or "") + "\n" + (help_result.stderr or "")
    # Match flag tokens, not a longer flag with the required spelling as prefix.
    advertised = set(help_text.replace(",", " ").replace("=", " ").split())
    missing = tuple(flag for flag in required_flags if flag not in advertised)
    if help_result.returncode != 0 or missing:
        detail = ", ".join(missing) if missing else f"exit {help_result.returncode}"
        return None, (
            "Grok CLI does not advertise the complete no-tool policy "
            f"({detail}); refusing a broader invocation")
    return {"path": path, "version": version_text, "launch": launch}, None


def no_tool_arguments() -> list[str]:
    """Fixed shared policy, appended only after capability admission."""
    return [
        "--tools", ",".join(TOOL_ALLOW_THEN_REMOVE),
        "--disallowed-tools", ",".join(TOOL_REMOVALS),
        "--deny", MCP_DENY_RULE,
        "--no-subagents", "--disable-web-search",
        "--sandbox", "read-only", "--permission-mode", "dontAsk",
    ]


def grok_judge_argv(cli: dict, prompt_file: str, cwd: str,
                    model: str) -> list[str]:
    """Build the fixed no-tool headless argv after capability admission."""
    argv = [
        "--prompt-file", prompt_file,
        "--output-format", "json",
        "--no-memory",
    ] + no_tool_arguments() + [
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
        "version": cli["version"],
        "builtins_removed": list(TOOL_REMOVALS[:-1]),
        "mcp_tool_invocations": "denied",
        "mcp_server_startup": MCP_STARTUP_LIMITATION,
        "sandbox": "read-only",
        "permission_mode": "dontAsk",
    }
