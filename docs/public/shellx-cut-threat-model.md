# ShellX Cut local-machine trust threat model

<!-- shellx-cut-release-truth: published; version=0.6.117; published=0.6.117 -->

## Executive summary

This v0.6.117 published contract covers the current source and the local
Debug API, WebSocket endpoint, and `cutd mcp` proxy. ShellX Cut supports **one personal workstation /
one trusted interactive environment**. Its unauthenticated loopback listener is
a whole-machine trust boundary, not same-user or per-process isolation. Under
that explicit deployment assumption, no per-caller capability token is present
and that is **NOT A DEFECT**.

This report is grounded in `app/server/src/http.rs` (default loopback bind,
non-loopback refusal, Origin/Host/Fetch-Metadata browser guard, and no-Origin HTTP test),
`app/server/src/mcp.rs`/`httpc.rs` (the stdio proxy reaches the running engine),
and `app/server/src/chat.rs` (separate contained-Claude capability broker).

## Scope and security decision

| Decision | Contract |
| --- | --- |
| Supported deployment | One personal workstation / one trusted interactive environment. |
| Local authorization | None: any local process or OS account able to connect to the loopback port can use the open editor. |
| Browser mitigation | Reject a non-loopback `Origin` or `Host`, plus a no-Origin `Sec-Fetch-Site: cross-site` browser request; this mitigates browser cross-origin and DNS-rebinding requests only. |
| Native callers | Can omit `Origin` and forge `Origin`/`Host`; headers are not authentication. |
| Remote listening | Native LAN/public binding is unsupported and refused by default. A debug build with `SHELLX_CUT_ALLOW_NON_LOCAL=1` permits that bind and skips the browser Origin/Host/Fetch-Metadata guard; packaged builds ignore the flag. Cut provides no remote authentication. |
| Future hardening | A native remote mode must introduce and verify per-caller/per-user capability authentication. It is not implemented in the current source. |

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
browser from another origin ─────> Origin/Host/Fetch-Metadata guard (rejected)
```

| Asset | Why it matters |
| --- | --- |
| Open project, timeline, edit history, and exports | Integrity and availability of the user's work. |
| Registered media, camera takes, captions, transcripts, and receipts | Private content and project-derived metadata. |
| Local API/MCP control of the open editor | A caller can read state and invoke the existing verb surface. |
| Agent Chat turn, attachments, diff, checkpoint, and revert data | Bounded editing review data; hostile content can influence an agent prompt. |

## Boundaries, attackers, and entry points

| Boundary / entry point | Attacker capability considered | Enforced mitigation | Residual risk |
| --- | --- | --- | --- |
| Off-machine browser reaching loopback through DNS rebinding or no-CORS subresource load | Browser supplies a hostile Origin/Host or no-Origin `Sec-Fetch-Site: cross-site`. | `guard_local_origin` rejects those browser signals before routing. | A native local client is not a browser and is not authenticated by this guard. |
| Default `127.0.0.1`/`::1` listener | Local process/account connects directly with no Origin or forged headers. | Accepted by design only inside the whole-machine trusted deployment. | On a shared or compromised machine it can operate the editor. |
| `cutd mcp` stdio proxy | A configured local MCP client invokes generated tools. | Before it forwards a verb, the proxy asks the peer at the `engine.addr` address who it is (`GET /api/agent`: product, exact version, engine instance) and sends the verb only on that verified connection; it adds no caller authentication. | It inherits the machine-wide API trust boundary. |
| Claude `agent.chat` | Hostile prompt/attachment attempts native or unrelated Cut actions. | Contained capability contract, native-tool denial, and Cut capability filtering limit that provider route. | Not an OS sandbox and does not protect the unauthenticated REST/MCP surface. |
| Codex `agent.chat` | A selected local Codex turn uses its configured native tools and integrations. | Cut adds only its filtered MCP surface and records every resulting Cut verb for review/revert. | Codex retains the user's native sandbox, permissions, rules, and configured tools; select it only when that local CLI is trusted. |
| Debug build with `SHELLX_CUT_ALLOW_NON_LOCAL=1` | LAN/public client connects or forges headers. | Default and packaged builds refuse this bind; debug opt-in also skips the browser guard and is unsupported as a Cut remote mode. | Cut adds no remote auth; direct exposure grants the full surface. |
| Tool downloads the person asks for (`system.fetch_tool`, Install captions) | A tampered or swapped download. | FFmpeg archives are pinned by exact URL, size and SHA-256 (and on macOS the publisher's Apple Developer ID); Microsoft's Visual C++ installer is fetched from Microsoft and run only after Windows' signature check accepts it with a chain to a Microsoft root and Microsoft Corporation as signer; nothing runs before those checks. | A download depends on its publisher keeping the file; a removed FFmpeg build is refused until a Cut update re-pins it. |
| Recording camera selection | A caller tries to enumerate native identities, trigger permission unexpectedly, or substitute another device. | Idle Doctor returns opaque expiring choices and opens no stream. Only explicit Start uses a selected choice, revalidates it, and requires a real first frame before retaining a separate local take. | Camera media is sensitive local project content; a trusted local caller that can invoke Start can request it and OS permission remains the final platform boundary. |

## Abuse paths and mitigations

1. A malicious web page attempts a loopback request through a DNS-rebound host
   or a no-CORS image load. The browser's non-loopback Origin/Host or explicit
   cross-site Fetch Metadata is rejected. This does not prove a
   local native process is authorized.
2. An untrusted local app, service, second account, or host-network container
   connects to the default port and edits/reads the open project. This is outside
   the supported one-workstation trusted-environment assumption; no same-user
   barrier exists.
3. An operator enables `SHELLX_CUT_ALLOW_NON_LOCAL=1` in a debug build and exposes the port.
   Cut has no remote token or capability check, so an external client receives
   the full surface. An independently authenticated SSH/VPN/broker/proxy may be
   a separate deployment, but Cut makes no assurance about it.
4. Hostile project content tries to steer `agent.chat` through native tools or
   broader Cut MCP verbs. Every brokered route forwards an attributed restricted
   MCP marker, so the server filters Cut verbs at both discovery and invocation.
   Claude and Grok also restrict native tools; Codex keeps its normal
   user-configured native capabilities. Antigravity retains its native toolset
   but runs in a new empty sandboxed project with only Cut's filtered MCP plugin
   added; unattended approval is bounded to that disposable turn. None of these routes
   turns local TCP into authenticated per-user access.

## Projects made by someone else (accepted risk)

Opening a project made by someone else is like opening any other file from an
untrusted source. A project file names the media it uses, and that media can be
anywhere the person's account can read: Cut does not ask before it uses media
outside the project folder, and a render includes it. A received project could
therefore pull a file of the person into an export they then share. On Windows,
a media path that names a network share (`\\host\share\...`) makes Windows
connect to that host when Cut reads it, which can send the person's Windows
sign-in credentials to that host. Cut does not confirm outside-project or
network media paths. This is a deliberate choice for a local tool that a person
uses with their own projects; it will be revisited if Cut gains project
sharing, templates from others, or agents exchanging projects. Open projects
only from people you trust.

What Cut does enforce for any project, trusted or not: renders, exports and
other outputs are refused, with a reason, when their target is outside every
authorized export folder (the project's `exports/`, or an output folder the
person chose); project files Cut reads or writes are opened as
ordinary files without following links; text from a project (folder names,
caption text, style and font names, asset paths) is escaped or refused before
it reaches FFmpeg, so it cannot add its own lines or options; media paths handed
to ffprobe that contain `%`, `*`, `?` or `[` are passed with `-pattern_type none`;
before FFmpeg starts an automatically selected input with such a path, a bounded
check of the normalized literal path must identify an allowed format, and
`image2` is told to read only that file. A failed or unclear check refuses the
operation with a reason and next step, and FFmpeg receives the same normalized
path that was checked. Declared input formats and Cut-owned storyboard sequences
and concat lists keep their explicit modes. A stored relative asset path is
resolved inside the project. Files Cut derives from a
project (proxies, mask and matte alphas, stabilization data,
analysis reports, segment windows, nested bakes, polished recording clips) are
reused only when this engine recorded their content hash outside the project;
a derived file that came with a received project is made again, and a proxy
pointer outside the project's own `proxies/` folder is dropped. The same holds
for a recording's own folder: its checkpoints, prepared constant-rate parts and
saved-take notes skip their checks only when this engine wrote them there; the
same files in a received recording folder are decoded and checked in full, and
a part record or journal that claims more frames than the recording plan is
refused. A project journal or other project
file that has more than one name (a hard link) is not opened for writing.
The programs Cut starts (FFmpeg, ffprobe, the archive tool and the
decompressor it uses, Python, the reviewer and agent command-line tools) are
found only in absolute folders and started by their full path. A relative or
empty `PATH` entry and a relative tool setting are never used, so a program in
the folder an engine was started from (for example a received project) never
runs. When no program is found in an absolute folder, Cut says the tool is not
installed.

Which output checks apply to an export (no sound, only still pictures) is
decided on this engine's own probe of the placed sources at render time, never
on media facts stored in the project, and a check the person asked for by name
(a loudness target, a footage profile) is never waived; a re-check relies on a
receipt's decisions only when this engine wrote that receipt.

A program running under the person's own account while Cut works (for example
one that changes a file between Cut's check and its use, or inside a review's
run folder) is outside this model: it already has every right Cut has.

An export is written to a private staging file in the export folder and moved
into place when it is finished. On Linux and macOS a chosen export folder that
everyone on the computer can write to is refused unless its sticky bit is set;
a group-writable folder is accepted. Cut records the staging file's identity
and refuses to publish it if it was replaced while the render ran. Residual
risk: another member of a group-writable export folder could redirect that
single write before FFmpeg opens the staging file. Two more limits apply on a
computer shared with another user account: a user who may rename a parent
folder of the export folder can redirect an export, and on Windows a user with
rights in the export folder can replace the finished staging file before it is
published (the replacement check above is made on Linux and macOS only). Use an
export folder that only you can change, parents included. Writing exports in a
private folder first is planned.

A transcript and visual-search data that come inside a project made by someone
else are used as they came: Cut does not check that they match the footage, so
captions made from such a transcript and search results can show what the
sender wrote. Transcribe the clips again when that matters.

## Residual risk and operator guidance

Do not use the default unauthenticated loopback mode on shared/multi-user
machines, with untrusted local apps/services, in containers that share host
networking, or with an exposed port. Keep the listener loopback-only. Treat
Agent Chat's provider posture as distinct from machine trust, inspect its edits,
and use its checkpoint/revert controls as recovery tools.

For a future supported remote or multi-principal deployment, Cut must add a
native, verified per-caller/per-user capability authentication mechanism and
test it at the server boundary. That hardening is intentionally not silently
claimed or partially implemented in the current source.

## Verification hooks

Platform-specific verification exercises pin these statements to `http.rs`,
while focused HTTP tests prove non-loopback bind refusal,
no-Origin local success, loopback-Origin success, and both cross-origin and
no-Origin cross-site Fetch-Metadata rejection before route work.
No verb, schema entry, or MCP tool changes in this slice: schema-generated MCP
tools remain the existing control surface, and the MCP proxy inherits the same
machine-wide trust contract.
