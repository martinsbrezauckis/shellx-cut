use super::*;

#[test]
fn grok_translation_command_removes_tools_without_widening_other_providers() {
    let command = crate::translate::build_cli_command("grok", None).unwrap();
    for pair in [
        ["--tools", "read_file,grep,list_dir"],
        ["--disallowed-tools", "read_file,grep,list_dir,Agent"],
        ["--deny", "MCPTool"],
        ["--sandbox", "read-only"],
        ["--permission-mode", "dontAsk"],
    ] {
        assert!(command.args.windows(2).any(|args| args == pair));
    }
    assert!(command.args.contains(&"--no-subagents".into()));
    assert!(!command.args.contains(&"bypassPermissions".into()));
    assert!(crate::translate::build_cli_command("codex", None)
        .unwrap()
        .args
        .windows(2)
        .any(|args| args == ["--sandbox", "read-only"]));
    for flag in REQUIRED_FLAGS {
        let near_match = REQUIRED_FLAGS
            .iter()
            .map(|candidate| {
                if candidate == flag {
                    format!("{flag}-future")
                } else {
                    candidate.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            crate::chat::broker::missing_required_help_tokens(&near_match, REQUIRED_FLAGS),
            vec![*flag]
        );
    }
}

#[cfg(unix)]
fn fixture(root: &Path, help: &str, help_exit: i32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let executable = root.join("grok");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--help" ]; then
    printf '%s\n' '{help}'
    exit {help_exit}
fi
printf '%s\n' "$@" > '{root}/turn-args'
pwd > '{root}/turn-cwd'
printf '%s' "$HOME" > '{root}/turn-home'
if [ "$1" != "--prompt-file" ] || [ ! -f "$2" ]; then exit 7; fi
/bin/cat "$2" > '{root}/turn-prompt'
printf '%s\n' '{{"status":"SUCCESS","response":"[{{\"i\":1,\"text\":\"Hola\"}}]"}}'
"#,
        root = root.display()
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let home = root.join("synthetic-home");
    std::fs::create_dir(&home).unwrap();
    let context = root.join("context.json");
    std::fs::write(
        &context,
        serde_json::to_vec(&serde_json::json!({
            "schema": "release-runner.provider-runtime/v1",
            "admission": {
                "schema": "release-runner.provider-runtime/v1",
                "resourceLease": "provider-login:test:grok",
                "enrollment": {
                    "id": "translation-grok", "provider": "grok", "userIdentity": "uid:1000",
                    "executable": {"path": executable, "sha256": "a".repeat(64)},
                    "runtimeCode": [], "canonicalEnvironment": {"home": home}
                }
            },
            "effectiveEnvironment": {"HOME": home, "PATH": "/usr/bin:/bin"}
        }))
        .unwrap(),
    )
    .unwrap();
    context
}

#[cfg(unix)]
#[test]
fn grok_translation_missing_or_failed_help_never_launches_a_turn() {
    for (help, exit) in [
        (
            REQUIRED_FLAGS
                .iter()
                .filter(|flag| **flag != "--deny")
                .copied()
                .collect::<Vec<_>>()
                .join(" "),
            0,
        ),
        (REQUIRED_FLAGS.join(" "), 1),
    ] {
        let root = tempfile::tempdir_in(crate::provider_runtime::test_fixture_root()).unwrap();
        let context = fixture(root.path(), &help, exit);
        let _context =
            crate::provider_runtime::install_test_provider_context(context.into_os_string());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = rt
            .block_on(crate::translate::run_translation_cli_agent(
                "grok",
                Some("en"),
                "es",
                &["Hello".into()],
                None,
                Some(10_000),
            ))
            .err()
            .expect("unsupported help must refuse translation");
        assert_eq!(error.code, error_codes::SIDECAR);
        assert!(error.message.contains("translation no-tool contract"));
        assert!(!root.path().join("turn-args").exists());
    }
}

#[cfg(unix)]
#[test]
fn grok_translation_admitted_help_preserves_environment_transport_and_parse() {
    let root = tempfile::tempdir_in(crate::provider_runtime::test_fixture_root()).unwrap();
    let context = fixture(root.path(), &REQUIRED_FLAGS.join(" "), 0);
    let _context = crate::provider_runtime::install_test_provider_context(context.into_os_string());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = rt
        .block_on(crate::translate::run_translation_cli_agent(
            "grok",
            Some("en"),
            "es",
            &["Hello".into()],
            Some("selected-model"),
            Some(10_000),
        ))
        .unwrap();
    assert_eq!(outcome.translations, ["Hola"]);
    assert_eq!(outcome.backend, "cli");
    assert_eq!(outcome.agent.as_deref(), Some("grok"));
    assert!(!outcome.proven);
    let args = std::fs::read_to_string(root.path().join("turn-args")).unwrap();
    assert!(args.contains("--deny\nMCPTool\n"));
    assert!(args.contains("--model\nselected-model\n"));
    assert!(!args.contains("bypassPermissions"));
    let prompt = std::fs::read_to_string(root.path().join("turn-prompt")).unwrap();
    assert!(prompt.contains("1. Hello"));
    assert_eq!(
        std::fs::read_to_string(root.path().join("turn-home")).unwrap(),
        root.path().join("synthetic-home").to_str().unwrap()
    );
    let cwd = std::fs::read_to_string(root.path().join("turn-cwd")).unwrap();
    assert!(
        !Path::new(cwd.trim()).exists(),
        "translation workspace must be cleaned"
    );
}
