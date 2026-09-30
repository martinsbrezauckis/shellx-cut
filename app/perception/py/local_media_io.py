"""Restrict media decoders to local FFmpeg protocols, including nested inputs."""

import os
import threading


LOCAL_PROTOCOLS = "file,pipe,crypto,data"
# Keep in sync with cut-media ffmpeg::LOCAL_INPUT_FORMATS. Demuxer admission
# happens before opening a playlist and therefore before reading nested files.
LOCAL_FORMATS = "mov,matroska,webm,avi,asf,mpeg,mpegts,mpegtsraw,mxf,flv,ogg,flac,wav,aiff,mp3,aac,ac3,eac3,dts,amr,au,caf,wv,ape,tta,mpc,mpc8,loas,png_pipe,jpeg_pipe,jpegls_pipe,bmp_pipe,webp_pipe,tiff_pipe,jpegxl_pipe,exr_pipe,dpx_pipe,ppm_pipe,pgm_pipe,pbm_pipe,pam_pipe,pcx_pipe,tga_pipe,image2,gif,apng,ico,rawvideo,s16le,lavfi"
_CAPTURE_LOCK = threading.Lock()


def input_args(path: str) -> list[str]:
    """FFmpeg/FFprobe options scoped to the immediately following input."""
    return ["-protocol_whitelist", LOCAL_PROTOCOLS,
            "-format_whitelist", LOCAL_FORMATS, "-i", path]


def movie_format_options() -> str:
    """Options for the nested demuxer opened by lavfi movie/amovie filters."""
    return (f"format_opts='protocol_whitelist={LOCAL_PROTOCOLS}"
            f"\\:format_whitelist={LOCAL_FORMATS}'")


def protect_inputs(args: list[str]) -> list[str]:
    """Add an input-local whitelist to commands assembled by instrument callers."""
    protected = []
    for arg in args:
        if arg == "-i" and protected[-4:] != input_args("")[:-2]:
            protected.extend(input_args("")[:-2])
        protected.append(arg)
    return protected


def video_capture(cv2, path: str):
    """Open through FFmpeg only; never fall back to another decoder backend."""
    key = "OPENCV_FFMPEG_CAPTURE_OPTIONS"
    with _CAPTURE_LOCK:
        previous = os.environ.get(key)
        os.environ[key] = f"protocol_whitelist;{LOCAL_PROTOCOLS}|format_whitelist;{LOCAL_FORMATS}"
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
