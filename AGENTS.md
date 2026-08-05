# AGENTS.md - ShellX Cut project rules

Scope: applies to the whole ShellX Cut repository.

This file is the complete repository-specific guide for coding agents and
contributors. If your environment supplies additional organization or account
rules, follow both sets and use the stricter rule when they differ.

## Feature Change Workflow

Every new feature, verb, UI surface, model/tool card, debug primitive, or
release-facing behavior must follow:

- `docs/public/FEATURE_CHANGE_WORKFLOW.md`

Do not rely on chat history or model memory to know where a feature needs to be
registered. If a feature is added, moved, renamed, hidden, parked, or made
agent-only, update the workflow surfaces in the same change set or leave an
explicit tracked blocker.

Minimum required surfaces for feature work:

- Contract: `schema/verbs.json` when a public/debug/agent verb changes.
- Engine: server dispatch, domain implementation, receipts/jobs/events as
  applicable.
- Code placement: choose the owning module up front; avoid growing already-large
  dispatch, app-shell, client, or panel index files with substantial new logic.
- UI: visible human surface or an explicit "agent-only" classification.
- Debug: `ui.open`, `ui.state`, `ui.screenshot`, selectors, CDP harness, or
  another reliable inspection path.
- Agent skill: `skill/shellx-cut/SKILL.md` and
  `skill/shellx-cut/reference.md` when agent behavior changes.
- Public docs: README/feature docs that users or agents are expected to trust.
- Tests: unit/integration/UI/full-coverage harness entries appropriate to the
  change.
- Packaging: update the desktop resource map and public-boundary tests when the
  shipped source or installed documentation changes.

## UI and UX Rules

ShellX Cut is an operational video editor, not an engine inventory screen.
Build UI for a non-specialist editor first, with advanced technical details
available only when they help troubleshooting.

Keep UI elements consistent with nearby surfaces. Reuse the local panel,
drawer, card, row, chip, segmented-control, tooltip, icon, and status patterns
instead of inventing a new style for each feature.

For installable tools, optional services, models, and Environment cards:

- Use the same compact card grammar everywhere.
- Lead with the user outcome, not implementation names.
- Show one clear status and one primary action.
- Keep the default copy short enough to scan.
- Put paths, ports, endpoints, model IDs, Python packages, venv details, and
  provider internals behind an Advanced/details affordance.
- Explain what the user gains by installing or selecting the tool.
- Do not include long technical explanations unless they change a user's
  decision.

Examples:

- Prefer "Captions and transcription" over "Perception (Python)".
- Prefer "Install captions" over "Set up perception".
- Prefer "Faster exports" over "Hardware acceleration".
- Prefer "Keep a copy in Library" over "portable".

## Debuggability Rule

If a human can open or operate a visible feature, an agent must have a reliable
debug path to inspect or drive it. For UI work, that normally means stable
`data-cut-*` selectors plus a working `ui.open`/`ui.state` route or CDP test
coverage.

Returning `ok: true` from a debug verb without visibly doing the requested
thing is a bug.
