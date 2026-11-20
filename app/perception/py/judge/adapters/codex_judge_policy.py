"""Pure Codex render-judge capability assessment.

Image and JSON transport are available in Codex CLI, but they do not establish
the judge's file/tool boundary.  In particular, local permission profiles do
not govern MCP, browser, web, or app tools.  Keep these distinct so a CLI flag
probe cannot accidentally admit a model turn.
"""

from __future__ import annotations

import os
import re
import json
from copy import deepcopy

TRANSPORT_FLAGS = ("--image", "--output-schema", "--ephemeral",
                   "--output-last-message", "--skip-git-repo-check")
CONFINEMENT_REASON = (
    "Codex tool-free effective policy and native direct-image confinement "
    "have not been qualified for this installed CLI"
)
_SAFE_MCP_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_-]*\Z")
_DISABLED_FEATURES = (
    "shell_tool", "view_image", "code_mode", "code_mode_only", "code_mode_host",
    "apps", "plugins", "multi_agent", "multi_agent_v2",
    "tool_search", "tool_suggest", "image_generation",
    "browser_use", "browser_use_external", "browser_use_full_cdp_access",
    "in_app_browser", "computer_use", "standalone_web_search",
    "sleep_tool", "deferred_executor", "request_permissions_tool", "token_budget",
    "current_time_reminder", "send_message_to_user_async",
    "default_mode_request_user_input",
    # Default-on optional execution, extension, and background surfaces in
    # tagged 0.162 FEATURES. The job has no reason to start any of them.
    "daemon_auto_start", "unified_exec_tty",
    "unified_exec_zsh_fork", "shell_snapshot", "worktrees",
    "remote_plugin", "plugin_sharing", "skill_mcp_dependency_install",
    "skill_search", "workspace_dependencies", "realtime_conversation",
    "goals", "in_app_local_automation", "api_key_model_discovery",
    # Installed opt-ins observed by R9. Memory can read/write file-backed
    # state; migration and sleep control have no role in a bounded judge job.
    "memories", "external_migration", "prevent_idle_sleep",
)
_BENIGN_ENABLED_FEATURES = frozenset("""content_item_kinds instant_interrupt
    terminal_resize_reflow sqlite enable_request_compression item_ids
    resize_all_images mentions_v2 steer collaboration_modes tui_app_server
    guardian_approval guardian_reuse_parent_compaction compaction_image_budget
    guardianv2.thread_context
    fast_mode ultrafast_mode browser_annotation_api in_app_chat
    in_app_dictation in_app_voice in_app_updates system_proxy_fallback
    unbounded_connection_retries tool_search_always_defer_mcp_tools
    write_stdin_approval tool_call_mcp_elicitation auth_elicitation
    hooks unified_exec secret_auth_storage""".split())
# ManagedFeatures forces unified_exec on without an administrator pin. In the
# selected catalog shell_type=disabled and effective shell_tool=false; tagged
# add_shell_tools returns before either unified-exec runtime is registered.
# SecretAuthStorage selects the canonical credential backend, not a model tool.
# Preserve the user's configured or platform-default choice without override.
_SAFE_FEATURE_NAME = re.compile(r"[a-z][a-z0-9_]{0,63}(?:\.[a-z][a-z0-9_]{0,63})?\Z")
_HOOK_CONFIG_EVENTS = frozenset(("PreToolUse", "PermissionRequest", "PostToolUse",
    "PreCompact", "PostCompact", "SessionStart", "SessionEnd",
    "UserPromptSubmit", "SubagentStart", "SubagentStop", "Stop", "Interrupt"))


def extension_config_failure(config: object) -> str | None:
    """Check typed hook shape; hooks/list judges effective enabled handlers."""
    if not isinstance(config, dict):
        return "shape"
    if config.get("notify") not in (None, []):
        return "notify"
    hooks = config.get("hooks")
    if hooks is None:
        return None
    if not isinstance(hooks, dict) or not set(hooks).issubset(_HOOK_CONFIG_EVENTS | {"state"}):
        return "hooks_shape"
    if any(not isinstance(value, list) for key, value in hooks.items()
           if key in _HOOK_CONFIG_EVENTS):
        return "hooks_shape"
    if "state" in hooks and not isinstance(hooks["state"], dict):
        return "hooks_shape"
    return None


def _disabled_feature_value(name: str, value: object) -> bool:
    # Tagged FeaturesToml uses FeatureToml<bool|config> for these fields;
    # --config may merge the bool as an `enabled=false` config leaf.
    return value is False or (name in _STRUCTURED_FEATURES
                              and isinstance(value, dict)
                              and value.get("enabled") is False)


def effective_value_diagnostics(response: object, mcp_names: object,
                                catalog_path: str) -> tuple[str, ...]:
    """Independent fixed failure categories; no config values or names escape."""
    config = response.get("config") if isinstance(response, dict) else None
    if not isinstance(config, dict):
        return ("shape",)
    failures = []
    if config.get("model_catalog_json") != catalog_path:
        failures.append("catalog_value")
    if config.get("web_search") != "disabled":
        failures.append("web_value")
    cloud = config.get("cloud")
    if (not isinstance(cloud, dict) or not isinstance(cloud.get("skills"), dict)
            or cloud["skills"].get("enabled") is not False):
        failures.append("cloud_skills_value")
    extension = extension_config_failure(config)
    if extension:
        failures.append("extension_" + extension)
    features = config.get("features")
    if not isinstance(features, dict):
        failures.append("features_shape")
    elif any(not _disabled_feature_value(name, features.get(name))
             for name in _DISABLED_FEATURES):
        failures.append("feature_value")
    servers = config.get("mcp_servers")
    if not isinstance(servers, dict) or not isinstance(mcp_names, (list, tuple)):
        failures.append("mcp_shape")
    else:
        if set(servers) != set(mcp_names):
            failures.append("mcp_names")
        if any(not isinstance(server, dict) or server.get("enabled") is not False
               for server in servers.values()):
            failures.append("mcp_value")
    return tuple(failures)


def disabled_feature_value_shapes(response: object) -> tuple[tuple[str, str], ...]:
    """Report bounded requested feature keys/types only, never their values."""
    config = response.get("config") if isinstance(response, dict) else None
    features = config.get("features") if isinstance(config, dict) else None
    if not isinstance(features, dict):
        return ()
    def shape(value: object) -> str:
        if value is None:
            return "missing"
        if type(value) is bool:
            return "bool"
        if isinstance(value, dict):
            return "object"
        return "other"
    return tuple((name, shape(features.get(name))) for name in _DISABLED_FEATURES
                 if not _disabled_feature_value(name, features.get(name)))[:32]


def unadmitted_enabled_feature_names(rows: object) -> tuple[str, ...]:
    """Return only bounded names, never feature descriptions or config values."""
    if not isinstance(rows, list):
        return ()
    names = {row["name"] for row in rows if isinstance(row, dict)
             and isinstance(row.get("name"), str) and row.get("enabled") is True}
    return tuple(sorted(name for name in names if _SAFE_FEATURE_NAME.fullmatch(name)
                        and name not in _DISABLED_FEATURES
                        and name not in _BENIGN_ENABLED_FEATURES))[:32]


def still_enabled_disabled_feature_names(rows: object) -> tuple[str, ...]:
    """Bounded names of requested disables not effective in the same process."""
    if not isinstance(rows, list):
        return ()
    names = {row.get("name") for row in rows if isinstance(row, dict)
             and row.get("enabled") is True and row.get("name") in _DISABLED_FEATURES}
    return tuple(sorted(names))[:64]


def feature_inventory_shape_diagnostics(rows: object) -> tuple[dict, ...]:
    """Bounded field names/types from feature metadata, never descriptions."""
    if not isinstance(rows, list):
        return ({"name": "<array>", "failure": "rows_shape"},)
    result = []
    for row in rows:
        if not isinstance(row, dict):
            result.append({"name": "<row>", "failure": "row_shape"})
            continue
        name = row.get("name")
        safe = (name if isinstance(name, str) and len(name) <= 129
                and re.fullmatch(r"[A-Za-z0-9_.-]+", name) else "<invalid>")
        if not isinstance(name, str) or not _SAFE_FEATURE_NAME.fullmatch(name):
            result.append({"name": safe, "failure": "name_shape"})
        for key in ("enabled", "defaultEnabled"):
            if type(row.get(key)) is not bool:
                result.append({"name": safe, "failure": key + "_type"})
        if len(result) >= 16:
            break
    return tuple(result[:16])


def feature_inventory_failure(rows: object) -> str | None:
    """Attest effective default feature state, including omitted config leaves."""
    if not isinstance(rows, list) or not rows:
        return "shape"
    seen = set()
    for row in rows:
        if (not isinstance(row, dict) or not isinstance(row.get("name"), str)
                or not _SAFE_FEATURE_NAME.fullmatch(row["name"])
                or type(row.get("enabled")) is not bool
                or type(row.get("defaultEnabled")) is not bool):
            return "shape"
        if row["name"] in seen:
            return "duplicate"
        seen.add(row["name"])
    if not set(_DISABLED_FEATURES).issubset(seen):
        return "missing_disabled"
    if any(row["enabled"] for row in rows if row["name"] in _DISABLED_FEATURES):
        return "disabled_enabled"
    if unadmitted_enabled_feature_names(rows):
        return "unknown_enabled"
    return None


_MANAGED_HOOK_SOURCES = frozenset(("system", "mdm", "cloudRequirements",
                                   "cloudManagedConfig", "legacyManagedConfigFile",
                                   "legacyManagedConfigMdm"))
_HOOK_EVENTS = frozenset(("preToolUse", "permissionRequest", "postToolUse",
                          "preCompact", "postCompact", "sessionStart", "sessionEnd",
                          "userPromptSubmit", "subagentStart", "subagentStop",
                          "stop", "interrupt"))
_HOOK_FIELDS = frozenset(("key", "eventName", "handlerType", "matcher", "timeoutSec",
                          "statusMessage", "additionalContextLimit", "sourcePath",
                          "source", "pluginId", "displayOrder", "enabled", "isManaged",
                          "currentHash", "trustStatus"))
_HOOK_SOURCE_DIAG = _MANAGED_HOOK_SOURCES | frozenset(
    ("user", "project", "sessionFlags", "plugin", "unknown"))
_HOOK_TRUST_DIAG = frozenset(("managed", "untrusted", "trusted", "modified"))


def enabled_unmanaged_hook_metadata(response: object) -> tuple[dict, ...]:
    """Identify a hook by tagged enum metadata, never path or executable."""
    data = response.get("data") if isinstance(response, dict) else None
    if not isinstance(data, list) or len(data) != 1 or not isinstance(data[0], dict):
        return ()
    rows = data[0].get("hooks")
    if not isinstance(rows, list):
        return ()
    result = []
    for row in rows:
        if not isinstance(row, dict) or row.get("enabled") is not True or row.get("isManaged") is True:
            continue
        def enum(key: str, allowed: frozenset[str]) -> str:
            value = row.get(key)
            return value if isinstance(value, str) and value in allowed else "other"
        result.append({"source": enum("source", _HOOK_SOURCE_DIAG),
                       "trustStatus": enum("trustStatus", _HOOK_TRUST_DIAG),
                       "eventName": enum("eventName", _HOOK_EVENTS),
                       "handlerType": enum("handlerType", frozenset(("command", "mcpTool", "prompt", "agent")))})
        if len(result) >= 4:
            break
    return tuple(result)


def hook_inventory_failure(response: object, cwd: str) -> str | None:
    """Keep managed safeguards, refuse visible unmanaged lifecycle commands."""
    if not isinstance(response, dict) or not isinstance(response.get("data"), list):
        return "shape"
    data = response["data"]
    if len(data) != 1 or not isinstance(data[0], dict):
        return "shape"
    row = data[0]
    if (set(row) != {"cwd", "hooks", "warnings", "errors"}
            or row.get("cwd") != cwd or not isinstance(row.get("hooks"), list)):
        return "cwd_or_shape"
    if row.get("warnings") != [] or row.get("errors") != []:
        return "warnings_or_errors"
    if len(row["hooks"]) > 128:
        return "limit"
    for hook in row["hooks"]:
        if not isinstance(hook, dict) or type(hook.get("enabled")) is not bool:
            return "shape"
        if not hook["enabled"]:
            continue
        managed = hook.get("isManaged") is True and hook.get("trustStatus") == "managed"
        # Tagged tool-event dispatch requires an actual tool invocation (or
        # permission request). Keep trusted user/project guards in place; the
        # narrowed model catalog has no admitted tool runtime to trigger them.
        tool_guard = (hook.get("isManaged") is False
                      and hook.get("trustStatus") == "trusted"
                      and hook.get("handlerType") == "command"
                      and hook.get("eventName") in
                      ("preToolUse", "permissionRequest", "postToolUse"))
        if not (managed or tool_guard):
            return "unmanaged_enabled"
        handler = hook.get("handlerType")
        extra = (frozenset(("command", "async")) if handler == "command" else
                 frozenset(("server", "tool")) if handler == "mcpTool" else None)
        if extra is None or set(hook) != _HOOK_FIELDS | extra:
            return "handler_shape"
        if (not isinstance(hook.get("eventName"), str)
                or hook["eventName"] not in _HOOK_EVENTS
                or not isinstance(hook.get("source"), str)
                or hook["source"] not in (_MANAGED_HOOK_SOURCES if managed else
                                          _HOOK_SOURCE_DIAG - {"unknown"})
                or not isinstance(hook.get("sourcePath"), str)
                or not os.path.isabs(hook["sourcePath"])
                or not isinstance(hook.get("key"), str)
                or not isinstance(hook.get("currentHash"), str)
                or type(hook.get("timeoutSec")) is not int
                or type(hook.get("displayOrder")) is not int):
            return "hook_metadata"
        if handler == "command" and (not isinstance(hook.get("command"), str)
                                      or type(hook.get("async")) is not bool):
            return "handler_shape"
        if handler == "mcpTool" and (not isinstance(hook.get("server"), str)
                                     or not isinstance(hook.get("tool"), str)):
            return "handler_shape"
    return None

# Exact field grammar for the tagged bundled catalog and the serialized
# ModelsResponse returned by `codex debug models --bundled`. That response
# adds a legacy base_instructions mirror and serialized ModelInfo defaults.
# Unknown fields may register a tool or change its safeguards and are refused.
# A null field becoming an object is refused unless its object keys are listed.
# Source: codex-rs/protocol/src/openai_models.rs at rust-v0.162.0.
_BUNDLED_OBJECT_FIELDS = {
    "": """additional_speed_tiers apply_patch_tool_type auto_compact_token_limit
        auto_review_model_override availability_nux available_in_plans
        available_access_programs base_instructions comp_hash
        context_window default_reasoning_level default_reasoning_summary
        default_service_tier default_verbosity description display_name
        effective_context_window_percent
        experimental_supported_tools guardian include_apps_usage_instructions
        include_plugin_usage_instructions include_skills_usage_instructions
        input_modalities max_context_window minimal_client_version model_messages
        model_specialty multi_agent_reasoning_effort multi_agent_version
        node_repl_auto_review_required node_repl_disabled prefer_websockets priority
        requires_sandboxed_review service_tiers shell_type slug support_verbosity
        supported_in_api supported_reasoning_levels supports_experimental_context
        supports_image_detail_original supports_parallel_tool_calls
        supports_reasoning_effort_updates supports_reasoning_summaries
        supports_reasoning_summary_parameter supports_search_tool tool_mode
        truncation_policy upgrade use_responses_lite visibility web_search_tool_type""",
    "availability_nux": "message",
    "available_access_programs": "cyber",
    "model_messages": """approvals auto_review collaboration_modes confirmation_policies
        content_filter_guidance guardian_v2 instructions_template instructions_variables multi_agent
        permissions persistent_instructions token_budget tools""",
    "model_messages.approvals": "never on_request on_request_auto_review unless_trusted",
    "model_messages.auto_review": """node_repl_policy policy policy_template
        rejection_instructions timeout_instructions""",
    "model_messages.collaboration_modes": "default plan",
    "model_messages.confirmation_policies": "browser_use computer_use",
    "model_messages.guardian_v2": """classifier_instructions max_action_tokens
        max_classifier_instruction_tokens max_parent_compaction_tokens
        max_tool_call_lag reasoning_effort reuse_parent_compaction
        review_threshold_basis_points transcript""",
    "model_messages.multi_agent": "mode role",
    "model_messages.multi_agent.role": "root subagent",
    "model_messages.permissions": "danger_full_access read_only workspace_write",
    "model_messages.token_budget": """auto_compact_fallback_buffer_tokens
        auto_compact_fallback_prompt enabled guidance_message reminder_message_template
        reminder_threshold_tokens use_history_notes_extension""",
    "service_tiers[]": "description id name",
    "supported_reasoning_levels[]": "description effort",
    "truncation_policy": "limit mode",
    "upgrade": "migration_markdown model retirement_at",
}
_BUNDLED_OBJECT_FIELDS = {path: frozenset(fields.split())
                          for path, fields in _BUNDLED_OBJECT_FIELDS.items()}
_BUNDLED_LIST_FIELDS = frozenset((
    "additional_speed_tiers", "available_in_plans", "available_access_programs.cyber",
    "experimental_supported_tools",
    "input_modalities", "service_tiers", "supported_reasoning_levels",
))
_BUNDLED_BOOL_FIELDS = frozenset("""include_apps_usage_instructions
    include_plugin_usage_instructions include_skills_usage_instructions
    model_messages.token_budget.enabled model_messages.token_budget.use_history_notes_extension
    node_repl_auto_review_required node_repl_disabled prefer_websockets
    requires_sandboxed_review support_verbosity supported_in_api
    supports_experimental_context supports_image_detail_original
    supports_parallel_tool_calls supports_reasoning_effort_updates
    supports_reasoning_summaries supports_reasoning_summary_parameter
    supports_search_tool use_responses_lite""".split())
_BUNDLED_INT_FIELDS = frozenset("""context_window max_context_window
    effective_context_window_percent
    model_messages.token_budget.auto_compact_fallback_buffer_tokens
    model_messages.token_budget.reminder_threshold_tokens priority
    truncation_policy.limit""".split())
_BUNDLED_NULL_ONLY_FIELDS = frozenset("""auto_compact_token_limit
    auto_review_model_override guardian model_messages.approvals.on_request
    model_messages.approvals.unless_trusted model_messages.auto_review.timeout_instructions
    model_messages.collaboration_modes.plan model_messages.guardian_v2.max_action_tokens
    model_messages.guardian_v2.max_classifier_instruction_tokens
    model_messages.guardian_v2.max_parent_compaction_tokens
    model_messages.guardian_v2.max_tool_call_lag
    model_messages.guardian_v2.reasoning_effort
    model_messages.guardian_v2.reuse_parent_compaction
    model_messages.guardian_v2.review_threshold_basis_points
    model_messages.guardian_v2.transcript model_messages.instructions_variables
    model_messages.multi_agent.mode model_messages.tools upgrade.retirement_at""".split())
_BUNDLED_NULLABLE_OBJECTS = frozenset("""availability_nux upgrade
    model_messages.approvals model_messages.auto_review
    model_messages.collaboration_modes model_messages.confirmation_policies
    model_messages.guardian_v2 model_messages.multi_agent
    model_messages.permissions model_messages.token_budget""".split())


class BundledFieldError(ValueError):
    """Fixed error text with a field path kept only for private diagnostics."""

    def __init__(self, message: str, path: str):
        super().__init__(message)
        self.path = path


def _validate_bundled_fields(value: object, path: str = "") -> None:
    if isinstance(value, dict):
        allowed = _BUNDLED_OBJECT_FIELDS.get(path)
        if allowed is None:
            raise BundledFieldError("bundled Codex model contains an unknown object field", path)
        unknown = set(value) - allowed
        if unknown:
            field = sorted(unknown)[0]
            raise BundledFieldError("bundled Codex model contains an unknown object field",
                                    f"{path}.{field}" if path else field)
        for key, child in value.items():
            _validate_bundled_fields(child, f"{path}.{key}" if path else key)
    elif isinstance(value, list):
        if path not in _BUNDLED_LIST_FIELDS:
            raise BundledFieldError("bundled Codex model contains an unknown array field", path)
        for child in value:
            _validate_bundled_fields(child, path + "[]")
    elif path in _BUNDLED_OBJECT_FIELDS and path not in _BUNDLED_NULLABLE_OBJECTS:
        raise BundledFieldError("bundled Codex model has an invalid object value", path)
    elif path in _BUNDLED_NULLABLE_OBJECTS and value is not None:
        raise BundledFieldError("bundled Codex model has an invalid optional object value", path)
    elif path in _BUNDLED_LIST_FIELDS:
        raise BundledFieldError("bundled Codex model has an invalid array value", path)
    elif path in _BUNDLED_BOOL_FIELDS and type(value) is not bool:
        raise BundledFieldError("bundled Codex model has an invalid boolean value", path)
    elif path in _BUNDLED_INT_FIELDS and type(value) is not int:
        raise BundledFieldError("bundled Codex model has an invalid integer value", path)
    elif path in _BUNDLED_NULL_ONLY_FIELDS and value is not None:
        raise BundledFieldError("bundled Codex model has an unsupported value", path)
    elif (path not in _BUNDLED_BOOL_FIELDS and path not in _BUNDLED_INT_FIELDS
          and path not in _BUNDLED_NULL_ONLY_FIELDS
          and path not in _BUNDLED_NULLABLE_OBJECTS
          and path not in _BUNDLED_OBJECT_FIELDS
          and type(value) not in (str, type(None))):
        raise BundledFieldError("bundled Codex model has an invalid text value", path)


def build_job_catalog(bundled: dict, model_slug: str) -> dict:
    """Narrow the installed binary's bundled catalog to one image model.

    `codex debug models --bundled` is a no-auth/no-network source of the full
    matching ModelsResponse.  We preserve every selected model field except
    tool-registration fields; an unavailable or non-vision model is
    refused rather than reconstructed from incomplete model/list metadata.
    """
    if not isinstance(bundled, dict) or set(bundled) != {"models"}:
        raise ValueError("bundled Codex catalog has an unknown top-level field")
    models = bundled.get("models")
    if not isinstance(models, list) or not model_slug:
        raise ValueError("bundled Codex model catalog or selected model is missing")
    selected = [model for model in models if isinstance(model, dict)
                and model.get("slug") == model_slug]
    if not selected:
        raise ValueError("selected Codex model is absent from bundled catalog")
    if len(selected) != 1:
        raise ValueError("selected Codex model is duplicated in bundled catalog")
    if selected[0].get("supported_in_api") is not True:
        raise ValueError("selected Codex model is unavailable in bundled catalog")
    model = deepcopy(selected[0])
    _validate_bundled_fields(model)
    template = model.get("model_messages")
    template = template.get("instructions_template") if isinstance(template, dict) else None
    has_legacy_base = "base_instructions" in model
    legacy_base = model.pop("base_instructions", None)
    if has_legacy_base and (not isinstance(legacy_base, str)
                            or not isinstance(template, str) or legacy_base != template):
        raise ValueError("bundled Codex legacy instructions disagree with model messages")
    if "image" not in model.get("input_modalities", []):
        raise ValueError("selected Codex model does not advertise image input")
    model["apply_patch_tool_type"] = None
    model["shell_type"] = "disabled"
    model["experimental_supported_tools"] = []
    model["supports_search_tool"] = False
    # ModelInfo.tool_mode takes precedence over the CodeMode feature switches.
    # Current GPT-6 bundled entries can force code_mode_only unless changed.
    # ToolMode uses serde snake_case in both tagged 0.160.1 and 0.162.0.
    # "Direct" would silently deserialize as None in the optional selector.
    model["tool_mode"] = "direct"
    model["multi_agent_version"] = None
    model["node_repl_disabled"] = True
    return {"models": [model]}


def effective_mcp_names(config_read_response: dict) -> tuple[str, ...]:
    """Take only server names from app-server config/read; discard all values.

    The RPC may carry transport environment or header values. Callers must
    never log or persist its raw response. Missing/opaque metadata is a refusal,
    not evidence of zero servers.
    """
    config = config_read_response.get("config") if isinstance(config_read_response, dict) else None
    servers = config.get("mcp_servers") if isinstance(config, dict) else None
    if not isinstance(servers, dict) or any(
            not isinstance(name, str) or not _SAFE_MCP_NAME.fullmatch(name)
            for name in servers):
        raise ValueError("effective Codex MCP server names are unavailable or unsupported")
    return tuple(sorted(servers))


def job_config_overrides(mcp_names: object, catalog_path: str) -> list[str]:
    """Build stricter per-process options; never alter canonical configuration.

    Effective server names must come from the provider's config/read metadata
    for this exact cwd and configuration stack. Unrepresentable names fail
    closed until the CLI key-path syntax is verified for them.
    """
    if not os.path.isabs(catalog_path):
        raise ValueError("Codex job catalog path must be absolute")
    if not isinstance(mcp_names, (list, tuple)) or any(
            not isinstance(name, str) or not _SAFE_MCP_NAME.fullmatch(name)
            for name in mcp_names):
        raise ValueError("effective MCP names include an unsupported key")
    if len(set(mcp_names)) != len(mcp_names):
        raise ValueError("effective MCP names are not unique")
    overrides = [f"model_catalog_json={json.dumps(catalog_path)}", *feature_only_overrides()]
    overrides.extend(f"mcp_servers.{name}.enabled=false" for name in sorted(mcp_names))
    return overrides


def feature_only_overrides() -> list[str]:
    """Tool-capability switches for the initial no-thread MCP-name probe."""
    return ['web_search="disabled"', 'cloud.skills.enabled=false',
            'tools.update_plan.enabled=false',
            'tools.experimental_request_user_input.enabled=false',
            *(f"features.{feature}=false" for feature in _DISABLED_FEATURES)]


_STRUCTURED_FEATURES = frozenset(("code_mode", "multi_agent_v2", "sleep_tool"))


def effective_config_failure(response: dict, mcp_names: object, catalog_path: str) -> str | None:
    """Check typed values and winning per-leaf override origins, without logging.

    The v2 ConfigReadResponse typed ``tools`` contains only web_search. Its
    ``origins`` map still reports the winning layer of every TOML leaf. For
    omitted ToolConfig fields, exact argv false plus sessionFlags provenance
    proves no later layer replaced our override; an exact managed requirement
    removes that origin and therefore refuses admission.
    """
    config = response.get("config") if isinstance(response, dict) else None
    origins = response.get("origins") if isinstance(response, dict) else None
    if not isinstance(config, dict) or not isinstance(mcp_names, (list, tuple)):
        return "shape"
    features = config.get("features")
    servers = config.get("mcp_servers")
    if not isinstance(features, dict) or not isinstance(servers, dict) or not isinstance(origins, dict):
        return "shape"
    def origin_failure(paths: list[str], label: str) -> str | None:
        if any(path not in origins for path in paths):
            return "origin_missing_" + label
        if any(not session_flag(path) for path in paths):
            return "origin_overridden_" + label
        return None
    def session_flag(path: str) -> bool:
        meta = origins.get(path)
        return (isinstance(meta, dict) and isinstance(meta.get("name"), dict)
                and meta["name"].get("type") == "sessionFlags")

    groups = {
        "catalog": ["model_catalog_json"], "web": ["web_search"],
        "cloud_skills": ["cloud.skills.enabled"],
        "tools": ["tools.update_plan.enabled",
                  "tools.experimental_request_user_input.enabled"],
        "features": [f"features.{feature}" + (".enabled" if feature in _STRUCTURED_FEATURES else "")
                     for feature in _DISABLED_FEATURES],
        "mcp": [f"mcp_servers.{name}.enabled" for name in mcp_names],
    }
    for label, paths in groups.items():
        failure = origin_failure(paths, label)
        if failure:
            return failure
    known_tool_leaves = {"tools.update_plan.enabled",
                         "tools.experimental_request_user_input.enabled"}
    if any(path.startswith("tools.") and path.endswith(".enabled")
           and path not in known_tool_leaves for path in origins):
        return "unknown_tool_config"
    # A new enabled feature may register an unreviewed tool. Keep managed
    # safeguards intact; unknown true features refuse rather than overriding.
    def enabled(value: object) -> bool:
        return value is True or (isinstance(value, dict) and value.get("enabled") is True)

    if any(enabled(value) for key, value in features.items()
           if key not in _DISABLED_FEATURES and key not in _BENIGN_ENABLED_FEATURES):
        return "unknown_enabled_feature"
    if config.get("model_catalog_json") != catalog_path:
        return "catalog_value"
    if config.get("web_search") != "disabled":
        return "web_value"
    cloud = config.get("cloud")
    if (not isinstance(cloud, dict) or not isinstance(cloud.get("skills"), dict)
            or cloud["skills"].get("enabled") is not False):
        return "cloud_skills_value"
    if extension_config_failure(config) is not None:
        return "extension_config"
    if any(not _disabled_feature_value(feature, features.get(feature))
           for feature in _DISABLED_FEATURES):
        return "feature_value"
    if set(servers) != set(mcp_names):
        return "mcp_names"
    if any(not isinstance(server, dict) or server.get("enabled") is not False
           for server in servers.values()):
        return "mcp_value"
    return None


def explicit_unadmitted_feature_names(response: object) -> tuple[str, ...]:
    """Diagnostic names only from typed config/read feature leaves."""
    config = response.get("config") if isinstance(response, dict) else None
    features = config.get("features") if isinstance(config, dict) else None
    if not isinstance(features, dict):
        return ()
    def enabled(value: object) -> bool:
        return value is True or (isinstance(value, dict) and value.get("enabled") is True)
    return tuple(sorted(key for key, value in features.items()
                        if isinstance(key, str) and _SAFE_FEATURE_NAME.fullmatch(key)
                        and key not in _DISABLED_FEATURES
                        and key not in _BENIGN_ENABLED_FEATURES and enabled(value)))[:32]


def effective_config_matches(response: dict, mcp_names: object, catalog_path: str) -> bool:
    return effective_config_failure(response, mcp_names, catalog_path) is None


def advertised_flags(help_text: str) -> set[str]:
    """Extract complete long-option tokens, never substring matches."""
    return set(re.findall(r"(?<![\w-])--[a-z][a-z0-9-]*\b", help_text))


def transport_status(help_text: str, *, exit_code: int = 0) -> tuple[bool, tuple[str, ...]]:
    """Report only whether direct images and strict output can be requested."""
    if exit_code != 0:
        return False, TRANSPORT_FLAGS
    found = advertised_flags(help_text)
    missing = tuple(flag for flag in TRANSPORT_FLAGS if flag not in found)
    return not missing, missing


def confinement_status(platform: str | None = None) -> tuple[bool, str]:
    """Fail closed until the tool-free route passes native admission."""
    return False, CONFINEMENT_REASON


def judge_admission(help_text: str, *, exit_code: int = 0,
                    platform: str | None = None) -> tuple[bool, str, dict]:
    """Return an honest admission result and receipt-safe capability detail."""
    transport, missing = transport_status(help_text, exit_code=exit_code)
    confined, reason = confinement_status(platform)
    detail = {
        "direct_image_and_schema": transport,
        "missing_transport_flags": list(missing),
        "frame_only_file_and_tool_confinement": confined,
    }
    if not transport:
        return False, (
            "Codex CLI does not advertise required direct-image/schema flags: "
            + ", ".join(missing)), detail
    return confined, reason, detail
