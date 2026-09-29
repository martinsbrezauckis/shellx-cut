"""Restrict media decoders to local FFmpeg protocols, including nested inputs."""

import os
import threading


LOCAL_PROTOCOLS = "file,pipe,crypto,data"
_CAPTURE_LOCK = threading.Lock()


def input_args(path: str) -> list[str]:
    """FFmpeg/FFprobe options scoped to the immediately following input."""
    return ["-protocol_whitelist", LOCAL_PROTOCOLS, "-i", path]


def protect_inputs(args: list[str]) -> list[str]:
    """Add an input-local whitelist to commands assembled by instrument callers."""
    protected = []
    for arg in args:
        if arg == "-i" and protected[-2:] != ["-protocol_whitelist", LOCAL_PROTOCOLS]:
            protected.extend(("-protocol_whitelist", LOCAL_PROTOCOLS))
        protected.append(arg)
    return protected


def video_capture(cv2, path: str):
    """Open through FFmpeg only; never fall back to another decoder backend."""
    key = "OPENCV_FFMPEG_CAPTURE_OPTIONS"
    with _CAPTURE_LOCK:
        previous = os.environ.get(key)
        os.environ[key] = f"protocol_whitelist;{LOCAL_PROTOCOLS}"
        try:
            capture = cv2.VideoCapture(path, cv2.CAP_FFMPEG)
            if capture.isOpened() and capture.getBackendName() != "FFMPEG":
                capture.release()
                raise RuntimeError("OpenCV did not use the FFmpeg backend")
            return capture
        finally:
            if previous is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = previous


def scene_video(path: str):
    """Make PySceneDetect use the guarded OpenCV FFmpeg capture exclusively."""
    import cv2
    from scenedetect.backends.opencv import VideoCaptureAdapter

    capture = video_capture(cv2, path)
    if not capture.isOpened():
        capture.release()
        raise RuntimeError(f"cannot open local video: {path}")
    try:
        return VideoCaptureAdapter(capture)
    except Exception:
        capture.release()
        raise
