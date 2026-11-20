"""No-model checks for Codex render-judge admission."""

import os
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import codex_judge
import codex_judge_policy as policy


class CodexJudgePolicyTests(unittest.TestCase):
    def test_job_catalog_preserves_model_except_tool_registration(self):
        original = {"models": [{"slug": "vision", "supported_in_api": True,
                                "input_modalities": ["text", "image"],
                                "apply_patch_tool_type": "freeform",
                                "shell_type": "unified_exec",
                                "experimental_supported_tools": ["clock"],
                                "supports_search_tool": True,
                                "tool_mode": "code_mode_only",
                                "multi_agent_version": "v2",
                                "context_window": 272000},
                               {"slug": "other", "supported_in_api": True,
                                "input_modalities": ["text"]}]}
        catalog = policy.build_job_catalog(original, "vision")
        self.assertEqual(len(catalog["models"]), 1)
        model = catalog["models"][0]
        self.assertIsNone(model["apply_patch_tool_type"])
        self.assertEqual(model["shell_type"], "disabled")
        self.assertEqual(model["experimental_supported_tools"], [])
        self.assertFalse(model["supports_search_tool"])
        self.assertEqual(model["tool_mode"], "direct")
        self.assertIsNone(model["multi_agent_version"])
        self.assertIs(model["node_repl_disabled"], True)
        self.assertEqual(model["context_window"], 272000)
        self.assertEqual(original["models"][0]["apply_patch_tool_type"], "freeform")

    def test_job_catalog_refuses_non_image_or_unavailable_model(self):
        original = {"models": [{"slug": "text", "supported_in_api": True,
                                "input_modalities": ["text"]}]}
        with self.assertRaises(ValueError):
            policy.build_job_catalog(original, "text")
        with self.assertRaises(ValueError):
            policy.build_job_catalog(original, "missing")

    def test_known_nested_bundled_schema_preserves_safeguard_messages(self):
        original = {"models": [{
            "slug": "vision", "supported_in_api": True,
            "input_modalities": ["text", "image"],
            "model_messages": {
                "instructions_template": "provider instructions",
                "approvals": {"on_request_auto_review": "review instruction"},
                "guardian_v2": {"classifier_instructions": "classify safely"},
                "confirmation_policies": {"browser_use": "confirm browser"},
                "tools": None,
            },
            "truncation_policy": {"mode": "tokens", "limit": 10000},
            "supported_reasoning_levels": [{"effort": "medium", "description": "normal"}],
            "service_tiers": [{"id": "priority", "name": "Fast", "description": "faster"}],
            "guardian": None,
        }]}
        model = policy.build_job_catalog(original, "vision")["models"][0]
        self.assertEqual(model["model_messages"], original["models"][0]["model_messages"])
        self.assertEqual(model["truncation_policy"], original["models"][0]["truncation_policy"])

    def test_serialized_model_info_wire_fixture_accepts_legacy_mirror_and_defaults(self):
        # ModelsResponse at rust-v0.162.0 serializes ModelInfo with a flattened
        # base_instructions mirror plus effective_context_window_percent.
        model = {"slug": "gpt-6.1-sol", "supported_in_api": True,
                 "input_modalities": ["text", "image"],
                 "base_instructions": "selected model template",
                 "model_messages": {"instructions_template": "selected model template",
                                    "content_filter_guidance": "model guidance"},
                 "effective_context_window_percent": 95,
                 "available_access_programs": {"cyber": ["standard"]},
                 "shell_type": "unified_exec", "apply_patch_tool_type": "freeform",
                 "experimental_supported_tools": ["clock"],
                 "supports_search_tool": True}
        narrowed = policy.build_job_catalog({"models": [model]}, "gpt-6.1-sol")["models"][0]
        self.assertNotIn("base_instructions", narrowed)
        self.assertEqual(narrowed["model_messages"], model["model_messages"])
        self.assertEqual(narrowed["effective_context_window_percent"], 95)
        self.assertEqual(narrowed["available_access_programs"], {"cyber": ["standard"]})
        self.assertEqual(narrowed["shell_type"], "disabled")
        for invalid in (None, "different template"):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(
                    ValueError, "legacy instructions disagree"):
                policy.build_job_catalog({"models": [{**model, "base_instructions": invalid}]},
                                         "gpt-6.1-sol")

    def test_unknown_model_or_nested_schema_fields_fail_closed(self):
        base = {"slug": "vision", "supported_in_api": True,
                "input_modalities": ["text", "image"]}
        candidates = [
            {**base, "future_file_tool": True},
            {**base, "model_messages": {"future_tool": {"enabled": True}}},
            {**base, "model_messages": {"tools": {"future_file_tool": {}}}},
            {**base, "service_tiers": [{"id": "a", "future_tool": "read"}]},
            {**base, "description": {"future_tool": True}},
            {**base, "supports_search_tool": "false"},
            {**base, "priority": True},
            {**base, "model_messages": {"approvals": "new-approval-mode"}},
        ]
        for candidate in candidates:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                policy.build_job_catalog({"models": [candidate]}, "vision")
        with self.assertRaises(ValueError):
            policy.build_job_catalog({"models": [base], "future_tools": []}, "vision")

    def test_job_overrides_and_effective_config_are_fail_closed(self):
        path = "/tmp/judge/model-catalog.json"
        overrides = policy.job_config_overrides(["docs", "store"], path)
        self.assertIn('model_catalog_json="/tmp/judge/model-catalog.json"', overrides)
        self.assertIn("mcp_servers.docs.enabled=false", overrides)
        self.assertIn("features.shell_tool=false", overrides)
        self.assertIn("features.view_image=false", overrides)
        self.assertIn("tools.update_plan.enabled=false", overrides)
        self.assertIn("tools.experimental_request_user_input.enabled=false", overrides)
        self.assertIn("features.request_permissions_tool=false", overrides)
        self.assertIn("cloud.skills.enabled=false", overrides)
        for name in ("memories", "external_migration", "prevent_idle_sleep"):
            self.assertIn(f"features.{name}=false", overrides)
        with self.assertRaises(ValueError):
            policy.job_config_overrides(["docs", "bad.name"], path)
        features = {name: False for name in policy._DISABLED_FEATURES}
        effective = {"model_catalog_json": path, "web_search": "disabled",
                     "cloud": {"skills": {"enabled": False}},
                     "features": features,
                     "mcp_servers": {"docs": {"enabled": False},
                                     "store": {"enabled": False}}}
        leafs = ["model_catalog_json", "web_search", "cloud.skills.enabled",
                 "tools.update_plan.enabled",
                 "tools.experimental_request_user_input.enabled"]
        leafs += [f"features.{name}" + (".enabled" if name in policy._STRUCTURED_FEATURES else "")
                  for name in policy._DISABLED_FEATURES]
        leafs += ["mcp_servers.docs.enabled", "mcp_servers.store.enabled"]
        response = {"config": effective, "origins": {
            leaf: {"name": {"type": "sessionFlags"}} for leaf in leafs}}
        self.assertTrue(policy.effective_config_matches(response, ["docs", "store"], path))
        effective["cloud"]["skills"]["enabled"] = True
        self.assertEqual(policy.effective_config_failure(response, ["docs", "store"], path),
                         "cloud_skills_value")
        self.assertIn("cloud_skills_value",
                      policy.effective_value_diagnostics(response, ["docs", "store"], path))
        effective["cloud"]["skills"]["enabled"] = False
        response["origins"]["cloud.skills.enabled"] = {"name": {"type": "user"}}
        self.assertEqual(policy.effective_config_failure(response, ["docs", "store"], path),
                         "origin_overridden_cloud_skills")
        response["origins"]["cloud.skills.enabled"] = {"name": {"type": "sessionFlags"}}
        effective["features"]["unified_exec"] = True
        self.assertTrue(policy.effective_config_matches(response, ["docs", "store"], path))
        effective["features"]["shell_tool"] = True
        self.assertEqual(policy.effective_config_failure(response, ["docs", "store"], path),
                         "feature_value")
        effective["features"]["shell_tool"] = False
        effective["features"]["code_mode"] = {"enabled": False}
        self.assertTrue(policy.effective_config_matches(response, ["docs", "store"], path))
        effective["features"]["code_mode"] = False
        effective["hooks"] = {name: [] for name in policy._HOOK_CONFIG_EVENTS}
        effective["hooks"]["SessionStart"] = [{"hooks": ["opaque"]}]
        self.assertTrue(policy.effective_config_matches(response, ["docs", "store"], path))
        del effective["hooks"]
        del response["origins"]["features.shell_tool"]
        self.assertEqual(policy.effective_config_failure(response, ["docs", "store"], path),
                         "origin_missing_features")
        response["origins"]["features.shell_tool"] = {"name": {"type": "sessionFlags"}}
        effective["features"]["future_file_tool"] = True
        self.assertFalse(policy.effective_config_matches(response, ["docs", "store"], path))
        effective["features"]["future_file_tool"] = {"enabled": True}
        self.assertFalse(policy.effective_config_matches(response, ["docs", "store"], path))
        del effective["features"]["future_file_tool"]
        response["origins"]["tools.update_plan.enabled"] = {"name": {"type": "user"}}
        self.assertFalse(policy.effective_config_matches(response, ["docs", "store"], path))
        self.assertEqual(policy.effective_config_failure(response, ["docs", "store"], path),
                         "origin_overridden_tools")
        response["origins"]["tools.update_plan.enabled"] = {"name": {"type": "sessionFlags"}}
        response["origins"]["tools.future_file_tool.enabled"] = {"name": {"type": "user"}}
        self.assertFalse(policy.effective_config_matches(response, ["docs", "store"], path))
        del response["origins"]["tools.future_file_tool.enabled"]
        effective["mcp_servers"]["surprise"] = {"enabled": True}
        self.assertFalse(policy.effective_config_matches(response, ["docs", "store"], path))

    def test_mcp_metadata_exposes_names_only_and_refuses_missing_table(self):
        response = {"config": {"mcp_servers": {
            "docs": {"env": {"TOKEN": "do-not-log"}}, "store": {"enabled": True}}}}
        self.assertEqual(policy.effective_mcp_names(response), ("docs", "store"))
        with self.assertRaises(ValueError):
            policy.effective_mcp_names({"config": {}})

    def test_effective_feature_inventory_includes_defaults_and_refuses_tool_flags(self):
        rows = [{"name": name, "enabled": False, "defaultEnabled": True}
                for name in policy._DISABLED_FEATURES]
        rows.append({"name": "guardian_approval", "enabled": True,
                     "defaultEnabled": True})
        rows.append({"name": "unified_exec", "enabled": True,
                     "defaultEnabled": True})
        self.assertIsNone(policy.feature_inventory_failure(rows))
        for name in ("memories", "external_migration", "prevent_idle_sleep"):
            self.assertIn(name, {row["name"] for row in rows})
        rows.append({"name": "future_file_tool", "enabled": True,
                     "defaultEnabled": True})
        self.assertEqual(policy.feature_inventory_failure(rows), "unknown_enabled")
        self.assertEqual(policy.unadmitted_enabled_feature_names(rows), ("future_file_tool",))
        rows[-1]["enabled"] = False
        self.assertIsNone(policy.feature_inventory_failure(rows))
        rows[0]["enabled"] = True
        self.assertEqual(policy.feature_inventory_failure(rows), "disabled_enabled")
        self.assertEqual(policy.still_enabled_disabled_feature_names(rows),
                         (rows[0]["name"],))
        rows.pop(0)
        self.assertEqual(policy.feature_inventory_failure(rows), "missing_disabled")
        rows.append({"name": "guardianv2.thread_context", "enabled": False,
                     "defaultEnabled": False})
        self.assertNotEqual(policy.feature_inventory_failure(rows), "shape")
        rows[-1]["defaultEnabled"] = None
        self.assertEqual(policy.feature_inventory_shape_diagnostics(rows),
                         ({"name": "guardianv2.thread_context",
                           "failure": "defaultEnabled_type"},))

    def test_secret_auth_storage_preserves_backend_without_admitting_unknown_tools(self):
        path = "/tmp/judge/model-catalog.json"
        overrides = policy.job_config_overrides([], path)
        self.assertFalse(any("secret_auth_storage" in item for item in overrides))
        features = {name: False for name in policy._DISABLED_FEATURES}
        features["secret_auth_storage"] = True
        rows = [{"name": name, "enabled": False, "defaultEnabled": True}
                for name in policy._DISABLED_FEATURES]
        rows.append({"name": "secret_auth_storage", "enabled": True,
                     "defaultEnabled": True})
        origins = ["model_catalog_json", "web_search", "cloud.skills.enabled",
                   "tools.update_plan.enabled",
                   "tools.experimental_request_user_input.enabled"]
        origins.extend(f"features.{name}" +
                       (".enabled" if name in policy._STRUCTURED_FEATURES else "")
                       for name in policy._DISABLED_FEATURES)
        response = {"config": {"model_catalog_json": path, "web_search": "disabled",
                               "cloud": {"skills": {"enabled": False}},
                               "features": features, "mcp_servers": {}},
                    "origins": {leaf: {"name": {"type": "sessionFlags"}}
                                for leaf in origins}}
        self.assertIsNone(policy.feature_inventory_failure(rows))
        self.assertTrue(policy.effective_config_matches(response, [], path))
        features["future_file_tool"] = True
        rows.append({"name": "future_file_tool", "enabled": True,
                     "defaultEnabled": False})
        self.assertEqual(policy.feature_inventory_failure(rows), "unknown_enabled")
        self.assertEqual(policy.effective_config_failure(response, [], path),
                         "unknown_enabled_feature")

    def test_hook_inventory_preserves_managed_controls_only(self):
        cwd = "/tmp/neutral-judge-job"
        managed = {"key": "managed-control", "eventName": "preToolUse",
                   "handlerType": "command", "command": "opaque-command", "async": False,
                   "matcher": None, "timeoutSec": 5, "statusMessage": None,
                   "additionalContextLimit": None, "sourcePath": "/managed/hooks.json",
                   "source": "cloudRequirements", "pluginId": None, "displayOrder": 1,
                   "enabled": True, "isManaged": True, "currentHash": "opaque",
                   "trustStatus": "managed"}
        response = {"data": [{"cwd": cwd, "hooks": [managed],
                              "warnings": [], "errors": []}]}
        self.assertIsNone(policy.hook_inventory_failure(response, cwd))
        response["data"][0]["hooks"][0] = {**managed, "isManaged": False}
        self.assertEqual(policy.hook_inventory_failure(response, cwd), "unmanaged_enabled")
        response["data"][0]["hooks"][0] = {**managed, "isManaged": False,
                                           "source": "user", "trustStatus": "trusted"}
        self.assertEqual(policy.enabled_unmanaged_hook_metadata(response),
                         ({"source": "user", "trustStatus": "trusted",
                           "eventName": "preToolUse", "handlerType": "command"},))
        self.assertIsNone(policy.hook_inventory_failure(response, cwd))
        response["data"][0]["hooks"][0]["eventName"] = "sessionStart"
        self.assertEqual(policy.hook_inventory_failure(response, cwd), "unmanaged_enabled")
        response["data"][0]["hooks"][0]["eventName"] = "permissionRequest"
        response["data"][0]["hooks"][0]["trustStatus"] = "untrusted"
        self.assertEqual(policy.hook_inventory_failure(response, cwd), "unmanaged_enabled")
        response["data"][0]["hooks"][0] = {**managed, "handlerType": "agent"}
        self.assertEqual(policy.hook_inventory_failure(response, cwd), "handler_shape")
        response["data"][0]["hooks"][0] = managed
        response["data"][0]["warnings"].append("opaque warning")
        self.assertEqual(policy.hook_inventory_failure(response, cwd), "warnings_or_errors")

    def test_serialized_empty_hook_table_and_safe_value_categories(self):
        # Tagged HooksToml serializes all default event Vecs, even when empty.
        empty_hooks = {name: [] for name in policy._HOOK_CONFIG_EVENTS}
        self.assertIsNone(policy.extension_config_failure({"hooks": empty_hooks,
                                                            "notify": None}))
        self.assertIsNone(policy.extension_config_failure({"hooks": {
            **empty_hooks, "PreToolUse": [{"hooks": ["opaque"]}]}}))
        self.assertEqual(policy.extension_config_failure({"hooks": empty_hooks,
                                                           "notify": ["private command"]}),
                         "notify")
        self.assertIsNone(policy.extension_config_failure({"hooks": {
            **empty_hooks, "state": {"opaque": {"enabled": True}}}}))
        self.assertEqual(policy.extension_config_failure({"hooks": {
            **empty_hooks, "PreToolUse": "opaque"}}), "hooks_shape")
        config = {"model_catalog_json": "/wrong", "web_search": "live",
                  "hooks": empty_hooks, "notify": ["private command"],
                  "features": {name: False for name in policy._DISABLED_FEATURES},
                  "mcp_servers": {"docs": {"enabled": True}}}
        config["features"]["memories"] = {"enabled": False}
        response = {"config": config}
        self.assertEqual(policy.effective_value_diagnostics(response, ["docs"], "/selected"),
                         ("catalog_value", "web_value", "cloud_skills_value", "extension_notify",
                          "feature_value", "mcp_value"))
        self.assertEqual(policy.disabled_feature_value_shapes(response),
                         (("memories", "object"),))
        config["features"]["memories"] = False
        config["features"]["code_mode"] = {"enabled": False}
        self.assertEqual(policy.disabled_feature_value_shapes(response), ())

    def test_direct_image_and_schema_are_transport_only(self):
        help_text = " ".join(policy.TRANSPORT_FLAGS)
        ready, reason, detail = policy.judge_admission(help_text, platform="posix")
        self.assertFalse(ready)
        self.assertTrue(detail["direct_image_and_schema"])
        self.assertFalse(detail["frame_only_file_and_tool_confinement"])
        self.assertIn("tool", reason)

    def test_partial_flag_spelling_does_not_admit_transport(self):
        help_text = "--images --output-schema-extra --ephemeral --output-last-message --skip-git-repo-check"
        ready, _, detail = policy.judge_admission(help_text)
        self.assertFalse(ready)
        self.assertEqual(detail["missing_transport_flags"], ["--image", "--output-schema"])

    def test_windows_tool_free_route_remains_pending_native_admission(self):
        ready, reason, detail = policy.judge_admission(
            " ".join(policy.TRANSPORT_FLAGS), platform="nt")
        self.assertFalse(ready)
        self.assertIn("native direct-image confinement", reason)
        self.assertFalse(detail["frame_only_file_and_tool_confinement"])

    def test_detect_runs_metadata_only_and_never_claims_ready(self):
        launch = {"executable": "/usr/bin/codex", "entrypoint": None,
                  "environment": {"PATH": "/usr/bin"}}
        calls = []

        def completed(argv, *_args, **_kwargs):
            calls.append(argv)
            output = "codex-cli 0.160.1" if argv[-1] == "--version" else " ".join(policy.TRANSPORT_FLAGS)
            return output.encode()

        with patch.object(codex_judge, "NATIVE_ADMISSION_VERIFIED", False), \
                patch.object(codex_judge.codex_judge_runtime, "run_bounded", side_effect=completed), \
                patch.object(codex_judge.provider_child_launch, "environment", return_value={"PATH": "/usr/bin"}):
            detected = codex_judge.detect(launch)
        self.assertEqual(calls, [["/usr/bin/codex", "--version"],
                                 ["/usr/bin/codex", "exec", "--help"]])
        self.assertTrue(detected["found"])
        self.assertFalse(detected["judge_ready"])
        self.assertTrue(detected["capabilities"]["direct_image_and_schema"])


if __name__ == "__main__":
    unittest.main()
