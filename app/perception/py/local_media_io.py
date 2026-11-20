"""Restrict media decoders to local FFmpeg protocols, including nested inputs."""

import os
import stat
import threading

# Windows looks for a program in the current folder before PATH, both in
# shutil.which and when a bare name is started, and a relative or empty PATH
# entry means the current folder everywhere. The current folder may be a
# project from someone else, so this process (and its children) opts out of
# both.
if os.name == "nt":
    os.environ["NoDefaultCurrentDirectoryInExePath"] = "1"
os.environ["PATH"] = os.pathsep.join(
    entry for entry in os.environ.get("PATH", "").split(os.pathsep) if entry and os.path.isabs(entry))


LOCAL_PROTOCOLS = "file,pipe,crypto,data"
# Keep in sync with cut-media ffmpeg::LOCAL_INPUT_FORMATS. Demuxer admission
# happens before opening a playlist and therefore before reading nested files.
LOCAL_FORMATS = "mov,matroska,webm,avi,asf,mpeg,mpegts,mpegtsraw,mxf,flv,ogg,flac,wav,aiff,mp3,aac,ac3,eac3,dts,amr,au,caf,wv,ape,tta,mpc,mpc8,loas,png_pipe,jpeg_pipe,jpegls_pipe,bmp_pipe,webp_pipe,tiff_pipe,jpegxl_pipe,exr_pipe,dpx_pipe,ppm_pipe,pgm_pipe,pbm_pipe,pam_pipe,pcx_pipe,tga_pipe,image2,gif,apng,ico,rawvideo,s16le,lavfi"
_CAPTURE_LOCK = threading.Lock()


def program_on_path(name: str, extra_dirs: tuple[str, ...] = ()) -> str | None:
    """Absolute path of program ``name`` in ``extra_dirs`` or an absolute PATH
    entry, or None. Never the current folder, never a relative PATH entry."""
    names = [name]
    if os.name == "nt" and not os.path.splitext(name)[1]:
        extensions = os.environ.get("PATHEXT", ".COM;.EXE;.BAT;.CMD").split(";")
        names = [name + ext for ext in extensions if ext] + [name]
    for directory in (*extra_dirs, *os.environ.get("PATH", "").split(os.pathsep)):
        if not directory or not os.path.isabs(directory):
            continue
        for candidate in names:
            path = os.path.join(directory, candidate)
            try:
                info = os.stat(path)
            except OSError:
                continue
            if stat.S_ISREG(info.st_mode) and (os.name == "nt" or os.access(path, os.X_OK)):
                return path
    return None


def media_tool(name: str) -> str:
    """ffmpeg or ffprobe in Cut's order: the explicit SHELLX_CUT_FFMPEG /
    SHELLX_CUT_FFPROBE file, SHELLX_CUT_FFMPEG_DIR (and its bin/), then PATH.
    Only absolute paths count. A tool found nowhere is returned as its bare
    name, which the current folder cannot supply (see the opt-out above)."""
    explicit = os.environ.get("SHELLX_CUT_FFMPEG" if name == "ffmpeg" else "SHELLX_CUT_FFPROBE")
    if explicit and os.path.isabs(explicit) and os.path.isfile(explicit):
        return explicit
    directory = os.environ.get("SHELLX_CUT_FFMPEG_DIR", "").strip()
    dirs = (directory, os.path.join(directory, "bin")) if os.path.isabs(directory) else ()
    return program_on_path(name, dirs) or name


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
