# Platform deliverables — specs and export settings per destination

Craft skill for the last mile: what each platform actually wants, and which
render/export verbs produce it. Numbers are 2026 platform norms — re-verify
against platform docs when a delivery matters commercially. Verb syntax:
`reference.md`.

## Quick reference

```
YOUTUBE (16:9):   1920×1080+ · H.264 High · -14 LUFS · TP ≤ -1.5 · SRT sidecar · chapters
SHORTS/REELS/TT:  1080×1920 (9:16) · ≤60s (Shorts) · burn-in captions · -14 LUFS · TP ≤ -1
LINKEDIN:         16:9 or 1:1 · ≤ ~2-3min sweet spot · burn-in captions (muted autoplay)
PODCAST (VIDEO):  16:9 · -16 LUFS · TP ≤ -1 · SRT + chapter markers
NLE HANDOFF:      export.xml{format:"fcpxml"|"premiere"|"resolve"} + SRT, no render needed
ASPECT IS A PROJECT SETTING: set width/height/fps at project.create — reframing isn't a render flag
```

## YouTube long-form (16:9)

- Project: 1920×1080 (or 4K source-permitting), 24/25/30 fps matching source —
  never resample frame rate for delivery, it stutters.
- `render.final{preset}` — H.264 high profile, quality-first bitrate (YouTube
  re-encodes everything; feed it more than you think: visibly transparent
  quality in ≈ 8–15 Mbps territory at 1080p30).
- Audio −14 LUFS / TP ≤ −1.5 (see `craft/audio-baseline.md`; remember the
  platform only turns loud content DOWN).
- Captions: `export.srt` sidecar (toggleable, indexed). Burn-in only as
  a brand choice.
- Chapters: your `marker.add` map exports as the chapter list (first marker at
  0:00, minimum 3 chapters, each ≥10s — YouTube's rules for showing them).

## Shorts / Reels / TikTok (9:16 vertical)

- Project: 1080×1920. Vertical is a different EDIT, not a crop of the 16:9 —
  faces bigger, captions bigger, pace faster (`craft/pacing-and-rhythm.md`
  short-form budget).
- Duration: ≤60s for Shorts eligibility; completion rate beats length — a tight
  35s outperforms a padded 59s.
- **The first 1–2 seconds carry everything**: front-load the strongest moment;
  no logo intro, no "hey everyone". Verify frame 0 is visually interesting with
  `render.frame{at_ms:0}`.
- Captions: burn-in mandatory (muted autoplay), inside safe zones — bottom
  ~300px / top ~110–210px / right ~100px belong to platform UI
  (`craft/captions-that-work.md`).
- Audio −14 LUFS / TP ≤ −1; phone-speaker reality: speech clarity first.

## LinkedIn

- 16:9 (feed) or 1:1 (more feed height); H.264 MP4.
- Autoplays MUTED with no caption toggle in feed → burn-in captions are not
  optional. Many viewers never unmute: the cut must work silent (captions +
  visuals carry the argument).
- Business pacing: tighter than a podcast, calmer than TikTok — `natural`
  preset, ~2–3 min sweet spot for feed pieces.

## Podcast with video

See `craft/podcast-episode.md` for the edit; delivery deltas: −16 LUFS (more
headroom for speech dynamics), SRT always, chapters always, and consider a
separate audio export with `audio.export` if the feed needs it.

## NLE handoff (the "finish in Premiere/Resolve" deliverable)

When a human editor takes over: `export.xml{format}` (+ `export.srt`).
The XML carries the cut, not the look — confirm with the recipient which format
their tool imports cleanly. Include the project diff summary in the handoff
message: editors want to know what was already cut and why (op rationales are
exactly that).

## Multi-platform from one master

Two paths, cheap-to-considered:

1. **Fast reframe (same project, no re-cut):** `render.final{aspect:"9:16"}`
   (or `"1:1"`/`"4:5"`/explicit `width`+`height`) renders the SAME cut at a new
   geometry WITHOUT mutating the project — defaults `fit:cover` (centre-crop).
   Render 16:9 for YouTube AND 9:16 for Shorts from one timeline in two calls.
   `render.final{dry_run:true, aspect:"9:16"}` shows the geometry first. Best
   when the subject sits centre-frame (most talking-heads) and you want the
   deliverable now.
2. **Considered vertical (a different EDIT):** a centre-crop is not a reframe —
   for hero verticals, faces bigger, captions bigger, pace faster. Branch the
   project (re-import reuses cached transcripts/perception by asset hash) and
   re-cut for the phone. Don't contort one timeline to serve two aspects when
   the vertical deserves its own pass.

Either way, derive the text deliverables once: `export.chapters` (YouTube/podcast
markers), `export.transcript` (show notes), `export.vtt`/`export.srt` (captions).

## Receipts to check

- **lufs**: measured vs the TARGET FOR THIS PLATFORM (−14 vs −16 matters — a
  pass against the wrong target is a fail).
- **duration_matches_edl**: must PASS; additionally check duration against the
  platform budget (Shorts ≤60s is a hard gate, check the measured duration, not
  the intended one).
- **caption_presence**: must PASS wherever burn-in was promised (all vertical +
  LinkedIn deliverables).
- **Frame receipts**: `render.frame{at_ms:0}` (hook frame, vertical) + one
  safe-zone check frame per vertical deliverable.
- **Export receipts**: paths returned by `export.srt` / `export.xml` exist;
  for handoffs, the diff summary accompanied the files.
- **black_or_frozen_frames / silence_at_edges / cut_on_word**: green across
  every deliverable — each derived project gets its own full receipt, not an
  inherited one from the master.
