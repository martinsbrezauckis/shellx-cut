"""Contract tests for the per-frame audio energy envelope used by subject framing.

The envelope must equal the one computed from the whole decoded signal at once
(the reference below), and it must be computed from a bounded stream so memory does
not grow with the length of the audio."""
import importlib.util
import math
import os
import shutil
import subprocess
import tempfile
import tracemalloc
import unittest
from unittest.mock import patch

HAVE_NUMPY = importlib.util.find_spec("numpy") is not None
if HAVE_NUMPY:
    import numpy as np

import instruments
from instruments import _audio_energy_envelope, _iter_decoded_audio, _rms_envelope

HAVE_FFMPEG = bool(shutil.which("ffmpeg") and shutil.which("ffprobe"))
NUMPY_REASON = "numpy not installed (install app/perception/py/requirements.txt)"
SR = 8000


def reference_envelope(pcm: bytes, fps: float):
    """The envelope computed from the whole signal held in one array."""
    samples = np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0
    if samples.size == 0:
        return None
    spf = SR / fps
    n = int(math.ceil(samples.size / spf))
    env = np.zeros(n, dtype=np.float32)
    for i in range(n):
        seg = samples[int(i * spf):int((i + 1) * spf)]
        if seg.size:
            env[i] = instruments.finite_number(np.sqrt(np.mean(seg * seg)))
    p95 = instruments.finite_number(np.percentile(env, 95)) if env.size else 0.0
    if p95 > 1e-5:
        env = np.clip(env / p95, 0.0, 1.0)
    return env


def decode_all(path: str) -> bytes:
    return subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(SR),
         "-f", "s16le", "-"], capture_output=True, check=True).stdout


def split(data: bytes, size: int):
    for i in range(0, len(data), size):
        yield data[i:i + size]


def synthetic_pcm(seconds: float) -> bytes:
    t = np.arange(int(seconds * SR)) / SR
    wave = 9000 * np.sin(2 * np.pi * 220 * t) * (0.5 + 0.5 * np.sin(2 * np.pi * 0.7 * t))
    return wave.astype("<i2").tobytes()


@unittest.skipUnless(HAVE_NUMPY, NUMPY_REASON)
class ChunkedEnvelopeTests(unittest.TestCase):
    def test_every_chunk_size_gives_the_whole_signal_result(self):
        pcm = synthetic_pcm(3.37)
        for fps in (24.0, 25.0, 29.97, 30.0, 60.0):
            expected = reference_envelope(pcm, fps)
            for size in (2, 3, 7, 1000, 4097, 16000, len(pcm), len(pcm) * 2):
                with self.subTest(fps=fps, chunk=size):
                    got = _rms_envelope(split(pcm, size), SR, fps)
                    self.assertEqual(got.dtype, np.float32)
                    self.assertEqual(got.tobytes(), expected.tobytes())

    def test_silence_stays_unnormalized_zeros(self):
        pcm = bytes(SR * 2 * 2)
        got = _rms_envelope(split(pcm, 5000), SR, 30.0)
        self.assertEqual(got.tobytes(), reference_envelope(pcm, 30.0).tobytes())
        self.assertFalse(got.any())

    def test_no_samples_gives_none(self):
        self.assertIsNone(_rms_envelope(iter(()), SR, 30.0))
        self.assertIsNone(_rms_envelope(iter((b"", b"")), SR, 30.0))

    def test_memory_does_not_grow_with_the_length_of_the_stream(self):
        piece = synthetic_pcm(4.0)[:64 * 1024]
        peaks = {}
        for pieces in (50, 500):  # about 1.6 MB and 16 MB of PCM
            tracemalloc.start()
            try:
                _rms_envelope((piece for _ in range(pieces)), SR, 30.0)
                peaks[pieces] = tracemalloc.get_traced_memory()[1]
            finally:
                tracemalloc.stop()
        # The 16 MB stream fits in well under 2 MB of working memory; holding the
        # decoded signal as float32 would need about 32 MB for the samples alone.
        self.assertLess(peaks[500], 2 * 1024 * 1024, peaks)
        self.assertLess(peaks[500], peaks[50] * 2 + 512 * 1024, peaks)


@unittest.skipUnless(HAVE_NUMPY and HAVE_FFMPEG, "numpy or FFmpeg unavailable")
class DecodedAudioEnvelopeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.root = cls.tmp.name
        cls.files = {}
        specs = {
            "tone.wav": ["-f", "lavfi", "-i", "sine=frequency=440:duration=3"],
            "silence.wav": ["-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-t", "2"],
            "swell.mp3": ["-f", "lavfi", "-i", "sine=f=300:d=6,tremolo=f=0.7:d=0.9",
                          "-ac", "2", "-b:a", "48k"],
        }
        for name, args in specs.items():
            path = os.path.join(cls.root, name)
            subprocess.run(["ffmpeg", "-v", "error", "-y", *args, path], check=True)
            cls.files[name] = path

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_envelope_matches_the_whole_signal_result(self):
        for name, path in self.files.items():
            pcm = decode_all(path)
            for fps in (24.0, 29.97, 30.0):
                with self.subTest(file=name, fps=fps):
                    got = _audio_energy_envelope(path, fps)
                    self.assertEqual(got.tobytes(), reference_envelope(pcm, fps).tobytes())

    def test_small_pieces_from_the_real_decoder_match(self):
        path = self.files["swell.mp3"]
        expected = reference_envelope(decode_all(path), 25.0)
        got = _rms_envelope(_iter_decoded_audio(path, SR, 4099), SR, 25.0)
        self.assertEqual(got.tobytes(), expected.tobytes())

    def test_file_without_audio_gives_none(self):
        video = os.path.join(self.root, "silent.mp4")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
                        "color=c=black:s=64x64:d=1", "-pix_fmt", "yuv420p", video],
                       check=True)
        self.assertIsNone(_audio_energy_envelope(video, 30.0))

    def test_unreadable_decode_gives_none_and_bad_fps_gives_none(self):
        broken = os.path.join(self.root, "broken.wav")
        with open(broken, "wb") as f:
            f.write(b"not audio")
        self.assertIsNone(_audio_energy_envelope(broken, 30.0))
        self.assertIsNone(_audio_energy_envelope(self.files["tone.wav"], 0.0))

    def test_decoder_failure_is_an_error_even_after_output(self):
        with patch.object(instruments, "FFMPEG_BIN", shutil.which("python3")), \
                patch.object(instruments, "input_args", lambda p: [p]):
            # python3 reads the "file" argument path as a script that prints
            # samples and then exits with an error status.
            script = os.path.join(self.root, "fail.py")
            with open(script, "w") as f:
                f.write("import sys\nsys.stdout.buffer.write(bytes(4000))\n"
                        "sys.stdout.buffer.flush()\nsys.exit(3)\n")
            with patch.object(instruments, "has_audio_stream", lambda p: True):
                self.assertIsNone(_audio_energy_envelope(script, 30.0))
                with self.assertRaises(RuntimeError):
                    list(_iter_decoded_audio(script, SR))

    def test_stopping_early_stops_and_reaps_the_decoder(self):
        started = []
        real = subprocess.Popen

        def spy(*args, **kwargs):
            proc = real(*args, **kwargs)
            started.append(proc)
            return proc

        path = self.files["swell.mp3"]
        with patch.object(instruments.subprocess, "Popen", spy):
            pieces = _iter_decoded_audio(path, SR, 1024)
            next(pieces)
            pieces.close()
        self.assertEqual(len(started), 1)
        self.assertIsNotNone(started[0].returncode)
        self.assertTrue(started[0].stdout.closed)


if __name__ == "__main__":
    unittest.main()
