#!/usr/bin/env python3
"""Prepare and load ordinary installed-app speech models without a host cache.

Runner-native input stays separate: when its context exists, the caller must
use the sealed model chosen by native_runtime_models. Ordinary installed apps
keep downloaded ONNX files below the app-data perception directory that the
Rust sidecar provisioner owns.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import gc
import hashlib
import json
import os
from pathlib import Path
import sys

from native_runtime_models import context_requested


class OrdinarySpeechModelError(RuntimeError):
    """An ordinary installed-app speech model cannot be prepared safely."""


def default_model_root() -> Path:
    """Return Rust's appdata_sidecar_dir()/models location."""
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA")
        if not base:
            raise OrdinarySpeechModelError("LOCALAPPDATA is required for the speech model directory")
        return Path(base) / "ShellX Cut" / "perception" / "models"
    if sys.platform == "darwin":
        home = os.environ.get("HOME")
        if not home:
            raise OrdinarySpeechModelError("HOME is required for the speech model directory")
        return Path(home) / "Library" / "Application Support" / "ShellX Cut" / "perception" / "models"
    base = os.environ.get("XDG_DATA_HOME")
    if base:
        return Path(base) / "shellx-cut" / "perception" / "models"
    home = os.environ.get("HOME")
    if not home:
        raise OrdinarySpeechModelError("HOME or XDG_DATA_HOME is required for the speech model directory")
    return Path(home) / ".local" / "share" / "shellx-cut" / "perception" / "models"


def _checked_root(root: Path | str | None) -> Path:
    candidate = default_model_root() if root is None else Path(root)
    if not candidate.is_absolute():
        raise OrdinarySpeechModelError("speech model root must be an absolute path")
    return candidate


def _checked_model(model: str) -> str:
    if not isinstance(model, str):
        raise OrdinarySpeechModelError("speech model identifier must be a nonempty string")
    model = model.strip()
    if not model or "\0" in model:
        raise OrdinarySpeechModelError("speech model identifier must be a nonempty string without NUL")
    return model


def _model_key(model: str) -> str:
    """Give any library-supported model identifier a deterministic safe leaf."""
    return hashlib.sha256(model.encode("utf-8")).hexdigest()


@contextmanager
def _exclusive_lock(path: Path, timeout_seconds: float = 120.0):
    """Serialize one model stage, publish, and load cycle with filelock."""
    from filelock import FileLock, Timeout

    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with FileLock(str(path), timeout=timeout_seconds):
            yield
    except Timeout as error:
        raise OrdinarySpeechModelError(f"timed out waiting for speech model lock: {path}") from error


def _strict_local_load(model: str, directory: Path, providers):
    import onnx_asr

    return onnx_asr.load_model(model, path=directory, providers=providers)


def _load_published_model(model: str, directory: Path, providers):
    try:
        return _strict_local_load(model, directory, providers)
    except Exception as error:  # noqa: BLE001 - preserve the loader cause without an online fallback
        raise OrdinarySpeechModelError(
            f"ordinary local speech model is unusable at {directory}; it was preserved and was not replaced: {error}"
        ) from error


def _resumable_stage_load(model: str, directory: Path, providers):
    """Open a stage through onnx-asr's public Manager API.

    offline=False overrides the library's existing-directory shortcut. Its
    canonical model mapping and snapshot_download(local_dir=...) then resume
    the stage using Hugging Face local metadata; Cut keeps no copied registry.
    """
    from onnx_asr.loader import Manager

    return Manager(providers=providers).create_asr(model, local_dir=directory, offline=False)


def _model_paths(model: str, root: Path) -> tuple[Path, Path, Path]:
    key = _model_key(model)
    return root / key, root / f".{key}.staging", root / f".{key}.lock"


def _prepare_locked(model: str, root: Path, providers) -> Path:
    final, stage, _ = _model_paths(model, root)
    if final.exists():
        if not final.is_dir():
            raise OrdinarySpeechModelError(f"ordinary speech model path is not a directory: {final}")
        return final

    stage.mkdir(parents=True, exist_ok=True)
    if not stage.is_dir():
        raise OrdinarySpeechModelError(f"ordinary speech staging path is not a directory: {stage}")
    adapter = _resumable_stage_load(model, stage, providers)
    del adapter
    gc.collect()
    try:
        os.replace(stage, final)
    except OSError as error:
        raise OrdinarySpeechModelError(
            f"speech model was prepared but could not be published; retry setup to resume {stage}: {error}"
        ) from error
    return final


def _checked_inputs(model: str, root: Path | str | None) -> tuple[str, Path]:
    if context_requested():
        raise OrdinarySpeechModelError("Runner native context requires its admitted speech model route")
    return _checked_model(model), _checked_root(root)


def prepare_model(model: str, root: Path | str | None = None, providers=None) -> Path:
    """Resume and publish a model; the caller can then strict-load the final leaf."""
    model, root = _checked_inputs(model, root)
    _, _, lock = _model_paths(model, root)
    with _exclusive_lock(lock):
        return _prepare_locked(model, root, providers)


def load_ordinary_model(model: str, root: Path | str | None = None, providers=None):
    """Prepare if needed, then load the published local leaf once under its lock."""
    model, root = _checked_inputs(model, root)
    _, _, lock = _model_paths(model, root)
    with _exclusive_lock(lock):
        final = _prepare_locked(model, root, providers)
        return _load_published_model(model, final, providers)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="prepare one ordinary ShellX Cut speech model")
    parser.add_argument("--prepare", action="store_true", help="prepare and verify the model without transcription")
    parser.add_argument("--model", required=True)
    parser.add_argument("--root", type=Path, help="absolute ordinary speech-model root")
    args = parser.parse_args(argv)
    if not args.prepare:
        parser.error("--prepare is required")
    adapter = load_ordinary_model(args.model, args.root, providers=["CPUExecutionProvider"])
    del adapter
    gc.collect()
    final = prepare_model(args.model, args.root, providers=["CPUExecutionProvider"])
    print(json.dumps({"model": args.model, "root": str(final), "prepared": True}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
