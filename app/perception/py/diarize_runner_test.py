"""Contract tests for the speaker diarization runner: what it sends to the service
for ordinary audio, and that long audio is bounded instead of expanded without
limit. The service is a local stub; nothing leaves the machine."""
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import tracemalloc
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch

# The runner builds its HTTP opener at import time; keep loopback traffic direct.
os.environ["no_proxy"] = os.environ["NO_PROXY"] = "127.0.0.1,localhost"
for _name in ("http_proxy", "HTTP_PROXY", "https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"):
    os.environ.pop(_name, None)

import diarize_runner as runner

HAVE_FFMPEG = bool(shutil.which("ffmpeg") and shutil.which("ffprobe"))
TURNS = {"turns": [{"start_ms": 10, "end_ms": 900, "speaker": "S1"},
                   {"start_ms": 900, "end_ms": 1500, "speaker": "S2"}],
         "model": "sortformer-v2", "rtf": 0.01, "audio_s": 3.0, "infer_s": 0.03,
         "device": "cuda"}


class Stub:
    """Records every request; `/old` answers 307 to `/diarize`."""

    def __init__(self, keep_body=True):
        self.requests = []
        stub = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, code, payload, extra=None):
                data = json.dumps(payload).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                for k, v in (extra or {}).items():
                    self.send_header(k, v)
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                stub.requests.append(("GET", self.path, {k.lower(): v for k, v in self.headers.items()}, b""))
                self.reply(200, {"status": "ok", "loaded": True})

            def do_POST(self):
                if "chunked" in (self.headers.get("transfer-encoding") or ""):
                    body, size = b"", -1
                else:
                    left = int(self.headers.get("content-length") or 0)
                    size, parts = 0, []
                    while left > 0:
                        piece = self.rfile.read(min(left, 1 << 16))
                        if not piece:
                            break
                        left -= len(piece)
                        size += len(piece)
                        if keep_body:
                            parts.append(piece)
                    body = b"".join(parts)
                stub.requests.append(("POST", self.path, {k.lower(): v for k, v in self.headers.items()}, body, size))
                if self.path.startswith("/old"):
                    self.reply(307, {}, {"Location": "/diarize" + self.path[len("/old"):]})
                else:
                    self.reply(200, TURNS)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def reference_wav(media: str, out: str) -> bytes:
    """The WAV the extraction has always produced for a short file."""
    subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i", media,
         "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-f", "wav", out],
        check=True)
    with open(out, "rb") as f:
        return f.read()


def run_main(job: dict):
    stdout = io.StringIO()
    with patch.object(sys, "stdin", io.StringIO(json.dumps(job))), \
            patch.object(sys, "stdout", stdout):
        code = runner.main()
    return code, json.loads(stdout.getvalue())


@unittest.skipUnless(HAVE_FFMPEG, "FFmpeg unavailable")
class OrdinaryAudioTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.root = cls.tmp.name
        cls.tone = os.path.join(cls.root, "tone.m4a")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
                        "sine=f=330:d=3.5", "-ac", "2", "-b:a", "64k", cls.tone], check=True)
        cls.video = os.path.join(cls.root, "clip.mp4")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
                        "color=c=blue:s=64x64:d=2", "-f", "lavfi", "-i", "sine=f=200:d=2",
                        "-pix_fmt", "yuv420p", "-shortest", cls.video], check=True)
        cls.expected = reference_wav(cls.tone, os.path.join(cls.root, "ref.wav"))
        cls.stub = Stub()

    @classmethod
    def tearDownClass(cls):
        cls.stub.close()
        cls.tmp.cleanup()

    def setUp(self):
        self.stub.requests.clear()

    def test_request_for_a_short_file_is_the_same_wav_as_before(self):
        code, out = run_main({"endpoint": self.stub.url, "media": self.tone,
                              "max_speakers": 3, "secret": "s3cret", "timeout_s": 20})
        self.assertEqual(code, 0)
        self.assertEqual([r[0] for r in self.stub.requests], ["GET", "POST"])
        method, path, headers, body, size = self.stub.requests[1]
        self.assertEqual(path, "/diarize?max_speakers=3")
        self.assertEqual(body, self.expected)
        self.assertEqual(size, len(self.expected))
        self.assertEqual(headers.get("content-type"), "audio/wav")
        self.assertEqual(headers.get("content-length"), str(len(self.expected)))
        self.assertNotIn("transfer-encoding", headers)
        self.assertEqual(headers.get("x-lv-va-auth"), "s3cret")
        self.assertEqual(self.stub.requests[0][2].get("x-lv-va-auth"), "s3cret")
        self.assertEqual(out["turns"], TURNS["turns"])
        self.assertEqual(out["n_speakers"], 2)
        self.assertEqual(out["model"], "sortformer-v2")
        self.assertEqual(out["backend"], "sortformer")
        self.assertEqual(out["sample_rate"], 16000)
        self.assertEqual(out["device"], "cuda")

    def test_request_without_options_and_audio_from_a_video(self):
        run_main({"endpoint": self.stub.url, "media": self.video})
        _, path, headers, body, _ = self.stub.requests[1]
        self.assertEqual(path, "/diarize")
        self.assertNotIn("x-lv-va-auth", headers)
        expected = reference_wav(self.video, os.path.join(self.root, "ref2.wav"))
        self.assertEqual(body, expected)

    def test_a_redirected_upload_is_still_an_error(self):
        wav = os.path.join(self.root, "redirect.wav")
        with open(wav, "wb") as f:
            f.write(self.expected)
        with patch.object(runner, "service_url",
                          lambda endpoint, route, query=None: endpoint + "/old"):
            with self.assertRaises(RuntimeError) as ctx:
                runner.diarize_file(self.stub.url, wav, None, None, 20)
        self.assertIn("HTTP 307", str(ctx.exception))
        self.assertEqual([r[1] for r in self.stub.requests if r[0] == "POST"], ["/old"])

    def test_diarize_bytes_still_sends_the_given_bytes(self):
        resp = runner.diarize_bytes(self.stub.url, b"RIFFdata", 2, None, 20)
        self.assertEqual(resp["model"], "sortformer-v2")
        _, path, headers, body, _ = self.stub.requests[-1]
        self.assertEqual((path, body), ("/diarize?max_speakers=2", b"RIFFdata"))
        self.assertEqual(headers.get("content-length"), "8")

    def test_audio_over_the_limit_is_refused_before_decoding(self):
        with patch.object(runner, "MAX_AUDIO_SECONDS", 2), \
                patch.object(runner.subprocess, "run", wraps=runner.subprocess.run) as spy:
            with self.assertRaises(RuntimeError) as ctx:
                runner.extract_wav16k(self.tone, os.path.join(self.root, "never.wav"))
        message = str(ctx.exception)
        self.assertIn("too long", message)
        self.assertIn("0m 02s", message)  # the limit
        self.assertIn("0m 03s", message)  # what the asset holds
        self.assertFalse(os.path.exists(os.path.join(self.root, "never.wav")))
        self.assertTrue(all("ffmpeg" not in os.path.basename(c.args[0][0]) for c in spy.call_args_list))

    def test_decode_stops_at_the_limit_when_the_length_is_not_declared(self):
        out = os.path.join(self.root, "bounded.wav")
        with patch.object(runner, "MAX_AUDIO_SECONDS", 1), \
                patch.object(runner, "probe_audio_seconds", lambda media: None):
            with self.assertRaises(RuntimeError) as ctx:
                runner.extract_wav16k(self.tone, out)
            written = os.path.getsize(out)
        self.assertIn("too long", str(ctx.exception))
        # about one second over the one-second limit at 32000 bytes per second
        self.assertLess(written, 44 + 3 * 32000)
        self.assertLess(written, os.path.getsize(os.path.join(self.root, "ref.wav")))

    def test_audio_at_the_limit_is_accepted(self):
        out = os.path.join(self.root, "fits.wav")
        with patch.object(runner, "MAX_AUDIO_SECONDS", 4):
            runner.extract_wav16k(self.tone, out)
        with open(out, "rb") as f:
            self.assertEqual(f.read(), self.expected)

    def test_a_too_long_asset_fails_the_run_with_that_message(self):
        with patch.object(runner, "MAX_AUDIO_SECONDS", 1):
            with self.assertRaises(RuntimeError) as ctx:
                run_main({"endpoint": self.stub.url, "media": self.tone})
        self.assertIn("too long", str(ctx.exception))
        self.assertEqual(ctx.exception.exit_code, runner.EXIT_AUDIO_TOO_LONG)
        self.assertEqual([r[0] for r in self.stub.requests], ["GET"])

    def test_the_limit_is_two_hours_of_audio(self):
        self.assertEqual(runner.MAX_AUDIO_SECONDS, 7200)
        self.assertEqual(runner.max_wav_bytes(), 44 + 7200 * 32000)


class ExitStatusTests(unittest.TestCase):
    """The engine classifies a failure on the process exit status, never on text."""

    def run_entry(self, raised):
        stderr = io.StringIO()
        with patch.object(runner, "main", side_effect=raised), \
                patch.object(sys, "stderr", stderr):
            with self.assertRaises(SystemExit) as ctx:
                runner.entry()
        return ctx.exception.code, stderr.getvalue()

    def test_the_status_codes_are_distinct_and_stable(self):
        self.assertEqual(runner.EXIT_AUDIO_TOO_LONG, 3)
        self.assertEqual(runner.EXIT_NO_USABLE_AUDIO, 4)

    def test_a_too_long_asset_exits_with_its_own_status_and_the_plain_message(self):
        code, err = self.run_entry(runner._too_long(7300))
        self.assertEqual(code, runner.EXIT_AUDIO_TOO_LONG)
        self.assertTrue(err.startswith("this asset's audio is too long to label speakers (about 2h 01m 40s)"), err)
        self.assertIn("the limit is 2h 00m 00s of audio per run.", err)
        self.assertNotIn("Trim or split", err)  # the next step is the engine's suggested action
        self.assertNotIn("diarize_runner:", err)

    def test_an_asset_without_readable_audio_exits_with_its_own_status(self):
        code, err = self.run_entry(runner.AssetRefusal("this asset has no audio that can be read", runner.EXIT_NO_USABLE_AUDIO))
        self.assertEqual(code, runner.EXIT_NO_USABLE_AUDIO)
        self.assertEqual(err.strip(), "this asset has no audio that can be read")

    def test_a_service_failure_keeps_the_general_status(self):
        code, err = self.run_entry(RuntimeError("diarization service unreachable (http://x): refused"))
        self.assertEqual(code, 1)
        self.assertTrue(err.startswith("diarize_runner: diarization service unreachable"), err)

    def test_an_unexpected_failure_keeps_the_general_status(self):
        code, _ = self.run_entry(ValueError("boom"))
        self.assertEqual(code, 1)


@unittest.skipUnless(HAVE_FFMPEG, "FFmpeg unavailable")
class DecodeRefusalStatusTests(unittest.TestCase):
    def test_a_file_without_audio_is_refused_with_its_status(self):
        with tempfile.TemporaryDirectory() as td:
            silent = os.path.join(td, "picture.mp4")
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
                            "color=c=blue:s=64x64:d=1", "-pix_fmt", "yuv420p", silent], check=True)
            with self.assertRaises(runner.AssetRefusal) as ctx:
                runner.extract_wav16k(silent, os.path.join(td, "out.wav"))
        self.assertEqual(ctx.exception.exit_code, runner.EXIT_NO_USABLE_AUDIO)


class StreamedUploadTests(unittest.TestCase):
    def test_a_large_file_is_sent_without_being_held_in_memory(self):
        stub = Stub(keep_body=False)
        try:
            with tempfile.TemporaryDirectory() as td:
                path = os.path.join(td, "big.wav")
                total = 48 * 1024 * 1024
                with open(path, "wb") as f:
                    block = bytes(1 << 20)
                    for _ in range(total >> 20):
                        f.write(block)
                tracemalloc.start()
                try:
                    runner.diarize_file(stub.url, path, None, None, 60)
                    peak = tracemalloc.get_traced_memory()[1]
                finally:
                    tracemalloc.stop()
        finally:
            stub.close()
        post = [r for r in stub.requests if r[0] == "POST"][0]
        self.assertEqual(post[4], total)
        self.assertEqual(post[2].get("content-length"), str(total))
        self.assertLess(peak, 4 * 1024 * 1024, peak)


if __name__ == "__main__":
    unittest.main()
