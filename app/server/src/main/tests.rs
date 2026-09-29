use super::*;
use clap::Parser;

#[test]
fn verb_cli_accepts_explicit_standalone_testing_mode() {
    let cli = Cli::parse_from([
        "cutd",
        "verb",
        "--standalone",
        "--project",
        "/tmp/sample.cutproj",
        "project.state",
        "{}",
    ]);

    let Command::Verb {
        standalone,
        project,
        name,
        args,
    } = cli.command
    else {
        panic!("expected verb command");
    };

    assert!(standalone);
    assert_eq!(project, Some(PathBuf::from("/tmp/sample.cutproj")));
    assert_eq!(name, "project.state");
    assert_eq!(args.as_deref(), Some("{}"));
}
