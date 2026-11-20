"""Prepared translation model selection tests; they do not import ML packages."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from native_runtime_models import CONTEXT_ENV, NativeRuntimeModelError
from translation_models import (
    MADLAD_FILES,
    MADLAD_GROUP,
    MADLAD_ID,
    OPUS_EN_ES_FILES,
    OPUS_EN_ES_GROUP,
    OPUS_EN_ES_ID,
    prepared_translation_model,
)


class TranslationModelsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.root = self.base / "native-runtime"
        self.root.mkdir()
        self.context_path = self.base / "native-runtime-context.json"
        self.assets = []
        self.opus = self.add_group(OPUS_EN_ES_GROUP, OPUS_EN_ES_FILES)
        self.madlad = self.add_group(MADLAD_GROUP, MADLAD_FILES)
        self.write_context()
        patch = mock.patch.dict(os.environ, {CONTEXT_ENV: str(self.context_path)}, clear=True)
        patch.start()
        self.addCleanup(patch.stop)

    def add_group(self, group, names):
        directory = self.root / "models" / group.rsplit(".", 1)[-1]
        directory.mkdir(parents=True)
        for name in names:
            path = directory / name
            path.write_bytes(name.encode())
            self.assets.append({
                "id": f"{group}/{name}",
                "path": str(path),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "provenanceSha256": "a" * 64,
            })
        return directory

    def write_context(self):
        self.context_path.write_text(json.dumps({
            "schema": "release-runner.native-runtime-context/v1",
            "root": str(self.root),
            "manifestSha256": "b" * 64,
            "receiptSha256": "c" * 64,
            "files": len(self.assets),
            "totalBytes": 123,
            "interpreter": {"path": str(self.root / "python"), "sha256": "d" * 64, "version": "3.12.13"},
            "imports": [],
            "models": self.assets,
        }))

    def test_context_selects_only_the_pinned_default_opus_pair(self):
        selected = prepared_translation_model(" en ", "ES", None)
        self.assertEqual(selected.model_id, OPUS_EN_ES_ID)
        self.assertEqual(selected.backend, "opus-mt")
        self.assertEqual(selected.directory, self.opus)
        with self.assertRaisesRegex(NativeRuntimeModelError, "only for en->es"):
            prepared_translation_model("en", "lv", None)

    def test_context_keeps_explicit_madlad_and_refuses_other_model_ids(self):
        selected = prepared_translation_model("lv", "es", MADLAD_ID)
        self.assertEqual(selected.model_id, MADLAD_ID)
        self.assertEqual(selected.backend, "madlad")
        self.assertEqual(selected.directory, self.madlad)
        with self.assertRaisesRegex(NativeRuntimeModelError, "no prepared local consumer"):
            prepared_translation_model("en", "es", "Helsinki-NLP/opus-mt-tc-big-en-es")

    def test_missing_required_asset_fails_before_runner_can_use_a_cache(self):
        self.assets = [a for a in self.assets if a["id"] != f"{OPUS_EN_ES_GROUP}/source.spm"]
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "incomplete"):
            prepared_translation_model("en", "es", None)

    def test_missing_group_and_wrong_context_schema_refuse_before_model_load(self):
        self.assets = [a for a in self.assets if not a["id"].startswith(f"{OPUS_EN_ES_GROUP}/")]
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "incomplete"):
            prepared_translation_model("en", "es", None)
        context = json.loads(self.context_path.read_text())
        context["schema"] = "wrong-schema"
        self.context_path.write_text(json.dumps(context))
        with self.assertRaisesRegex(NativeRuntimeModelError, "schema"):
            prepared_translation_model("en", "es", None)

    def test_undeclared_host_cache_file_fails_before_runner_can_use_it(self):
        cache = self.opus / "host-cache.bin"
        cache.write_bytes(b"unadmitted")
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "undeclared"):
            prepared_translation_model("en", "es", None)
        self.assets.append({
            "id": f"{OPUS_EN_ES_GROUP}/host-cache.bin",
            "path": str(cache),
            "sha256": hashlib.sha256(cache.read_bytes()).hexdigest(),
            "provenanceSha256": "a" * 64,
        })
        self.write_context()
        self.assertEqual(prepared_translation_model("en", "es", None).directory, self.opus)

    def test_no_context_preserves_the_existing_runner_model_resolution(self):
        del os.environ[CONTEXT_ENV]
        self.assertIsNone(prepared_translation_model("en", "lv", "anything"))


if __name__ == "__main__":
    unittest.main()
