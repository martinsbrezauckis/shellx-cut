//! Closing or quitting the app never drops a running recording, and never
//! drops a running export without asking (see `quit_question`).
//!
//! The shell stops its own `cutd` when the window goes away, and a recording
//! lives inside that engine. Before this guard a close during a recording
//! killed the engine mid-capture: the take was lost without a word. Now a
//! close (window button, Alt+F4, Cmd+Q, app exit) first asks the engine whether
//! a recording is running. If one is, the close is held, the app window is
//! told (`cut:close-requested`) so it can stop the recording and add the take
//! to the timeline exactly as after a normal Stop while showing that it is
//! saving, and only then does the app close. If the window does not answer in
//! a few seconds (it may be stuck or not loaded), the shell stops the recording
//! itself with `screen_record.stop{reason:"app_exit"}`; the take is then saved
//! in the project and the next start says so. A crash or power cut, where no
//! code runs, is covered by the engine's checkpoint journal and the
//! `screen_record.pending` notice instead.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

/// How long the app window has to acknowledge the close request.
const ACK_WAIT: Duration = Duration::from_secs(6);
/// Upper bound for the window's Stop + add-to-timeline after acknowledging.
const UI_SAVE_LIMIT: Duration = Duration::from_secs(20 * 60);
/// Once the recording itself has ended, how long adding it may still take.
const AFTER_CAPTURE_LIMIT: Duration = Duration::from_secs(5 * 60);
/// Stop's own finalization budget is at most 15 minutes.
const STOP_LIMIT: Duration = Duration::from_secs(16 * 60);
/// How long the shell keeps asking an engine that answered slowly or not at
/// all whether a recording runs, before it closes without stopping the engine.
const UNKNOWN_LIMIT: Duration = Duration::from_secs(20);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Open,
    Saving,
    Released,
}

#[derive(Default)]
struct Guard {
    phase: Phase,
    acknowledged: bool,
    finished: bool,
    /// The engine could not say whether a recording runs: on close it is left
    /// to stop by itself (it finishes a running recording when its shell goes
    /// away) instead of being stopped by the shell.
    leave_engine: bool,
}

#[derive(Default)]
pub struct CloseGuardState(Mutex<Guard>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Close,
    Hold,
}

/// The capture id of a running project recording in a `screen_record.active`
/// reply body, or `None` (no recording, a rehearsal, or an unreadable reply).
fn recording_capture_id(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let result = value.get("result")?;
    (result.get("active")? == true && result.get("kind")? == "recording")
        .then(|| result.get("capture_id")?.as_str().map(str::to_string))
        .flatten()
}

/// What the engine says about a running recording. A slow, failed or
/// unreadable answer is `Unknown`, never "no recording".
#[derive(Debug, Clone, PartialEq, Eq)]
enum Activity {
    Recording(String),
    Idle,
    Unknown,
}

fn activity_from_reply(body: &str) -> Activity {
    let value: Option<serde_json::Value> = serde_json::from_str(body).ok();
    if value.as_ref().and_then(|value| value.get("ok")) != Some(&serde_json::Value::Bool(true)) {
        return Activity::Unknown;
    }
    if let Some(capture_id) = recording_capture_id(body) {
        return Activity::Recording(capture_id);
    }
    let result = value.as_ref().and_then(|value| value.get("result"));
    match result.and_then(|result| result.get("active")) {
        Some(serde_json::Value::Bool(false)) => Activity::Idle,
        // A disposable rehearsal take has no project recording to save.
        Some(serde_json::Value::Bool(true))
            if result
                .and_then(|result| result.get("kind"))
                .and_then(serde_json::Value::as_str)
                == Some("rehearsal") =>
        {
            Activity::Idle
        }
        _ => Activity::Unknown,
    }
}

/// Ask the engine once. A refused connection does not establish that the
/// shell's owned engine process has exited.
fn activity(addr: &str, read_timeout: Duration) -> Activity {
    match post(addr, "screen_record.active", "{}", read_timeout) {
        Ok(body) => activity_from_reply(&body),
        Err(_) => Activity::Unknown,
    }
}

pub(crate) fn post(
    addr: &str,
    verb: &str,
    body: &str,
    read_timeout: Duration,
) -> Result<String, String> {
    let sock_addr: std::net::SocketAddr = addr.parse().map_err(|e| format!("bad addr: {e}"))?;
    let mut stream =
        TcpStream::connect_timeout(&sock_addr, Duration::from_secs(2)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::ConnectionRefused {
                "connection refused".to_string()
            } else {
                e.to_string()
            }
        })?;
    let _ = stream.set_read_timeout(Some(read_timeout));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let request = format!(
        "POST /api/verb/{verb} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).map_err(|e| e.to_string())?;
    raw.split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .ok_or_else(|| "malformed engine response".to_string())
}

/// True once the engine process this shell started has exited, or the shell
/// has already let go of it (stopped it, or left it to finish a recording by
/// itself): there is nothing more for the guard to ask it. (On Windows a
/// connection to a closed loopback port is not refused at once, so the process
/// itself is asked. Without the empty-slot case, the exit request that follows
/// the window's own close waited about 30 s on the stopped engine with no
/// window left on screen.)
fn owned_engine_exited(app: &AppHandle) -> bool {
    app.try_state::<crate::EngineProcess>()
        .and_then(|state| {
            let mut slot = state.0.lock().ok()?;
            Some(engine_gone(slot.as_mut().map(|child| child.try_wait())))
        })
        .unwrap_or(false)
}

/// `None`: the slot is empty, the shell no longer holds an engine.
fn engine_gone(wait: Option<std::io::Result<Option<std::process::ExitStatus>>>) -> bool {
    match wait {
        None => true,
        Some(Ok(Some(_))) => true,
        Some(_) => false,
    }
}

/// Keep asking an engine that has not answered clearly, for at most
/// `UNKNOWN_LIMIT`. Returns as soon as the answer is clear, or `Idle` once
/// `exited()` says the engine process is gone.
fn settle_unknown(addr: &str, exited: impl Fn() -> bool) -> Activity {
    let started = Instant::now();
    loop {
        if exited() {
            return Activity::Idle;
        }
        let answer = activity(addr, Duration::from_secs(3));
        if answer != Activity::Unknown || started.elapsed() >= UNKNOWN_LIMIT {
            return answer;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn stop_for_exit(addr: &str, capture_id: &str) {
    let body = serde_json::json!({ "capture_id": capture_id, "reason": "app_exit" }).to_string();
    match post(addr, "screen_record.stop", &body, STOP_LIMIT) {
        Ok(reply) if reply.contains("\"ok\":true") => {}
        Ok(reply) => eprintln!("[shellx-cut] Stop before closing did not confirm: {reply}"),
        Err(error) => eprintln!("[shellx-cut] Stop before closing failed: {error}"),
    }
}

/// The loopback address of the engine this shell started. An adopted
/// external engine keeps running after the shell closes, and so does its
/// recording, so it needs no guard.
fn owned_engine_addr(app: &AppHandle) -> Option<String> {
    let status = app.try_state::<crate::EngineStatus>()?;
    let status = status.0.lock().ok()?;
    match &*status {
        crate::EngineState::Wired {
            url,
            mode: "spawned",
            ..
        } => crate::validated_engine_origin(url)
            .ok()
            .map(|origin| origin.trim_start_matches("http://").to_string()),
        _ => None,
    }
}

/// What the first, quick look at the engine found.
#[derive(Debug, PartialEq, Eq)]
enum FirstLook {
    /// No recording and no work someone asked for: close at once.
    Nothing,
    /// A recording runs (`Some` id) or may run (`None`, slow answer): it is
    /// saved first. It wins over the export question, which follows.
    Recording(Option<String>),
    /// No recording, but an export (or other asked-for work) runs or the
    /// engine was too slow to say: ask about it.
    Work,
}

fn first_look(addr: &str) -> FirstLook {
    match activity(addr, Duration::from_millis(1_500)) {
        Activity::Idle => match crate::quit_question::jobs(addr, Duration::from_millis(1_500)) {
            crate::quit_question::Jobs::None => FirstLook::Nothing,
            _ => FirstLook::Work,
        },
        Activity::Recording(capture_id) => FirstLook::Recording(Some(capture_id)),
        // A slow answer is not "no recording": hold and keep asking.
        Activity::Unknown => FirstLook::Recording(None),
    }
}

/// Decide a close or quit request. `Hold` means the caller must prevent it;
/// the guard closes (or exits, when `exit_after`) by itself once the
/// recording is saved.
pub(crate) fn on_close_request(app: &AppHandle, exit_after: bool) -> Decision {
    let Some(state) = app.try_state::<CloseGuardState>() else {
        return Decision::Close;
    };
    {
        let guard = state.0.lock().unwrap();
        match guard.phase {
            Phase::Released => return Decision::Close,
            Phase::Saving => {
                // Asked again while held: show the open question again.
                crate::quit_question::nudge(app);
                return Decision::Hold;
            }
            Phase::Open => {}
        }
    }
    let Some(addr) = owned_engine_addr(app) else {
        return Decision::Close;
    };
    if owned_engine_exited(app) {
        return Decision::Close;
    }
    let (capture_id, recording) = match first_look(&addr) {
        FirstLook::Nothing => return Decision::Close,
        FirstLook::Recording(capture_id) => (capture_id, true),
        FirstLook::Work => (None, false),
    };
    {
        let mut guard = state.0.lock().unwrap();
        if guard.phase != Phase::Open {
            return Decision::Hold;
        }
        *guard = Guard {
            phase: Phase::Saving,
            acknowledged: false,
            finished: false,
            leave_engine: false,
        };
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if recording {
            save_recording(&app, &addr, capture_id);
        }
        if owned_engine_exited(&app) {
            release_and_close(&app, exit_after);
            return;
        }
        // A recording is saved first; then running exports ask. When the
        // engine could not even say whether a recording runs, it is not known
        // whether anything runs: ask that plainly (closing would leave the
        // engine to stop by itself, ending any export with it).
        let outcome = if leave_engine_running(&app) {
            crate::quit_question::ask_when_unknown(&app)
        } else {
            let exited = || owned_engine_exited(&app);
            crate::quit_question::decide(&app, &addr, exited)
        };
        if let crate::quit_question::Outcome::Keep = outcome {
            keep_open(&app);
            return;
        }
        release_and_close(&app, exit_after);
    });
    Decision::Hold
}

/// What runs in this shell's engine, for the update hand-off, which stops
/// the engine without passing through the window's close.
// Linux has no in-app update install.
#[cfg_attr(target_os = "linux", allow(dead_code))]
#[derive(Debug, PartialEq)]
pub(crate) enum Work {
    Nothing,
    Recording,
    Jobs(Vec<crate::quit_question::RunningJob>),
    /// The engine did not say.
    Unknown,
}

// Linux has no in-app update install.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn work_at(addr: &str, read_timeout: Duration) -> Work {
    match activity(addr, read_timeout) {
        Activity::Recording(_) => Work::Recording,
        Activity::Unknown => Work::Unknown,
        Activity::Idle => match crate::quit_question::jobs(addr, read_timeout) {
            crate::quit_question::Jobs::None => Work::Nothing,
            crate::quit_question::Jobs::Running { running, .. } => Work::Jobs(running),
            crate::quit_question::Jobs::Unknown => Work::Unknown,
        },
    }
}

/// What the update asks before it stops the engine: nothing when nothing
/// runs; a running recording is never stopped this way (it is saved by Stop
/// or by quitting); work that runs, or an engine that does not answer, is
/// stopped only when the person says so.
// Linux has no in-app update install.
#[cfg_attr(target_os = "linux", allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UpdateAsk {
    StopRecordingFirst,
    StopWork(String),
    NotKnown,
}

// Linux has no in-app update install.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn update_ask(work: &Work) -> Option<UpdateAsk> {
    match work {
        Work::Nothing => None,
        Work::Recording => Some(UpdateAsk::StopRecordingFirst),
        Work::Jobs(jobs) => Some(UpdateAsk::StopWork(
            jobs.iter()
                .map(|job| {
                    format!(
                        "{} ({:.0}% done)",
                        job.kind.replace(['_', '-'], " "),
                        job.progress * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        )),
        Work::Unknown => Some(UpdateAsk::NotKnown),
    }
}

/// Before an update stops this shell's engine: true when nothing runs or the
/// person chose to stop it. Later is the safe side of every question.
// Linux has no in-app update install.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn update_may_stop_engine(app: &AppHandle) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let Some(addr) = owned_engine_addr(app) else {
        return true;
    };
    if owned_engine_exited(app) {
        return true;
    }
    let ask = |title: &str, text: String, yes: &str| {
        app.dialog()
            .message(text)
            .title(title)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                yes.into(),
                "Later".into(),
            ))
            .blocking_show()
    };
    match update_ask(&work_at(&addr, Duration::from_secs(5))) {
        None => true,
        Some(UpdateAsk::StopRecordingFirst) => {
            app.dialog()
                .message("A recording is running. Stop it first so it is saved, then install the update.")
                .title("Recording in progress")
                .kind(MessageDialogKind::Info)
                .blocking_show();
            false
        }
        Some(UpdateAsk::StopWork(jobs)) => ask(
            "Work is still running",
            format!("Still running: {jobs}.\n\nInstalling the update now stops it; the stopped work is lost. Install later to let it finish."),
            "Install and stop it",
        ),
        Some(UpdateAsk::NotKnown) => ask(
            crate::quit_question::UNKNOWN_TITLE,
            "ShellX Cut could not find out whether work is still running: its engine did not answer.\n\nInstall the update anyway? Work that is running, such as an export, would be stopped and lost.".to_string(),
            "Install anyway",
        ),
    }
}

fn flags(app: &AppHandle) -> (bool, bool) {
    app.try_state::<CloseGuardState>()
        .map(|state| {
            let guard = state.0.lock().unwrap();
            (guard.acknowledged, guard.finished)
        })
        .unwrap_or((false, true))
}

fn save_recording(app: &AppHandle, addr: &str, capture_id: Option<String>) {
    let app = app.clone();
    if capture_id.is_none() && owned_engine_exited(&app) {
        return;
    }
    let _ = app.emit_to(
        "main",
        "cut:close-requested",
        serde_json::json!({ "capture_id": capture_id }),
    );
    let asked = Instant::now();
    let capture_id = match capture_id {
        Some(capture_id) => Some(capture_id),
        None => {
            // Tell the person why the window is still open while asking again.
            let window = app.get_webview_window("main");
            if let Some(window) = window.as_ref() {
                let _ =
                    window.set_title("ShellX Cut: checking for a running recording before closing");
            }
            match settle_unknown(addr, || owned_engine_exited(&app)) {
                Activity::Recording(capture_id) => Some(capture_id),
                Activity::Idle => None,
                Activity::Unknown => {
                    set_leave_engine(&app);
                    None
                }
            }
        }
    };
    if capture_id.is_none() && owned_engine_exited(&app) {
        return;
    }
    while asked.elapsed() < ACK_WAIT
        && !flags(&app).0
        && !(capture_id.is_none() && owned_engine_exited(&app))
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    if capture_id.is_none() && owned_engine_exited(&app) {
        return;
    }
    if flags(&app).0 {
        let started = Instant::now();
        let mut capture_ended_at: Option<Instant> = None;
        while started.elapsed() < UI_SAVE_LIMIT && !flags(&app).1 {
            if capture_id.is_none() && owned_engine_exited(&app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
            let ended = match (&capture_id, activity(addr, Duration::from_millis(1_500))) {
                (Some(id), Activity::Recording(running)) => &running != id,
                (_, Activity::Unknown) => false,
                _ => true,
            };
            if capture_ended_at.is_none() && ended {
                capture_ended_at = Some(Instant::now());
            }
            if capture_ended_at.is_some_and(|ended| ended.elapsed() > AFTER_CAPTURE_LIMIT) {
                break;
            }
        }
    }
    if capture_id.is_none() && owned_engine_exited(&app) {
        return;
    }
    // Whatever the window did, never close over a recording that still runs,
    // and never stop an engine that cannot say whether one runs.
    match activity(addr, Duration::from_secs(3)) {
        Activity::Recording(running) => stop_for_exit(addr, &running),
        Activity::Unknown => set_leave_engine(&app),
        Activity::Idle => {}
    }
}

/// The person chose to keep working: the close is over, nothing closes.
fn keep_open(app: &AppHandle) {
    if let Some(state) = app.try_state::<CloseGuardState>() {
        *state.0.lock().unwrap() = Guard::default();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title("ShellX Cut");
    }
}

fn release_and_close(app: &AppHandle, exit_after: bool) {
    if let Some(state) = app.try_state::<CloseGuardState>() {
        state.0.lock().unwrap().phase = Phase::Released;
    }
    if exit_after {
        app.exit(0);
    } else if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }
}

fn set_leave_engine(app: &AppHandle) {
    if let Some(state) = app.try_state::<CloseGuardState>() {
        state.0.lock().unwrap().leave_engine = true;
    }
}

/// The app is ending and the guard was never asked: Windows ends the session
/// (the windowing layer turns WM_ENDSESSION straight into the final exit), or
/// macOS terminates without the quit hook. There is no time to ask anything.
/// A recording that may still run is not cut off: the shell leaves its engine,
/// which saves the recording and stops by itself once this shell is gone. An
/// export stops with the engine; the next start says so and tidies up.
pub(crate) fn on_unasked_exit(app: &AppHandle) {
    let asked = app
        .try_state::<CloseGuardState>()
        .is_some_and(|state| state.0.lock().unwrap().phase == Phase::Released);
    let engine_held = app
        .try_state::<crate::EngineProcess>()
        .is_some_and(|state| state.0.lock().map(|slot| slot.is_some()).unwrap_or(false));
    if asked || !engine_held || owned_engine_exited(app) {
        return;
    }
    let Some(addr) = owned_engine_addr(app) else {
        return;
    };
    if unasked_exit_leaves_engine(activity(&addr, Duration::from_millis(800))) {
        set_leave_engine(app);
    }
}

/// Only a clear "no recording" lets the shell stop its engine at an unasked exit.
fn unasked_exit_leaves_engine(answer: Activity) -> bool {
    answer != Activity::Idle
}

/// True when the close went ahead without knowing whether a recording runs:
/// the shell must not stop the engine then. The engine notices its shell is
/// gone and shuts down by itself, finishing any recording first.
/// A close or quit is being decided or carried out.
pub(crate) fn closing(app: &AppHandle) -> bool {
    app.try_state::<CloseGuardState>()
        .is_some_and(|state| state.0.lock().unwrap().phase != Phase::Open)
}

pub(crate) fn leave_engine_running(app: &AppHandle) -> bool {
    app.try_state::<CloseGuardState>()
        .is_some_and(|state| state.0.lock().unwrap().leave_engine)
}

/// The app window has taken over: it stops the recording and adds the take.
#[tauri::command]
pub fn acknowledge_close_request(state: tauri::State<'_, CloseGuardState>) -> bool {
    let mut guard = state.0.lock().unwrap();
    if guard.phase != Phase::Saving {
        return false;
    }
    guard.acknowledged = true;
    true
}

/// The app window is done (saved, or failed and told the person); close now.
#[tauri::command]
pub fn finish_close_request(state: tauri::State<'_, CloseGuardState>) -> bool {
    let mut guard = state.0.lock().unwrap();
    if guard.phase != Phase::Saving {
        return false;
    }
    guard.finished = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_running_project_recording_holds_the_close() {
        assert_eq!(
            recording_capture_id(
                r#"{"ok":true,"result":{"active":true,"capture_id":"cap_1_2_3","kind":"recording"}}"#
            )
            .as_deref(),
            Some("cap_1_2_3")
        );
        assert_eq!(
            recording_capture_id(
                r#"{"ok":true,"result":{"active":true,"capture_id":"cap_1_2_3","kind":"rehearsal"}}"#
            ),
            None
        );
        assert_eq!(
            recording_capture_id(r#"{"ok":true,"result":{"active":false,"capture_id":null}}"#),
            None
        );
        assert_eq!(recording_capture_id("not json"), None);
    }

    #[test]
    fn a_closed_port_is_unknown_until_the_owned_engine_exits() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        drop(listener);
        assert_eq!(
            activity(&addr, Duration::from_millis(500)),
            Activity::Unknown
        );
        assert_eq!(first_look(&addr), FirstLook::Recording(None));
        // A gone engine settles at once, without the bounded wait.
        let started = Instant::now();
        assert_eq!(settle_unknown(&addr, || true), Activity::Idle);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// A loopback engine that answers `body` after `delay`.
    fn slow_engine(delay: Duration, body: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let Ok(mut stream) = stream else { continue };
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                std::thread::sleep(delay);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        addr
    }

    const RECORDING: &str =
        r#"{"ok":true,"result":{"active":true,"capture_id":"cap_1_2_3","kind":"recording"}}"#;

    #[test]
    fn a_slow_answer_is_not_read_as_no_recording() {
        let addr = slow_engine(Duration::from_millis(2_500), RECORDING);
        assert_eq!(
            activity(&addr, Duration::from_millis(1_500)),
            Activity::Unknown,
            "an answer slower than the first check must keep the window open"
        );
        // Asked again with the longer wait, the same engine names the recording.
        assert_eq!(
            settle_unknown(&addr, || false),
            Activity::Recording("cap_1_2_3".into())
        );
    }

    /// A loopback engine answering each verb with its own body.
    fn engine(answers: &'static [(&'static str, &'static str)]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(16) {
                let Ok(mut stream) = stream else { continue };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]).to_string();
                let body = answers
                    .iter()
                    .find(|(verb, _)| request.contains(&format!("/api/verb/{verb} ")))
                    .map(|(_, body)| *body)
                    .unwrap_or(r#"{"ok":false,"error":{"code":"not_found"}}"#);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        addr
    }

    const IDLE: &str = r#"{"ok":true,"result":{"active":false,"capture_id":null}}"#;
    const EXPORTING: &str = r#"{"ok":false,"error":{"code":"jobs_running","message":"quitting ShellX Cut would cancel 1 running job(s)","cause":"running: job_002 (render, 40% done)","suggested_action":"wait"}}"#;
    const NO_WORK: &str = r#"{"ok":true,"result":{"running":[],"cancelled_jobs":[]}}"#;

    #[test]
    fn an_update_never_stops_the_engine_over_work_without_asking() {
        let exporting = engine(&[
            ("screen_record.active", IDLE),
            ("jobs.before_quit", EXPORTING),
        ]);
        let work = work_at(&exporting, Duration::from_millis(1_500));
        assert!(matches!(update_ask(&work), Some(UpdateAsk::StopWork(_))));
        let idle = engine(&[
            ("screen_record.active", IDLE),
            ("jobs.before_quit", NO_WORK),
        ]);
        assert_eq!(
            update_ask(&work_at(&idle, Duration::from_millis(1_500))),
            None
        );
        let recording = engine(&[("screen_record.active", RECORDING)]);
        assert_eq!(
            update_ask(&work_at(&recording, Duration::from_millis(1_500))),
            Some(UpdateAsk::StopRecordingFirst)
        );
        // An engine that takes the connection and never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stalled = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().take(8) {
                held.push(stream);
            }
        });
        assert_eq!(
            update_ask(&work_at(&stalled, Duration::from_millis(300))),
            Some(UpdateAsk::NotKnown)
        );
    }

    #[test]
    fn a_close_during_an_export_is_held_and_a_recording_wins() {
        // Window close, app exit request and the macOS terminate hook all call
        // on_close_request, which decides from this first look.
        let exporting = engine(&[
            ("screen_record.active", IDLE),
            ("jobs.before_quit", EXPORTING),
        ]);
        assert_eq!(first_look(&exporting), FirstLook::Work);
        let idle = engine(&[
            ("screen_record.active", IDLE),
            ("jobs.before_quit", NO_WORK),
        ]);
        assert_eq!(first_look(&idle), FirstLook::Nothing);
        let both = engine(&[
            ("screen_record.active", RECORDING),
            ("jobs.before_quit", EXPORTING),
        ]);
        assert_eq!(
            first_look(&both),
            FirstLook::Recording(Some("cap_1_2_3".into()))
        );
        // An engine without the export check has not established that work is idle.
        let older = engine(&[("screen_record.active", IDLE)]);
        assert_eq!(first_look(&older), FirstLook::Work);
    }

    #[test]
    fn an_engine_the_shell_already_let_go_of_is_not_asked_again() {
        // The window's own close stops the engine and empties the slot; the
        // exit request that follows must not wait on it.
        assert!(engine_gone(None));
        assert!(!engine_gone(Some(Ok(None))), "a running engine is asked");
        let failed = std::io::Error::other("probe failed");
        assert!(!engine_gone(Some(Err(failed))));
    }

    #[test]
    fn an_unasked_exit_never_stops_an_engine_that_may_be_recording() {
        assert!(unasked_exit_leaves_engine(Activity::Recording(
            "cap_1".into()
        )));
        assert!(unasked_exit_leaves_engine(Activity::Unknown));
        assert!(!unasked_exit_leaves_engine(Activity::Idle));
    }

    #[test]
    fn clear_answers_decide_and_unreadable_ones_stay_unknown() {
        assert_eq!(
            activity_from_reply(RECORDING),
            Activity::Recording("cap_1_2_3".into())
        );
        assert_eq!(
            activity_from_reply(r#"{"ok":true,"result":{"active":false,"capture_id":null}}"#),
            Activity::Idle
        );
        assert_eq!(
            activity_from_reply(
                r#"{"ok":true,"result":{"active":true,"capture_id":"cap_1","kind":"rehearsal"}}"#
            ),
            Activity::Idle
        );
        assert_eq!(
            activity_from_reply(r#"{"ok":true,"result":{"active":true,"kind":"recording"}}"#),
            Activity::Unknown
        );
        assert_eq!(activity_from_reply("not json"), Activity::Unknown);
        assert_eq!(
            activity_from_reply(r#"{"ok":false,"error":{"code":"io"}}"#),
            Activity::Unknown
        );
        assert_eq!(
            activity_from_reply(r#"{"ok":false,"result":{"active":false},"error":{"code":"io"}}"#),
            Activity::Unknown
        );
        assert_eq!(
            activity_from_reply(
                r#"{"ok":false,"result":{"active":true,"kind":"recording","capture_id":"forged"},"error":{"code":"io"}}"#
            ),
            Activity::Unknown
        );
    }
}
