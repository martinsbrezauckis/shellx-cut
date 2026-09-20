"""Prepared offline model selection for Cut's local translation runner.

An ordinary installation keeps the runner's existing Hugging Face model and
cache behavior.  A supplied Runner context instead names one sealed model group
and returns its verified directory.  It never treats host caches, an arbitrary
model ID, or a second Opus candidate as an acceptable substitute.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from native_runtime_models import (
    NativeRuntimeModelError,
    context_requested,
    prepared_model_directory,
)

OPUS_EN_ES_ID = "Helsinki-NLP/opus-mt-en-es"
MADLAD_ID = "jbochi/madlad400-3b-mt"

OPUS_EN_ES_GROUP = "cut.translate.opus-mt-en-es"
MADLAD_GROUP = "cut.translate.madlad400-3b-mt"

# These are the current Transformers load paths, intentionally smaller than
# either full repository.  The release preparation manifest pins every byte.
OPUS_EN_ES_FILES = frozenset({
    "config.json",
    "generation_config.json",
    "pytorch_model.bin",
    "source.spm",
    "target.spm",
    "tokenizer_config.json",
    "vocab.json",
})
MADLAD_FILES = frozenset({
    "config.json",
    "generation_config.json",
    "model.safetensors",
    "special_tokens_map.json",
    "spiece.model",
    "tokenizer.json",
    "tokenizer_config.json",
})


@dataclass(frozen=True)
class PreparedTranslationModel:
    """One model admitted by the context and its immutable local directory."""

    model_id: str
    backend: str
    directory: Path


def prepared_translation_model(
    source_language: str,
    target_language: str,
    explicit_model: str | None,
) -> PreparedTranslationModel | None:
    """Select the one allowed native model, or retain the normal runner route.

    The default prepared asset deliberately covers the published light-weight
    en-to-es Opus pair.  MADLAD remains available only through its existing
    explicit ID, keeping its broader language scope without opening arbitrary
    model discovery in a native-context run.
    """
    if not context_requested():
        return None

    source = source_language.strip().lower()
    target = target_language.strip().lower()
    explicit = explicit_model.strip() if explicit_model else None
    if explicit is None:
        if (source, target) != ("en", "es"):
            raise NativeRuntimeModelError(
                "prepared local translation supports default Opus-MT only for en->es"
            )
        directory = prepared_model_directory(OPUS_EN_ES_GROUP, OPUS_EN_ES_FILES)
        if directory is None:  # context_requested above makes this defensive only
            raise NativeRuntimeModelError("prepared local translation context is absent")
        return PreparedTranslationModel(OPUS_EN_ES_ID, "opus-mt", directory)

    if explicit != MADLAD_ID:
        raise NativeRuntimeModelError(
            f"translation model has no prepared local consumer: {explicit}"
        )
    directory = prepared_model_directory(MADLAD_GROUP, MADLAD_FILES)
    if directory is None:
        raise NativeRuntimeModelError("prepared local translation context is absent")
    return PreparedTranslationModel(MADLAD_ID, "madlad", directory)
