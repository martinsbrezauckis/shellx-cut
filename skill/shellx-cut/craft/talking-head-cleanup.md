# Talking-head cleanup — silence / filler / bad-take recipe

Craft skill for the bread-and-butter job: one person talking to camera, raw
recording in, tight watchable cut out. Verb syntax: `reference.md`. Craft only here.

## Quick reference

```
PASS ORDER:        bad takes → fillers → silences  (content first, mechanical last)
CHECKPOINT:        project.checkpoint before EVERY pass — revert is per-pass, not per-job
PRESET (default):  remove_silences{aggressiveness:"natural"}
PAUSE FLOOR:       never remove pauses under ~700ms — those are breathing, not dead air
PADDING:           80–150ms around speech; clipped word starts sound worse than slow pacing
TYPICAL SHRINK:    10–30% duration removed; over 40% → stop and re-review before render
EVERY CUT:         one op per span, rationale filled in — the human skims ops, not video
```

## Pass 0 — recon before touching anything

1. `media.import` → wait for the probe→transcribe→perception chain (`jobs.status`).
2. `transcript.get` — read the whole transcript. You are deciding content here,
   not just trimming air. Note: restarts, repeated takes, tangents, the strongest
   delivery of any duplicated point.
3. Read `perception.json` silence + loudness facts: how much dead air exists and
   where. This calibrates expectations for the receipt later.
4. `project.checkpoint{name:"raw-import"}`.

## Pass 1 — bad takes and false starts (judgment pass)

Use `transcript.cut_words{asset, word_range, rationale}` — never raw `edit.*`
verbs for speech; cut_words pads to word edges (±40ms) so the cut_on_word check
stays green by construction.

What to look for in the transcript:
- **Restarts**: "So the way this— okay let me start that again. The way this works…"
  → keep the LAST complete take unless an earlier one is clearly stronger.
- **Repeated points**: same idea said twice → keep the tighter delivery.
- **Tangents**: drift that doesn't serve the piece → cut to the next on-topic word.
- **Trailing meta-talk**: "was that okay?", "I'll re-record that" → always cut.

One `cut_words` call per decision, each with a rationale a human can accept or
reject in one glance ("false start, take 2 at word 210 is the keeper").
Checkpoint when done: `project.checkpoint{name:"after-takes"}`.

## Pass 2 — fillers

`transcript.remove_fillers{}` with the default lexicon (um, uh, erm, ah, hmm,
mhm — conservative; pass `lexicon` to also cut discourse words like "you
know"/"sort of"/"I mean", reviewing those extra carefully). Then review the
ops it emitted (`project.ops{since}`) BEFORE making further edits:
- **Reject as you review.** `edit.restore{op_id}` recomputes and undoes the
  LATEST timeline op only; deeper targets are refused with a guardrail.
  Review this pass's ops newest-first and restore the keepers' removals
  immediately; once you've edited past them, the path back is
  `project.revert{to:"after-takes"}` + re-run with a trimmed lexicon. Keepers:
  "like" as a comparison, "you know" addressed to the viewer, a deliberate
  hesitation before a punchline.
- A filler mid-sentence leaves a butt-joint; that's fine on video (small jump
  cut) but listen for audio pops at 2–3 of them via `render.preview{at_ms}`.
Checkpoint: `after-fillers`.

## Pass 3 — silences (mechanical pass, preset judgment)

`transcript.remove_silences{min_ms, padding_ms, aggressiveness}` — run LAST:
the earlier passes expose new gaps this pass then catches.

Choosing the preset:
| Preset | When | Feel |
|---|---|---|
| `calm` | long-form explainer, interview, anything contemplative | only true dead air goes; pacing untouched |
| `natural` | DEFAULT — talking-head YouTube, course content, updates | tightened but human; breaths survive |
| `jumpy` | shorts/reels source, high-energy social | every gap closed; jump-cut energy is the aesthetic |

Override knobs when the preset isn't quite right:
- Speaker is a slow, deliberate talker → raise `min_ms` rather than dropping to
  calm wholesale; their pauses ARE the style.
- Emphatic pauses after key claims: protect them — either raise `min_ms` past
  their length or restore those specific ops afterwards. A landed point needs
  ~1s of air to land.
- `padding_ms` low end (≈80) for jumpy, high end (≈150) for calm.

Checkpoint: `after-silences`.

## Review, render, verify

1. `project.diff{from:"raw-import", to:"after-silences"}` — read the summary:
   total duration delta, per-track ranges, op count. Sanity: shrink in the
   10–30% band; a cluster of many cuts in one 30s region means that region was
   a mess — consider whether it should have been one big take-cut instead.
2. `render.preview` at 2–3 of the densest cut regions — listen for clipped
   word starts and visual stutter.
3. `render.final{}` → wait for the RenderReceipt (auto verify.checks).
4. Failures → `craft/fix-failed-checks.md`. Done = receipt green, not "render
   command exited 0".

## Receipts to check

- **cut_on_word**: must PASS. Any fail = a cut landed inside a word — find the
  op via the evidence timestamp, undo it (`edit.restore` if latest, else
  `project.revert{to}` a checkpoint before it), recut with `transcript.cut_words`.
- **silence_at_edges**: must PASS — cleanup that leaves dead air at the head is
  half-finished.
- **duration_matches_edl**: must PASS (engine honesty check).
- **lufs**: read the measured value even if the check passes — you'll need it
  for `craft/platform-deliverables.md`.
- **Op-log receipt**: every removal op has a rationale; count of restore ops ≈
  your judgment overrides, each explainable.
- diff summary duration delta matches what you told the human you removed.
