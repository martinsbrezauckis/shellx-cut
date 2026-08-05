# Screen-demo polish — cutting recordings of software

Craft skill for screen recordings with narration (product demos, tutorials, bug
repros). The defining hazard: **in a demo, silence is often the action** —
typing, a page loading, a build running. Naive silence removal destroys demos.
Verb syntax: `reference.md`.

## Quick reference

```
SILENCE ≠ DEAD AIR:  cross-check every silent span against perception SCENE facts first
PRESET:              natural, min_ms ≥ 2000 — only true dead air goes
NEVER CUT:           mid-action (click → result must stay contiguous on screen)
EDGES:               trim dead head/tail hard — demos start at the first meaningful frame
CAPTIONS:            keep clear of the UI focus area; verify with render.frame, not hope
FROZEN FRAMES:       often legitimate (static UI) — read check evidence before "fixing"
SPOT-CHECK EYES:     render.frame{at_ms} at every cut boundary near on-screen action
```

## Workflow

### 1. Recon — build the silence×scene map
After `media.import` + perception completes, read BOTH facts from
`perception.json`:
- `silence` spans (VAD), and
- `scenes` (visual change events).
Classify every silent span ≥ 2s:
| Silent span has… | Meaning | Action |
|---|---|---|
| no scene events inside | true dead air (narrator paused, screen idle) | safe to cut |
| scene events inside | **silent action** — something happened on screen | KEEP, or trim only the idle tail |
This map is the whole game for demos. Checkpoint `raw` first, as always.

### 2. Edge trim
Demos almost always start with desktop fumbling and end with a hanging "so…
yeah". `edit.ripple_delete{range_ms}` the head up to the first meaningful frame
(verify with `render.frame{at_ms:0}` after) and the tail after the last spoken
word + ~1s hold on the final state.

### 3. Narration cleanup
Same recipe as `craft/talking-head-cleanup.md` passes 1–2 (takes, fillers) with
one demo-specific rule: **a narration cut must not break screen continuity**.
`transcript.cut_words` ripple-cuts video too — so before each cut, check the
scene map for visual events inside the cut range. If the speaker flubbed a line
WHILE clicking through a flow, prefer keeping the flub over teleporting the UI
(or flag for a re-record — that's a human escalation, not an edit).

### 4. Silence pass — conservative
`transcript.remove_silences{aggressiveness:"natural", min_ms:2000}` and then
review every emitted op against the silence×scene map from step 1. Restore any
op whose span contained scene events. Long unavoidable waits (build running,
page loading) you can shorten — cut the MIDDLE of the wait, keeping the start
(action initiated) and end (result appears) so causality stays on screen.
(For a constant speed change use `edit.speed`; use trims for pauses that should disappear.)

### 5. Captions placement
Demo UIs often have content at the bottom (terminals, status bars, docks).
After `captions.generate`, take `render.frame` at 3–4 caption-visible moments
spread across the timeline and LOOK: does the caption cover the UI element being
demonstrated? If yes, `captions.set_style` the style ref to `pos:"top"` (or a
smaller size) and re-check the same frames.

### 6. Render + verify
`render.preview` across the densest cut region and one silent-action region you
chose to keep, then `render.final`.

## Receipts to check

- **black_or_frozen_frames**: read the EVIDENCE, don't auto-remediate. Frozen
  spans at timestamps where the scene map shows static UI = expected, note and
  accept. Frozen spans you can't explain = a gap or bad trim — fix per
  `craft/fix-failed-checks.md`.
- **cut_on_word**: must PASS.
- **silence_at_edges**: must PASS — the edge-trim step makes this green.
- **duration_matches_edl**: must PASS.
- **Frame receipts**: keep the `render.frame` captures from the caption check
  (they are your visual evidence the captions don't cover the demo).
- **Op-log**: every kept-silent-action decision visible as a restore op with
  rationale "silent action: scene events at …" — the human can audit your
  silence×scene judgment without rewatching.
