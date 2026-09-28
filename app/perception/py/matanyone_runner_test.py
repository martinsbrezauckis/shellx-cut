"""Offline MatAnyone2 loader regression; no ML package or checkpoint is required."""

from __future__ import annotations

from contextlib import contextmanager
import importlib.util
from pathlib import Path
import sys
import types
import unittest
from unittest import mock


HERE = Path(__file__).resolve().parent
RUNNER = HERE / "matanyone_runner.py"


def package(name: str) -> types.ModuleType:
    module = types.ModuleType(name)
    module.__path__ = []
    return module


class MatAnyoneRunnerTest(unittest.TestCase):
    def load_runner(self):
        torch = package("torch")
        torch.load = mock.Mock(name="torch.load", return_value={"full": "checkpoint"})
        torch_nn = package("torch.nn")
        torch_functional = types.ModuleType("torch.nn.functional")
        torch_nn.functional = torch_functional
        torch.nn = torch_nn

        numpy = types.ModuleType("numpy")
        pil = package("PIL")
        image = types.ModuleType("PIL.Image")
        image.MAX_IMAGE_PIXELS = None
        pil.Image = image
        safe_numbers = types.ModuleType("safe_numbers")
        safe_numbers.finite_number = lambda value: value
        stubs = {
            "torch": torch,
            "torch.nn": torch_nn,
            "torch.nn.functional": torch_functional,
            "numpy": numpy,
            "PIL": pil,
            "PIL.Image": image,
            "safe_numbers": safe_numbers,
        }
        spec = importlib.util.spec_from_file_location("cut_matanyone_runner_test", RUNNER)
        self.assertIsNotNone(spec)
        self.assertIsNotNone(spec.loader)
        module = importlib.util.module_from_spec(spec)
        with mock.patch.dict(sys.modules, stubs):
            spec.loader.exec_module(module)
        return module, torch

    def test_load_model_uses_full_local_checkpoint_without_resnet_download(self):
        runner, torch = self.load_runner()
        checkpoint = "/admitted/models/matanyone2.pth"
        device = object()
        cfg = types.SimpleNamespace(model=types.SimpleNamespace(), weights=None)
        initialize_config_dir = mock.Mock(name="initialize_config_dir")
        compose = mock.Mock(name="compose", return_value=cfg)

        @contextmanager
        def open_dict(value):
            yield value

        model = mock.Mock(name="matanyone2-model")
        model.to.return_value = model
        model.eval.return_value = model
        model.cfg = object()
        matanyone2 = package("matanyone2")
        matanyone2.__file__ = "/admitted/site-packages/matanyone2/__init__.py"
        inference = package("matanyone2.inference")
        inference_core = types.ModuleType("matanyone2.inference.inference_core")
        processor = object()
        inference_core.InferenceCore = mock.Mock(return_value=processor)
        inference.inference_core = inference_core
        model_package = package("matanyone2.model")
        model_module = types.ModuleType("matanyone2.model.matanyone2")
        model_module.MatAnyone2 = mock.Mock(return_value=model)
        model_package.matanyone2 = model_module
        hydra = types.ModuleType("hydra")
        hydra.compose = compose
        hydra.initialize_config_dir = initialize_config_dir
        omegaconf = types.ModuleType("omegaconf")
        omegaconf.open_dict = open_dict
        hub = types.ModuleType("torch.hub")
        download = mock.Mock(side_effect=AssertionError("network download is forbidden"))
        hub.load_state_dict_from_url = download

        stubs = {
            "hydra": hydra,
            "omegaconf": omegaconf,
            "matanyone2": matanyone2,
            "matanyone2.inference": inference,
            "matanyone2.inference.inference_core": inference_core,
            "matanyone2.model": model_package,
            "matanyone2.model.matanyone2": model_module,
            "torch.hub": hub,
        }
        with mock.patch.dict(sys.modules, stubs):
            actual = runner.load_model(checkpoint, device)

        self.assertIs(actual, processor)
        initialize_config_dir.assert_called_once_with(
            version_base="1.3.2",
            config_dir="/admitted/site-packages/matanyone2/config",
            job_name="shellx_cut_matanyone",
        )
        compose.assert_called_once_with(config_name="eval_matanyone_config")
        self.assertEqual(cfg.weights, checkpoint)
        self.assertFalse(cfg.model.pretrained_resnet)
        model_module.MatAnyone2.assert_called_once_with(cfg, single_object=True)
        model.to.assert_called_once_with(device)
        model.eval.assert_called_once_with()
        torch.load.assert_called_once_with(checkpoint, map_location=device)
        model.load_weights.assert_called_once_with({"full": "checkpoint"})
        inference_core.InferenceCore.assert_called_once_with(model, cfg=model.cfg)
        download.assert_not_called()


if __name__ == "__main__":
    unittest.main()
