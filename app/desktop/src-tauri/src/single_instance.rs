//! One ShellX Cut window per user, as desktop editors do.
//!
//! Opening the app (a shortcut, the Start menu, the Dock, a command line)
//! while it already runs used to start a second window process that adopted
//! the first one's engine as an "external" engine: two windows on one
//! project, the second without the close guard or recording ownership. That
//! was never a designed feature (the engine's several UI clients are for a
//! browser tab next to the app and for tests). Now a second launch brings the
//! running window to the front and ends; a project path given to it
//! (`ShellX Cut <folder>.cutproj`) is opened in that window, the same way the
//! first launch opens one.
//!
//! An isolated state home (`SHELLX_CUT_HOME`, used by tests and portable
//! copies) is its own world: such a launch is not tied to the ordinary one.

use std::path::Path;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The project path the app was started with, until the window takes it.
#[derive(Default)]
pub struct LaunchProject(Mutex<Option<String>>);

/// The first argument naming an existing `.cutproj` folder, resolved
/// against `base` and returned as an absolute path (the window hands it to
/// the engine, which runs in another folder).
pub(crate) fn project_argument_in<I, S>(base: &Path, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().find_map(|arg| {
        let path = base.join(arg.as_ref());
        let named = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cutproj"));
        (named && path.is_absolute() && path.is_dir()).then(|| path.display().to_string())
    })
}

/// As [`project_argument_in`], for this process's own arguments.
pub(crate) fn project_argument<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let base = std::env::current_dir().ok()?;
    project_argument_in(&base, args)
}

/// The project a second launch named. A relative name means the folder that
/// launch ran in, not this process's: it is resolved there and passed on as
/// an absolute path.
pub(crate) fn second_launch_project(argv: &[String], cwd: &str) -> Option<String> {
    project_argument_in(Path::new(cwd), argv.iter().skip(1))
}

/// Whether this launch takes part in the one-window rule.
pub(crate) fn applies() -> bool {
    std::env::var_os("SHELLX_CUT_HOME").is_none_or(|home| home.is_empty())
}

/// Bring the running window forward and hand it the project a second launch
/// named. Runs in the first process, called by the single-instance plugin.
pub(crate) fn on_second_launch<R: Runtime>(app: &AppHandle<R>, argv: Vec<String>, cwd: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(path) = second_launch_project(&argv, cwd) {
        let _ = app.emit_to(
            "main",
            "cut:open-project",
            serde_json::json!({ "path": path }),
        );
    }
}

/// Remember the project the first launch was given, for the window to open.
pub(crate) fn remember_launch_project<R: Runtime>(app: &AppHandle<R>) {
    let path = project_argument(std::env::args().skip(1));
    if let (Some(path), Some(state)) = (path, app.try_state::<LaunchProject>()) {
        *state.0.lock().unwrap() = Some(path);
    }
}

/// The window asks once at start which project the app was launched with.
#[tauri::command]
pub fn take_launch_project(state: tauri::State<'_, LaunchProject>) -> Option<String> {
    state.0.lock().unwrap().take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_existing_project_folder_is_taken_from_the_arguments() {
        let root = std::env::temp_dir().join(format!("cut-single-instance-{}", std::process::id()));
        let project = root.join("Talk.cutproj");
        std::fs::create_dir_all(&project).unwrap();
        let missing = root.join("Gone.cutproj");
        let file = root.join("clip.mp4");
        std::fs::write(&file, b"x").unwrap();
        let project = project.display().to_string();
        assert_eq!(
            project_argument([
                "--flag",
                file.to_str().unwrap(),
                missing.to_str().unwrap(),
                &project
            ]),
            Some(project.clone())
        );
        assert_eq!(project_argument(["--version"]), None);
        assert_eq!(project_argument(Vec::<String>::new()), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_relative_project_of_a_second_launch_is_the_one_in_that_launch_folder() {
        // Two folders each hold a Demo.cutproj; the second launch runs in B.
        let root = std::env::temp_dir().join(format!("cut-second-launch-{}", std::process::id()));
        let a = root.join("A");
        let b = root.join("B");
        std::fs::create_dir_all(a.join("Demo.cutproj")).unwrap();
        std::fs::create_dir_all(b.join("Demo.cutproj")).unwrap();
        let argv = vec!["shellx-cut".to_string(), "Demo.cutproj".to_string()];
        let opened = second_launch_project(&argv, b.to_str().unwrap());
        assert_eq!(
            opened.as_deref().map(std::path::Path::new),
            Some(b.join("Demo.cutproj").as_path())
        );
        // A name that is not an existing project folder there is not passed on.
        let missing = vec!["shellx-cut".to_string(), "Gone.cutproj".to_string()];
        assert_eq!(second_launch_project(&missing, b.to_str().unwrap()), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
