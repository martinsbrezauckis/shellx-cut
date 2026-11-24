"""Actual sound fallback: real FFmpeg measures a WAV; optional model fails.
No package installation, model download, or optional native import is needed.
"""
import contextlib
import io
import json
import sys
import tempfile
import types
import unittest
import wave
from pathlib import Path
from unittest import mock
import instruments
import attempt_evidence


class SoundEvidence(unittest.TestCase):
    def test_failed_stream_probe_is_recorded_not_waived_as_no_audio(self):
        attempt_evidence.requested(["silence"])
        stderr = io.StringIO()
        result = types.SimpleNamespace(returncode=1, stdout="", stderr="bad stream headers")
        with mock.patch("subprocess.run", return_value=result), mock.patch("attempt_evidence.binary_version", return_value=("version 8", None)), contextlib.redirect_stderr(stderr):
            self.assertFalse(instruments.has_audio_stream("bad.wav"))
        events = [json.loads(line.removeprefix("CUT_ATTEMPT ")) for line in stderr.getvalue().splitlines() if line.startswith("CUT_ATTEMPT ")]
        self.assertTrue(any(e["state"] == "failed" and "bad stream headers" in e["error"]["cause"] for e in events))
        self.assertFalse(any(e["state"] == "not_applicable" for e in events))

    def test_confirmed_no_audio_is_recorded_as_inapplicable(self):
        attempt_evidence.requested(["silence"])
        stderr = io.StringIO()
        result = types.SimpleNamespace(returncode=0, stdout="", stderr="")
        with mock.patch("subprocess.run", return_value=result), mock.patch("attempt_evidence.binary_version", return_value=("version 8", None)), contextlib.redirect_stderr(stderr):
            self.assertFalse(instruments.has_audio_stream("silent-video.mp4"))
        events = [json.loads(line.removeprefix("CUT_ATTEMPT ")) for line in stderr.getvalue().splitlines() if line.startswith("CUT_ATTEMPT ")]
        self.assertTrue(any(e["provider"] == "perception" and e["state"] == "not_applicable" for e in events))

    def test_real_basic_measurement_survives_a_damaged_optional_model(self):
        damaged = types.ModuleType("silero_vad")
        def fail():
            raise RuntimeError("model file is damaged")
        damaged.load_silero_vad = fail
        damaged.get_speech_timestamps = lambda *args: []
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "silence.wav"
            with wave.open(str(path), "wb") as wav:
                wav.setnchannels(1)
                wav.setsampwidth(2)
                wav.setframerate(16000)
                wav.writeframes(b"\x00\x00" * 32000)
            stderr = io.StringIO()
            with mock.patch.dict(sys.modules, {"silero_vad": damaged,
                    "soundfile": types.ModuleType("soundfile"), "torch": types.ModuleType("torch")}):
                with contextlib.redirect_stderr(stderr):
                    spans = instruments.instrument_silence(str(path), str(path))
            self.assertTrue(any(s["source"] == "ffmpeg" and s["end_ms"] - s["start_ms"] >= 1000 for s in spans))
            events = [json.loads(line.removeprefix("CUT_ATTEMPT ")) for line in stderr.getvalue().splitlines() if line.startswith("CUT_ATTEMPT ")]
            self.assertTrue(any(e["provider"] == "ffmpeg.silencedetect" and e["state"] == "succeeded" and e["tool_version"] for e in events))
            self.assertTrue(any(e["provider"] == "silero" and e["state"] == "failed" and "model file is damaged" in e["error"]["cause"] for e in events))
            self.assertTrue(any(e["provider"] == "perception" and e["state"] == "succeeded" for e in events))

    def test_words_fallback_records_each_tried_engine_and_missing_vs_damaged(self):
        attempt_evidence.requested(["words"])
        attempt_evidence._FATAL = None
        stderr = io.StringIO()
        missing_torch = types.ModuleType("torch")
        missing_torch.cuda = types.SimpleNamespace(is_available=lambda: False)
        whisperx_mod = types.ModuleType("whisperx")
        whisperx_mod.load_model = lambda *a, **kw: (_ for _ in ()).throw(RuntimeError("whisperx model load failed"))
        class FakeSeg:
            words = [types.SimpleNamespace(word="hello", start=0.0, end=0.5, probability=0.9)]
        class FakeWhisperModel:
            def __init__(self, *a, **kw): pass
            def transcribe(self, *a, **kw): return ([FakeSeg()], types.SimpleNamespace(language="en"))
        fw_mod = types.ModuleType("faster_whisper")
        fw_mod.WhisperModel = FakeWhisperModel
        missing_onnx = mock.patch.dict(sys.modules, {"onnx_asr": None, "torch": missing_torch, "whisperx": whisperx_mod, "faster_whisper": fw_mod})
        with missing_onnx, mock.patch("instruments._decode_audio_16k_float", return_value=[0.0]), \
             mock.patch("attempt_evidence.package_version", side_effect=lambda name: (None, "not installed") if name == "onnx-asr" else ("1.0.0", None)), \
             contextlib.redirect_stderr(stderr):
            res = instruments.instrument_words("dummy.wav", "a1", "small")
        self.assertEqual(res["words"][0]["word"], "hello")
        events = [json.loads(line.removeprefix("CUT_ATTEMPT ")) for line in stderr.getvalue().splitlines() if line.startswith("CUT_ATTEMPT ")]
        self.assertTrue(any(e["instrument"] == "words" and e["provider"] == "parakeet" and e["state"] == "not_available" for e in events), events)
        self.assertTrue(any(e["instrument"] == "words" and e["provider"] == "whisperx@cpu" and e["state"] == "failed" and "whisperx model load failed" in e["error"]["cause"] for e in events), events)
        self.assertTrue(any(e["instrument"] == "words" and e["provider"] == "faster_whisper@cpu" and e["state"] == "succeeded" and e["tool_version"] == "1.0.0" for e in events), events)
        self.assertTrue(any(e["instrument"] == "words" and e["provider"] == "perception" and e["state"] == "succeeded" for e in events), events)

    def test_scenes_loudness_and_cropdetect_record_substeps_with_real_versions(self):
        attempt_evidence.requested(["scenes", "loudness"])
        attempt_evidence._FATAL = None
        stderr = io.StringIO()
        with tempfile.TemporaryDirectory() as directory:
            wav = Path(directory) / "tone.wav"
            with wave.open(str(wav), "wb") as handle:
                handle.setnchannels(1)
                handle.setsampwidth(2)
                handle.setframerate(16000)
                handle.writeframes(b"\x00\x00" * 16000)
            with mock.patch.dict(sys.modules, {"scenedetect": None}), \
                 mock.patch("attempt_evidence.package_version", side_effect=lambda name: (None, "not installed") if name == "scenedetect" else ("1.0.0", None)), \
                 mock.patch("instruments.run_ffmpeg", return_value=""), \
                 mock.patch("instruments.probe_video_geometry", return_value=(64, 64, 1000)), \
                 contextlib.redirect_stderr(stderr):
                instruments.instrument_scenes(str(wav))
                with attempt_evidence.attempt("scenes", "ffmpeg.cropdetect"):
                    instruments.detect_content_bbox(str(wav))
                with mock.patch("instruments.run_ffmpeg", return_value="I: -16.0 LUFS\nPeak: -1.5 dBFS"):
                    instruments.instrument_loudness(str(wav))
        events = [json.loads(line.removeprefix("CUT_ATTEMPT ")) for line in stderr.getvalue().splitlines() if line.startswith("CUT_ATTEMPT ")]
        self.assertTrue(any(e["instrument"] == "scenes" and e["provider"] == "pyscenedetect" and e["state"] == "not_available" for e in events), events)
        self.assertTrue(any(e["instrument"] == "scenes" and e["provider"] == "ffmpeg.select" and e["state"] == "succeeded" and e["tool_version"] for e in events), events)
        self.assertTrue(any(e["instrument"] == "scenes" and e["provider"] == "ffmpeg.blackdetect" and e["state"] == "succeeded" and e["tool_version"] for e in events), events)
        self.assertTrue(any(e["instrument"] == "scenes" and e["provider"] == "ffmpeg.cropdetect" and e["state"] == "succeeded" and e["tool_version"] for e in events), events)
        self.assertTrue(any(e["instrument"] == "loudness" and e["provider"] == "ffmpeg.ebur128" and e["state"] == "succeeded" and e["tool_version"] for e in events), events)


if __name__ == "__main__":
    unittest.main()
