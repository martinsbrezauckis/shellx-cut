# Pacing and rhythm — cut timing craft

The cross-cutting skill: WHERE a cut lands and WHAT it leaves behind matters more
than how much you remove. Format recipes (talking-head, podcast, demo) reference
this file for the underlying judgment. Verb syntax: `reference.md`.

## Quick reference

```
BREATH ROOM:     0.3–0.8s pauses between sentences are speech, not silence — keep
LANDING AIR:     ~1s of air after a key claim; cutting it kills the emphasis
FORMAT BUDGET:   short-form = cut everything cuttable · 1–10min = balanced · 10min+ = only problems
JUMP CUT OK:     same shot, talking-head, energy aesthetic, social formats
JUMP CUT BAD:    formal/interview tone, mid-gesture, mid-action on screen
J/L THINKING:    let audio lead (J) or trail (L) the picture ~0.3–0.5s across a cut
HARD CUT:        reserve for topic changes — it signals "new section" to the viewer
DECLARE FIRST:   name the rhythm before editing; don't let pacing be an accident
```

## Declare the rhythm before you cut

Decide and write down (op rationales + a marker plan) what the piece's pulse is:
fast open → settle → build → tight close? Even pace throughout? A pattern you can
NAME ("tight-tight-breathe-tight") will survive 80 ops; an unstated one won't.
Practical move: `marker.add` at intended section boundaries first, then edit each
section to its intended energy. The markers double as chapter receipts later.

## Breath room and landings

- The pause between two sentences (0.3–0.8s) is part of how humans parse speech.
  Presets protect these (`natural` floor sits above them) — your overrides
  shouldn't go below ~700ms `min_ms` except in deliberate jumpy social cuts.
- After a thesis statement, a number, a reveal: leave ~1s. The viewer needs the
  beat to process. If `remove_silences` ate one, `edit.restore` that span's op
  while it is still the latest edit (restore is tip-only — review the pass's
  ops right after it runs) — that's exactly why removals are one-op-per-span.
- Before a topic change, a slightly LONGER gap + hard cut reads as intentional
  structure. Uniform gap length everywhere reads as machine-edited.

## Jump cuts — when a visible cut is fine

A ripple cut in a single continuous shot makes the speaker visibly "jump."
- **Fine**: social formats, energetic talking-heads, anywhere the jump-cut
  grammar is established (viewers read it as pace, not error). Many small jumps
  beat one awkward disguised cut.
- **Not fine**: formal interviews, emotional moments, mid-gesture (a hand
  teleporting is what viewers notice most), screen demos mid-action.
- Softening options: place the cut where motion is minimal (check
  `render.frame` just before/after the boundary), or move the cut a few words
  earlier/later to land on a natural head position — `transcript.cut_words`
  with a slightly shifted `word_range` is cheap; re-cutting after render isn't.

## J/L thinking with split tracks

`transcript.cut_words` cuts audio+video together (butt joint). When a joint
feels abrupt, offset the picture edit from the sound edit:
- **J-cut** (audio first): viewer HEARS the next segment ~0.3–0.5s before seeing
  it — smooths topic transitions. Build it with `edit.split` on the video track
  slightly AFTER the audio track split, then `edit.trim` the video clip edges.
- **L-cut** (audio trails): the previous segment's sound carries over the
  incoming picture — keeps continuity through a visual change.
Use sparingly (it's manual per-track surgery; the automated J/L check is
deferred to v1): reserve for the 2–3 most prominent transitions in a piece —
usually section boundaries. Verify each with `render.preview` spanning the joint.

## Pacing budgets by format

| Format | Posture |
|---|---|
| Short-form source (<60s target) | Every removable frame goes; visual change every 1–3s; `jumpy` |
| Standard piece (1–10 min) | Balanced; keep breaths; cut all fillers/takes; `natural` |
| Long-form (10 min+) | Cut only problems (dead air >1.5–2s, bad takes); pacing belongs to the speaker; `calm` |

The test for "did I over-cut": preview 30s and ask whether the speaker still
sounds like a person thinking, or like a supercut. Supercut = restore some air.

## Receipts to check

- **project.diff per-track ranges**: J/L offsets show up as video-track and
  audio-track ranges differing by your intended 300–500ms — confirm the offset
  exists and is the right direction.
- **render.preview at every section boundary**: rhythm is only auditable by
  playback; preview receipts (the rendered files) are your evidence of having
  listened.
- **cut_on_word**: manual `edit.split`/`edit.trim` J/L surgery is a common way to
  land a cut inside a word — this check is your safety net; must PASS.
- **silence_at_edges**: must PASS.
- **Op rationales**: every restore op from a landing/breath judgment says WHY
  ("landing air after revenue number") — pacing decisions must be skimmable.
- **Marker map**: section markers in `project.state` match the declared rhythm
  plan — if they don't, the plan drifted silently.
