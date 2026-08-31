use std::sync::{Arc, Mutex};

use super::super::audio_run::{WindowsPauseAudioFactory, WindowsPauseAudioOwner};
use super::super::input_run::DisabledWindowsPauseInputFactory;
use super::test_support::*;
use super::*;
use crate::windows_pause_pilot::{
    channel, WindowsPausePilotRequest, WindowsPausePilotStarted, WindowsSealedAudioRun,
};
use crate::windows_wgc_run::WgcRunOwner;
use record_recovery::RecordingStream;

struct AudioFactory {
    calls: Arc<Mutex<Vec<String>>>,
}

impl WindowsPauseAudioFactory for AudioFactory {
    fn start(
        &mut self,
        profile: &WindowsPausePilotProfile,
        started: &WindowsPausePilotStarted,
    ) -> Result<Vec<Box<dyn WindowsPauseAudioOwner>>, ()> {
        profile
            .selected_audio_streams()
            .into_iter()
            .map(|stream| {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("start:{stream:?}:{}", started.physical_generation));
                Ok(Box::new(AudioOwner {
                    stream,
                    calls: self.calls.clone(),
                    started: started.clone(),
                }) as Box<dyn WindowsPauseAudioOwner>)
            })
            .collect()
    }
}

struct AudioOwner {
    stream: RecordingStream,
    calls: Arc<Mutex<Vec<String>>>,
    started: WindowsPausePilotStarted,
}

impl WindowsPauseAudioOwner for AudioOwner {
    fn stream(&self) -> RecordingStream {
        self.stream
    }

    fn seal_after_screen(self: Box<Self>, raw_end_ms: u64) -> Result<WindowsSealedAudioRun, ()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("seal:{:?}:{raw_end_ms}", self.stream));
        Ok(WindowsSealedAudioRun {
            stream: self.stream,
            source_generation: self.started.physical_generation,
            artifact: format!(
                "audio-{:?}-{}.wav",
                self.stream, self.started.physical_generation
            ),
            bytes: 48,
            sha256: "a".repeat(64),
            media_duration_ms: raw_end_ms - self.started.observed_start_ms,
            native_ready_unix_ms: self.started.unix_ms,
            native_ready_raw_ms: self.started.observed_start_ms,
            raw_start_ms: self.started.observed_start_ms,
            raw_end_ms,
        })
    }
}

#[test]
fn microphone_and_system_audio_restart_per_generation_and_seal_after_screen_publication() {
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let resolutions = std::rc::Rc::new(std::cell::RefCell::new(vec![
        Some("monitor:first"),
        Some("monitor:second"),
    ]));
    let resolver: Resolver = Box::new({
        let resolutions = resolutions.clone();
        move |_| resolutions.borrow_mut().remove(0).map(str::to_owned)
    });
    let owner_log = log.clone();
    let publisher_log = log.clone();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let profile = WindowsPausePilotProfile::admit(WindowsPausePilotRequest::screen_with_audio(
        exact_monitor_id(),
        30.0,
        true,
        true,
    ))
    .unwrap();
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
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(AudioFactory {
            calls: calls.clone(),
        }),
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
    let WindowsPausePilotEvent::PauseSealed { audio, .. } = events.try_recv().unwrap().unwrap()
    else {
        panic!("all selected audio workers must seal with the first WGC run")
    };
    assert_eq!(
        audio.iter().map(|item| item.stream).collect::<Vec<_>>(),
        vec![
            RecordingStream::MicrophoneAudio,
            RecordingStream::SystemAudio
        ]
    );

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
        WindowsPausePilotEvent::PauseSealed { audio, .. }
            if audio.len() == 2
    ));
    assert_eq!(
        calls.lock().unwrap().len(),
        8,
        "each selected source must start and seal once per logical generation"
    );
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|entry| entry.as_str() == "publish")
            .count(),
        2,
        "audio seal follows one published screen run per pause generation"
    );
}

#[test]
fn stop_dominates_a_queued_pause_without_reassigning_the_selected_audio_leaf() {
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let owner_log = log.clone();
    let publisher_log = log.clone();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resolver: Resolver = Box::new(|_| Some("monitor:first".to_string()));
    let profile = WindowsPausePilotProfile::admit(WindowsPausePilotRequest::screen_with_audio(
        exact_monitor_id(),
        30.0,
        true,
        false,
    ))
    .unwrap();
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
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(AudioFactory {
            calls: calls.clone(),
        }),
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
    commands
        .send(WindowsPausePilotCommand::Stop { epoch: 2 })
        .unwrap();
    process(&mut worker, &receiver, &sender, 50, 60);

    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::StopSealed { epoch: 2, audio, .. }
            if audio.len() == 1 && audio[0].source_generation == 1
    ));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["start:MicrophoneAudio:1", "seal:MicrophoneAudio:60"],
        "Stop seals the owned leaf at its terminal screen boundary and invalidates queued Pause"
    );
    assert!(events.try_recv().unwrap().is_none());
}
