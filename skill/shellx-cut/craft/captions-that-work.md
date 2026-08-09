# Captions that work — styling, segmentation, platform norms

Craft skill for the caption pass: generating, styling, and verifying captions
that people can actually read. Captions are a primary surface — most social
viewing is muted. Verb syntax: `reference.md`.

## Quick reference

```
LINE LENGTH:    ≤ 42 chars (16:9) · ≤ 20 chars (9:16 vertical) · 2 lines max
CUE DURATION:   ~1–5s on screen; never under 0.8s; reading speed ≈ 15–17 chars/s
SIZE:           ≥ 42px at 1080p (≈ 4% of frame height); bold sans-serif
CONTRAST:       4.5:1 minimum vs background — in practice: light text + dark pill/shadow
POSITION:       bottom-center default; AVOID bottom ~300px on vertical (platform UI)
BURN-IN vs SRT: social = burn-in mandatory · YouTube long-form = SRT sidecar (+optional burn)
VERIFY BY EYE:  render.frame at 3–4 caption moments — styling is only provable visually
AUTO-QC:        verify.captions (reading speed / duration / gaps / line length) → captions.reflow to fix
BRAND:          verify.brand (font / colour / position / aspect vs the kit) before ship
```

## Generate, then read

1. Captions inherit transcript quality — if `transcript.get` shows misheard
   words (names, jargon, product terms), fix understanding FIRST (re-run
   `media.transcribe` with a bigger model if systematic), because every caption
   error is on screen in 42px bold.
2. `captions.generate{style_ref}` → caption track from the word timestamps.
3. Read the generated cues in `project.state`. Check segmentation craft:
   - cues break at phrase boundaries, not mid-prepositional-phrase
     ("…the fastest way | to ship video" — never "…the fastest | way to ship video"),
   - no orphan single-word cues (except deliberate emphasis),
   - numbers and names not split across cues.

## Styling

Define once as a named style (`captions.set_style{ref, style}`), reference it
everywhere — brand consistency is a style-ref, not 40 per-cue tweaks.

- **Font**: bold sans-serif (Inter/Roboto-class). Decorative fonts are for title
  cards, never for captions in motion.
- **Legibility armor**: video backgrounds change every second — bare text WILL
  hit a white shirt eventually. Use a semi-opaque dark pill (`bg`) or strong
  shadow/outline so contrast holds on every frame, not the average frame.
- **Size**: 42px+ at 1080p; for 9:16 vertical, bigger (the phone is further from
  the eye than you think) — 4–5% of frame height.
- **Color highlights** (the social "karaoke" style — current-word emphasis) are
  outside the current caption contract; one clean style beats an unreliable animated one.

## Placement and safe zones

- 16:9: bottom-center, clear of lower-third graphics.
- 9:16 vertical: the bottom ~300px and right ~100px are covered by platform UI
  (captions/share/like rails); top ~110–210px by account chrome. Place captions
  in the center-lower band INSIDE the safe area.
- Screen demos: captions must not cover the UI being demonstrated — see
  `craft/screen-demo-polish.md` (top position is often correct there).

## Burn-in vs sidecar

| Deliverable | Choice | Why |
|---|---|---|
| Shorts/Reels/TikTok | burn-in (rendered) | muted autoplay; platform caption styling is uncontrollable |
| YouTube long-form | `export.srt` sidecar; burn-in only if brand wants it | user-toggleable, indexed, translatable |
| LinkedIn | burn-in + SRT upload | feed autoplays muted; SRT helps accessibility |
| Handoff to desktop NLE | SRT + `export.xml` | editor restyles in their tool |

## Verify visually — always

Styling claims are only provable by looking. Take `render.frame{at_ms}` at:
- the first cue (most-seen frame of the piece),
- the longest cue (line-length worst case),
- one bright/busy background moment (contrast worst case),
- on vertical: one frame checked against the safe-zone bounds.
Look at each: readable? inside safe area? not covering faces or demo UI?
If any fail → adjust the style ref once, re-render frames, re-look.

## Receipts to check

- **caption_presence**: must PASS on any deliverable that promised captions —
  it catches the "generated but track muted/empty" failure class.
- **Frame receipts**: the 3–4 `render.frame` captures from the visual check are
  the styling evidence; keep them with the project receipts. A caption pass
  without frame captures is unverified by definition.
- **SRT receipt**: `export.srt` returns the path — confirm the file
  exists and spot-read the first/last cues for timing sanity (0:00 start bug,
  cue past video end).
- **cut_on_word**: still must PASS — caption timing inherits the same word
  boundaries; a red cut_on_word usually means captions drift at the same spot.
- **Op-log**: style changes are ops with rationale ("bumped size 42→52, vertical
  legibility") — the brand owner can audit styling drift.
