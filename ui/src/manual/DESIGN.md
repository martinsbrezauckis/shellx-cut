# ShellX Cut real-frontend manual

## User job

Find a Cut feature, understand its purpose and prerequisites, then reveal the
real control without the documentation performing the edit for the user.

## Structural fingerprint

- Left rail: manual identity, search, selected explanation, requirements, API
  reference, and the indexed feature list.
- Main plane: the exact compiled Cut React editor. There is no reconstructed UI,
  screenshot map, or synthetic menu layer.
- Online direction: index selection opens and highlights the embedded editor;
  exploration of recognized editor controls selects the matching explanation.
- Installed direction: index selection sends the same feature id to the running
  editor and reveals the control in the user's current app window.

## Interaction contract

| State | Entry | Available action | Guard and feedback |
| --- | --- | --- | --- |
| Loading | Manual opens | Search and index remain visible | Editor plane reports that the real UI is loading. |
| Ready | Embedded app reports protocol readiness | Select an article or explore the editor | Selection and URL stay synchronized. |
| Exact target | Feature has a registered selector | Open surface, focus, highlight | Result reports `shown`. |
| Surface target | A contextual or multi-step topic has no single always-visible control | Open the truthful owning surface | Result reports `surface-only`; it never invents an exact target. |
| Unavailable | Feature is API-only or has no editor control | Read the explanation | A specific reason replaces a fake target. |
| Read-only refusal | A final action would mutate or call an external system | Continue exploring | The editor returns `manual_read_only`; no mutation is simulated. |

## Visual thesis

The manual should feel like Cut with a precise documentation rail attached: the
working editor dominates, the rail is dense but calm, and the existing Cut
tokens and typography remain the authority. Highlights explain location rather
than decorating the interface.

## Verification thesis

Completion requires browser and installed-app evidence. Type checks and static
contracts are necessary but cannot prove real menu opening, selector placement,
bidirectional selection, focus behavior, or local reveal in the running editor.

## Highest-cost failures

1. A manual route executes or simulates a project mutation.
2. The documentation points at reconstructed or stale UI instead of Cut.
3. Local reveal opens a browser but does not reveal the control in the app.
4. A missing selector silently appears successful.
