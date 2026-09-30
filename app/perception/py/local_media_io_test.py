"""Regression checks for media decoder protocol boundaries."""

import os
import importlib.util
import shutil
import subprocess
import tempfile
import unittest
import wave
from pathlib import Path
from unittest.mock import patch

from local_media_io import (
    LOCAL_PROTOCOLS, LOCAL_FORMATS, input_args, protect_inputs,
    video_capture, movie_format_options,
)


class LocalMediaIoTests(unittest.TestCase):
    @unittest.skipUnless(importlib.util.find_spec("cv2") and shutil.which("ffmpeg"),
                         "OpenCV/FFmpeg unavailable")
    def test_real_opencv_keeps_mp4_and_refuses_nested_local_playlist(self):
        import cv2
        sample = Path(__file__).resolve().parents[2] / "server/assets/first-edit-sample.mp4"
        before = os.environ.get("OPENCV_FFMPEG_CAPTURE_OPTIONS")
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            segment = root / "frames.ts"
            subprocess.run(["ffmpeg", "-v", "error", "-i", str(sample), "-c", "copy",
                            "-f", "mpegts", str(segment)],
                           check=True, capture_output=True, timeout=10)
            playlist = root / "local.m3u8"
            playlist.write_text("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:60\n"
                                "#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:60.0,\n"
                                "frames.ts\n#EXT-X-ENDLIST\n")
            control = cv2.VideoCapture(str(playlist), cv2.CAP_FFMPEG)
            try:
                self.assertTrue(control.isOpened())
                self.assertTrue(control.read()[0], "unrestricted control must actually decode")
            finally:
                control.release()
            restricted = video_capture(cv2, str(playlist))
            try:
                self.assertFalse(restricted.isOpened())
            finally:
                restricted.release()
            ordinary = video_capture(cv2, str(sample))
            try:
                self.assertTrue(ordinary.isOpened())
                self.assertEqual(ordinary.getBackendName(), "FFMPEG")
                self.assertTrue(ordinary.read()[0])
            finally:
                ordinary.release()
        self.assertEqual(os.environ.get("OPENCV_FFMPEG_CAPTURE_OPTIONS"), before)

    def test_every_input_gets_its_own_whitelist(self):
        args = protect_inputs(["-i", "first.mp4", "-i", "second.wav"])
        self.assertEqual(args, [
            "-protocol_whitelist", LOCAL_PROTOCOLS, "-format_whitelist", LOCAL_FORMATS, "-i", "first.mp4",
            "-protocol_whitelist", LOCAL_PROTOCOLS, "-format_whitelist", LOCAL_FORMATS, "-i", "second.wav",
        ])
        self.assertEqual(protect_inputs(input_args("clip.mp4")), input_args("clip.mp4"))

    def test_opencv_forces_ffmpeg_and_restores_environment(self):
        class Capture:
            def isOpened(self): return True
            def getBackendName(self): return "FFMPEG"

        class Cv2:
            CAP_FFMPEG = 1900

            @staticmethod
            def VideoCapture(path, backend):
                self.assertEqual((path, backend), ("clip.mp4", Cv2.CAP_FFMPEG))
                self.assertEqual(os.environ["OPENCV_FFMPEG_CAPTURE_OPTIONS"],
                                 f"protocol_whitelist;{LOCAL_PROTOCOLS}|format_whitelist;{LOCAL_FORMATS}")
                return Capture()

        with patch.dict(os.environ, {"OPENCV_FFMPEG_CAPTURE_OPTIONS": "prior"}):
            video_capture(Cv2, "clip.mp4")
            self.assertEqual(os.environ["OPENCV_FFMPEG_CAPTURE_OPTIONS"], "prior")

    def test_opencv_rejects_a_different_backend(self):
        class Capture:
            released = False
            def isOpened(self): return True
            def getBackendName(self): return "OTHER"
            def release(self): self.released = True

        capture = Capture()
        class Cv2:
            CAP_FFMPEG = 1900
            @staticmethod
            def VideoCapture(path, backend): return capture

        with self.assertRaisesRegex(RuntimeError, "FFmpeg backend"):
            video_capture(Cv2, "clip.mp4")
        self.assertTrue(capture.released)

    @unittest.skipUnless(shutil.which("ffprobe"), "ffprobe not installed")
    def test_ffprobe_reads_local_wav_and_blocks_nested_http(self):
        with tempfile.TemporaryDirectory() as td:
            wav = Path(td) / "local.wav"
            with wave.open(str(wav), "wb") as out:
                out.setnchannels(1)
                out.setsampwidth(2)
                out.setframerate(8000)
                out.writeframes(b"\0\0" * 800)
            command = ["ffprobe", "-v", "error", "-show_entries",
                       "format=duration", "-of", "default=nw=1:nk=1"]
            result = subprocess.run(command + input_args(str(wav)),
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)

            playlist = Path(td) / "nested.m3u8"
            playlist.write_text("#EXTM3U\n#EXT-X-VERSION:3\n"
                                "#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n"
                                "#EXTINF:1.0,\n"
                                "http://127.0.0.1:9/segment.ts\n#EXT-X-ENDLIST\n")
            result = subprocess.run(command + input_args(str(playlist)),
                                    capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("hls", result.stderr.lower())
            self.assertIn("whitelist", result.stderr.lower())

            if shutil.which("ffmpeg"):
                command = ["ffmpeg", "-v", "error"]
                result = subprocess.run(command + input_args(str(wav)) +
                                        ["-f", "null", "-"],
                                        capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 0, result.stderr)
                result = subprocess.run(command + input_args(str(playlist)) +
                                        ["-f", "null", "-"],
                                        capture_output=True, text=True, timeout=5)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("whitelist", result.stderr.lower())

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
    def test_whitelisted_stdin_encodes_matte_frame(self):
        with tempfile.TemporaryDirectory() as td:
            output = Path(td) / "alpha.mkv"
            command = ["ffmpeg", "-v", "error", "-y", "-f", "rawvideo",
                       "-pix_fmt", "gray", "-s", "2x2", "-framerate", "1",
                       *input_args("pipe:0"), "-frames:v", "1", "-c:v", "ffv1",
                       str(output)]
            result = subprocess.run(command, input=b"\0\x7f\xff\x40",
                                    capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", "replace"))
            self.assertGreater(output.stat().st_size, 0)

    @unittest.skipUnless(shutil.which("ffprobe") and shutil.which("ffmpeg"), "FFmpeg unavailable")
    def test_movie_nested_demuxer_rejects_local_playlist_and_keeps_audio(self):
        with tempfile.TemporaryDirectory() as td:
            private = Path(td) / "private.ts"
            result = subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi",
                                     "-i", "sine=duration=1", "-c:a", "aac",
                                     "-f", "mpegts", str(private)],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            playlist = Path(td) / "incoming.m3u8"
            playlist.write_text("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n"
                                f"#EXTINF:1.0,\n{private}\n#EXT-X-ENDLIST\n")
            for path, allowed in [(private, True), (playlist, False)]:
                graph = f"amovie={path}:{movie_format_options()},astats=metadata=1"
                result = subprocess.run(["ffprobe", "-v", "error", "-f", "lavfi",
                                         *input_args(graph), "-show_entries",
                                         "frame_tags=lavfi.astats.Overall.RMS_level",
                                         "-of", "csv=p=0"],
                                        capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode == 0, allowed, result.stderr)
                if allowed:
                    self.assertTrue(result.stdout.strip())
                else:
                    self.assertIn("whitelist", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
