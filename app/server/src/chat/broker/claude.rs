//! Required containment capabilities for Claude Code.

use super::missing_required_help_tokens;

pub(crate) const CONTAINED_CLAUDE_CAPABILITY_POSTURE: &str =
    "contained: Claude Code capability contract (required flags verified)";

pub(crate) const REQUIRED_CONTAINED_CLAUDE_HELP_TOKENS: &[&str] = &[
    "--print",
    "--output-format",
    "--mcp-config",
    "--setting-sources",
    "--disable-slash-commands",
    "--allowedTools",
    "--strict-mcp-config",
    "--disallowedTools",
    "--permission-mode",
    "--no-session-persistence",
    "--model",
];

pub(crate) fn verify_capability_contract(help: &str) -> Result<(), String> {
    let missing = missing_required_help_tokens(help, REQUIRED_CONTAINED_CLAUDE_HELP_TOKENS);
    if !missing.is_empty() {
        return Err(format!(
            "Claude Code did not advertise required containment flags: {}. Cut refuses the turn.",
            missing.join(", ")
        ));
    }
    Ok(())
}
