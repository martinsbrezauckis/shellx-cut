//! Passive input availability inside the disposable cutd probe child only.
//!
//! ALSA's `default` and Pulse bridge devices exist even without an input. A
//! completed registry round trip counts capture sources instead; it never
//! creates a stream or requests permission. The parent enforces the deadline.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use pipewire as pw;
use pw::types::ObjectType;

const MAX_GLOBALS: usize = 1024;
const MAX_IDS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrophoneAvailability {
    Available,
    Absent,
    Unknown,
}

#[derive(Default)]
struct Inventory {
    sources: BTreeSet<u32>,
    pulse_clients: BTreeSet<u32>,
    custom_route: bool,
    complete: bool,
    failed: bool,
    globals_seen: usize,
}

impl Inventory {
    fn node(
        &mut self,
        id: u32,
        class: Option<&str>,
        name: Option<&str>,
        device_class: Option<&str>,
    ) {
        if matches!(class, Some("Audio/Source" | "Audio/Source/Virtual"))
            && device_class != Some("monitor")
            && !name.is_some_and(|name| name.ends_with(".monitor"))
        {
            if self.sources.contains(&id) || self.sources.len() < MAX_IDS {
                self.sources.insert(id);
            } else {
                self.failed = true;
            }
        }
    }

    fn client(&mut self, id: u32, api: Option<&str>) {
        if api == Some("pipewire-pulse") {
            if self.pulse_clients.contains(&id) || self.pulse_clients.len() < MAX_IDS {
                self.pulse_clients.insert(id);
            } else {
                self.failed = true;
            }
        }
    }

    fn remove(&mut self, id: u32) {
        self.sources.remove(&id);
        self.pulse_clients.remove(&id);
    }

    fn availability(&self) -> MicrophoneAvailability {
        if self.failed || !self.complete {
            MicrophoneAvailability::Unknown
        } else if !self.sources.is_empty() {
            MicrophoneAvailability::Available
        } else if !self.custom_route && !self.pulse_clients.is_empty() {
            MicrophoneAvailability::Absent
        } else {
            // A video-only server is not the microphone's audio inventory.
            MicrophoneAvailability::Unknown
        }
    }
}

/// Fixed one-line protocol. Only this disposable child calls PipeWire.
pub(super) fn probe_answer() -> &'static str {
    let no_core = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &no_core) } != 0 {
        return "UNKNOWN\n";
    }
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) } != 0 {
        return "UNKNOWN\n";
    }
    limit.rlim_max = limit.rlim_max.min(512 * 1024 * 1024);
    limit.rlim_cur = limit.rlim_cur.min(limit.rlim_max);
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) } != 0 {
        return "UNKNOWN\n";
    }
    match query().unwrap_or(MicrophoneAvailability::Unknown) {
        MicrophoneAvailability::Available => "PRESENT\n",
        MicrophoneAvailability::Absent => "ABSENT\n",
        MicrophoneAvailability::Unknown => "UNKNOWN\n",
    }
}

fn pulse_bridge_client(
    api: Option<&str>,
    pid: Option<&str>,
    read_comm: impl FnOnce(u32) -> Option<String>,
) -> bool {
    if api == Some("pipewire-pulse") {
        return true;
    }
    // Registry globals may omit client.api. The server's peer PID identifies
    // a matching bridge process name without trusting an application's display name. Hidden
    // process metadata leaves the audio route unknown.
    pid.and_then(|pid| pid.parse::<u32>().ok())
        .and_then(read_comm)
        .is_some_and(|comm| comm.trim_end() == "pipewire-pulse")
}

fn custom_audio_route() -> bool {
    if [
        "PULSE_SERVER",
        "PULSE_SOURCE",
        "ALSA_CONFIG_PATH",
        "ALSA_CONFIG_DIR",
        "ALSA_CARD",
        "ALSA_PCM_CARD",
        "PIPEWIRE_REMOTE",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
    {
        return true;
    }
    // Do not interpret a custom ALSA route or assume it uses this server.
    let mut paths = vec![std::path::PathBuf::from("/etc/asound.conf")];
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        paths.push(home.join(".asoundrc"));
        paths.push(home.join(".config/alsa/asoundrc"));
    }
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        paths.push(std::path::PathBuf::from(config).join("alsa/asoundrc"));
    }
    paths
        .into_iter()
        .any(|path| match std::fs::symlink_metadata(path) {
            Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        })
}

fn query() -> Option<MicrophoneAvailability> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).ok()?;
    let context = pw::context::ContextRc::new(&mainloop, None).ok()?;
    let core = context.connect_rc(None).ok()?;
    let registry = core.get_registry_rc().ok()?;
    let inventory = Rc::new(RefCell::new(Inventory {
        custom_route: custom_audio_route(),
        ..Inventory::default()
    }));
    let globals = inventory.clone();
    let removed = inventory.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |object| {
            let props = object.props.as_ref();
            if object.type_ == ObjectType::Node {
                let mut inventory = globals.borrow_mut();
                inventory.globals_seen += 1;
                if inventory.globals_seen > MAX_GLOBALS {
                    inventory.failed = true;
                    return;
                }
                inventory.node(
                    object.id,
                    props.and_then(|p| p.get("media.class")),
                    props.and_then(|p| p.get("node.name")),
                    props.and_then(|p| p.get("device.class")),
                );
            } else if object.type_ == ObjectType::Client {
                let mut inventory = globals.borrow_mut();
                inventory.globals_seen += 1;
                if inventory.globals_seen > MAX_GLOBALS {
                    inventory.failed = true;
                    return;
                }
                let pulse = pulse_bridge_client(
                    props.and_then(|p| p.get("client.api")),
                    props.and_then(|p| p.get("pipewire.sec.pid")),
                    |pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok(),
                );
                inventory.client(object.id, pulse.then_some("pipewire-pulse"));
            }
        })
        .global_remove(move |id| {
            removed.borrow_mut().remove(id);
        })
        .register();
    // Only the matching Done after GetRegistry proves the inventory complete.
    let pending = core.sync(0).ok()?;
    let completed = inventory.clone();
    let failed = inventory.clone();
    let done_loop = mainloop.downgrade();
    let error_loop = mainloop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                completed.borrow_mut().complete = true;
                if let Some(mainloop) = done_loop.upgrade() {
                    mainloop.quit();
                }
            }
        })
        .error(move |_, _, _, _| {
            failed.borrow_mut().failed = true;
            if let Some(mainloop) = error_loop.upgrade() {
                mainloop.quit();
            }
        })
        .register();
    mainloop.run();
    let result = inventory.borrow().availability();
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_empty_bridge_and_output_monitor_inventories_are_absent() {
        let mut inventory = Inventory {
            complete: true,
            ..Inventory::default()
        };
        inventory.client(10, Some("pipewire-pulse"));
        assert_eq!(inventory.availability(), MicrophoneAvailability::Absent);
        for (id, class, name, device_class) in [
            (1, None, Some("default"), None),
            (2, Some("Audio/Sink"), Some("output"), None),
            (3, Some("Stream/Input/Audio"), Some("pulse"), None),
            (4, Some("Audio/Source"), Some("output.monitor"), None),
            (5, Some("Audio/Source"), Some("output"), Some("monitor")),
            (6, Some("Midi/Bridge"), Some("bridge"), None),
        ] {
            inventory.node(id, class, name, device_class);
        }
        assert_eq!(inventory.availability(), MicrophoneAvailability::Absent);
    }

    #[test]
    fn physical_and_virtual_inputs_are_available_even_when_silent() {
        for class in ["Audio/Source", "Audio/Source/Virtual"] {
            let mut inventory = Inventory::default();
            inventory.node(1, Some(class), Some("input"), None);
            inventory.complete = true;
            assert_eq!(inventory.availability(), MicrophoneAvailability::Available);
            inventory.client(10, Some("pipewire-pulse"));
            inventory.remove(1);
            assert_eq!(inventory.availability(), MicrophoneAvailability::Absent);
        }
    }

    #[test]
    fn bridge_identity_uses_the_api_or_peer_process_name() {
        assert!(pulse_bridge_client(
            Some("pipewire-pulse"),
            None,
            |_| panic!("no read needed")
        ));
        assert!(pulse_bridge_client(None, Some("42"), |pid| {
            assert_eq!(pid, 42);
            Some("pipewire-pulse\n".into())
        }));
        assert!(!pulse_bridge_client(None, Some("42"), |_| Some(
            "pulseaudio\n".into()
        )));
        assert!(!pulse_bridge_client(None, Some("42"), |_| None));
        assert!(!pulse_bridge_client(None, Some("not-a-pid"), |_| panic!(
            "invalid PID"
        )));
    }

    #[test]
    fn a_video_only_server_or_custom_audio_route_cannot_prove_absence() {
        let mut inventory = Inventory {
            complete: true,
            ..Inventory::default()
        };
        inventory.node(1, Some("Audio/Sink"), Some("output"), None);
        inventory.client(2, Some("another-audio-api"));
        assert_eq!(inventory.availability(), MicrophoneAvailability::Unknown);
        inventory.client(3, Some("pipewire-pulse"));
        assert_eq!(inventory.availability(), MicrophoneAvailability::Absent);
        inventory.custom_route = true;
        assert_eq!(inventory.availability(), MicrophoneAvailability::Unknown);
        inventory.node(4, Some("Audio/Source"), Some("input"), None);
        assert_eq!(inventory.availability(), MicrophoneAvailability::Available);
        inventory.remove(4);
        inventory.custom_route = false;
        inventory.remove(3);
        assert_eq!(inventory.availability(), MicrophoneAvailability::Unknown);
    }

    #[test]
    fn incomplete_or_failed_enumeration_is_unknown() {
        for has_input in [false, true] {
            let mut inventory = Inventory::default();
            if has_input {
                inventory.node(1, Some("Audio/Source"), None, None);
            }
            assert_eq!(inventory.availability(), MicrophoneAvailability::Unknown);
            inventory.complete = true;
            inventory.failed = true;
            assert_eq!(inventory.availability(), MicrophoneAvailability::Unknown);
        }
    }

    #[test]
    fn unique_id_inventory_is_bounded_and_overflow_is_unknown() {
        let mut inventory = Inventory {
            complete: true,
            ..Inventory::default()
        };
        for id in 0..MAX_IDS as u32 {
            inventory.node(id, Some("Audio/Source"), Some("input"), None);
        }
        assert_eq!(inventory.sources.len(), MAX_IDS);
        inventory.node(0, Some("Audio/Source"), Some("input"), None);
        assert!(!inventory.failed);
        inventory.node(MAX_IDS as u32, Some("Audio/Source"), Some("input"), None);
        assert_eq!(inventory.sources.len(), MAX_IDS);
        assert_eq!(inventory.availability(), MicrophoneAvailability::Unknown);
    }
}
