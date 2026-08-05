# Security policy

## Supported versions

Security fixes are made for the latest published ShellX Cut release. Preview,
tester, and older builds may be used to reproduce a report, but users should
update to the latest release before relying on a fix.

| Version | Supported |
| --- | --- |
| Latest published release | Yes |
| Older releases and source snapshots | No |

## Report a vulnerability

Please use [GitHub private vulnerability reporting](https://github.com/martinsbrezauckis/shellx-cut/security/advisories/new).
Include the affected version and operating system, the smallest safe
reproduction you can provide, the impact you observed, and whether the issue
requires a malicious project, media file, local process, or agent prompt.

Do not open a public issue containing credentials, private media, local paths,
personal data, exploit code, or an unpatched vulnerability. Do not send real
API keys or account tokens; use clearly synthetic replacements in a
reproduction. Reports are acknowledged and assessed on a best-effort basis.

## Trust boundaries

ShellX Cut is a local-first desktop editor, but local does not mean isolated:

- `cutd` exposes its editing API, WebSocket events, and MCP proxy on loopback.
  It refuses non-loopback binds by default and rejects browser requests with
  non-loopback Host or Origin values. There is intentionally no API token, so
  any process running as the same user can operate the open editor.
- `SHELLX_CUT_ALLOW_NON_LOCAL=1` removes the loopback restriction. Use it only
  behind a trusted authenticating reverse proxy or tunnel. Exposing that port
  directly exposes the full editing surface.
- Projects, media, captions, transcripts, Motion packages, plugin responses,
  and agent messages are untrusted inputs. Keep backups of irreplaceable
  source media and review imported projects and generated edits before export.
- Optional providers, subscription CLI agents, stock-media searches, dubbing,
  and update checks can make network requests. The normal edit and render path
  stays local. See the Network activity section in the README.
- Updater packages are signature-verified on supported release platforms.
  Source builds do not inherit the trust or update guarantees of published
  signed installers.

## Headless agent control

`agent.chat` launches a CLI already installed and authenticated by the user and
connects it to Cut's MCP tools. Attachments are limited to registered project
asset IDs, each turn records its operations, and Cut provides diff,
checkpoint, and guarded-revert review. These controls make edits reviewable;
they do not confine the launched CLI process.

The current headless Codex route requires full filesystem and process access
with non-interactive approvals disabled. The current Grok route uses automatic
tool approval with a Cut MCP allowlist. Claude uses an MCP-only allowed-tools
configuration. CLI behavior and enforcement can change between upstream
versions, and prompts, transcripts, filenames, media metadata, Motion packages,
and other attached content may contain hostile instructions.

Before using unattended agent control:

1. Use a dedicated working copy and an account with only the access needed for
   the edit.
2. Remove unrelated secrets and private files from the working directory and
   environment.
3. Treat project and media content as untrusted, especially when it came from
   another person or an automated generator.
4. Inspect the reported actions and diff, preview the result, and use the
   checkpoint or guarded revert when the turn did more than intended.
5. Prefer an interactive or more restricted agent route when the task does not
   require unattended control.

Do not describe `agent.chat` as a security sandbox. Its safety properties are
operation logging, bounded Cut attachments, review, and revert—not operating
system containment.

## Secrets and diagnostic material

ShellX Cut does not need provider keys for its normal local editing path.
Optional CLI integrations use the user's existing CLI authentication. Keep
credentials in the provider's supported credential store or inject them only
into the process that needs them. Never place secrets in a project, transcript,
prompt, log, screenshot, issue, or release artifact.

Debug API responses, receipts, crash logs, screenshots, and project archives
can contain filenames, edit history, transcript text, and media-derived facts.
Review and redact them before sharing.
