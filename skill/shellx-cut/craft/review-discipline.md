# Review discipline — how the agent self-reviews an edit

ShellX Cut keeps edits verifiable:
every mutation is an op with rationale, every render ends in a measured
RenderReceipt. This skill is the protocol that turns those primitives into
trustworthy editing. Verb syntax: `reference.md`.

## Quick reference

```
BEFORE RENDER:   project.diff vs last checkpoint — read EVERY op, not the summary alone
RENDER LADDER:   render.frame (spot) → render.preview (region) → render.final (receipt)
AFTER RENDER:    read the FULL RenderReceipt — every check, its evidence, the measured numbers
DONE MEANS:      receipt green (or failures explained + accepted by the human) — nothing else
REMEDIATION CAP: 2 rounds per failing check, then escalate with evidence
NEVER:           claim "done" from exit code 0 · fake a judge pass · skip the diff "because small"
```

## Before render: diff review

`project.checkpoint` at every pass boundary makes this cheap. Before any
`render.final`, run `project.diff{from: last_reviewed_checkpoint, to: "now"}`
and review like a code reviewer reads a PR:

1. **Every op has a rationale.** An op you can't justify in one line is an op
   you shouldn't have made. Fix the rationale or reject it — `edit.restore`
   undoes the LATEST timeline op only (snapshot inverses; deeper targets are
   refused), so skim-reject newest-first as you work; a bad op buried under
   later edits means `project.revert{to}` and redo from there.
2. **Duration delta sanity** vs the format budget (10–30% talking-head, 3–8%
   podcast — see the format skills). Outside the band = re-review, don't render.
3. **Cut clustering**: many ops inside one short region usually means the region
   should have been one decision (a take-cut), not twenty micro-cuts. Consider
   reverting the cluster and recutting deliberately.
4. **Orphan checks**: ops touching tracks you didn't intend (a video-track trim
   without its audio twin = drift), markers stranded inside removed ranges.
5. **The skim test**: could the human accept/reject each op from its one-line
   rationale alone? That's the Review-rail contract — write rationales for it.

## The render ladder

Don't buy the expensive render before the cheap looks:
- `render.frame{at_ms}` — single composed frame; for captions, safe zones, cut
  boundaries near motion. Seconds, not minutes.
- `render.preview{at_ms, duration_ms}` — low-res region render; for listening
  to cut joints, ducking transitions, pacing feel.
- `render.final` — the full deliverable; emits the RenderReceipt via auto
  `verify.checks`.
A final render that surprises you means the ladder was skipped.

## After render: receipt reading order

Read the RenderReceipt fully, in this order:
1. **duration_matches_edl** — engine honesty; if this fails nothing else is
   trustworthy (see `craft/fix-failed-checks.md` — likely a bug escalation).
2. **cut_on_word** — edit correctness.
3. **silence_at_edges**, **black_or_frozen_frames** — finish quality; read the
   EVIDENCE timestamps before remediating (frozen frames can be legitimate —
   `craft/screen-demo-polish.md`).
4. **lufs** — measured numbers vs the platform target, not just pass/fail.
5. **caption_presence** — if captions were promised.
6. **verify.judge** — if implemented, read the critique; if stubbed, the receipt
   says NO model watched. Report that honestly — "checks green, judge
   unavailable" is an honest status, "reviewed" is a lie.

Failures → `craft/fix-failed-checks.md`, max two remediation rounds per check,
then escalate with the receipt attached.

## When to escalate to the human

Editing judgment is yours; these are not:
- **Content decisions**: which of two clean takes is better, whether a tangent
  is charming or cuttable, anything sensitive a speaker said.
- **Brand/style calls** beyond the established style refs.
- **Scope walls**: duration delta would exceed the format band; source quality
  is unusable (noise floor, unintelligible audio) — flag at import, not after
  hours of editing.
- **Repeated failure**: any check still red after 2 remediation rounds.
- **Irreversible/destructive intents**: publishing, deleting source assets,
  overwriting deliverables.
Escalate WITH evidence: the receipt, the diff summary, frame captures, and a
recommendation. "Here's the problem, here are two options, I'd pick A because…"
— never a bare "what should I do?".

## Reporting done

The done message contains: what changed (diff summary + op count), the measured
facts (duration, LUFS, TP), the receipt verdict per check, and paths to
deliverables + receipts. If the UI is open, `ui.screenshot` after moving the
playhead to a representative moment — the human sees what you see.

## Receipts to check

This file IS the receipt protocol; the standing checklist:
- Full RenderReceipt read in the order above, every claim in the done-report
  traceable to a receipt field.
- diff reviewed against the last checkpoint before every final render (the
  checkpoint names in `project.state` are the audit trail that you did).
- Remediation history visible in the op log (fix ops reference the failing
  check in their rationale).
- Judge status reported truthfully (completed review / honest not-run or error).
- Escalations carry receipt + diff + frames as attachments, not adjectives.
