# Cut Recorder Crates

These crates are the in-process recorder stack used by `cutd`:

- `record-core` owns recording/event/edit-plan data models.
- `record-engine` converts captured events into an edit plan.
- `record-render` renders polished MP4/GIF output from a source and plan.
- `record-capture` owns replay/live screen, input, audio, and camera capture.

They were integrated into the Cut workspace so recording builds, tests, and
release packaging no longer depend on a sibling `shellx-record` checkout.
