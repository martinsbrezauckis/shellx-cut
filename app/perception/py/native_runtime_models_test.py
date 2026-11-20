"""Model identity and real instruments routing tests; no native ML imports."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import types
import unittest
from unittest import mock

from native_runtime_models import (
    CONTEXT_ENV,
    NativeRuntimeModelError,
    parakeet_directory,
    prepared_model_directory,
    SIGLIP_MODEL_GROUP,
    SIGLIP_MODEL_ID,
    SIGLIP_REQUIRED_FILES,
    siglip_directory,
)

MODEL = "nemo-parakeet-tdt-0.6b-v3"


class NativeRuntimeModelsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.root = self.base / "native-runtime"
        self.model = self.root / "models" / MODEL
        self.model.mkdir(parents=True)
        self.context_path = self.base / "native-runtime-context.json"
        assets = []
        for name in ("config.json", "encoder-model.onnx", "decoder_joint-model.onnx", "vocab.txt", "encoder-model.onnx.data"):
            path = self.model / name
            path.write_bytes(name.encode())
            assets.append({"id": f"cut.stt.{MODEL}/{name}", "path": str(path),
                           "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "provenanceSha256": "a" * 64})
        self.context = {"schema": "release-runner.native-runtime-context/v1", "root": str(self.root),
                        "manifestSha256": "b" * 64, "receiptSha256": "c" * 64, "files": 6, "totalBytes": 123,
                        "interpreter": {"path": str(self.root / "python"), "sha256": "d" * 64, "version": "3.12.13"},
                        "imports": [], "models": assets}
        self.write_context()
        patch = mock.patch.dict(os.environ, {CONTEXT_ENV: str(self.context_path)}, clear=True)
        patch.start()
        self.addCleanup(patch.stop)

    def write_context(self):
        self.context_path.write_text(json.dumps(self.context))

    def test_exact_group_includes_external_data_without_writes(self):
        before = {p: p.read_bytes() for p in self.model.iterdir()}
        self.assertEqual(parakeet_directory(MODEL), self.model)
        self.assertEqual(before, {p: p.read_bytes() for p in self.model.iterdir()})

    def test_context_set_selects_python_model_member(self):
        self.context = {
            "schema": "release-runner.native-runtime-context-set/v1",
            "runtimes": [
                {"id": "media-tools", "context": {"schema": "release-runner.pinned-executable-bundle-context/v1"}},
                {"id": "premium", "context": {"schema": "release-runner.native-runtime-context/v1"}},
                {"id": "python", "context": self.context},
            ],
        }
        self.write_context()
        self.assertEqual(parakeet_directory(MODEL), self.model)

    def test_context_set_refuses_missing_or_duplicate_python_member(self):
        ordinary = self.context
        self.context = {"schema": "release-runner.native-runtime-context-set/v1",
                        "runtimes": [{"id": "premium", "context": ordinary}]}
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "no python member"):
            parakeet_directory(MODEL)
        self.context["runtimes"] = [{"id": "python", "context": ordinary}] * 2
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "duplicated"):
            parakeet_directory(MODEL)

    def test_absent_context_preserves_normal_route_but_empty_refuses(self):
        del os.environ[CONTEXT_ENV]
        self.assertIsNone(parakeet_directory(MODEL))
        os.environ[CONTEXT_ENV] = ""
        with self.assertRaises(NativeRuntimeModelError):
            parakeet_directory(MODEL)

    def test_changed_external_data_refuses(self):
        (self.model / "encoder-model.onnx.data").write_bytes(b"changed")
        with self.assertRaisesRegex(NativeRuntimeModelError, "bytes changed"):
            parakeet_directory(MODEL)

    def test_missing_declared_asset_or_unknown_file_refuses(self):
        self.context["models"].pop()
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "undeclared"):
            parakeet_directory(MODEL)

    def test_missing_required_asset_refuses(self):
        self.context["models"] = [a for a in self.context["models"] if not a["id"].endswith("/vocab.txt")]
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "incomplete"):
            parakeet_directory(MODEL)

    def test_duplicate_ids_and_json_keys_refuse(self):
        self.context["models"].append(self.context["models"][0])
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "unique"):
            parakeet_directory(MODEL)
        self.context_path.write_text('{"schema":"a","schema":"b"}')
        with self.assertRaisesRegex(NativeRuntimeModelError, "duplicate JSON"):
            parakeet_directory(MODEL)

    def test_escape_and_parent_link_refuse(self):
        asset = self.context["models"][0]
        outside = self.base / "config.json"
        outside.write_bytes(b"config.json")
        asset["path"] = str(outside)
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "path or bytes"):
            parakeet_directory(MODEL)
        link = self.base / "linked-model"
        try:
            link.symlink_to(self.model, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"symlink creation unavailable: {error}")
        asset["path"] = str(link / "config.json")
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "link"):
            parakeet_directory(MODEL)

    def test_wrong_group_and_oversized_context_refuse(self):
        with self.assertRaisesRegex(NativeRuntimeModelError, "incomplete"):
            parakeet_directory("nemo-parakeet-tdt-0.6b-v2")
        self.context_path.write_bytes(b" " * (1024 * 1024 + 1))
        with self.assertRaisesRegex(NativeRuntimeModelError, "1 MiB"):
            parakeet_directory(MODEL)

    def test_v2_group_is_selected_independently(self):
        model_v2 = "nemo-parakeet-tdt-0.6b-v2"
        for asset in self.context["models"]:
            asset["id"] = asset["id"].replace(MODEL, model_v2)
        self.write_context()
        self.assertEqual(parakeet_directory(model_v2), self.model)
        with self.assertRaisesRegex(NativeRuntimeModelError, "incomplete"):
            parakeet_directory(MODEL)

    def test_generic_group_requires_exact_declared_nested_layout(self):
        group = "cut.fixture.visual"
        directory = self.root / "models" / "fixture-visual"
        nested = directory / "tokenizer"
        nested.mkdir(parents=True)
        config = directory / "config.json"
        tokenizer = nested / "tokenizer.json"
        config.write_bytes(b"config")
        tokenizer.write_bytes(b"tokenizer")
        for relative, path in (("config.json", config), ("tokenizer/tokenizer.json", tokenizer)):
            self.context["models"].append({
                "id": f"{group}/{relative}",
                "path": str(path),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "provenanceSha256": "a" * 64,
            })
        self.write_context()
        self.assertEqual(
            prepared_model_directory(group, {"config.json", "tokenizer/tokenizer.json"}),
            directory,
        )
        (directory / "host-cache.bin").write_bytes(b"unadmitted")
        with self.assertRaisesRegex(NativeRuntimeModelError, "undeclared"):
            prepared_model_directory(group, {"config.json", "tokenizer/tokenizer.json"})

    def test_generic_group_rejects_normalized_or_duplicate_relative_ids(self):
        group = "cut.fixture.visual"
        directory = self.root / "models" / "fixture-visual"
        tokenizer_dir = directory / "tokenizer"
        tokenizer_dir.mkdir(parents=True)
        config = tokenizer_dir / "config.json"
        config.write_bytes(b"config")
        asset = {
            "id": f"{group}/tokenizer/config.json",
            "path": str(config),
            "sha256": hashlib.sha256(config.read_bytes()).hexdigest(),
            "provenanceSha256": "a" * 64,
        }
        self.context["models"].append(asset)
        for malformed in ("tokenizer//config.json", "tokenizer/./config.json", ".", "tokenizer/config.json/"):
            with self.subTest(relative=malformed):
                self.context["models"].append({**asset, "id": f"{group}/{malformed}"})
                self.write_context()
                with self.assertRaisesRegex(NativeRuntimeModelError, "relative path"):
                    prepared_model_directory(group, {"tokenizer/config.json"})
                self.context["models"].pop()

    def install_siglip_group(self):
        directory = self.root / "models" / "siglip2-base-patch16-224"
        directory.mkdir(parents=True)
        for name in SIGLIP_REQUIRED_FILES:
            path = directory / name
            path.write_bytes(name.encode())
            self.context["models"].append({
                "id": f"{SIGLIP_MODEL_GROUP}/{name}",
                "path": str(path),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "provenanceSha256": "e" * 64,
            })
        self.write_context()
        return directory

    def siglip_indexer(self):
        spec = importlib.util.spec_from_file_location("cut_siglip_index_test", Path(__file__).with_name("siglip_index.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_siglip_index_writer_refuses_oversize_before_replacing_cache(self):
        indexer = self.siglip_indexer()
        index = {"schema": "fixture", "frames": [{"ms": 0, "v": [0.1, 0.2]}]}
        payload = json.dumps(index).encode("utf-8")
        target = self.base / "visual-index.json"
        indexer.write_index_bounded(index, str(target), len(payload))
        self.assertEqual(target.read_bytes(), payload)
        with self.assertRaisesRegex(ValueError, "byte limit"):
            indexer.write_index_bounded(index, str(target), len(payload) - 1)
        self.assertEqual(target.read_bytes(), payload)

    @unittest.skipUnless(os.name == "posix", "symlink fixture needs Unix")
    def test_siglip_index_writer_refuses_linked_parent_and_leaf(self):
        indexer = self.siglip_indexer()
        index = {"schema": "fixture", "frames": [{"ms": 0, "v": [0.1]}]}
        outside = self.base / "outside"
        outside.mkdir()
        sentinel = outside / "a1.json"
        sentinel.write_bytes(b"outside-sentinel")
        embeddings = self.base / "embeddings"
        embeddings.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "plain directory"):
            indexer.write_index_bounded(index, str(embeddings / "a1.json"), 1024)
        self.assertEqual(sentinel.read_bytes(), b"outside-sentinel")
        embeddings.unlink()
        embeddings.mkdir()
        leaf = embeddings / "a1.json"
        leaf.symlink_to(sentinel)
        with self.assertRaisesRegex(ValueError, "plain file"):
            indexer.write_index_bounded(index, str(leaf), 1024)
        self.assertEqual(sentinel.read_bytes(), b"outside-sentinel")
        leaf.unlink()
        indexer.write_index_bounded(index, str(leaf), 1024)
        self.assertEqual(json.loads(leaf.read_text()), index)
        rebuilt = {"schema": "fixture", "frames": [{"ms": 1000, "v": [0.2]}]}
        indexer.write_index_bounded(rebuilt, str(leaf), 1024)
        self.assertEqual(json.loads(leaf.read_text()), rebuilt)

    def test_siglip_group_is_exact_and_refuses_host_cache_entries(self):
        directory = self.install_siglip_group()
        self.assertEqual(siglip_directory(), directory)
        (directory / "host-cache.bin").write_bytes(b"unadmitted")
        with self.assertRaisesRegex(NativeRuntimeModelError, "undeclared"):
            siglip_directory()

    def test_siglip_indexer_uses_only_admitted_local_model_in_context(self):
        directory = self.install_siglip_group()
        indexer = self.siglip_indexer()
        self.assertEqual(
            indexer.resolve_model_source(str(directory), SIGLIP_MODEL_ID),
            (str(directory), {"local_files_only": True}, SIGLIP_MODEL_ID),
        )
        with self.assertRaisesRegex(NativeRuntimeModelError, "does not match"):
            indexer.resolve_model_source("google/other", SIGLIP_MODEL_ID)
        with self.assertRaisesRegex(NativeRuntimeModelError, "does not match"):
            indexer.resolve_model_source(str(directory), "other")

    def test_siglip_invalid_context_never_falls_back_to_a_hf_model(self):
        self.install_siglip_group()
        indexer = self.siglip_indexer()
        self.context["schema"] = "other"
        self.write_context()
        with self.assertRaisesRegex(NativeRuntimeModelError, "schema"):
            indexer.resolve_model_source("google/other", "")
        self.context_path.write_text("not JSON")
        with self.assertRaises(NativeRuntimeModelError):
            indexer.resolve_model_source("google/other", "")

    def test_siglip_absent_context_preserves_normal_model_route(self):
        del os.environ[CONTEXT_ENV]
        indexer = self.siglip_indexer()
        self.assertEqual(
            indexer.resolve_model_source("google/other", ""),
            ("google/other", {}, "other"),
        )

    def test_siglip_indexer_requires_one_finite_pooled_vector(self):
        indexer = self.siglip_indexer()

        class Tensor:
            def __init__(self, shape, values, finite=True):
                self.shape = shape
                self.ndim = len(shape)
                self.values = values
                self.finite = finite

            def __getitem__(self, item):
                if item != 0:
                    raise AssertionError(f"expected batch item 0, got {item}")
                return Tensor((self.shape[1],), self.values, self.finite)

            def numel(self):
                return len(self.values)

            def float(self):
                return self

            def cpu(self):
                return self

            def tolist(self):
                return self.values

        class Finite:
            def __init__(self, value):
                self.value = value

            def all(self):
                return self.value

        fake_torch = types.SimpleNamespace(
            nn=types.SimpleNamespace(
                functional=types.SimpleNamespace(normalize=lambda tensor, dim: tensor)
            ),
            isfinite=lambda tensor: Finite(tensor.finite),
            is_tensor=lambda value: isinstance(value, Tensor),
        )
        output = types.SimpleNamespace(pooler_output=Tensor((1, 3), [0.1, 0.2, 0.3]))
        self.assertEqual(indexer.pooled_embedding(fake_torch, output, "image"), [0.1, 0.2, 0.3])
        direct_tensor = Tensor((1, 3), [0.4, 0.5, 0.6])
        self.assertEqual(indexer.pooled_embedding(fake_torch, direct_tensor, "text"), [0.4, 0.5, 0.6])
        nested = types.SimpleNamespace(pooler_output=Tensor((1, 1, 3), [0.1, 0.2, 0.3]))
        with self.assertRaisesRegex(RuntimeError, "shape"):
            indexer.pooled_embedding(fake_torch, nested, "image")
        nonfinite = types.SimpleNamespace(pooler_output=Tensor((1, 3), [0.1, 0.2, 0.3], finite=False))
        with self.assertRaisesRegex(RuntimeError, "non-finite"):
            indexer.pooled_embedding(fake_torch, nonfinite, "image")

    def instruments(self):
        spec = importlib.util.spec_from_file_location("cut_native_stt_test", Path(__file__).with_name("instruments.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_base_requirements_declare_direct_parakeet_runtime(self):
        requirements = Path(__file__).with_name("requirements.txt").read_text()
        declared = {
            line.split("#", 1)[0].strip()
            for line in requirements.splitlines()
            if line.split("#", 1)[0].strip()
        }
        self.assertIn("numpy", declared)
        self.assertIn("soundfile", declared)

    def test_real_parakeet_route_uses_generic_type_and_exact_local_path(self):
        module = self.instruments()
        model = types.SimpleNamespace(recognize=lambda _: types.SimpleNamespace(tokens=[], timestamps=[]))
        load = mock.Mock(return_value=types.SimpleNamespace(with_timestamps=lambda: model))
        fake_modules = {"onnx_asr": types.SimpleNamespace(load_model=load),
                        "soundfile": types.SimpleNamespace(read=lambda *args, **kwargs: ([], 16000))}
        with mock.patch.dict("sys.modules", fake_modules), mock.patch.object(module, "extract_wav16k", return_value="fixture.wav"):
            result = module.instrument_words("fixture.mp4", "a1", "small")
        self.assertEqual(load.call_args.args, ("nemo-conformer-tdt",))
        self.assertEqual(load.call_args.kwargs["path"], self.model)
        self.assertIn(MODEL, result["model"])

    def test_prepared_backend_failure_never_enters_fallback(self):
        module = self.instruments()
        with mock.patch.object(module, "_words_via_parakeet", side_effect=RuntimeError("native load failed")):
            with self.assertRaisesRegex(RuntimeError, "native load failed"):
                module.instrument_words("fixture.mp4", "a1", "small")

    def test_prepared_model_choice_is_normalized_once(self):
        module = self.instruments()
        os.environ["SHELLX_CUT_STT_MODEL"] = f" {MODEL} "
        with mock.patch.object(module, "_words_via_parakeet", return_value={}) as transcribe:
            module.instrument_words("fixture.mp4", "a1", "small")
            transcribe.assert_called_once_with("fixture.mp4", "a1", model_override=MODEL)

    def test_absent_context_keeps_named_model_route(self):
        del os.environ[CONTEXT_ENV]
        module = self.instruments()
        model = types.SimpleNamespace(recognize=lambda _: types.SimpleNamespace(tokens=[], timestamps=[]))
        load = mock.Mock(return_value=types.SimpleNamespace(with_timestamps=lambda: model))
        fake_modules = {"soundfile": types.SimpleNamespace(read=lambda *args, **kwargs: ([], 16000))}
        with (
            mock.patch.dict("sys.modules", fake_modules),
            mock.patch.object(module, "extract_wav16k", return_value="fixture.wav"),
            mock.patch.object(module, "load_ordinary_model", load),
        ):
            module.instrument_words("fixture.mp4", "a1", "small")
        load.assert_called_once_with(MODEL, providers=None)

    def test_explicit_other_engine_and_preferred_language_are_not_substituted(self):
        module = self.instruments()
        with mock.patch.object(module, "_words_via_parakeet") as transcribe:
            os.environ["SHELLX_CUT_STT_MODEL"] = "whisperx-small"
            with self.assertRaisesRegex(NativeRuntimeModelError, "no prepared local consumer"):
                module.instrument_words("fixture.mp4", "a1", "small")
            del os.environ["SHELLX_CUT_STT_MODEL"]
            os.environ["SHELLX_CUT_STT_LANG"] = "lv"
            with self.assertRaisesRegex(NativeRuntimeModelError, "Canary"):
                module.instrument_words("fixture.mp4", "a1", "small")
            transcribe.assert_not_called()


if __name__ == "__main__":
    unittest.main()
