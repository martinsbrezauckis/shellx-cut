"""Translation runner context tests with fake Transformers objects, never a model."""
from __future__ import annotations

from contextlib import nullcontext
import io
from pathlib import Path
import sys
import types
import unittest
from unittest import mock

from native_runtime_models import NativeRuntimeModelError
from translation_models import PreparedTranslationModel
import translate_runner


class TranslateRunnerTest(unittest.TestCase):
    def setUp(self):
        self.directory = Path("/prepared/translation-model")
        self.prepared = PreparedTranslationModel(
            "Helsinki-NLP/opus-mt-en-es", "opus-mt", self.directory
        )
        self.tokenizer = mock.Mock()
        self.tokenizer.return_value = {"input_ids": [1]}
        self.tokenizer.batch_decode.return_value = ["hola"]
        self.model = mock.Mock()
        self.model.generate.return_value = [1]
        self.auto_tokenizer = mock.Mock(from_pretrained=mock.Mock(return_value=self.tokenizer))
        self.auto_model = mock.Mock(from_pretrained=mock.Mock(return_value=self.model))
        self.modules = {
            "transformers": types.SimpleNamespace(
                AutoTokenizer=self.auto_tokenizer,
                AutoModelForSeq2SeqLM=self.auto_model,
            ),
            "torch": types.SimpleNamespace(no_grad=nullcontext),
        }

    def call_main(self, *, source="en", target="es", model=None, segments=None):
        args = ["translate_runner.py", "--src", source, "--tgt", target]
        if model is not None:
            args.extend(["--model", model])
        stdout, stderr = io.StringIO(), io.StringIO()
        payload = '{"segments":' + repr(segments if segments is not None else ["hello"]).replace("'", '"') + "}"
        with mock.patch.object(sys, "argv", args), mock.patch.object(sys, "stdin", io.StringIO(payload)), \
                mock.patch.object(sys, "stdout", stdout), mock.patch.object(sys, "stderr", stderr), \
                mock.patch.dict(sys.modules, self.modules):
            translate_runner.main()
        return stdout.getvalue(), stderr.getvalue()

    def test_prepared_model_loads_only_its_directory_with_local_files_only(self):
        with mock.patch.object(translate_runner, "prepared_translation_model", return_value=self.prepared), \
                mock.patch.object(translate_runner, "_resolve_models", side_effect=AssertionError("fallback candidate")):
            stdout, _ = self.call_main()
        self.auto_tokenizer.from_pretrained.assert_called_once_with(
            str(self.directory), local_files_only=True
        )
        self.auto_model.from_pretrained.assert_called_once_with(
            str(self.directory), local_files_only=True
        )
        self.assertIn('"model": "Helsinki-NLP/opus-mt-en-es"', stdout)
        self.assertIn('"backend": "opus-mt"', stdout)

    def test_prepared_load_failure_never_tries_host_cache_or_second_candidate(self):
        stderr = io.StringIO()
        with mock.patch.object(translate_runner, "prepared_translation_model", return_value=self.prepared), \
                mock.patch.object(translate_runner, "_load", side_effect=RuntimeError("not present")), \
                mock.patch.object(translate_runner, "_resolve_models", side_effect=AssertionError("fallback candidate")), \
                mock.patch.object(sys, "argv", ["translate_runner.py", "--src", "en", "--tgt", "es"]), \
                mock.patch.object(sys, "stdin", io.StringIO('{"segments":["hello"]}')), \
                mock.patch.object(sys, "stderr", stderr):
            with self.assertRaises(SystemExit):
                translate_runner.main()
        self.assertIn("does not download models or use a host cache", stderr.getvalue())

    def test_invalid_prepared_selection_never_enters_ordinary_model_resolution(self):
        stderr = io.StringIO()
        with mock.patch.object(
            translate_runner,
            "prepared_translation_model",
            side_effect=NativeRuntimeModelError("prepared group is absent"),
        ), mock.patch.object(translate_runner, "_resolve_models", side_effect=AssertionError("fallback candidate")), \
                mock.patch.object(sys, "argv", ["translate_runner.py", "--src", "en", "--tgt", "es"]), \
                mock.patch.object(sys, "stdin", io.StringIO('{"segments":["hello"]}')), \
                mock.patch.object(sys, "stderr", stderr):
            with self.assertRaises(SystemExit):
                translate_runner.main()
        self.assertIn("prepared local translation rejected", stderr.getvalue())

    def test_no_prepared_context_keeps_ordinary_first_candidate_behavior(self):
        with mock.patch.object(translate_runner, "prepared_translation_model", return_value=None), \
                mock.patch.object(translate_runner, "_resolve_models", return_value=[("normal-id", "custom")]), \
                mock.patch.object(translate_runner, "_load", return_value=(self.tokenizer, self.model)) as load:
            stdout, _ = self.call_main(source="lv", target="es")
        load.assert_called_once_with("normal-id", local_files_only=False)
        self.assertIn('"model": "normal-id"', stdout)
        self.assertIn('"backend": "custom"', stdout)


if __name__ == "__main__":
    unittest.main()
