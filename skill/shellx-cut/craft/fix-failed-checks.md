# Fix failed checks — remediation recipe per verify.checks failure

Companion to `craft/review-discipline.md`: each receipt check, what a failure
means, and the concrete verb sequence that fixes it. General loop: **read the
evidence → smallest targeted fix → re-run `verify.checks` → max 2 rounds →
escalate with the receipt.** Never "fix" what the evidence doesn't show.

## cut_on_word — a cut landed inside a word

**Evidence:** boundary timestamp(s) + the word span each one violates.
**Causes:** manual `edit.split`/`edit.trim`/`edit.ripple_delete` at a raw
timestamp (J/L surgery is the usual suspect); rarely, transcript drift on a
re-imported asset.
**Fix:**
1. Find the op that created the offending boundary: `project.ops` → match the
   effect range to the evidence timestamp.
2. Undo it: `edit.restore{op_id}` if it is the latest timeline op (restore is
   tip-only — snapshot inverses); if edits landed after it, `project.revert{to}`
   the nearest checkpoint before it and re-apply the keepers.
3. Recut through the transcript layer: `transcript.cut_words` with the
   `word_range` covering your intent — it pads to word edges by construction.
4. If the boundary must stay a raw `edit.*` (e.g. a music-track split), move it
   into the gap between words: `transcript.get` → place the cut at
   prev_word.end + 40ms or later.
**Escalate if:** evidence words don't match what you hear at that timestamp —
transcript may be stale for this asset (re-run `media.transcribe`, then report).

## lufs — loudness off target / true peak over ceiling

**Evidence:** measured integrated LUFS, true peak dBTP, the target profile.
**Fix (loudness):**
1. `needed_db = target − measured` → `edit.gain{track, db}` on the speech
   track(s), rationale with the math.
2. Re-render, re-check — the receipt is the truth, not the arithmetic.
**Fix (true peak):** gain DOWN until TP ≤ ceiling; the ceiling always wins over
the loudness target.
**Escalate if:** meeting TP leaves you >2 dB under loudness target. This needs
additional mastering beyond the built-in cleanup and normalization tools; the
editor decides whether to accept the quieter result or process it elsewhere.
Full reasoning: `craft/audio-baseline.md`.

## caption_presence — no captions where captions were promised

**Evidence:** ranges lacking caption coverage.
**Causes (in order of likelihood):** `captions.generate` never ran; transcript
missing for the asset (transcribe job failed/skipped); caption track exists but
cues don't cover the flagged range (e.g. captions generated BEFORE later edits).
**Fix:**
1. `project.state` → does a caption track with cues exist?
2. No track → check `jobs.status` for the transcribe job → `media.transcribe`
   if needed → `captions.generate{style_ref}`.
3. Track exists but stale coverage → re-run `captions.generate` (cheap; it
   derives from transcript + current timeline) and re-apply the style ref.
4. Re-render, re-check, plus one `render.frame` in the previously-bare range.

## black_or_frozen_frames — dead picture in the render

**Evidence:** timestamp ranges flagged black or frozen.
**INTERPRET FIRST:** frozen ≠ broken. Screen demos hold static UI; title cards
hold. Cross-check evidence ranges against perception scene facts — explainable
static = accept, note in the done-report, move on.
**Causes when real:** a gap clip left by non-ripple editing (`edit.move` that
left a hole); `edit.trim` past the asset's actual bounds (frozen last-frame
padding); a corrupt source region (probe vs perception disagree).
**Fix:**
1. `render.frame` at the flagged timestamps — SEE the problem.
2. Gap → close it: `edit.move` the downstream clip up, or
   `edit.ripple_delete{range_ms}` over the hole.
3. Out-of-bounds trim → `edit.trim` back inside `src_in_ms/src_out_ms` limits
   from `media.probe`.
4. Suspected source corruption → escalate with the frame captures; editing
   can't fix a broken source.

## silence_at_edges — dead air at head or tail

**Evidence:** silent span duration at start and/or end; `details.detector`
names the gate (silero-vad speech absence + ffmpeg silencedetect at −35 dB,
min span 0.3s).
**READ THE DETECTOR FIRST:** "silence" here is level/speech-gated, NOT
"inaudible". A quiet-but-audible music bed under −35 dB at the edge (e.g. a
−28 LUFS intro bed) still registers as silence — a fade-in tweak will NOT fix
that check; either raise the bed above the gate at the edge, trim the edge,
accept-with-note, or use the `silent_screen_demo` profile when silence is by
design.
**Fix (real dead air) — the dedicated verb:** `edit.trim_edges{}` is the
top-and-tail fix that closes THIS check. It anchors to SPEECH (first/last spoken
word via the transcript), trims the leading + trailing dead air with two
ripple_deletes, and leaves `keep_pad_ms` (default 200ms) of breath each side so
nothing slams shut — internal pacing untouched (unlike `remove_silences`). Tune
`keep_pad_ms`/`min_trim_ms`; silent footage is an honest no-op. (Manual path if
you need asymmetric pads: `edit.ripple_delete{range_ms:[0, first_sound_ms−150]}`
for the head, mirror at the tail keeping ~500–1000ms after the last word.)
Verify intent: a deliberate cold-open hold or fade-out tail is a
pass-with-note, not a bug — say so in the report instead of mangling the intro.

## duration_matches_edl — render length ≠ timeline math

**Evidence:** EDL-computed duration vs measured output duration + delta.
**This one is different: it's an ENGINE honesty check, not an edit problem.**
1. Re-run `render.final` once (transient encode hiccup happens).
2. Still failing → STOP remediating through edits. Collect: the receipt, the
   diff, `project.state`, ops since last green render — and escalate as a
   probable cut-media bug. Do not ship a render whose length the engine can't
   account for, and do not "fix" it by trimming the timeline to match the
   broken output.

## verify.judge — reading the judge's verdict

The judge is WIRED (subscription-CLI adapter as a job): `status` is
`completed` (real verdict + confidence + issues w/ timestamps), `not_run`
(adapter/backend unavailable — report "no model watched this" honestly), or
`error` (job failed; the receipt carries the cause). `watched`/`listened`
flags say what the model actually perceived. Real critique → treat findings
like check evidence: locate the moment (`render.frame`/`render.preview` at
the cited timestamp), judge whether it's right (the judge sees rendered
output and can mis-localize; measurement-class claims are post-filtered as
documented in reference.md), fix or accept with a note. Judge findings are
advisory; measured checks are binding.

## Receipts to check

- After ANY remediation: full `verify.checks` re-run — fixing one check can
  break another (gain-up after edge-trim can re-expose a TP fail; closing a gap
  shifts caption coverage).
- Two consecutive receipts for the same check still red → escalation with both
  receipts attached; that is the cap, not a suggestion.
- Fix ops reference the failing check in their rationale ("close 400ms gap at
  61.2s — black_or_frozen_frames evidence") so the op log tells the remediation
  story end to end.
- Accepted-as-legitimate failures (static demo UI, cold open) are written into
  the done-report next to the check name — an explained red is honest; a
  silently ignored red is not.
