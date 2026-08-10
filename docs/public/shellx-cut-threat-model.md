# ShellX Cut local-machine trust threat model

## Executive summary

This is the v0.6.107 deployment contract for the local Debug API, WebSocket
endpoint, and `cutd mcp` proxy. ShellX Cut supports **one personal workstation /
one trusted interactive environment**. Its unauthenticated loopback listener is
a whole-machine trust boundary, not same-user or per-process isolation. Under
that explicit deployment assumption, no per-caller capability token is present
and that is **NOT A DEFECT**.

This report is grounded in `app/server/src/http.rs` (default loopback bind,
non-loopback refusal, optional Origin/Host guard, and no-Origin HTTP test),
`app/server/src/mcp.rs`/`httpc.rs` (the stdio proxy reaches the running engine),
and `app/server/src/chat.rs` (separate contained-Claude capability broker).

## Scope and security decision

| Decision | Contract |
| --- | --- |
| Supported deployment | One personal workstation / one trusted interactive environment. |
| Local authorization | None: any local process or OS account able to connect to the loopback port can use the open editor. |
| Browser mitigation | Reject a non-loopback `Origin` or `Host`; this mitigates browser cross-origin and DNS-rebinding requests only. |
| Native callers | Can omit `Origin` and forge `Origin`/`Host`; headers are not authentication. |
| Remote listening | Native LAN/public binding is unsupported and refused by default. `SHELLX_CUT_ALLOW_NON_LOCAL=1` changes only the bind check and provides no Cut authentication. |
| Future hardening | A native remote mode must introduce and verify per-caller/per-user capability authentication. It is not implemented in v0.6.107. |

Remote use is supported only through an SSH/VPN/external ShellX broker or
equivalent transport that independently authenticates and authorizes the
caller. That protection is outside Cut and must be separately evidenced;
without it, remote access must be refused. Cut does not claim that an external
transport exists, supplies a remote token, or verifies an external transport's
authentication.

## System model and assets

```text
trusted local user/processes ─┐
local MCP client ─ cutd mcp ──┼──> cutd loopback REST/WS ──> open project, media, jobs
desktop UI ───────────────────┘
browser from another origin ─────> Origin/Host guard (rejected)
```

| Asset | Why it matters |
| --- | --- |
| Open project, timeline, edit history, and exports | Integrity and availability of the user's work. |
| Registered media, captions, transcripts, and receipts | Private content and project-derived metadata. |
| Local API/MCP control of the open editor | A caller can read state and invoke the existing verb surface. |
| Agent Chat turn, attachments, diff, checkpoint, and revert data | Bounded editing review data; hostile content can influence an agent prompt. |

## Boundaries, attackers, and entry points

| Boundary / entry point | Attacker capability considered | Enforced mitigation | Residual risk |
| --- | --- | --- | --- |
| Off-machine browser reaching loopback through DNS rebinding | Browser supplies a hostile Origin/Host. | `guard_local_origin` rejects non-loopback headers. | A native local client is not a browser and is not authenticated by this guard. |
| Default `127.0.0.1`/`::1` listener | Local process/account connects directly with no Origin or forged headers. | Accepted by design only inside the whole-machine trusted deployment. | On a shared or compromised machine it can operate the editor. |
| `cutd mcp` stdio proxy | A configured local MCP client invokes generated tools. | Proxy reaches the same running engine; it adds no caller authentication. | It inherits the machine-wide API trust boundary. |
| Claude `agent.chat` | Hostile prompt/attachment attempts native or unrelated Cut actions. | Pinned CLI, native-tool denial, and Cut capability filtering limit that provider route. | Not an OS sandbox and does not protect the unauthenticated REST/MCP surface. |
| Codex `agent.chat` | A selected local Codex turn uses its configured native tools and integrations. | Cut adds only its filtered MCP surface and records every resulting Cut verb for review/revert. | Codex retains the user's native sandbox, permissions, rules, and configured tools; select it only when that local CLI is trusted. |
| `SHELLX_CUT_ALLOW_NON_LOCAL=1` | LAN/public client connects or forges headers. | Default refuses this bind; the opt-in is unsupported as a Cut remote mode. | Cut adds no remote auth; direct exposure grants the full surface. |

## Abuse paths and mitigations

1. A malicious web page attempts a loopback request through a DNS-rebound host.
   The browser's non-loopback Origin/Host is rejected. This does not prove a
   local native process is authorized.
2. An untrusted local app, service, second account, or host-network container
   connects to the default port and edits/reads the open project. This is outside
   the supported one-workstation trusted-environment assumption; no same-user
   barrier exists.
3. An operator enables `SHELLX_CUT_ALLOW_NON_LOCAL=1` and exposes the port.
   Cut has no remote token or capability check, so an external client receives
   the full surface. An independently authenticated SSH/VPN/broker/proxy may be
   a separate deployment, but Cut makes no assurance about it.
4. Hostile project content tries to steer `agent.chat` through native tools or
   broader Cut MCP verbs. The contained Claude route restricts both layers.
   Codex intentionally keeps its normal user-configured native capabilities.
   Both routes filter Cut's MCP verbs and record review/revert material, but
   neither can turn local TCP into authenticated per-user access.

## Residual risk and operator guidance

Do not use the default unauthenticated loopback mode on shared/multi-user
machines, with untrusted local apps/services, in containers that share host
networking, or with an exposed port. Keep the listener loopback-only. Treat
Agent Chat's provider posture as distinct from machine trust, inspect its edits,
and use its checkpoint/revert controls as recovery tools.

For a future supported remote or multi-principal deployment, Cut must add a
native, verified per-caller/per-user capability authentication mechanism and
test it at the server boundary. That hardening is intentionally not silently
claimed or partially implemented in v0.6.107.

## Verification hooks

Platform-specific verification exercises pin these statements to `http.rs`,
while focused HTTP tests prove non-loopback bind refusal,
no-Origin local success, loopback-Origin success, and cross-origin rejection.
No verb, schema entry, or MCP tool changes in this slice: schema-generated MCP
tools remain the existing control surface, and the MCP proxy inherits the same
machine-wide trust contract.
