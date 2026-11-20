use super::*;
use std::path::PathBuf;

#[test]
fn the_automatic_pick_is_on_unless_a_source_build_or_turned_off() {
    use AutomaticPick::*;
    // Installed or copied engine, nothing set: on (the shell no longer has to ask).
    assert_eq!(automatic_pick_for(None, false), On);
    assert_eq!(automatic_pick_for(Some(""), false), On);
    assert_eq!(automatic_pick_for(Some("maybe"), false), On);
    // A source build stays on the deterministic search order.
    assert_eq!(automatic_pick_for(None, true), OffSourceBuild);
    assert_eq!(automatic_pick_for(Some(""), true), OffSourceBuild);
    // The variable decides either way, for any engine.
    for on in ["1", "true", "TRUE", "yes", "on", " On "] {
        assert_eq!(automatic_pick_for(Some(on), true), OnByEnvironment, "{on}");
        assert_eq!(automatic_pick_for(Some(on), false), OnByEnvironment, "{on}");
    }
    for off in ["0", "false", "no", "off", "OFF"] {
        assert_eq!(
            automatic_pick_for(Some(off), false),
            OffByEnvironment,
            "{off}"
        );
    }
    assert!(On.is_on() && OnByEnvironment.is_on());
    assert!(!OffByEnvironment.is_on() && !OffSourceBuild.is_on());
}

fn cargo_target(root: &std::path::Path) -> PathBuf {
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(
        target.join("CACHEDIR.TAG"),
        format!("{CARGO_CACHEDIR_SIGNATURE}\n# cargo\n"),
    )
    .unwrap();
    target
}

#[test]
fn engines_and_test_binaries_in_a_cargo_target_folder_are_source_builds() {
    let dir = tempfile::tempdir().unwrap();
    let target = cargo_target(dir.path());
    for exe in [
        target.join("debug").join("cutd"),
        target.join("release").join("cutd"),
        target.join("debug").join("deps").join("cut_media-0123abcd"),
        target
            .join("x86_64-unknown-linux-gnu")
            .join("release")
            .join("cutd"),
    ] {
        assert!(is_cargo_build_exe(&exe), "{}", exe.display());
    }
    // Installed layouts are never source builds, even under a folder named debug
    // without cargo's tag, or beside a planted tag.
    let untagged = dir.path().join("plain").join("debug").join("cutd");
    assert!(!is_cargo_build_exe(&untagged));
    for exe in [
        dir.path().join("usr").join("bin").join("cutd"),
        dir.path().join("ShellX Cut").join("cutd.exe"),
        dir.path()
            .join("ShellX Cut.app")
            .join("Contents")
            .join("MacOS")
            .join("cutd"),
        target.join("cutd"),
    ] {
        assert!(!is_cargo_build_exe(&exe), "{}", exe.display());
    }
}

/// `cargo test` runs this from the target folder, so tests keep the fixed
/// search order unless they opt in.
#[test]
fn this_test_binary_is_a_source_build() {
    let exe = std::env::current_exe().unwrap();
    assert!(is_cargo_build_exe(&exe), "{}", exe.display());
}

#[test]
fn the_reason_shown_is_the_choice_actually_used() {
    let selection = ffmpeg_selection();
    assert_eq!(selection.program, super::super::ffmpeg());
    assert!(!selection.text.is_empty());
    let json = serde_json::to_value(&selection).unwrap();
    assert!(json["reason"].is_string() && json["automatic"].is_string());
    assert!(
        json.get("program").is_none(),
        "the path is already the card's 'resolved'"
    );
}

#[test]
fn every_reason_reads_as_one_plain_sentence() {
    use SelectionReason::*;
    for (reason, automatic, needle) in [
        (Environment, AutomaticPick::On, "SHELLX_CUT_FFMPEG names it"),
        (Chosen, AutomaticPick::On, "system.set_ffmpeg"),
        (
            AutomaticHardware,
            AutomaticPick::On,
            "of the 3 FFmpeg builds",
        ),
        (AutomaticNoHardware, AutomaticPick::On, "none of the 3"),
        (OnlyOneFound, AutomaticPick::On, "only FFmpeg found"),
        (
            AutomaticOff,
            AutomaticPick::OffSourceBuild,
            "SHELLX_CUT_FFMPEG_AUTO=1",
        ),
        (
            AutomaticOff,
            AutomaticPick::OffByEnvironment,
            "SHELLX_CUT_FFMPEG_AUTO turns it off",
        ),
        (NotFound, AutomaticPick::On, "No FFmpeg"),
    ] {
        let text = describe(reason, automatic, 3);
        assert!(text.contains(needle), "{reason:?}: {text}");
        assert!(text.ends_with('.'), "{text}");
    }
    assert!(describe(AutomaticHardware, AutomaticPick::On, 1).contains("the only FFmpeg found"));
}
