//! Recover the selected media program behind the launchers used by renders.

use cut_core::CutError;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus, Output};

fn selected_tool(command: &Command) -> Option<(&'static str, &Path)> {
    let mut program = command.get_program();
    let mut args = command.get_args();
    // Read launcher structure, never stderr or an arbitrary media argument.
    if Path::new(program).file_stem()? == "systemd-run" {
        args.find(|arg| *arg == "--")?;
        program = args.next()?;
    }
    if Path::new(program).file_stem()? == "nice" {
        if args.next()? != "-n" {
            return None;
        }
        args.next()?.to_str()?.parse::<i32>().ok()?;
        program = args.next()?;
    }
    let path = Path::new(program);
    let stem = match path.file_stem()?.to_str()? {
        "ffmpeg" => "ffmpeg",
        "ffprobe" => "ffprobe",
        _ => configured_tool(program)?,
    };
    // Engine selections are absolute. A relative name needs PATH lookup and
    // cannot establish that this selected file is absent on disk.
    path.is_absolute().then_some((stem, path))
}

fn configured_tool(program: &OsStr) -> Option<&'static str> {
    for (key, stem) in [
        ("SHELLX_CUT_FFMPEG", "ffmpeg"),
        ("SHELLX_CUT_FFPROBE", "ffprobe"),
    ] {
        if std::env::var_os(key).as_deref() == Some(program) {
            return Some(stem);
        }
    }
    (crate::toolpath::read_override_setting()
        .as_deref()
        .map(OsStr::new)
        == Some(program))
    .then_some("ffmpeg")
}

fn absent_tool(command: &Command) -> Option<CutError> {
    let (stem, path) = selected_tool(command)?;
    // ENOENT can name a missing script interpreter or shared loader while the
    // selected tool still exists. Permission errors also do not prove absence.
    std::fs::metadata(path)
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        .then(|| crate::tool_setup::missing_tool_error(stem))
}

pub(super) fn spawn_error(command: &Command, error: &io::Error) -> Option<CutError> {
    let (_, selected) = selected_tool(command)?;
    if selected != Path::new(command.get_program()) {
        // A missing launcher has not reported a failure to start the tool.
        return None;
    }
    let mut missing = (error.kind() == io::ErrorKind::NotFound)
        .then(|| absent_tool(command))
        .flatten()?;
    missing.cause = format!("{} ({error})", missing.cause);
    Some(missing)
}

pub(super) fn exit_error(command: &Command, status: ExitStatus) -> Option<CutError> {
    matches!(status.code(), Some(126 | 127))
        .then(|| absent_tool(command))
        .flatten()
}

pub(super) fn output(
    command: &Command,
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
) -> Result<Output, CutError> {
    match exit_error(command, status) {
        Some(error) => Err(error),
        None => Ok(Output {
            status,
            stdout,
            stderr,
        }),
    }
}
