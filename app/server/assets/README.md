# Bundled sample media

`first-edit-sample.mp4` is a small synthetic clip embedded in `cutd` for the
guided First edit workflow. Its script, visual composition, and generation
recipe were authored for ShellX Cut; it contains no third-party footage, music,
logos, fonts, or personal data.

Regenerate it with:

```bash
scripts/generate-first-edit-sample.sh
```

The script uses eSpeak NG for synthetic narration and FFmpeg filters for the
abstract video. The two deliberate 2.2-second gaps are long enough for the
recipe's `calm` silence-removal preset. The committed MP4 is the distribution
asset; regeneration is a maintainer task and is not required at runtime.

`first-edit-sample.perception.json` is an authored receipt template for that
exact script and timeline. `project.create` fills in the embedded MP4's current
SHA-256 and installed path, then seeds `a1`'s transcript and full perception
receipt. The recipe therefore remains demonstrable on a cold install without
pretending that the user's own media can be analyzed without the configured
speech/perception runtime.
