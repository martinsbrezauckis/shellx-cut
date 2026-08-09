# Cut Recorder Crates

These crates are the in-process recorder stack used by `cutd`:

- `record-core` owns recording/event/edit-plan data models.
- `record-engine` converts captured events into an edit plan.
- `record-render` renders polished MP4/GIF output from a source and plan.
- `record-capture` owns replay/live screen, input, audio, and camera capture.
- `record-recovery` owns append-only, synced checkpoint manifests and fail-closed
  restart recovery.

They were integrated into the Cut workspace so recording builds, tests, and
release packaging no longer depend on a sibling `shellx-record` checkout.

## Crash recovery contract

Live encoder containers are never recovery inputs. Linux closes a GStreamer or
FFmpeg segment, Windows waits for `VideoEncoder::finish`, and macOS waits for the
`SCRecordingOutput` completion callback before the immutable segment is hash-checked,
fully decoded, renamed, and appended to its synced JSONL manifest. The owner identity
binds a PID to boot/process-start facts plus a capture nonce; a reused or inaccessible
PID is deferred, never signalled. A missing PID is recoverable even when identity
inspection was unavailable.

Each checkpoint records measured start/end/event/audio offsets and media duration,
frame count, audio presence, byte size, and streaming SHA-256. The backend opens one
shared capture clock only after its portal/WGC/SCK session is ready: event timestamps,
mic first-packet silence padding, system-audio timing, and checkpoint starts all consume
that origin. Restart recovery uses
only the contiguous prefix whose hash and media facts still match; corrupt checkpoints
and an exact torn journal tail go under `quarantine/`. The synced valid journal prefix
is then sealed with the recovery receipt, so a later restart is idempotent. A malformed
non-final journal record is fail-closed and quarantined rather than guessed.

`source.mp4` is the normal-completion output. It is stitched from all verified segments
only after each real encoder-restart gap is materialized as cloned video frames, keeping
the video on the same wall clock as `events.json` and continuous mic audio. Windows
places system audio by its persisted first-WASAPI-packet offset; macOS pads its Core Audio
tap from the measured first callback. Linux records its native PipeWire default-sink monitor's
first nonempty packet on that same clock before WAV I/O. A successfully finalized Linux WAV
with no packet publishes a null offset and is not placed automatically; a PipeWire
connection, format, or capture failure deletes its partial WAV rather than claiming a raw
artifact exists. A normal `Complete` receipt is the authoritative journal
record and is written after atomically publishing `project.json`; a restart repairs that
narrow project/receipt boundary as `Complete`, never as a second recovered output. `recovered.mp4`
is a distinct, independently playable salvage output for a dead interrupted capture; it
never overwrites `source.mp4`, and it does not claim to be a completed RecordingProject.
An open final segment has an unknown upper lost-tail bound; a corrupted finalized segment
has an exact known checkpoint-tail bound.

Linux's segment implementation is exercised with real FFmpeg finalized media.
Wayland consent/reopen behavior must be tested in a native portal session. Windows
rotation uses WGC and requires a native Windows session for capture timing tests.
macOS uses ScreenCaptureKit `SCRecordingOutput` (not AVAssetWriter), requires its
completion callback before probe/publication, and requires Xcode for native builds.
