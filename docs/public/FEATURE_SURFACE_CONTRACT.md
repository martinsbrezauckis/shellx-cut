# ShellX Cut Feature Surface Contract

This public reference describes how supported capabilities are represented to
people and agents. It covers observable product surfaces, not internal
implementation, change-control, packaging, or release procedures.

## Supported exposure taxonomy

Each verb's `ui_exposure` value uses one of these supported categories:

| Exposure | Public meaning | Observable expectation |
| --- | --- | --- |
| `human` | A person can use the capability in the editor. | It has a discoverable UI path, stable `data-cut-*` selectors where automation or assistive tooling needs them, and a matching `ui.open` or `ui.state` inspection route when applicable. |
| `agent_only` | The capability is intentionally callable by an agent without a human editor control. | Its `schema/verbs.json` contract, `skill/shellx-cut/SKILL.md`, and `skill/shellx-cut/reference.md` describe the supported behavior and result. |
| `internal` | The capability is an implementation detail, not an advertised human or agent feature. | Public references do not present it as available functionality. |
| `rig_only` | The capability exists only for a deterministic verification rig or fixture. | Public references do not represent it as product availability. |

## Observable product expectations

- A callable supported capability has a machine-readable `schema/verbs.json`
  contract and is observable through the documented Debug API or MCP surface.
- A visible human capability has a clear editor route and exposes truthful
  state through `ui.open` and `ui.state` when those surfaces apply. Use
  `ui.screenshot` for visual inspection when a screenshot is the relevant
  observable result.
- Optional capabilities and installables communicate the user outcome,
  readiness/degraded state, one primary next action, and concise requirements;
  implementation details belong in Advanced details.
- A desktop-only UI affordance may be unavailable in a browser. Its visible
  refusal explains that boundary, and any native request remains scoped to the
  registered product identity rather than a caller-supplied filesystem path.
- User-facing behavior is described consistently in the public `README.md`,
  `docs/public/FEATURES.md`, and user manual when those surfaces cover it.
- Agent-facing behavior is available through the installed skill/reference and
  the bundled agent-doc index exposed by `GET /api/agent`.

The live verb registry remains the source of truth for callable behavior:
`GET /api/verbs` from a running Cut server, or `schema/verbs.json` from the
matching source or installed agent-doc bundle.
