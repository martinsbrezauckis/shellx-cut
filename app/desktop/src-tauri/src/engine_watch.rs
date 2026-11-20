//! The engine this shell started can end on its own: a crash, a kill, the
//! system stopping it. Nothing watched for that: the window stayed up showing
//! "disconnected" next to the last job chips ("Rendering 13%"), and nothing
//! offered a way forward. This watch says it plainly and offers to restart the
//! engine (the window then reloads from the new engine, so no chip of the dead
//! job stays, and the start says which work was interrupted) or to quit.
//!
//! It is the same on every system: it looks only at the engine process this
//! shell owns and asks with a native dialog, which works whatever the window
//! shows. An engine exit the shell caused itself (quit, close, update) is
//! expected and never reported.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::AppHandle;

static ENDING: AtomicBool = AtomicBool::new(false);

/// The shell itself is ending or replacing its engine (quit, close, update):
/// an engine exit from now on is expected.
pub(crate) fn mark_ending() {
    ENDING.store(true, Ordering::SeqCst);
}

fn ending() -> bool {
    ENDING.load(Ordering::SeqCst)
}

/// What one look at the owned engine found.
pub(crate) enum Look {
    Running,
    /// It exited by itself; the text describes how ("exit code 1").
    Exited(String),
    /// The shell no longer holds an engine (it let it go): nothing to watch.
    Gone,
}

/// Look every second. `look` inspects the owned engine (and lets go of it
/// when it exited); `restart` starts a new one and reloads the window.
pub(crate) fn start<L, R>(app: AppHandle, look: L, restart: R)
where
    L: Fn(&AppHandle) -> Look + Send + 'static,
    R: Fn(&AppHandle) -> Result<(), String> + Send + 'static,
{
    let spawned = std::thread::Builder::new()
        .name("engine-watch".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            if ending() {
                return;
            }
            match look(&app) {
                Look::Running => {}
                Look::Gone => return,
                Look::Exited(how) => {
                    if ending() {
                        return;
                    }
                    eprintln!("[shellx-cut] the engine stopped unexpectedly ({how})");
                    if !offer_restart(&app, &how, &restart) {
                        return;
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("[shellx-cut] cannot watch the engine process: {error}");
    }
}

/// Ask until the engine runs again or the person quits. Returns whether the
/// watch goes on (a new engine runs).
fn offer_restart<R>(app: &AppHandle, how: &str, restart: &R) -> bool
where
    R: Fn(&AppHandle) -> Result<(), String>,
{
    let mut text = stopped_text(how);
    loop {
        if !ask(app, &text) {
            mark_ending();
            app.exit(0);
            return false;
        }
        match restart(app) {
            Ok(()) => return true,
            Err(reason) => text = restart_failed_text(&reason),
        }
    }
}

pub(crate) const RESTART: &str = "Restart the engine";
pub(crate) const QUIT: &str = "Quit ShellX Cut";

pub(crate) fn stopped_text(how: &str) -> String {
    format!(
        "The ShellX Cut engine stopped unexpectedly ({how}). Work that was running, such as an export, stopped with it; your project keeps the changes saved before this.\n\nRestart the engine to go on working: the window reloads and says which work was interrupted. Or quit ShellX Cut."
    )
}

pub(crate) fn restart_failed_text(reason: &str) -> String {
    format!(
        "The engine could not be restarted: {reason}\n\nTry again, or quit ShellX Cut and open it again."
    )
}

fn ask(app: &AppHandle, text: &str) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    app.dialog()
        .message(text)
        .title("ShellX Cut engine stopped")
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom(
            RESTART.into(),
            QUIT.into(),
        ))
        .blocking_show()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_says_what_happened_and_both_ways_forward() {
        let text = stopped_text("exit code 1");
        assert!(text.starts_with("The ShellX Cut engine stopped unexpectedly (exit code 1)."));
        assert!(text.contains("stopped with it"));
        assert!(text.contains("Restart the engine"));
        assert!(text.contains("quit ShellX Cut"));
        let failed = restart_failed_text("cutd exited during startup");
        assert!(failed.contains("cutd exited during startup"));
        assert!(failed.contains("open it again"));
    }
}
