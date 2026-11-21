use super::super::*;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn executable(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn nice(tool: &Path) -> Command {
    let mut command = Command::new("nice");
    command.args(["-n", "10"]).arg(tool).arg("-version");
    command
}

fn assert_missing(error: CutError, stem: &str) {
    assert_eq!(error.code, error_codes::TOOL_MISSING, "{error:?}");
    assert_eq!(error.message, format!("{stem} is not installed"));
    assert_eq!(error.requirement.unwrap().id, stem);
    assert!(error.suggested_action.is_some());
}

#[test]
fn removed_tools_through_nice_refuse_in_every_owned_runner() {
    let dir = tempfile::tempdir().unwrap();
    for stem in ["ffmpeg", "ffprobe"] {
        let tool = dir.path().join(stem);
        executable(&tool, "exit 0");
        let command = nice(&tool); // Select while the file still exists.
        std::fs::remove_file(&tool).unwrap();
        for mode in 0..4 {
            let mut command = match mode {
                0 => command_from(&command),
                _ => nice(&tool),
            };
            let control = OwnedProcessControl::bounded(Duration::from_secs(5), || false);
            let error = match mode {
                0 => run_bounded_command(&mut command, "probe removed tool").unwrap_err(),
                1 => run_owned_command_with_stdout_lines(
                    &mut command,
                    &control,
                    "observe removed tool",
                    Arc::new(|_| {}),
                )
                .unwrap_err(),
                2 => run_owned_command_with_input(
                    &mut command,
                    b"",
                    &control,
                    "feed removed tool",
                    None,
                )
                .unwrap_err(),
                _ => drive_with_control(&mut command, 1000, &|_| {}, &control).unwrap_err(),
            };
            assert_missing(error, stem);
        }
    }
}

fn command_from(command: &Command) -> Command {
    let mut copy = Command::new(command.get_program());
    copy.args(command.get_args());
    copy
}

#[test]
fn governed_launcher_checks_the_selected_file_for_both_launch_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let wrapper = dir.path().join("systemd-run");
    let tool = dir.path().join("ffmpeg");
    for code in [126, 127] {
        executable(
            &wrapper,
            &format!("echo 'unrelated diagnostic' >&2; exit {code}"),
        );
        let mut command = Command::new(&wrapper);
        command
            .args([
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "-p",
                "MemoryHigh=1G",
                "--",
                "nice",
                "-n",
                "10",
            ])
            .arg(&tool);
        assert_missing(
            run_bounded_command(&mut command, "governed removed tool").unwrap_err(),
            "ffmpeg",
        );
    }
}

#[test]
fn existing_tools_keep_their_status_even_with_missing_file_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("ffmpeg");
    for code in [0, 7, 126, 127] {
        executable(
            &tool,
            &format!("echo 'No such file or directory' >&2; exit {code}"),
        );
        let output = run_bounded_command(&mut nice(&tool), "existing tool").unwrap();
        assert_eq!(output.status.code(), Some(code));
        if code != 0 {
            let error = drive_with_control(
                &mut nice(&tool),
                1000,
                &|_| {},
                &OwnedProcessControl::bounded(Duration::from_secs(5), || false),
            )
            .unwrap_err();
            assert_eq!(error.code, error_codes::FFMPEG);
            assert!(error.cause.contains("No such file or directory"));
        }
    }
}

#[test]
fn missing_launcher_and_missing_script_interpreter_are_not_missing_tools() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("ffmpeg");
    executable(&tool, "exit 0");
    let mut command = Command::new(dir.path().join("nice"));
    command.args(["-n", "10"]).arg(&tool);
    let error = run_bounded_command(&mut command, "missing launcher").unwrap_err();
    assert_eq!(error.code, error_codes::FFMPEG);
    std::fs::remove_file(&tool).unwrap();
    let error = run_bounded_command(&mut command, "missing launcher and tool").unwrap_err();
    assert_eq!(error.code, error_codes::FFMPEG);

    // ENOENT can mean the script's interpreter vanished while the tool exists.
    executable(&tool, "exit 0");
    std::fs::write(&tool, "#!/shellx-cut-absent-interpreter\n").unwrap();
    let error = run_bounded_command(&mut Command::new(&tool), "missing interpreter").unwrap_err();
    assert_eq!(error.code, error_codes::FFMPEG);
}

#[test]
fn unrelated_exit_or_tool_argument_does_not_become_a_missing_tool() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("ffmpeg");
    let wrapper = dir.path().join("nice");
    executable(&wrapper, "exit 7");
    let mut command = Command::new(&wrapper);
    command.args(["-n", "10"]).arg(&tool);
    let output = run_bounded_command(&mut command, "unrelated failure").unwrap();
    assert_eq!(output.status.code(), Some(7));
    let output = run_bounded_command(
        Command::new("sh").args(["-c", "exit 127"]).arg(&tool),
        "unrelated helper",
    )
    .unwrap();
    assert_eq!(output.status.code(), Some(127));
}

#[test]
fn configured_media_tool_keeps_its_identity_after_removal() {
    const CHILD: &str = "SHELLX_CUT_REMOVED_TOOL_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let dir = tempfile::tempdir().unwrap();
        for (stem, key) in [
            ("ffmpeg", "SHELLX_CUT_FFMPEG"),
            ("ffprobe", "SHELLX_CUT_FFPROBE"),
        ] {
            let tool = dir.path().join(format!("custom-{stem}"));
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ffmpeg::process::tests::tool_missing::configured_media_tool_keeps_its_identity_after_removal",
                    "--nocapture",
                ])
                .env(CHILD, stem)
                .env(key, &tool)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        }
        return;
    }
    let stem = std::env::var(CHILD).unwrap();
    let key = if stem == "ffmpeg" {
        "SHELLX_CUT_FFMPEG"
    } else {
        "SHELLX_CUT_FFPROBE"
    };
    let tool = std::env::var_os(key).unwrap();
    executable(Path::new(&tool), "exit 0");
    let mut command = nice(Path::new(&tool));
    std::fs::remove_file(&tool).unwrap();
    assert_missing(
        run_bounded_command(&mut command, "custom tool").unwrap_err(),
        &stem,
    );
    assert_missing(
        run_bounded_command(&mut Command::new(tool), "custom tool").unwrap_err(),
        &stem,
    );
}
