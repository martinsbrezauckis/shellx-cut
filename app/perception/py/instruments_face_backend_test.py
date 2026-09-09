#!/usr/bin/env python3
"""Unit tests for the Director face-enhancement backend selection.

These do not import native MediaPipe or OpenCV. They prove the macOS branch
selects the already-shipped YuNet contract before any MediaPipe import, maps
YuNet's documented pixel boxes to the shared framing tuple, and reports an
honest unavailable fallback. A native macOS Director run remains the required
runtime qualification for the packaged OpenCV/model pair.
"""
from __future__ import annotations

import contextlib
import importlib.util
import io
import sys
import types
import unittest
from pathlib import Path
from unittest import mock

MODULE_PATH = Path(__file__).with_name("instruments.py")


def load_instruments():
    spec = importlib.util.spec_from_file_location("cut_instruments_face_backend", MODULE_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeDetector:
    def __init__(self):
        self.input_sizes = []

    def setInputSize(self, size):
        self.input_sizes.append(size)

    def detect(self, frame):
        return None, [[10.0, 20.0, 30.0, 40.0, *([0.0] * 11)]]


class FakeFaceDetectorYN:
    detector = None
    create_args = None

    @classmethod
    def create(cls, *args):
        cls.create_args = args
        cls.detector = FakeDetector()
        return cls.detector


class FakeFrame:
    shape = (480, 640, 3)


class InstrumentsFaceBackendTest(unittest.TestCase):
    def setUp(self):
        self.module = load_instruments()

    def test_macos_uses_bundled_yunet_and_maps_shared_face_tuple(self):
        fake_cv2 = types.SimpleNamespace(FaceDetectorYN=FakeFaceDetectorYN)
        with (
            mock.patch.object(self.module.sys, "platform", "darwin"),
            mock.patch.dict(sys.modules, {"cv2": fake_cv2}),
        ):
            face = self.module._load_face_detector()
            self.assertIsNotNone(face)
            self.assertEqual(face[0], "opencv-yunet")
            self.assertTrue(Path(FakeFaceDetectorYN.create_args[0]).is_file())
            self.assertEqual(self.module._face_backend(face), "opencv-yunet")
            self.assertEqual(
                self.module._detect_faces(face, FakeFrame()),
                [(25.0, 40.0, 10.0, 20.0, 40.0, 60.0)],
            )
            self.assertEqual(FakeFaceDetectorYN.detector.input_sizes, [(640, 480)])

    def test_macos_model_absence_is_explicit_body_saliency_fallback(self):
        missing = Path("/definitely-missing-face-model.onnx")
        stderr = io.StringIO()
        with (
            mock.patch.object(self.module.sys, "platform", "darwin"),
            mock.patch.object(self.module, "_yunet_model_path", return_value=missing),
            contextlib.redirect_stderr(stderr),
        ):
            face = self.module._load_face_detector()
        self.assertIsNone(face)
        self.assertEqual(self.module._face_backend(face), "unavailable")
        self.assertIn("YuNet model unavailable; body/saliency framing only", stderr.getvalue())

    def test_non_macos_retains_the_mediapipe_loader(self):
        sentinel = ("mediapipe-blaze-face", object(), object())
        with (
            mock.patch.object(self.module.sys, "platform", "linux"),
            mock.patch.object(self.module, "_load_mediapipe_face_detector", return_value=sentinel) as loader,
        ):
            self.assertIs(self.module._load_face_detector(), sentinel)
            loader.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
