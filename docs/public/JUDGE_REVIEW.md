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

## Provider ladder

Auto mode tries detected CLIs in this order:

1. Claude Code (`claude`)
2. Codex (`codex`)
3. Antigravity (`agy`)
4. Grok Build (`grok`)

A named backend forces exactly that rung. It never silently substitutes a
different provider.

In auto mode only, an infrastructure-class failure steps down to the next
detected rung. This includes a present Claude CLI whose frame-Read preflight
fails: the render is still offered to Codex, Antigravity, then Grok instead of
ending as `not_run`. The final envelope records every attempted rung. A real
`completed` verdict—including `fail`—is terminal and never triggers provider
fallback.

## Runtime and overrides

The adapter is installed under the perception resource payload and discovered
beside `instruments.py`. It is stdlib-only but needs a usable Python
interpreter plus ffmpeg/ffprobe for frame sampling. Settings > AI & Services
reports the CLI and adapter runtime separately.

`CUTD_JUDGE_ADAPTER` may override the bundled ladder for testing or advanced
operation. The override is authoritative: a missing override path does not
fall back silently. `CUTD_ADAPTER_PYTHON` selects the interpreter; otherwise
Cut uses its managed perception runtime, with a PATH Python fallback on
platforms where that does not trigger an OS installer prompt.

The deterministic no-quota contract is exercised by:

```bash
node scripts/public-tests/judge-adapter.test.mjs
cargo test -p server verify_judge_
```
