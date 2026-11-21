#[cfg(unix)]
#[test]
fn old_ffmpeg_stays_usable_and_reports_the_tested_versions() {
    use std::os::unix::fs::PermissionsExt;

    // Other tests refresh the process-wide tool cache. Count probes in a child
    // containing only this test so those refreshes cannot affect the assertion.
    const CHILD: &str = "SHELLX_CUT_VERSION_ADVICE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "doctor::ffmpeg_version_tests::old_ffmpeg_stays_usable_and_reports_the_tested_versions", "--nocapture"]).env(CHILD, "1");
        let control = cut_media::ffmpeg::OwnedProcessControl::bounded(
            std::time::Duration::from_secs(60),
            || false,
        );
        let output =
            cut_media::ffmpeg::run_owned_command(&mut command, &control, "check version advice")
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("ffmpeg");
    std::fs::write(
        &fake,
        "#!/bin/sh\ncase \"$*\" in\n*version*) echo version >> \"${0%/*}/calls\"; echo 'ffmpeg version 6.1.1-3ubuntu5 Copyright';;\n*filters*) printf ' .. ass V->V x\\n .. vidstabtransform V->V x\\n .S zscale V->V x\\n';;\n*encoders*) printf ' V....D libx265 x\\n V....D libvpx-vp9 x\\n V....D libsvtav1 x\\n';;\nesac\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let card = super::media_tool_card("ffmpeg", fake.clone().into_os_string());
    assert_eq!(card.status, super::CardStatus::Ok);
    assert_eq!(card.details["runnable"], true);
    assert_eq!(card.details["version_advice"]["in_use"], "6.1.1-3ubuntu5");
    assert_eq!(
        card.details["version_advice"]["tested_versions"],
        serde_json::json!(["8.0.1", "9.0.2"])
    );
    assert!(card.hint.is_none(), "version advice is not an error hint");
    let probe = super::Card {
        id: "ffprobe".into(),
        ..card.clone()
    };
    assert!(super::essential_ok_for(&[card, probe]));
    let version_reads = || {
        std::fs::read_to_string(dir.path().join("calls"))
            .unwrap()
            .lines()
            .count()
    };
    let reads = version_reads();
    let again = super::media_tool_card("ffmpeg", fake.clone().into_os_string());
    assert_eq!(again.details["version_advice"]["in_use"], "6.1.1-3ubuntu5");
    assert_eq!(
        version_reads(),
        reads + 1,
        "only the existing availability check repeats"
    );
    assert_eq!(
        cut_media::hwencode::ffmpeg_version_at(fake.as_os_str()).as_deref(),
        Some("6.1.1-3ubuntu5")
    );
    assert_eq!(
        version_reads(),
        reads + 1,
        "advice and media share the version cache"
    );

    for version in ["n9.0.2-39-g7adfbe40dc", "8.0.1", "unreadable"] {
        let alternate = dir.path().join(version);
        let script = std::fs::read_to_string(&fake)
            .unwrap()
            .replace("6.1.1-3ubuntu5", version);
        std::fs::write(&alternate, script).unwrap();
        std::fs::set_permissions(&alternate, std::fs::Permissions::from_mode(0o755)).unwrap();
        let card = super::media_tool_card("ffmpeg", alternate.into_os_string());
        assert_eq!(card.status, super::CardStatus::Ok, "{version}");
        assert!(card.details["version_advice"].is_null(), "{version}");
        assert!(card.hint.is_none(), "{version}");
    }
}

#[test]
fn version_advice_only_names_readable_versions_older_than_seven() {
    use super::ffmpeg_version_advice::for_version;
    let old = for_version(Some("6.1.1-3ubuntu5"), "linux", "x86_64").unwrap();
    assert_eq!(old["in_use"], "6.1.1-3ubuntu5");
    assert!(old["message"].as_str().unwrap().contains("8.0.1 and 9.0.2"));
    for version in [
        Some("n9.0.2-39-g7adfbe40dc"),
        Some("8.0.1"),
        Some("7.0"),
        Some("unreadable"),
        Some("N-12345-gabc"),
        None,
    ] {
        assert!(
            for_version(version, "linux", "x86_64").is_none(),
            "{version:?}"
        );
    }
}

#[test]
fn version_advice_offers_the_existing_upgrade_route_for_each_system() {
    use super::ffmpeg_version_advice::for_version;
    for (os, arch, words) in [
        ("windows", "x86_64", "Download"),
        ("linux", "x86_64", "Download"),
        ("macos", "aarch64", "Homebrew"),
        ("macos", "x86_64", "Homebrew"),
        ("linux", "aarch64", "package manager"),
    ] {
        let advice = for_version(Some("6.1.1-3ubuntu5"), os, arch).unwrap();
        assert!(
            advice["upgrade"].as_str().unwrap().contains(words),
            "{os}/{arch}: {advice}"
        );
    }
}

#[test]
fn version_advice_explains_that_a_chosen_binary_stays_selected() {
    use cut_media::toolpath::SelectionReason;
    for reason in [SelectionReason::Chosen, SelectionReason::Environment] {
        let mut card = super::Card {
            id: "ffmpeg".into(),
            kind: "tool".into(),
            status: super::CardStatus::Ok,
            source: None,
            version: Some("6.1.1-3ubuntu5".into()),
            hint: None,
            details: serde_json::json!({"resolved":"/chosen/ffmpeg", "version_advice": super::ffmpeg_version_advice::for_version(Some("6.1.1-3ubuntu5"), "linux", "x86_64")}),
        };
        super::ffmpeg_version_advice::preserve_choice(&mut card, reason);
        assert_eq!(card.details["resolved"], "/chosen/ffmpeg");
        assert_eq!(card.status, super::CardStatus::Ok);
        assert!(card.hint.is_none());
        assert!(card.details["version_advice"]["upgrade"]
            .as_str()
            .unwrap()
            .contains("Your chosen FFmpeg stays selected"));
    }
}
