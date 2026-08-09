# ShellX Cut Feature Change Workflow

Use this checklist when adding a verb, editor control, model integration, setup
card, or visible behavior. The goal is to keep the application, its public API,
and its documentation consistent for contributors and users.

## 1. Define the contract

- Update `schema/verbs.json` when the behavior is callable through the Debug API,
  MCP, an agent, or a typed UI action.
- Keep argument names, defaults, bounds, result shapes, mutation behavior, and
  project-state requirements accurate.
- Run `node scripts/generate-verb-contract.mjs --check` after schema changes.
- If the feature is intentionally UI-only, document that choice in the change
  description instead of inventing an unused verb.

## 2. Implement the behavior

- Put the implementation in the smallest existing module that owns the domain.
- Keep new source modules at or below 350 lines and focused test modules at or
  below 600 lines. Prefer extraction over extending a large routing file.
- Return actionable errors. Long-running work must expose progress and a durable
  final result rather than returning success before the work finishes.
- Preserve project replay and idempotency rules for every persisted mutation.

## 3. Connect the human interface

- Add a discoverable UI path unless the feature is intentionally agent-only.
- Reuse nearby components, spacing, copy style, and keyboard conventions.
- Add stable `data-cut-*` selectors to controls that automated tests or assistive
  tooling must locate.
- Cover empty, disabled, loading, success, failure, and degraded states with
  concise user-facing copy.
- Update `ui.open` and `ui.state` when the change adds or renames a panel, tab,
  modal, drawer, or workspace.

## 4. Keep setup understandable

Tool, model, and service cards should show:

1. the user outcome;
2. whether the capability is ready, optional, missing, or degraded;
3. one primary next action; and
4. a short requirement note.

Put package names, model identifiers, endpoints, ports, and filesystem paths in
Advanced details rather than the default view.

## 5. Update agent and user documentation

- Update `skill/shellx-cut/SKILL.md` for workflow changes.
- Update `skill/shellx-cut/reference.md` for verbs, arguments, and return shapes.
- Update `README.md`, `docs/public/FEATURES.md`, and the user manual when visible
  behavior changes.
- When adding or removing a bundled document, update the Tauri resource map.
- Keep examples reproducible from a fresh checkout and avoid machine-specific
  paths or historical run results.

## 6. Add focused verification

- Add unit or integration tests beside the module that owns the behavior.
- Add component or browser coverage for user-visible flows.
- Run `scripts/schema-validation-parity.mjs` when validation changes so direct,
  REST, CLI, and MCP errors remain aligned.
- Run `scripts/verbargs-sync.sh` when verbs or typed UI arguments change.
- Run `npm --prefix ui run test:lib` and `npm --prefix ui run build` for UI work.
- Run the relevant Cargo tests and Clippy checks for changed Rust crates.

## 7. Check packaging impact

- Confirm generated sidecars and bundled resources still resolve from a clean
  checkout.
- Keep source builds independent of local credentials. Public smoke builds are
  unsigned; distributable signatures are applied by the release operator.
- Run the repository checks in `docs/public/BUILDING.md` before submitting a
  release-affecting change.

## Change summary

A useful change description answers:

```text
Feature:
User outcome:
Contract changed:
Implementation changed:
UI changed:
Agent documentation changed:
User documentation changed:
Tests run:
Packaging impact:
```

If a line is not applicable, state why.
