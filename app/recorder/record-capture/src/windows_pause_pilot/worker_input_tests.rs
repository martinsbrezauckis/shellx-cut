use std::sync::{Arc, Mutex};

use super::super::input_run::WindowsPauseInputOwner;
use super::test_support::*;
use super::*;
use crate::windows_pause_pilot::{channel, WindowsPausePilotStarted, WindowsSealedInputRun};
use crate::windows_wgc_run::WgcRunOwner;

struct InputFactory {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl WindowsPauseInputFactory for InputFactory {
    fn start(
        &mut self,
        _: &WindowsPausePilotStarted,
    ) -> Result<Box<dyn WindowsPauseInputOwner>, ()> {
        self.calls.lock().unwrap().push("start-input");
        Ok(Box::new(InputOwner {
            calls: self.calls.clone(),
        }))
    }
}

struct InputOwner {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl WindowsPauseInputOwner for InputOwner {
    fn seal_after_screen(
        self: Box<Self>,
        screen_duration_ms: u64,
    ) -> Result<WindowsSealedInputRun, ()> {
        self.calls.lock().unwrap().push("seal-input");
        Ok(WindowsSealedInputRun {
            cursor: vec![record_core::CursorSample {
                t_ms: screen_duration_ms - 1,
                x: 2.0,
                y: 3.0,
            }],
            clicks: Vec::new(),
            scrolls: Vec::new(),
            keys: Vec::new(),
        })
    }
}

#[test]
fn test_only_input_owner_restarts_per_generation_and_seals_only_with_wgc_runs() {
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let resolutions = std::rc::Rc::new(std::cell::RefCell::new(vec![
        Some("monitor:first"),
        Some("monitor:resolved-again"),
    ]));
    let resolver: Resolver = Box::new({
        let resolutions = resolutions.clone();
        move |_id| resolutions.borrow_mut().remove(0).map(str::to_owned)
    });
    let owner_log = log.clone();
    let publisher_log = log.clone();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let profile =
        WindowsPausePilotProfile::test_profile_with_passive_input(exact_monitor_id(), 30.0);
    let (mut worker, _) = WindowsPausePilotWorker::start(
        profile,
        resolver,
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                Factory(owner_log),
                Publisher {
                    log: publisher_log,
                    next: 0,
                    reject: false,
                },
            ))
        },
        Box::new(InputFactory {
            calls: calls.clone(),
        }),
        Box::new(super::super::audio_run::DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
    )
    .unwrap();
    let (commands, receiver, sender, events) = channel();

    commands
        .send(WindowsPausePilotCommand::Pause {
            generation: 1,
            epoch: 1,
        })
        .unwrap();
    process(&mut worker, &receiver, &sender, 50, 60);
    let WindowsPausePilotEvent::PauseSealed {
        input: Some(input), ..
    } = events.try_recv().unwrap().unwrap()
    else {
        panic!("selected input must seal with the first published WGC run")
    };
    assert_eq!(input.cursor[0].t_ms, 49);

    commands
        .send(WindowsPausePilotCommand::Resume {
            generation: 2,
            epoch: 2,
        })
        .unwrap();
    process(&mut worker, &receiver, &sender, 70, 80);
    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::ResumeReady { generation: 2, .. }
    ));
    commands
        .send(WindowsPausePilotCommand::Pause {
            generation: 3,
            epoch: 3,
        })
        .unwrap();
    process(&mut worker, &receiver, &sender, 90, 100);
    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::PauseSealed {
            generation: 3,
            input: Some(_),
            ..
        }
    ));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["start-input", "seal-input", "start-input", "seal-input"]
    );
    assert_eq!(
        log.borrow().as_slice(),
        [
            "reserve",
            "start:monitor:first",
            "close",
            "publish",
            "reserve",
            "start:monitor:resolved-again",
            "close",
            "publish"
        ]
    );
}

#[test]
fn test_only_input_start_failure_closes_wgc_before_reporting_start_failure() {
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let owner_log = log.clone();
    let publisher_log = log.clone();
    let profile =
        WindowsPausePilotProfile::test_profile_with_passive_input(exact_monitor_id(), 30.0);
    let resolver: Resolver = Box::new(|_: &str| Some("monitor:first".to_string()));
    let result = WindowsPausePilotWorker::start(
        profile,
        resolver,
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                Factory(owner_log),
                Publisher {
                    log: publisher_log,
                    next: 0,
                    reject: false,
                },
            ))
        },
        Box::new(super::super::input_run::DisabledWindowsPauseInputFactory),
        Box::new(super::super::audio_run::DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
    );
    assert!(matches!(
        result,
        Err(WindowsPausePilotStartError::NativeStartFailed)
    ));
    assert_eq!(
        log.borrow().as_slice(),
        ["reserve", "start:monitor:first", "close"]
    );
}
