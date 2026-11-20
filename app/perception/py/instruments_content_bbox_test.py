"""Contract tests for content_bbox (the uniform_border check's measurement).

A border is what every sample of the clip agrees on: a baked-in letterbox or
pillarbox is detected; a scene whose edge is dark for a while, a vignette, or a
single flash frame is not a border (row 214: a real clip read a 48 px border
that FFmpeg's own whole-clip reading does not find).

Fixtures are made with FFmpeg in a temporary folder; run from this folder:
    python3 -m unittest instruments_content_bbox_test
"""
import os
import shutil
import subprocess
import tempfile
import unittest

import instruments
from instruments import CONTENT_BBOX_EDGE_TOL_PX, detect_content_bbox

HAVE_FFMPEG = bool(shutil.which("ffmpeg") and shutil.which("ffprobe"))
SRC = "testsrc2=size={w}x{h}:rate=25:duration=8"


def make(path, vf, w=640, h=360):
    """An 8 s 640x360 clip: testsrc2 at w x h, then the filter chain `vf`."""
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", SRC.format(w=w, h=h),
         "-vf", vf, "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", path],
        check=True,
    )
    return path


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg/ffprobe not installed")
class ContentBboxTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp(prefix="cut-bbox-")
        d = cls.dir
        cls.full = make(os.path.join(d, "full.mp4"), "null")
        cls.letterbox = make(os.path.join(d, "letterbox.mp4"),
                             "pad=640:360:0:44:black", w=640, h=272)
        cls.pillarbox = make(os.path.join(d, "pillarbox.mp4"),
                             "pad=640:360:80:0:black", w=480, h=360)
        cls.vignette = make(os.path.join(d, "vignette.mp4"), "vignette")
        # The real-clip case: the right edge is dark for 2 s only (one sample).
        cls.dark_edge = make(os.path.join(d, "dark_edge.mp4"),
                             "drawbox=x=592:y=0:w=48:h=360:color=black:t=fill:"
                             "enable='between(t,2.8,4.9)'")
        # A letterboxed clip with a white flash over the whole frame.
        cls.flash = make(os.path.join(d, "flash.mp4"),
                         "pad=640:360:0:44:black,drawbox=x=0:y=0:w=640:h=360:"
                         "color=white:t=fill:enable='between(t,1.6,1.68)'", w=640, h=272)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def bbox(self, path):
        box = detect_content_bbox(path)
        self.assertIsNotNone(box)
        return box

    def inset(self, box):
        return max(box["x"], box["y"], box["frame_width"] - box["x"] - box["width"],
                   box["frame_height"] - box["y"] - box["height"])

    def test_full_frame_has_no_border(self):
        box = self.bbox(self.full)
        self.assertFalse(box["uniform_border"], box)

    def test_a_letterbox_is_a_border(self):
        box = self.bbox(self.letterbox)
        self.assertTrue(box["uniform_border"], box)
        self.assertGreaterEqual(box["y"], 44 - CONTENT_BBOX_EDGE_TOL_PX, box)

    def test_a_pillarbox_is_a_border(self):
        box = self.bbox(self.pillarbox)
        self.assertTrue(box["uniform_border"], box)
        self.assertGreaterEqual(box["x"], 80 - CONTENT_BBOX_EDGE_TOL_PX, box)

    def test_a_vignette_is_not_a_border(self):
        box = self.bbox(self.vignette)
        self.assertFalse(box["uniform_border"], box)

    def test_an_edge_dark_for_a_while_is_not_a_border(self):
        box = self.bbox(self.dark_edge)
        self.assertFalse(box["uniform_border"], box)
        self.assertLessEqual(self.inset(box), CONTENT_BBOX_EDGE_TOL_PX, box)

    def test_a_flash_does_not_hide_a_letterbox(self):
        box = self.bbox(self.flash)
        self.assertTrue(box["uniform_border"], box)


if __name__ == "__main__":
    unittest.main()
