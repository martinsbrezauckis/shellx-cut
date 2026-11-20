"""Ordinary app-data speech model preparation tests with no native downloads."""
from __future__ import annotations

from contextlib import nullcontext
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest import mock

import ordinary_speech_models as models
from native_runtime_models import CONTEXT_ENV


class OrdinarySpeechModelsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = (Path(self.temp.name) / "models").resolve()
        self.env = mock.patch.dict(os.environ, {}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    def no_lock(self):
        return mock.patch.object(models, "_exclusive_lock", return_value=nullcontext())

    @staticmethod
    def fake_modules(load_model, manager_type):
        return {
            "onnx_asr": types.SimpleNamespace(load_model=load_model),
            "onnx_asr.loader": types.SimpleNamespace(Manager=manager_type),
        }

    def test_hash_leaf_is_safe_for_an_arbitrary_library_model_identifier(self):
        model = "org/example-model-v7"
        final, stage, lock = models._model_paths(model, self.root)
        key = hashlib.sha256(model.encode("utf-8")).hexdigest()
        self.assertEqual(final, self.root / key)
        self.assertEqual(stage, self.root / f".{key}.staging")
        self.assertEqual(lock, self.root / f".{key}.lock")
        self.assertNotIn("/", final.name)

    def test_prepare_resumes_the_same_stage_via_public_manager_and_publishes(self):
        calls = []
        partial_seen = []
        model = "org/example-model-v7"
        _final, stage, _lock = models._model_paths(model, self.root)
        stage.mkdir(parents=True)
        (stage / "partial.bin").write_bytes(b"partial")

        class Manager:
            def __init__(self, providers=None):
                self.providers = providers

            def create_asr(self, model, local_dir=None, *, offline=None, **_kwargs):
                calls.append((self.providers, model, Path(local_dir), offline))
                partial_seen.append((Path(local_dir) / "partial.bin").is_file())
                Path(local_dir, "resumed.bin").write_bytes(b"complete")
                return object()

        load = mock.Mock()
        with self.no_lock(), mock.patch.dict(sys.modules, self.fake_modules(load, Manager)):
            final = models.prepare_model(model, self.root, providers=["CPUExecutionProvider"])

        self.assertEqual(calls, [(["CPUExecutionProvider"], model, stage, False)])
        self.assertEqual(partial_seen, [True])
        self.assertEqual((final / "partial.bin").read_bytes(), b"partial")
        self.assertTrue((final / "resumed.bin").is_file())
        self.assertFalse(stage.exists())
        load.assert_not_called()

    def test_runtime_loads_existing_final_once_without_manager(self):
        model = "org/example-model-v7"
        final = self.root / hashlib.sha256(model.encode("utf-8")).hexdigest()
        final.mkdir(parents=True)
        adapter = object()
        load = mock.Mock(return_value=adapter)

        class Manager:
            def __init__(self, **_kwargs):
                raise AssertionError("existing published model must not enter Manager")

        with self.no_lock(), mock.patch.dict(sys.modules, self.fake_modules(load, Manager)):
            actual = models.load_ordinary_model(model, self.root, providers=["CPUExecutionProvider"])

        self.assertIs(actual, adapter)
        load.assert_called_once_with(model, path=final, providers=["CPUExecutionProvider"])

    def test_runtime_publishes_stage_then_strict_loads_final_once(self):
        model = "org/example-model-v7"
        adapter = object()
        load = mock.Mock(return_value=adapter)
        manager_calls = []

        class Manager:
            def __init__(self, providers=None):
                self.providers = providers

            def create_asr(self, selected, local_dir=None, *, offline=None, **_kwargs):
                manager_calls.append((self.providers, selected, Path(local_dir), offline))
                Path(local_dir, "model.bin").write_bytes(b"complete")
                return object()

        with self.no_lock(), mock.patch.dict(sys.modules, self.fake_modules(load, Manager)):
            actual = models.load_ordinary_model(model, self.root, providers=None)

        final, stage, _ = models._model_paths(model, self.root)
        self.assertIs(actual, adapter)
        self.assertEqual(manager_calls, [(None, model, stage, False)])
        load.assert_called_once_with(model, path=final, providers=None)
        self.assertTrue((final / "model.bin").is_file())

    def test_corrupt_published_model_is_preserved_without_manager_fallback(self):
        model = "org/example-model-v7"
        final, _stage, _lock = models._model_paths(model, self.root)
        final.mkdir(parents=True)
        load = mock.Mock(side_effect=ValueError("missing ONNX external data"))

        class Manager:
            def __init__(self, **_kwargs):
                raise AssertionError("corrupt final must not silently download over it")

        with self.no_lock(), mock.patch.dict(sys.modules, self.fake_modules(load, Manager)):
            with self.assertRaisesRegex(models.OrdinarySpeechModelError, "was preserved and was not replaced"):
                models.load_ordinary_model(model, self.root)

        load.assert_called_once_with(model, path=final, providers=None)

    def test_failed_publish_keeps_resumable_stage(self):
        model = "org/example-model-v7"

        class Manager:
            def __init__(self, **_kwargs):
                pass

            def create_asr(self, _selected, local_dir=None, **_kwargs):
                Path(local_dir, "partial.bin").write_bytes(b"partial")
                return object()

        with (
            self.no_lock(),
            mock.patch.dict(sys.modules, self.fake_modules(mock.Mock(), Manager)),
            mock.patch.object(models.os, "replace", side_effect=OSError("simulated publish failure")),
        ):
            with self.assertRaisesRegex(models.OrdinarySpeechModelError, "could not be published"):
                models.prepare_model(model, self.root)

        _final, stage, _lock = models._model_paths(model, self.root)
        self.assertEqual((stage / "partial.bin").read_bytes(), b"partial")

    def test_runner_context_refuses_ordinary_route_before_any_dependency_import(self):
        os.environ[CONTEXT_ENV] = str(self.root / "native-runtime-context.json")
        with self.assertRaisesRegex(models.OrdinarySpeechModelError, "Runner native context"):
            models.prepare_model("org/example-model-v7", self.root)
        self.assertNotIn("onnx_asr", sys.modules)

    def test_invalid_model_and_relative_root_refuse(self):
        with self.assertRaisesRegex(models.OrdinarySpeechModelError, "nonempty"):
            models.prepare_model("", self.root)
        with self.assertRaisesRegex(models.OrdinarySpeechModelError, "absolute"):
            models.prepare_model("org/example-model-v7", "relative-models")

    def test_lock_uses_filelock_without_a_bespoke_prewrite(self):
        events = []

        class Timeout(Exception):
            pass

        class FileLock:
            def __init__(self, path, timeout):
                events.append(("init", path, timeout))

            def __enter__(self):
                events.append(("enter",))
                return self

            def __exit__(self, *_args):
                events.append(("exit",))

        path = self.root / ".model.lock"
        fake_filelock = types.SimpleNamespace(FileLock=FileLock, Timeout=Timeout)
        with mock.patch.dict(sys.modules, {"filelock": fake_filelock}):
            with models._exclusive_lock(path):
                events.append(("held",))

        self.assertEqual(events, [("init", str(path), 120.0), ("enter",), ("held",), ("exit",)])
        self.assertTrue(path.parent.is_dir())
        self.assertFalse(path.exists())

    def test_prepare_cli_strict_loads_once_after_publish(self):
        model = "org/example-model-v7"
        load = mock.Mock(return_value=object())

        class Manager:
            def __init__(self, providers=None):
                self.providers = providers

            def create_asr(self, selected, local_dir=None, *, offline=None, **_kwargs):
                Path(local_dir, "model.bin").write_bytes(b"complete")
                return object()

        output = io.StringIO()
        with (
            self.no_lock(),
            mock.patch.dict(sys.modules, self.fake_modules(load, Manager)),
            mock.patch("sys.stdout", output),
        ):
            self.assertEqual(models.main(["--prepare", "--model", model, "--root", str(self.root)]), 0)

        final, _stage, _lock = models._model_paths(model, self.root)
        load.assert_called_once_with(model, path=final, providers=["CPUExecutionProvider"])
        self.assertEqual(json.loads(output.getvalue()), {"model": model, "prepared": True, "root": str(final)})


if __name__ == "__main__":
    unittest.main()
