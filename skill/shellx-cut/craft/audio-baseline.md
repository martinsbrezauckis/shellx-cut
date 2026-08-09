# Audio baseline — loudness, balance, and what the measurements mean

Craft skill for the audio pass. ShellX Cut measures audio for you (perception
loudness facts + the `lufs` receipt check); this file teaches what the numbers
mean and how to move them with the verbs you have. Verb syntax: `reference.md`.

## Quick reference

```
TARGETS:        -14 LUFS (YouTube/Shorts/TikTok/IG/LinkedIn) · -16 LUFS (podcasts/Apple)
TRUE PEAK:      ≤ -1.5 dBTP (YouTube, safe default) · ≤ -1.0 dBTP (TikTok/IG minimum)
MUSIC BED:      18–20 dB below speech while anyone talks
GAIN MATH:      needed_db = target_LUFS − measured_LUFS  (apply via edit.gain, re-measure)
HEADROOM RULE:  if gain-up pushes TP over ceiling, you're out of headroom → gain less, accept lower loudness, escalate if way off
PHONE TEST:     most viewers are on phone speakers — speech clarity beats richness
```

## The three numbers

- **Integrated LUFS** — perceived loudness of the WHOLE program (EBU R128,
  speech-gated). This is what platforms normalize on. YouTube turns down
  anything above −14; it does NOT turn quiet content up (your −19 LUFS video
  simply plays quiet next to everyone else's). So: hitting target matters most
  from BELOW.
- **True peak (dBTP)** — inter-sample peak. Over ~−1 dBTP risks audible clipping
  after the platform's lossy re-encode. The ceiling is a hard limit, loudness is
  a target: when they conflict, the ceiling wins.
- **Per-window loudness** (perception facts) — where the quiet and loud spans
  are. Use it to find the SPECIFIC quiet clip instead of gaining the whole mix.

## Gain staging with `edit.gain`

Cut supports per-clip and per-track gain, parametric EQ, denoise, gate, and
compression. Use these conservatively and measure the rendered result. That's
enough for talking content if you stage deliberately:

1. **Within-source balance first.** Read per-window loudness; if one segment
   (different mic distance, second speaker) sits >3 dB off the rest, fix THAT
   clip with `edit.gain{clip, db}` before touching program level.
2. **Program level second.** After a preview render, read measured integrated
   LUFS from the receipt; apply `needed_db = target − measured` as track-level
   gain on the speech track(s). One op, clear rationale ("−18.7 → −14: +4.7 dB").
3. **Re-render, re-measure.** Gain math on LUFS is approximately linear for
   uniform content but verify — the receipt is the truth, not the arithmetic.
4. **Peak conflict:** if the gain-up makes true peak fail, reduce gain until TP
   ≤ ceiling and accept the lower loudness. If you end >2 dB short of target,
   use `audio.cleanup_voice` or render-time normalization; if the conflict
   remains, escalate with both numbers rather than shipping clipped audio.

## Music under speech — `edit.duck`

ONE verb does the whole job: `edit.duck{music_track, against_track, db}`
computes speech windows from the against-track's perception facts mapped
through the current EDL and applies windowed gain with linear attack/release
ramps (honest semantics: windowed gain computed NOW and recorded on the op,
not a live sidechain compressor — re-run it after timeline changes that add
or move speech). Pair with `edit.fade{clip|track, in_ms, out_ms, kind:"audio"}`
for bed entrances/exits.

- Duck depth: 18–20 dB below the dialogue level while anyone talks; the bed
  can ride up in speech-free spans (duck windows only cover speech).
- Err toward QUIETER beds: the classic mistake is music 6 dB too loud. If you
  notice the music while listening to the words, it's too loud.
- Instrumental beds only under speech — lyrics fight the narration for the
  same attention channel (and the same frequency band).
- Manual fallback (`edit.split` at speech boundaries + per-segment
  `edit.gain`) still works for surgical cases — e.g. one specific bed swell —
  but it's ~10 ops where `edit.duck` is one.

## Silence and noise

- `silence_at_edges` failing = dead air at head/tail; trim it (see
  `craft/fix-failed-checks.md`).
- A noisy floor (hiss, hum, room tone) is not fixable with gain — gain raises
  the noise too. Use `edit.effect{type:"denoise"}` or `audio.cleanup_voice`, and
  flag sources that still need specialist restoration at import time
  (perception loudness facts show a high floor), before anyone invests
  edit work in unusable audio.

## Receipts to check

- **lufs**: the central receipt of this file. Read all of: measured integrated
  LUFS (vs the platform target from `craft/platform-deliverables.md`), true
  peak (vs ceiling), and pass/fail. Quote the measured numbers when reporting
  done — "audio at −14.2 LUFS, TP −1.8" is a claim with evidence; "audio
  normalized" is not.
- **silence_at_edges**: must PASS.
- **Gain op trail**: every `edit.gain` op carries the before→after math in its
  rationale, so the human can audit level decisions without a meter.
- **Preview-at-transition receipts**: for ducked music, `render.preview` files
  spanning 2–3 speech↔music boundaries are the evidence the ducking works.
- `verify.judge` for an ear-level review: status `completed` carries the
  verdict (check `listened:true` — it says whether a model actually heard the
  mix); `not_run`/`error` mean no model listened — say so rather than implying
  it was heard.
