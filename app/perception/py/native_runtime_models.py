"""Cut-owned model selection from Runner's immutable native runtime context.

The desktop/engine validates the admitted interpreter. This consumer rechecks
the selected model bytes immediately before handing an existing local directory
to onnx-asr. Scripts remain in the separately inventoried application payload.
No cache discovery, download, directory creation, or model fallback occurs here.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import stat
from pathlib import Path, PurePosixPath

CONTEXT_ENV = "RELEASE_RUNNER_NATIVE_RUNTIME_CONTEXT"
CONTEXT_SCHEMA = "release-runner.native-runtime-context/v1"
CONTEXT_LIMIT = 1024 * 1024
PARAKEET_MODELS = frozenset({"nemo-parakeet-tdt-0.6b-v2", "nemo-parakeet-tdt-0.6b-v3"})
REQUIRED_FILES = frozenset({"config.json", "encoder-model.onnx", "decoder_joint-model.onnx", "vocab.txt"})
SIGLIP_MODEL_ID = "google/siglip2-base-patch16-224"
SIGLIP_MODEL_GROUP = "cut.siglip.google.siglip2-base-patch16-224"
SIGLIP_REQUIRED_FILES = frozenset({
    "config.json",
    "model.safetensors",
    "preprocessor_config.json",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer.model",
    "tokenizer_config.json",
})
HASH = re.compile(r"[a-f0-9]{64}\Z")
FILENAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,255}\Z")


class NativeRuntimeModelError(RuntimeError):
    """An admitted runtime cannot satisfy the selected product model."""


def context_requested() -> bool:
    # Empty is invalid, not permission to fall through to a host cache.
    return CONTEXT_ENV in os.environ


def _unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise NativeRuntimeModelError("native runtime context has duplicate JSON keys")
        value[key] = item
    return value


def _path(value, *, directory=False) -> Path:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise NativeRuntimeModelError("native runtime path is invalid")
    path = Path(value)
    if not path.is_absolute() or ".." in path.parts or str(path) != value:
        raise NativeRuntimeModelError("native runtime path must be canonical and absolute")
    for part in (path, *path.parents):
        mode = part.lstat().st_mode
        if stat.S_ISLNK(mode) or (part != path and not stat.S_ISDIR(mode)):
            raise NativeRuntimeModelError("native runtime path contains a link or non-directory")
    mode = path.lstat().st_mode
    if not (stat.S_ISDIR(mode) if directory else stat.S_ISREG(mode)):
        raise NativeRuntimeModelError("native runtime path has the wrong file type")
    return path


def _sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _context():
    path = _path(os.environ.get(CONTEXT_ENV, ""))
    if path.stat().st_size > CONTEXT_LIMIT:
        raise NativeRuntimeModelError("native runtime context exceeds 1 MiB")
    with path.open("rb") as stream:
        data = stream.read(CONTEXT_LIMIT + 1)
    if len(data) > CONTEXT_LIMIT:
        raise NativeRuntimeModelError("native runtime context changed beyond 1 MiB")
    value = json.loads(data, object_pairs_hook=_unique_object)
    keys = {"schema", "root", "manifestSha256", "receiptSha256", "files", "totalBytes", "interpreter", "imports", "models"}
    if not isinstance(value, dict) or set(value) != keys or value["schema"] != CONTEXT_SCHEMA:
        raise NativeRuntimeModelError("native runtime context schema is invalid")
    if any(not isinstance(value[key], str) or not HASH.fullmatch(value[key]) for key in ("manifestSha256", "receiptSha256")):
        raise NativeRuntimeModelError("native runtime context identity is invalid")
    if not isinstance(value["models"], list) or len(value["models"]) > 4096:
        raise NativeRuntimeModelError("native runtime model inventory is invalid")
    return value, _path(value["root"], directory=True)


def _relative_path(value: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        raise NativeRuntimeModelError("prepared model relative path is invalid")
    path = PurePosixPath(value)
    if (
        not path.parts
        or value != path.as_posix()
        or path.is_absolute()
        or any(part in (".", "..") or not FILENAME.fullmatch(part) for part in path.parts)
    ):
        raise NativeRuntimeModelError("prepared model relative path is invalid")
    return path.as_posix()


def _declared_tree_entries(directory: Path, selected: dict[str, Path]) -> set[Path]:
    expected = set(selected.values())
    for path in selected.values():
        parent = path.parent
        while parent != directory:
            expected.add(parent)
            parent = parent.parent
    actual = set()
    for path in directory.rglob("*"):
        mode = path.lstat().st_mode
        if stat.S_ISLNK(mode) or not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
            raise NativeRuntimeModelError("prepared model directory contains a link or special entry")
        actual.add(path)
    if actual != expected:
        raise NativeRuntimeModelError("prepared model directory has undeclared entries")
    return actual


def prepared_model_directory(group_id: str, required_relative_paths) -> Path | None:
    """Return one sealed, complete local model directory for a product group.

    ``group_id`` names a Cut-owned semantic group such as
    ``cut.stt.nemo-parakeet-tdt-0.6b-v3``.  Every model entry must use that
    group plus a clean POSIX-relative path, live under one regular non-link
    directory, and match its streamed context hash.  Required paths express
    the product format; every other discovered file is still declared and
    checked, so a cache directory cannot add an unadmitted model artifact.

    An absent locator preserves an ordinary installed-app route.  A supplied
    locator with a missing, changed, linked, or incomplete group raises rather
    than falling through to an environment override or model cache.
    """
    if not context_requested():
        return None

    try:
        if not isinstance(group_id, str) or not FILENAME.fullmatch(group_id):
            raise NativeRuntimeModelError("prepared model group ID is invalid")
        if isinstance(required_relative_paths, (str, bytes)):
            raise NativeRuntimeModelError("prepared model required paths are invalid")
        required = {_relative_path(value) for value in required_relative_paths}
        if not required:
            raise NativeRuntimeModelError("prepared model required paths are invalid")
        context, root = _context()
        prefix = f"{group_id}/"
        seen = set()
        selected = {}
        for asset in context["models"]:
            if not isinstance(asset, dict) or set(asset) != {"id", "path", "sha256", "provenanceSha256"}:
                raise NativeRuntimeModelError("native runtime model entry is invalid")
            asset_id = asset["id"]
            if not isinstance(asset_id, str) or not asset_id or asset_id in seen:
                raise NativeRuntimeModelError("native runtime model IDs must be unique")
            seen.add(asset_id)
            if not asset_id.startswith(prefix):
                continue
            relative = _relative_path(asset_id[len(prefix):])
            if any(not isinstance(asset[key], str) or not HASH.fullmatch(asset[key]) for key in ("sha256", "provenanceSha256")):
                raise NativeRuntimeModelError("prepared model asset identity is invalid")
            path = _path(asset["path"])
            parts = PurePosixPath(relative).parts
            parent_depth = len(parts) - 1
            if parent_depth >= len(path.parents):
                raise NativeRuntimeModelError("prepared model asset path is too shallow")
            directory = path.parents[parent_depth]
            if not path.is_relative_to(root) or path.relative_to(directory).as_posix() != relative or _sha(path) != asset["sha256"]:
                raise NativeRuntimeModelError("prepared model asset path or bytes changed")
            if relative in selected:
                raise NativeRuntimeModelError("prepared model relative paths must be unique")
            selected[relative] = path
        if not required.issubset(selected):
            raise NativeRuntimeModelError(f"prepared model is incomplete: {group_id}")
        directories = {path.parents[len(PurePosixPath(relative).parts) - 1] for relative, path in selected.items()}
        if len(directories) != 1:
            raise NativeRuntimeModelError("prepared model must occupy one exact directory")
        directory = directories.pop()
        _declared_tree_entries(directory, selected)
        return directory
    except NativeRuntimeModelError:
        raise
    except (OSError, ValueError, TypeError, KeyError) as error:
        raise NativeRuntimeModelError(f"cannot read prepared model: {error}") from error


def parakeet_directory(model_id: str) -> Path | None:
    """Return the exact local Parakeet model group; absent context preserves setup."""
    if not context_requested():
        return None
    if model_id not in PARAKEET_MODELS:
        raise NativeRuntimeModelError(f"STT model has no prepared local consumer: {model_id}")
    return prepared_model_directory(f"cut.stt.{model_id}", REQUIRED_FILES)


def siglip_directory() -> Path | None:
    """Return the sealed SigLIP directory; native mode never uses a HF cache."""
    return prepared_model_directory(SIGLIP_MODEL_GROUP, SIGLIP_REQUIRED_FILES)
