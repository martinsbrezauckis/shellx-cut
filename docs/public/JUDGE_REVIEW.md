# Render Judge Contract

`verify.judge` is ShellX Cut's optional perceptual review of a completed
render. Deterministic receipt checks remain the source of truth for measurable
facts; the judge answers the separate question, “What would a viewer notice?”

The installed app ships the adapter. Users do not need an API key or an
external script. The adapter drives an already installed, logged-in
subscription CLI and records an honest `completed`, `not_run`, or `error`
result on the render receipt.

## 2. Sampling

### 2.1 Global review

The default global pass samples the complete render at up to 1 frame per
second, 512 pixels wide, with a 20-frame cap. When the cap would omit the tail,
the effective sample rate is reduced so frames remain spread across the whole
render.

### 2.2 Window review

A focused window may sample at up to 5 frames per second. Timestamp evidence
is quantized to the actual sampling grid and clamped to the render duration.
The envelope records requested and effective sample rates.

The current CLI backends are visual reviewers. They always record
`watched:true` and `listened:false`; transcript and measured audio facts may be
provided as context, but the model must not claim it heard the output.

## 4. Instrument digest

The adapter supplies compact facts from the rendered output's own perception
receipt: duration, scene changes, silence spans, transcript words, loudness,
black/frozen spans, and other available instruments. Source-asset timestamps
must never be substituted for render coordinates.

Before sampling or spending a CLI turn, the adapter rejects perception
timestamps that exceed the render duration beyond the documented small
instrument slack.

The render's analysis file (`receipts/<render_id>.output.perception.json`) is
used only when it is an ordinary file of the project (not a link, a Windows
junction, or a file shared with another path; a cloud-synced file such as a
OneDrive file is ordinary) and records the render's
own content hash. Otherwise the review is not run: no facts or transcript from
that file reach a prompt, and the job fails with a message naming the file.
Deleting that file lets the next review analyse the render itself.

### 4.1 Project files the review uses

A project may come from someone else, so the review treats its files as
untrusted. It never reads the render through a link at the render path, and a
verdict is attached only to the render file the receipt names: the render's
content hash must equal the one the receipt recorded. A render in the project
or its export folders is checked before the review and again after it; when
either check finds other content the review is not run (or its verdict is
discarded) and the job fails with a message naming both hashes. A render
outside them (for example a one-off Save As destination) is copied into the
run folder while it is hashed, and the copy is reviewed. Each run works in a new
folder under `.scratch/judge/<render_id>/` that no project content can occupy,
and the latest result is also saved as `.scratch/judge/<render_id>/envelope.json`.
A link or shared file already at that saved path stops the review before any
provider is used, and the file it points to is left unchanged.

## 5. Review request and result

The request combines:

- the editor's operation-derived intent;
- the render's instrument digest;
- an ordered frame/time map; and
- a strict structured-output schema.

The model review contains:

```json
{
  "verdict": "pass | fail | needs_review",
  "confidence": 0.0,
  "summary": "short viewer-facing assessment",
  "issues": [
    {
      "at_ms": 0,
      "end_ms": 0,
      "kind": "visual_artifact",
      "severity": "blocker | major | minor",
      "evidence": "what is visible",
      "suggested_fix": "optional editor action"
    }
  ],
  "cannot_assess": []
}
```

cutd validates the outer `shellx-cut/judge-review/1` envelope again before it
is attached to a receipt. A malformed adapter result becomes `error`; it never
becomes a pass.

## 6. Cost and limits

Subscription CLI calls may consume the user's plan quota. The UI therefore
runs `verify.judge` only after an explicit user action. Detection and
`system.doctor` never make a model call.

Frame count, prompt size, timeout, provider metadata, and available CLI usage
metadata are retained in the envelope where the provider exposes them. Cost
fields are estimates only and must remain absent when pricing is unknown.

## 7. Honesty and post-filtering

- `completed` means a provider returned a schema-valid review.
- `not_run` means no selected provider/runtime was usable. It is advisory and
  never a pass.
- `error` means a review was attempted but failed. The job fails while the
  error envelope remains attached for audit.
- A refused project file is also recorded as `error`, with a reason that
  starts with `review not run` and names the file to remove.
- Failed CLI diagnostics retain bounded, labelled `stdout` and `stderr` text.
  A structured error envelope may be on stdout; Cut reports that evidence and
  does not infer a root cause from a keyword alone.
- `pass`, `fail`, and `needs_review` are model verdicts. The normalized job
  outcome is respectively `approve`, `reject`, or `advisory`.
- Instrument measurements own loudness, exact timing, duration, and other
  measured claims.
- A vision-only model's audio claims are removed or explicitly distrusted.
- Evidence timestamps are quantized to observable frame granularity.
- A model that could not read the sampled frames is an infrastructure error,
  not a low-confidence completed review.

### 7.1 Consumer-side filter

Every provider result passes the same filter before receipt attachment. The
filter strips measurement-class numbers and unsupported audio assertions,
normalizes timestamps, validates enums and confidence, and preserves both the
raw review and a report of filtering decisions for audit.

## Provider admission and ladder

The request union remains stable: `claude`, `codex`, `antigravity` (`agy`), and
`grok` are accepted backend IDs. Detection (`found`) is separate from render
judge admission (`judge_ready`). These render-review rungs are admitted when
their capability checks pass:

| Backend | Admission requirement |
|---|---|
| Claude Code (`claude`) | Version 2.1.248 or later, restricted Read capability, and a platform-safe copied-frame workspace. Each review uses a fresh copied-frame working directory. |
| Codex (`codex`) | Direct sampled-frame image input, strict structured output, and a tool-free review route. Doctor must report `judge.codex.details.judge_ready:true`; binary discovery or version alone is insufficient. |
| Grok Build (`grok`) | Version 1.0.21 or later with the verified no-model-tools policy. |
| Antigravity (`agy`) | Deferred until restricted tool and file access are verified. A detected installation may still report `found:true`, but is not `judge_ready`. |

For a Codex installation that fails admission, or for Antigravity, an explicit
render-judge request returns `status:"not_run"` before render probing, frame
extraction, prompt construction, or a model subprocess. The recorded reason
explains the unavailable capability. This render-judge limit does not remove
those providers from Agent Chat or other product features.

Auto retains its configured order—Claude, Codex, Antigravity, then Grok—but
skips every unready rung. After a ready rung is actually attempted, an
infrastructure-class error may continue to the next ready rung. A completed
review, including `fail`, is terminal. A named backend always selects only that
rung and never falls back. The receipt records the selected, skipped, and
attempted ladder state.

Doctor admission reports capability, while the completed receipt records an
actual review. `not_run` remains unreviewed rather than a passing review.

## Runtime and overrides

The adapter is installed under the perception resource payload and discovered
beside `instruments.py`. It is stdlib-only but needs a usable Python
interpreter plus ffmpeg/ffprobe for frame sampling. Settings > Services & integrations
reports app discovery separately from a bounded export review availability check; an
unready card names its reason. Agent Chat uses its own capability and session
readiness, so it is not implied by the export review card.

`CUTD_JUDGE_ADAPTER` may override the bundled ladder for testing or advanced
operation. The override is authoritative: a missing override path does not
fall back silently. `CUTD_ADAPTER_PYTHON` selects the interpreter; otherwise
Cut uses its managed perception runtime, with a PATH Python fallback on
platforms where that does not trigger an OS installer prompt.

The deterministic no-quota contract is exercised by:

```bash
cargo test --manifest-path app/Cargo.toml -p server verify_judge_
```
