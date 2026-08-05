# ShellX Cut Feature Change Workflow

Purpose: make feature additions repeatable and auditable without relying on
model memory. A ShellX Cut feature is not done until its contract, UI/debug
surface, docs, skill surface, and tests are deliberately updated or explicitly
classified as not applicable.

The synchronized feature surfaces are `schema/verbs.json`,
`app/server/src/registry.rs`, `skill/shellx-cut/reference.md`,
`skill/shellx-cut/SKILL.md`, `README.md`, `scripts/coverage-audit.sh`,
`scripts/schema-validation-parity.mjs`, `scripts/verbargs-sync.sh`, and
`ui/public-tests/full-coverage-verify.mjs`.
A feature is Debug-API covered only when its public verb and observable UI state
are both represented by the applicable debug and coverage gates.

## Feature Definition Of Done

Use this checklist for every new feature, verb, model, tool integration, UI
drawer, setup card, or behavior rename.

1. Contract
   - Update `schema/verbs.json` for any public, debug, agent, or UI-driven verb.
   - Keep names, args, defaults, bounds, and result shape current.
   - If no verb is added, write down why the feature is UI-only or internal-only.

2. Engine
   - Wire dispatch and domain implementation.
   - Add receipts, jobs, events, progress messages, and persistence when the
     feature changes user-visible project state or async work.
   - Error messages must be actionable and human-readable.

3. Code Placement and Ownership
   - Put new code in the smallest existing domain module that owns the behavior.
   - Do not add substantial feature logic to monolithic shell files such as
     dispatch, app shell, or panel index files unless the change is only routing.
   - If a file is already large or risky to edit, either extract the feature into
     a named module in the same change or record a tracked blocker explaining why
     extraction is deferred.
   - Keep the public contract stable while splitting: schema, debug verbs,
     receipts, selectors, and harness expectations should not change unless the
     feature itself changes.

4. Human UI
   - Add a visible UI path unless the feature is intentionally agent-only.
   - Use existing ShellX Cut components and nearby layout patterns.
   - Add stable `data-cut-*` selectors for every control an agent or harness
     needs to inspect.
   - Empty, disabled, loading, success, failure, and degraded states must have
     concise copy.

5. Environment and Installables
   - Tool/model/service cards must use a common compact structure:
     `name`, `user outcome`, `status`, `primary action`, `small requirement note`,
     `Advanced details`.
   - Hide implementation details by default: raw model IDs, package imports,
     venv paths, endpoints, ports, local filesystem paths, and secrets.
   - If multiple models are available, present them as comparable user choices:
     recommended use, quality/speed tradeoff, download/runtime cost, language or
     hardware requirement.

6. Debug Surface
   - Update `ui.open` for any visible panel, drawer, tab, modal, or tool mode.
   - Update `ui.state` so agents can confirm what is open and selected.
   - Ensure `ui.screenshot` or a CDP harness can capture the surface.
   - If a real OS screenshot/debug primitive exists, keep it working on the
     supported platform.
   - A debug command that returns success but does not change observable UI state
     must be treated as a failing bug.

7. Agent Skill
   - Update `skill/shellx-cut/SKILL.md` for workflow-level changes.
   - Update `skill/shellx-cut/reference.md` for verbs, args, return shapes, and
     examples.
   - When adding or removing a skill/craft document, update the canonical
     installed-doc manifest in `scripts/lib/agent-docs.mjs` and the desktop
     bundle resource map. The contract test requires the manifest to cover the
     complete skill directory.
   - Do not leave old verb counts, stale model names, or stale setup guidance.
   - If reference content can be generated from `schema/verbs.json`, prefer
     generation over hand editing.

8. Public Docs
   - Update README and public feature docs when users, agents, or release notes
     would otherwise see stale behavior.
   - Keep internal notes, research, receipts, machine paths, and signing/debug
     details out of `docs/public/` and every shipped source surface.
   - Do not add a new dated planning document for routine feature work; update
     the current register or public workflow instead.

9. Tests and Harnesses
   - Add or update unit/integration tests for engine behavior.
   - Update `scripts/verbargs-sync.sh` expectations through typed UI args when
     verbs change.
   - Update `scripts/coverage-audit.sh` and coverage expectations when the verb
     count or public contract changes.
   - Run `scripts/schema-validation-parity.mjs` when input schemas or shared
     dispatch validation change; REST, CLI, and MCP errors must stay identical.
   - Update `ui/public-tests/full-coverage-verify.mjs` classification for every verb:
     human UI, agent-only intentional, internal helper, rig-only, or no-UI by
     design.
   - Add CDP or Playwright coverage for user-visible UI flows.
   - Update `scripts/release/full-coverage-gate.mjs` only when gate behavior or
     rig preconditions change.

10. Packaging and release checks
   - If the public boundary changes, update the resource map, public-source
     inventory, and relevant surface tests.
   - Local receipts, signing logs, internal plans, and machine-specific paths
     must remain outside the publishable source snapshot.
   - Native build/install qualification must verify every canonical agent doc is
     bundled and byte-identical to the candidate source, then fetch it through
     `/api/agent-doc/*path` from the installed engine.
   - Run the repository release checks before claiming release readiness.

## Surface Matrix

Classify every feature into one state.

| State | Meaning | Required proof |
| --- | --- | --- |
| Human UI | Normal users can discover and use it in the app. | Screenshot/CDP test plus UI selectors. |
| Agent-only intentional | No normal UI by design. | Skill/reference docs plus debug/API test. |
| Internal helper | Only supports another user-visible feature. | Code tests plus parent feature link. |
| Rig-only | Needs a specific platform/device/service. | Platform harness receipt. |
| Parked | Designed but not active. | Tracked blocker and no misleading UI/docs. |

No feature may remain in "wired but hidden" state without one of these
classifications.

## UI Copy Standard

Use copy that answers:

- What does this do for my edit?
- Is it ready, missing, optional, or degraded?
- What is the one next action?
- What changes after I install or enable it?

Avoid default visible copy that only names implementation details:

- Python package names
- venv/sidecar language
- model IDs
- local filesystem paths
- ports/endpoints/secrets
- raw provider names unless the user is choosing that provider

Those details belong under Advanced, diagnostics, or logs.

## Pull Request Or Change Summary Template

Every feature change summary should include:

```text
Feature:
User outcome:
Classification: Human UI | Agent-only intentional | Internal helper | Rig-only | Parked
Contract updated:
Engine updated:
Code placement:
UI updated:
Debug updated:
Skill/reference updated:
Docs updated:
Tests/harness updated:
Packaging/release impact:
Known blockers:
```

If any line is "not applicable", say why.
