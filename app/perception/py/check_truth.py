#!/usr/bin/env python3
"""check_truth.py — assert a PerceptionReport against generated ground truth.

Role: the perception self-test (end-to-end test contract "known ground truth = assertable
tests"). Compares instruments.py output for testdata/talking_head.mp4 with
testdata/talking_head.truth.json (written by scripts/make-test-assets.sh).
Stdlib only — runs with system python3 or the venv alike.

Usage:
    python3 check_truth.py <perception.json> <truth.json>

Assertions (rationale inline):
  1. SILENCES — every truth-inserted silence (exact by construction) is
     covered >= 70% by detected silence. Extra detected silences are ALLOWED:
     espeak's intra-segment sentence pauses legitimately register.
  2. FILLERS — each filler type (um/uh/so) is found at least once, and >= 60%
     of all instances are found inside their segment window (+-1s). Not 100%:
     ASR models are trained on cleaned transcripts and legitimately drop some
     fillers; the wedge product only needs to find MOST of them.
  3. SCENE — a detected scene cut within +-500ms of the known hard cut.
  4. LOUDNESS — integrated LUFS is a finite negative number; true peak <= 0.
  5. WORDS — >= 60% of the unique script vocabulary appears in the transcript
     (espeak articulation + whisper small is imperfect; 60% proves the
     pipeline transcribes THIS clip, not noise).
  6. BEATS — beat grid present with bpm > 0 (speech has no musical beat, but
     librosa always fits a grid; presence proves the instrument ran).

Exit 0 = all assertions pass; exit 1 with a FAIL list otherwise.
Primary callers: scripts/e2e.sh, perception track verification.
"""

import json
import re
import sys

failures: list = []
notes: list = []


def check(name: str, ok: bool, detail: str) -> None:
    """Record one assertion result and print it."""
    tag = "PASS" if ok else "FAIL"
    print(f"  [{tag}] {name}: {detail}")
    if not ok:
        failures.append(name)


def norm(word: str) -> str:
    """Lowercase, strip punctuation — ASR words come back like 'Um,'."""
    return re.sub(r"[^a-z']", "", word.lower())


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    with open(sys.argv[1]) as f:
        report = json.load(f)
    with open(sys.argv[2]) as f:
        truth = json.load(f)

    print(f"check_truth: report={sys.argv[1]} truth={sys.argv[2]}")

    # --- 1. silences -------------------------------------------------------
    detected = report.get("silences", [])
    for i, t in enumerate(truth["inserted_silences_ms"]):
        t_len = t["end_ms"] - t["start_ms"]
        covered = 0
        for d in detected:
            inter = min(t["end_ms"], d["end_ms"]) - max(t["start_ms"], d["start_ms"])
            covered += max(0, inter)
        frac = covered / t_len if t_len else 0
        check(
            f"silence[{i}]",
            frac >= 0.7,
            f"truth {t['start_ms']}-{t['end_ms']}ms covered {frac:.0%} by detection",
        )

    # --- 2. fillers ---------------------------------------------------------
    words = (report.get("words") or {}).get("words", [])
    found_total, want_total = 0, 0
    found_types: set = set()
    for seg in truth["segments"]:
        budget = list(seg["fillers"])  # instances expected in this segment
        want_total += len(budget)
        lo, hi = seg["start_ms"] - 1000, seg["end_ms"] + 1000
        for w in words:
            nw = norm(w["word"])
            mid = (w["start_ms"] + w["end_ms"]) // 2
            if nw in budget and lo <= mid <= hi:
                budget.remove(nw)
                found_total += 1
                found_types.add(nw)
    if want_total:
        frac = found_total / want_total
        check("fillers.ratio", frac >= 0.6,
              f"{found_total}/{want_total} filler instances located in-window")
        for f_type in truth["filler_lexicon"]:
            expected = any(f_type in s["fillers"] for s in truth["segments"])
            if expected:
                check(f"fillers.type.{f_type}", f_type in found_types,
                      f"'{f_type}' {'found' if f_type in found_types else 'NOT found'}")

    # --- 3. scene cut -------------------------------------------------------
    scene_truth = truth["scene_change_ms"]
    scenes = report.get("scenes", [])
    nearest = min((abs(s["at_ms"] - scene_truth) for s in scenes), default=None)
    check("scene_cut", nearest is not None and nearest <= 500,
          f"known cut at {scene_truth}ms; nearest detection delta="
          f"{nearest if nearest is not None else 'none detected'}ms "
          f"({len(scenes)} cuts total)")
    if len(scenes) > 1:
        notes.append(f"note: {len(scenes)} scene cuts detected (1 expected) — "
                     "testsrc patterns can over-trigger; only the known cut is asserted")

    # --- 4. loudness ---------------------------------------------------------
    loud = report.get("loudness")
    check("loudness", bool(loud)
          and -70 < loud["integrated_lufs"] < 0
          and loud["true_peak_dbtp"] <= 0,
          f"integrated={loud and loud.get('integrated_lufs')} LUFS, "
          f"true_peak={loud and loud.get('true_peak_dbtp')} dBTP")

    # --- 5. words vocabulary -------------------------------------------------
    vocab = {norm(w) for seg in truth["segments"] for w in seg["text"].split()}
    vocab.discard("")
    heard = {norm(w["word"]) for w in words}
    hit = len(vocab & heard) / len(vocab) if vocab else 0
    check("words.vocab", hit >= 0.6,
          f"{hit:.0%} of {len(vocab)} unique script words transcribed "
          f"(engine: {(report.get('words') or {}).get('model')})")

    # --- 6. beats -------------------------------------------------------------
    beats = report.get("beats")
    check("beats", bool(beats) and beats.get("bpm", 0) > 0 and beats.get("beats_ms"),
          f"bpm={beats and beats.get('bpm')}, {len((beats or {}).get('beats_ms', []))} beats")

    for n in notes:
        print(n)
    if failures:
        print(f"RESULT: FAIL ({len(failures)}): {failures}")
        return 1
    print("RESULT: PASS — perception output matches ground truth")
    return 0


if __name__ == "__main__":
    sys.exit(main())
