"""Supplemental execution evidence. Never runs analysis, imports a model, or
changes a provider's returned value/exception. Version probes are lazy/bounded
and occur only inside an already-requested instrument call.
"""
from contextlib import contextmanager
from functools import lru_cache, wraps
from importlib import metadata
from itertools import count
import json
import platform
import subprocess
import sys
import time

_FATAL = None
_REQUESTED = ()
_PROBES = count(1)


def requested(instruments):
    global _REQUESTED
    _REQUESTED = tuple(instruments)


def fatal(code, message, cause):
    global _FATAL
    _FATAL = {"code": code, "message": message, "cause": cause}


@lru_cache(maxsize=16)
def package_version(name):
    try:
        return metadata.version(name), None
    except Exception as error:
        return None, str(error)[:768]


@lru_cache(maxsize=2)
def binary_version(tool):
    try:
        from local_media_io import media_tool
        result = subprocess.run([media_tool(tool), "-version"], capture_output=True,
                                text=True, timeout=3)
        if result.returncode != 0:
            raise RuntimeError(f"{tool} version probe exited {result.returncode}: {result.stderr[:256]}")
        banner = result.stdout.splitlines()[0].strip() if result.stdout else ""
        if not banner:
            raise RuntimeError(f"{tool} returned no version banner")
        return banner[:256], None
    except Exception as error:
        return None, str(error)[:768]


_OPTIONAL_PROVIDER_PACKAGES = {
    "silero": "silero-vad",
    "pyscenedetect": "scenedetect",
    "canary": "onnx-asr",
    "parakeet": "onnx-asr",
    "whisperx": "whisperx",
    "faster_whisper": "faster-whisper",
    "torchvision": "torchvision",
}


def _base_provider(provider):
    return provider.split("@", 1)[0].split(".", 1)[0]


def provider_version(provider):
    if provider.startswith(("ffmpeg.", "ffprobe.")):
        return binary_version(provider.split(".")[0])
    base = _base_provider(provider)
    if base in _OPTIONAL_PROVIDER_PACKAGES:
        return package_version(_OPTIONAL_PROVIDER_PACKAGES[base])
    if base == "numpy":
        return package_version("numpy")
    return platform.python_version(), None


def emit(instrument, provider, state, version, version_reason, error=None):
    event = {"instrument": instrument, "provider": provider, "state": state,
             "at_unix_ms": time.time_ns() // 1_000_000,
             "tool_version": version, "version_reason": version_reason,
             "tool_versions": {"python": platform.python_version()}, "error": error}
    # Metadata is a version snapshot, NOT a claim every installed backend ran.
    # Keep unknown-version reasons; never import an optional engine/model here.
    event["tool_version_errors"] = {}
    if provider == "perception":
        for name in ("onnx-asr", "whisperx", "faster-whisper", "librosa", "numpy", "scenedetect", "opencv-python", "torch", "torchvision", "soundfile", "silero-vad", "ffmpeg"):
            value, problem = binary_version("ffmpeg") if name == "ffmpeg" else package_version(name)
            event["tool_versions"][name] = value
            if problem:
                event["tool_version_errors"][name] = problem
    try:
        print("CUT_ATTEMPT " + json.dumps(event), file=sys.stderr, flush=True)
    except (OSError, ValueError):
        pass  # lost telemetry cannot invalidate a successful measurement


@contextmanager
def attempt(instrument, provider="perception"):
    version, reason = provider_version(provider)
    emit(instrument, provider, "running", version, reason)
    try:
        yield
    except BaseException as error:
        detail = _FATAL or {"code": type(error).__name__, "message": "The instrument could not finish.",
                            "cause": str(error)[:768]}
        # No distribution + missing optional import is absence, not a damaged
        # installed provider. Keep the actual exception either way.
        base = _base_provider(provider)
        state = "not_available" if base in _OPTIONAL_PROVIDER_PACKAGES and version is None and isinstance(error, ModuleNotFoundError) else "failed"
        emit(instrument, provider, state, version, reason, detail)
        raise
    else:
        emit(instrument, provider, "succeeded", version, reason)


def stream_probe(kind, operation):
    relevant = ("scenes", "subject", "contact_sheet", "qc_sheet") if kind == "video" else ("words", "silence", "beats", "loudness")
    instruments = [name for name in _REQUESTED if name in relevant]
    provider = f"ffprobe.{kind}.{next(_PROBES)}"
    version, reason = provider_version(provider) if instruments else (None, None)
    for name in instruments:
        emit(name, provider, "running", version, reason)
    try:
        result = operation()
    except BaseException as error:
        for name in instruments:
            emit(name, provider, "failed", version, reason, {"code": type(error).__name__, "message": "Could not read streams.", "cause": str(error)[:768]})
        raise
    problem = None if result.returncode == 0 else {"code": "ffprobe", "message": "Could not read streams.", "cause": result.stderr[-768:], "stderr_truncated": len(result.stderr) > 768, "exit_code": result.returncode}
    for name in instruments:
        emit(name, provider, "succeeded" if problem is None else "failed", version, reason, problem)
        if problem is None and not result.stdout.strip():
            emit(name, "perception", "not_applicable", platform.python_version(), None)
    return result


def instrument(name, provider="perception"):
    def decorate(function):
        @wraps(function)
        def wrapped(*args, **kwargs):
            with attempt(name, provider):
                return function(*args, **kwargs)
        return wrapped
    return decorate
