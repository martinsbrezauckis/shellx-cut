//! Closing or quitting while an export (or other work someone asked for) runs.
//!
//! Before this, closing the window during an export closed at once: the export
//! was lost, its staging and lock files stayed in `exports`, and nothing was
//! said. Now the close guard asks the engine `jobs.before_quit` (the same rule
//! as a project change decides which work counts). When work runs, the close is
//! held and the person is asked, as large editors do ("An export is in
//! progress. Quit anyway?"), with the work named ("An export is still running",
//! or "Work is still running" when it is not only exports):
//!
//! - **Keep working** (the default: Enter, Esc, closing the question): nothing
//!   closes and the export goes on.
//! - **Quit and stop the export**: the engine cancels the work the normal way
//!   (its staging and lock files are removed) and the app closes once that has
//!   finished, at most `STOP_WAIT`.
//!
//! The app window shows the question (`cut:quit-jobs-running`) and answers with
//! `answer_quit_question`. A window that does not take the question within a
//! few seconds gets a native dialog from the shell instead, where the native
//! buttons put "Keep working" on the cancel side (Esc and closing the dialog
//! keep working). If the export ends while the question is open, the reason to
//! hold is gone and the app closes as asked.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

/// How long the app window has to take the question before the shell asks.
const ACK_WAIT: Duration = Duration::from_secs(6);
/// The engine waits up to 20 s for each worker to stop; all stop together.
const STOP_WAIT: Duration = Duration::from_secs(60);
/// How long a slow engine is asked again before the close goes ahead.
const UNKNOWN_LIMIT: Duration = Duration::from_secs(20);

/// One running job as the engine's refusal lists it (`result.running`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RunningJob {
    pub(crate) job_id: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) progress: f64,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

/// What `jobs.before_quit` said.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Jobs {
    /// Nothing someone asked for is running.
    None,
    /// Work runs: the engine's sentence, the named jobs and the way forward.
    Running {
        message: String,
        cause: String,
        action: String,
        /// One record per job. A job's own message can hold ';' or ',', so
        /// the question is never built by splitting the cause line.
        running: Vec<RunningJob>,
    },
    /// No clear answer (slow, unreachable, unreadable).
    Unknown,
}

pub(crate) fn jobs_from_reply(body: &str) -> Jobs {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Jobs::Unknown;
    };
    if value.get("ok") == Some(&serde_json::Value::Bool(true))
        && value
            .pointer("/result/running")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
    {
        return Jobs::None;
    }
    let Some(error) = value.get("error") else {
        return Jobs::Unknown;
    };
    let text = |key: &str| {
        error
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match error.get("code").and_then(serde_json::Value::as_str) {
        Some("jobs_running") => Jobs::Running {
            message: text("message"),
            cause: text("cause"),
            action: text("suggested_action"),
            running: value
                .pointer("/result/running")
                .cloned()
                .and_then(|running| serde_json::from_value(running).ok())
                .unwrap_or_default(),
        },
        _ => Jobs::Unknown,
    }
}

pub(crate) fn jobs(addr: &str, read_timeout: Duration) -> Jobs {
    match crate::close_guard::post(addr, "jobs.before_quit", "{}", read_timeout) {
        Ok(body) => jobs_from_reply(&body),
        Err(_) => Jobs::Unknown,
    }
}

fn settle(addr: &str, exited: impl Fn() -> bool) -> Jobs {
    settle_within(addr, exited, UNKNOWN_LIMIT, Duration::from_secs(3))
}

/// Ask again while the engine gives no clear answer, up to `limit`.
fn settle_within(addr: &str, exited: impl Fn() -> bool, limit: Duration, read: Duration) -> Jobs {
    let started = Instant::now();
    loop {
        if exited() {
            return Jobs::None;
        }
        let answer = jobs(addr, read);
        if answer != Jobs::Unknown || started.elapsed() >= limit {
            return answer;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// What a close does once the engine has said (or not said) what runs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Nothing someone asked for runs, or the engine is gone: close.
    Close,
    /// Work runs: the quit question.
    AskRunning,
    /// The engine gave no answer: ask whether to quit anyway.
    AskUnknown,
}

/// No answer is never "nothing runs": an export could be running.
pub(crate) fn plan(jobs: &Jobs) -> Plan {
    match jobs {
        Jobs::None => Plan::Close,
        Jobs::Running { .. } => Plan::AskRunning,
        Jobs::Unknown => Plan::AskUnknown,
    }
}

pub(crate) const UNKNOWN_TITLE: &str = "Work may still be running";
pub(crate) const UNKNOWN_TEXT: &str = "ShellX Cut could not find out whether work is still running: its engine did not answer.\n\nQuit anyway? Work that is running, such as an export, would be stopped and lost. Keep working to wait and try again.";
pub(crate) const QUIT_ANYWAY: &str = "Quit anyway";
pub(crate) const KEEP_WORKING: &str = "Keep working";

/// Ask with a plain native question whether to quit while it is not known
/// whether work runs. Keep working is the safe side: Esc and closing the
/// dialog keep the app open.
pub(crate) fn ask_when_unknown(app: &AppHandle) -> Outcome {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let _ = with(app, |q| {
        *q = Question {
            open: true,
            ..Question::default()
        }
    });
    let quit = app
        .dialog()
        .message(UNKNOWN_TEXT)
        .title(UNKNOWN_TITLE)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            QUIT_ANYWAY.into(),
            KEEP_WORKING.into(),
        ))
        .blocking_show();
    let _ = with(app, |q| *q = Question::default());
    if quit {
        Outcome::Close
    } else {
        Outcome::Keep
    }
}

#[derive(Default)]
struct Question {
    open: bool,
    acknowledged: bool,
    answer: Option<bool>,
    nudged: bool,
}

#[derive(Default)]
pub struct QuitQuestionState(Mutex<Question>);

fn with<T>(app: &AppHandle, f: impl FnOnce(&mut Question) -> T) -> Option<T> {
    let state = app.try_state::<QuitQuestionState>()?;
    let mut question = state.0.lock().unwrap();
    Some(f(&mut question))
}

/// The quit question is being asked.
pub(crate) fn is_open(app: &AppHandle) -> bool {
    with(app, |q| q.open).unwrap_or(false)
}

/// The person asked to close again while the question is open: show it again.
pub(crate) fn nudge(app: &AppHandle) {
    let _ = with(app, |q| q.nudged = true);
}

/// The text of the question, from the engine's refusal: one line per job.
/// Without records (an older engine) the cause is shown whole, never split.
pub(crate) fn question_text(message: &str, running: &[RunningJob], cause: &str) -> String {
    let message = message.trim().trim_end_matches('.');
    let listed = if running.is_empty() {
        format!("Running: {}", cause.trim().trim_start_matches("running: "))
    } else {
        let lines: Vec<String> = running
            .iter()
            .map(|job| {
                format!(
                    "- {} ({:.0}% done)",
                    job.kind.replace(['_', '-'], " "),
                    job.progress * 100.0
                )
            })
            .collect();
        format!("Running:\n{}", lines.join("\n"))
    };
    format!("{message}.\n\n{listed}\n\nQuit anyway? The stopped work is lost and must be started again.")
}

fn is_export_kind(kind: &str) -> bool {
    matches!(
        kind,
        "render" | "render_queue" | "screen_record_export" | "reframe"
    ) || kind.starts_with("reframe-")
}

/// "An export" when every running job writes an export file, else "Work".
pub(crate) fn question_title(running: &[RunningJob]) -> &'static str {
    if !running.is_empty() && running.iter().all(|job| is_export_kind(&job.kind)) {
        "An export is still running"
    } else {
        "Work is still running"
    }
}

fn native_question(app: &AppHandle, message: &str, running: &[RunningJob], cause: &str) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    // The cancel side is "Keep working": Esc and closing the dialog keep the
    // export running. The native OK button is the destructive one.
    app.dialog()
        .message(question_text(message, running, cause))
        .title(question_title(running))
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Quit and stop it".into(),
            "Keep working".into(),
        ))
        .blocking_show()
}

pub(crate) enum Outcome {
    /// Close now (nothing runs, or the person chose to stop it and it stopped).
    Close,
    /// The person chose to keep working: do not close.
    Keep,
}

fn set_title(app: &AppHandle, title: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(title);
    }
}

/// Ask the engine and, when work runs, the person. Runs on the guard's own
/// thread while the close is held.
pub(crate) fn decide(app: &AppHandle, addr: &str, exited: impl Fn() -> bool + Copy) -> Outcome {
    set_title(app, "ShellX Cut: checking for running work before closing");
    let answer = settle(addr, exited);
    match plan(&answer) {
        Plan::Close => return Outcome::Close,
        Plan::AskUnknown => return ask_when_unknown(app),
        Plan::AskRunning => {}
    }
    let Jobs::Running {
        message,
        cause,
        action,
        running,
    } = answer
    else {
        return ask_when_unknown(app);
    };
    let _ = with(app, |q| {
        *q = Question {
            open: true,
            ..Question::default()
        }
    });
    let payload = serde_json::json!({
        "message": message,
        "cause": cause,
        "suggested_action": action,
        "running": running,
        "question": question_text(&message, &running, &cause),
    });
    let _ = app.emit_to("main", "cut:quit-jobs-running", payload.clone());
    let mut asked = Instant::now();
    let mut native_answer: Option<bool> = None;
    let stop = loop {
        if let Some(answer) = native_answer.or_else(|| with(app, |q| q.answer).flatten()) {
            break answer;
        }
        let (acknowledged, nudged) = with(app, |q| (q.acknowledged, std::mem::take(&mut q.nudged)))
            .unwrap_or((false, false));
        if nudged {
            let _ = app.emit_to("main", "cut:quit-jobs-running", payload.clone());
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
            asked = Instant::now();
        }
        if !acknowledged && asked.elapsed() >= ACK_WAIT {
            // The window did not take the question: ask natively.
            native_answer = Some(native_question(app, &message, &running, &cause));
            continue;
        }
        // The work ended while the person was asked: nothing to stop any more.
        if jobs(addr, Duration::from_secs(2)) == Jobs::None {
            let _ = app.emit_to("main", "cut:quit-jobs-settled", serde_json::json!({}));
            break true;
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let _ = with(app, |q| *q = Question::default());
    if !stop {
        return Outcome::Keep;
    }
    set_title(app, "ShellX Cut: stopping the export before closing");
    let body = serde_json::json!({ "cancel_running_jobs": true }).to_string();
    match crate::close_guard::post(addr, "jobs.before_quit", &body, STOP_WAIT) {
        Ok(reply) if reply.contains("\"ok\":true") => {}
        // Still stopping, or no answer: close anyway. The next start says which
        // work was interrupted and removes its leftover files.
        Ok(reply) => eprintln!("[shellx-cut] stopping work before closing: {reply}"),
        Err(error) => eprintln!("[shellx-cut] stopping work before closing failed: {error}"),
    }
    Outcome::Close
}

/// The app window shows the question.
#[tauri::command]
pub fn acknowledge_quit_question(state: tauri::State<'_, QuitQuestionState>) -> bool {
    let mut question = state.0.lock().unwrap();
    if !question.open {
        return false;
    }
    question.acknowledged = true;
    true
}

/// The person answered: `stop` true quits and stops the work, false keeps working.
#[tauri::command]
pub fn answer_quit_question(stop: bool, state: tauri::State<'_, QuitQuestionState>) -> bool {
    let mut question = state.0.lock().unwrap();
    if !question.open {
        return false;
    }
    question.answer = Some(stop);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: &str = r#"{"ok":false,"result":{"running":[{"job_id":"job_002","kind":"render","progress":0.4,"message":"encoding; it will take longer, or choose HEVC"}]},"error":{"code":"jobs_running","message":"quitting ShellX Cut would cancel 1 running job(s) that were asked for (exports, renders, reviews, analysis, downloads) and nothing has been stopped","cause":"running: job_002 (render, 40% done, encoding; it will take longer, or choose HEVC)","suggested_action":"wait for them to finish"}}"#;

    fn job(id: &str, kind: &str, progress: f64) -> RunningJob {
        RunningJob {
            job_id: id.into(),
            kind: kind.into(),
            progress,
            message: None,
        }
    }

    #[test]
    fn a_running_export_is_named_and_nothing_running_closes() {
        match jobs_from_reply(RUNNING) {
            Jobs::Running {
                message,
                cause,
                running,
                ..
            } => {
                assert!(message.starts_with("quitting ShellX Cut would cancel 1"));
                assert!(cause.starts_with("running: job_002 (render, 40% done"));
                // One job, although its message holds ';' and ','.
                assert_eq!(running.len(), 1);
                assert_eq!(running[0].job_id, "job_002");
                assert_eq!(running[0].kind, "render");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            jobs_from_reply(r#"{"ok":true,"result":{"running":[],"cancelled_jobs":[]}}"#),
            Jobs::None
        );
        assert_eq!(jobs_from_reply("not json"), Jobs::Unknown);
        assert_eq!(
            jobs_from_reply(r#"{"ok":false,"error":{"code":"io"}}"#),
            Jobs::Unknown
        );
        assert_eq!(
            jobs_from_reply(r#"{"ok":false,"error":{"code":"not_found"}}"#),
            Jobs::Unknown
        );
        assert_eq!(jobs_from_reply(r#"{"ok":true}"#), Jobs::Unknown);
    }

    #[test]
    fn the_question_names_the_work_and_what_quitting_costs() {
        let text = question_text(
            "quitting ShellX Cut would cancel 1 running job(s)",
            &[job("job_002", "render", 0.4)],
            "running: job_002 (render, 40% done, encoding; it will take longer)",
        );
        assert!(
            text.contains("Running:\n- render (40% done)\n\nQuit anyway?"),
            "{text}"
        );
        assert!(!text.contains("it will take longer"));
        // An older engine without records: the cause whole, not split.
        let text = question_text("m", &[], "running: job_002 (render, 40% done; x)");
        assert!(text.contains("Running: job_002 (render, 40% done; x)"));
        assert!(text.contains("Quit anyway?"));
        assert!(text.contains("lost"));
    }

    #[test]
    fn only_exports_are_called_an_export() {
        assert_eq!(
            question_title(&[job("job_002", "render", 0.4)]),
            "An export is still running"
        );
        assert_eq!(
            question_title(&[
                job("job_002", "render", 0.4),
                job("job_003", "transcribe", 0.0)
            ]),
            "Work is still running"
        );
        assert_eq!(question_title(&[]), "Work is still running");
    }

    /// An engine that takes the connection and never answers, as a stalled
    /// one does. The connections are held open until the test ends.
    fn stalled_engine() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().take(64) {
                held.push(stream);
            }
        });
        addr
    }

    #[test]
    fn an_engine_that_never_answers_is_not_taken_for_one_with_nothing_running() {
        let addr = stalled_engine();
        let answer = settle_within(
            &addr,
            || false,
            Duration::from_millis(1_200),
            Duration::from_millis(300),
        );
        assert_eq!(answer, Jobs::Unknown);
        // No answer is never "nothing runs": the close asks first.
        assert_eq!(plan(&answer), Plan::AskUnknown);
        assert_eq!(plan(&Jobs::None), Plan::Close);
        assert_eq!(plan(&jobs_from_reply(RUNNING)), Plan::AskRunning);
    }

    #[test]
    fn a_slow_engine_is_asked_again_and_a_gone_one_settles_at_once() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        drop(listener);
        assert_eq!(jobs(&addr, Duration::from_millis(500)), Jobs::Unknown);
        assert_eq!(
            plan(&jobs(&addr, Duration::from_millis(500))),
            Plan::AskUnknown
        );
        let started = Instant::now();
        assert_eq!(settle(&addr, || true), Jobs::None);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
