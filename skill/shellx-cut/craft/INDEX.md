# ShellX Cut craft library — index

The CRAFT layer: how to edit video WELL with ShellX Cut's verbs. The TOOL layer
(verb syntax, connection, envelopes) lives in `../SKILL.md` + `../reference.md` —
craft files assume you've read it. Every craft skill is grounded in the public verb contract
verb registry and ends with a **"Receipts to check"** section: editing claims
are backed by measured evidence (op log, diff, RenderReceipt), never by vibes.
The library is organized around the editor's supported workflow and quality
contracts.

## Skills

| Skill | Read when… |
|---|---|
| [talking-head-cleanup.md](talking-head-cleanup.md) | cleaning a person-to-camera recording — bad takes, fillers, silences; the flagship 3-pass recipe with preset choice |
| [podcast-episode.md](podcast-episode.md) | editing a long-form conversation — minimal-touch cutting, chapters, music beds, −16 LUFS delivery |
| [screen-demo-polish.md](screen-demo-polish.md) | cutting screen recordings — the silence×scene map, why silence is often the action, caption placement over UI |
| [pacing-and-rhythm.md](pacing-and-rhythm.md) | deciding WHERE cuts land — breath room, landings, jump-cut judgment, J/L thinking, format pacing budgets |
| [captions-that-work.md](captions-that-work.md) | generating/styling captions — line lengths, legibility armor, safe zones, burn-in vs SRT, frame-verify protocol |
| [audio-baseline.md](audio-baseline.md) | working the audio pass — what LUFS/true-peak mean, gain math, manual ducking, when gain isn't enough |
| [platform-deliverables.md](platform-deliverables.md) | exporting — YouTube/Shorts/LinkedIn/podcast specs, NLE handoff, multi-platform masters |
| [review-discipline.md](review-discipline.md) | ALWAYS — the self-review doctrine: diff before render, render ladder, receipt reading order, when to escalate |
| [fix-failed-checks.md](fix-failed-checks.md) | any verify.checks failure — per-check evidence reading + concrete remediation verbs + the 2-round escalation cap |
| [generate-director-questioning.md](generate-director-questioning.md) | guiding Generate storyboard intake in Agent Chat — one focused question per turn, stated/inferred/missing fields |
| [generate-storyboard-planning.md](generate-storyboard-planning.md) | planning multi-scene Generate Storyboard IR — real template IDs, explicit missing assets, evidence before preview/insert claims |

## Reading order for a first job

1. `review-discipline.md` (the doctrine — applies to every job)
2. The format skill matching the footage (talking-head / podcast / screen-demo)
3. `fix-failed-checks.md` when the receipt comes back with red

Cross-cutting files (pacing, captions, audio, platforms) are referenced from
the format skills where they matter — follow the links, don't pre-read everything.
