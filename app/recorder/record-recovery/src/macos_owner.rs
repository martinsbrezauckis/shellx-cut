//! Conservative macOS owner observation for mutating restart recovery.

use crate::contract::OwnerProbe;

pub(crate) fn owner_probe(pid: u32) -> OwnerProbe {
    match liveness(pid) {
        Liveness::Dead => return OwnerProbe::Dead,
        Liveness::Ambiguous => return OwnerProbe::Ambiguous,
        Liveness::Alive => {}
    }
    let mut ps = std::process::Command::new("ps");
    ps.args(["-p", &pid.to_string(), "-o", "lstart="]);
    match owned_output(&mut ps, "probe macOS capture owner") {
        Some(output) if output.status.success() => String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .and_then(|started| {
                let mut sysctl = std::process::Command::new("sysctl");
                sysctl.args(["-n", "kern.boottime"]);
                owned_output(&mut sysctl, "read macOS boot identity")
                    .filter(|boot| boot.status.success())
                    .and_then(|boot| String::from_utf8(boot.stdout).ok())
                    .map(|boot| format!("{}:{started}", boot.trim()))
            })
            .map(OwnerProbe::Identity)
            .unwrap_or(OwnerProbe::Ambiguous),
        // A nonzero `ps` result can mean permission, command, or formatting
        // failure.  Only `kill(pid, 0) == ESRCH` below proves a dead owner.
        Some(_) | None => OwnerProbe::Ambiguous,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Liveness {
    Alive,
    Dead,
    Ambiguous,
}

fn liveness(pid: u32) -> Liveness {
    if pid == 0 {
        return Liveness::Ambiguous;
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    let errno = std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or_default();
    liveness_from_kill(result, errno)
}

fn liveness_from_kill(result: i32, errno: i32) -> Liveness {
    if result == 0 {
        Liveness::Alive
    } else if errno == libc::ESRCH {
        Liveness::Dead
    } else {
        Liveness::Ambiguous
    }
}

fn owned_output(
    command: &mut std::process::Command,
    context: &str,
) -> Option<std::process::Output> {
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(std::time::Duration::from_secs(2), || {
            false
        })
        .with_output_cap(8 * 1024);
    cut_media::ffmpeg::run_owned_command(command, &control, context).ok()
}

#[cfg(test)]
mod tests {
    use super::{liveness_from_kill, Liveness};

    #[test]
    fn only_esrch_is_positive_dead_evidence() {
        assert_eq!(liveness_from_kill(-1, libc::ESRCH), Liveness::Dead);
        assert_eq!(liveness_from_kill(-1, libc::EPERM), Liveness::Ambiguous);
        assert_eq!(liveness_from_kill(-1, libc::EINVAL), Liveness::Ambiguous);
        assert_eq!(liveness_from_kill(0, 0), Liveness::Alive);
    }
}
