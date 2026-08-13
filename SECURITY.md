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

ShellX Cut's supported default is **one personal workstation / one trusted
interactive environment**. The trust boundary is the **whole local machine**,
not an OS user account or an individual process. This is an intentional
v0.6.109 deployment contract, not an omitted same-user security feature.

- `cutd` exposes its editing API, WebSocket events, and MCP proxy on loopback.
  It refuses non-loopback binds by default and rejects browser requests with
  non-loopback Host or Origin values, plus no-Origin requests whose Fetch
  Metadata explicitly says `Sec-Fetch-Site: cross-site`. There is intentionally no API token:
  any local process or OS account able to connect to the loopback port can
  operate the open editor. Loopback TCP does not carry a caller identity.
- Origin, Host, and browser Fetch Metadata checks mitigate browser cross-origin
  and DNS-rebinding requests. They are not authentication: native local callers
  can omit Origin and can forge headers. Do not treat them as same-user isolation.
- Native LAN/public listening is unsupported and refused by default. The
  `SHELLX_CUT_ALLOW_NON_LOCAL=1` escape changes that bind check only; Cut does
  not add or verify a remote token, capability, or caller identity. Any
  Remote use is supported only through an independently authenticated and
  authorized SSH/VPN/ShellX broker or equivalent transport. That protection
  belongs to the transport and must be separately evidenced; otherwise remote
  access must be refused. Exposing the port directly exposes the full editing
  surface.
- Shared or multi-user machines, untrusted local apps/services, containers
  sharing host networking, and exposed ports are outside this default trust
  boundary. Per-caller or per-user capability authentication is future
hardening, not present in v0.6.109. Under the stated deployment assumption,
  lack of that token is **NOT A DEFECT**.
- See [`docs/public/shellx-cut-threat-model.md`](docs/public/shellx-cut-threat-model.md)
  for assets, abuse paths, mitigations, and residual risk.
- Projects, media, captions, transcripts, Motion packages, plugin responses,
  and agent messages are untrusted inputs. Keep backups of irreplaceable
  source media and review imported projects and generated edits before export.
- Optional providers, subscription CLI agents, stock-media searches, dubbing,
  and update checks (automatic at launch and every 6 hours unless turned off,
  or manual from Settings > About) can make network requests. The normal edit
  and render path stays local. See the Network activity section in the README.
- Updater packages are signature-verified on supported release platforms.
  Source builds do not inherit the trust or update guarantees of published
  signed installers.

## Headless agent control

`agent.chat` launches locally verified Claude Code, Codex, Grok, or Antigravity
routes. Before every turn, Cut probes the resolved CLI's `--version` and
`--help` and refuses it when that provider's required launch contract is absent.
Claude remains pinned to `2.1.224`; Codex and Grok are flag-verified without a
version pin; Antigravity must advertise Cut's sandbox and non-interactive
launch flags on every platform, including Windows. An upstream CLI must not
silently weaken a selected route's policy.

Each supported turn starts in a new empty disposable directory. Claude and
Grok use provider-specific restricted environments; Codex and Antigravity keep
their normal user configuration, sandbox, permissions, and login routing. Each
route passes an attributed restricted marker only to its spawned `cutd mcp`
child. The server requires both that marker and the `agent:*:agent.chat` actor
before it filters Cut tools at discovery and invocation; a normal
user-configured MCP child has neither and keeps the full machine-local surface.

Claude runs with an explicit MCP-only allowlist plus a deny list for file
read/write/edit, shell/process, web/search, skills, and recursive `agent_chat`,
a strict one-server Cut MCP config, and no session persistence. In the pinned
2.1.224 CLI those native and unrelated tools are absent from the turn's tool
registry rather than producing per-call denial receipts. `--tools ""` and
`--safe-mode` suppress even an explicitly supplied MCP server, so Cut does not
use either incompatible flag; the allowlist and deny list are probed before
every turn instead.

To preserve the user's existing subscription login without reloading the
user's Claude customizations, Cut passes `--setting-sources ""` and
`--disable-slash-commands`. The pinned CLI then loads no user/project/local
settings, hooks, plugins, commands, or skills; a fresh disposable cwd contains
only the generated Cut MCP config. Cut does not use `--bare`: that mode also
suppresses the keychain/OAuth authentication needed for subscription CLI use.
Only `mcp__cutd__*` is allowed, and Cut applies a second capability filter at
both MCP discovery and invocation: the contained turn can inspect its already
open project and make reversible in-project edits only. Project switching,
deletion, navigation/revert, imports, path or local-folder search, provider
fetches, plugins, render/export, installers, process actions, and recursive
`agent.chat` are neither exposed nor callable. Attachments are registered
project asset IDs; each turn records its operations and provides diff,
checkpoint, and guarded-revert review.

Grok uses a disposable config/home with native tools disabled and only the
live Cut MCP child while leaving its existing login file in place. Codex and
Antigravity intentionally retain their provider-native capabilities according
to the user's own configuration, but their Cut MCP child receives the same
server-enforced restricted tool surface as Claude and Grok. Review every edit,
especially for a native-policy provider.

This is a provider capability policy, not an operating-system sandbox. Each
CLI process still runs as the user's account and contacts its provider; Cut
cannot protect the host from a compromised provider binary or another local
process. Its restricted MCP route does not authenticate the underlying
loopback REST/MCP API or narrow the machine-wide local trust boundary. Prompts,
transcripts, filenames, media metadata, Motion packages, and other attached
content may contain hostile instructions.

Before using unattended agent control:

1. Confirm Doctor reports a ready provider route; a selected turn will probe
   its exact launch contract again immediately before it starts.
2. Use a dedicated account with only the access needed for the edit.
3. Treat project and media content as untrusted, especially when it came from
   another person or an automated generator.
4. Inspect the reported actions and diff, preview the result, and use the
   checkpoint or guarded revert when the turn did more than intended.
5. Prefer an interactive workflow when the task does not require unattended
   control.

Do not describe `agent.chat` as an operating-system security sandbox. Its
enforced CLI policy reduces the exposed tool surface; operation logging, bounded
Cut attachments, review, and revert remain separate recovery controls.

## Secrets and diagnostic material

ShellX Cut does not need provider keys for its normal local editing path.
Optional CLI integrations use the user's existing CLI authentication. Keep
credentials in the provider's supported credential store or inject them only
into the process that needs them. Never place secrets in a project, transcript,
prompt, log, screenshot, issue, or release artifact.

Debug API responses, receipts, crash logs, screenshots, and project archives
can contain filenames, edit history, transcript text, and media-derived facts.
Review and redact them before sharing.
