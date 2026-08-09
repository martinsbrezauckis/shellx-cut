# Podcast episode — long-form conversation cut

Craft skill for editing a recorded conversation (one or more speakers, audio-first,
possibly with a static video frame). The goal is invisible editing: listeners
should never notice a cut. Verb syntax: `reference.md`.

## Quick reference

```
LOUDNESS:        -16 LUFS integrated, true peak ≤ -1 dBTP (podcast platforms)
SILENCE PRESET:  calm — long-form conversation must breathe
DEAD AIR:        only remove gaps over ~1.5–2s; leave 0.5–0.8s behind, never butt-join
TYPICAL SHRINK:  3–8% of duration; podcasts are NOT shorts — don't compress the talk
CROSSTALK:       never cut inside overlapping speech — cuts there are always audible
MUSIC BED:       18–20 dB below speech; duck by splitting the music clip + edit.gain
CHAPTERS:        marker.add at every topic change as you read the transcript
```

## Workflow

### 1. Recon
`media.import` each source → wait for transcribe + perception. `transcript.get`
and read it end-to-end once before any edit — you cannot chapter or cut a
conversation you haven't read. `project.checkpoint{name:"raw"}`.

### 2. Chapters first (markers cost nothing)
While reading, `marker.add{at_ms, label}` at every topic shift. This pays three
ways: chapter export for YouTube/show notes, navigation for the human reviewer,
and a map for your own later passes. Use the transcript word timestamps to place
markers at the START of the sentence that opens the topic, not mid-handover.

### 3. Content cuts (sparing)
Podcasts tolerate far less cutting than talking-heads. Cut with
`transcript.cut_words` only:
- pre-show/post-show chatter ("are we recording?", "I'll edit this out"),
- explicit redo requests by the speakers,
- long derails the hosts themselves abandoned,
- legally/personally sensitive remarks (flag these to the human — content
  judgment, not editing judgment).
Do NOT clean up natural conversation: stumbles, overlaps, laughing through a
sentence — that texture is the product. Rationale on every cut.
Checkpoint: `after-content`.

### 4. Silence pass — calm preset
`transcript.remove_silences{aggressiveness:"calm"}` (raise `min_ms` toward
2000 if the show has a thoughtful, pause-heavy style). Two cautions:
- **Crosstalk edges**: if perception silence spans border overlapping speech,
  the VAD boundary is unreliable — review those ops with `render.preview` and
  restore any that clip a speaker's overlap.
- **Thinking pauses** in interviews are content. When the guest pauses 3s before
  a vulnerable answer, that silence is the best part of the episode. Restore it.
Checkpoint: `after-silences`.

### 5. Fillers — selective, not global
A global `transcript.remove_fillers` pass is usually WRONG for podcasts — it
makes humans sound like text-to-speech. Run it only if a speaker's filler rate
is genuinely distracting (subjective threshold: you notice it on every sentence),
and review every op it emits; restore generously.

### 6. Music, intro/outro
- `edit.insert{asset, track, at_ms}` for intro/outro beds on a music track.
- Prefer `edit.duck` for speech-aware music reduction. Bed segments under speech
  should sit 18–20 dB below the dialogue level; intro/outro segments without
  speech can ride higher. Use `edit.split` plus `edit.gain{clip, db}` when a
  manually shaped transition is needed, then preview the transitions.

### 7. Render + deliver
`render.final` → receipt. Then `export.srt` (transcript-faithful SRT is
the podcast deliverable, even audio-first platforms ingest it) and `export.xml`
if a human finishes in a desktop NLE.

## Receipts to check

- **lufs**: measured integrated vs **−16**, true peak ≤ −1 dBTP. Off-target →
  `craft/fix-failed-checks.md` gain recipe; don't ship a −19 LUFS episode.
- **cut_on_word**: must PASS — doubly important with multiple speakers.
- **silence_at_edges**: must PASS (no dead air before the intro).
- **duration_matches_edl**: must PASS.
- **Diff sanity**: duration delta in the 3–8% band. Double digits on a podcast
  means you edited it like a YouTube video — re-review before render.
- **Marker receipt**: `project.state` shows markers covering the full runtime
  (no 20-minute topic gaps — you probably stopped reading there).
- Per-op rationale present; restored-op count documents your judgment calls.
