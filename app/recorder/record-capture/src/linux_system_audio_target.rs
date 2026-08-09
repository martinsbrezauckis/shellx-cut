//! Resolve the PipeWire/WirePlumber default sink without spawning a helper.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use pipewire as pw;
use pw::types::ObjectType;

const DEFAULT_METADATA: &str = "default";
const DEFAULT_SINK_KEY: &str = "default.audio.sink";
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(1);

fn parse_sink(value: Option<&str>) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(value?)
        .ok()?
        .get("name")?
        .as_str()
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

/// Resolve the current default sink name from WirePlumber's `default` metadata.
/// The returned name becomes `target.object` for the native input stream; no CLI
/// child and no Pulse compatibility server is involved.
pub(super) fn default_sink_target(
    core: &pw::core::CoreRc,
    mainloop: &pw::main_loop::MainLoopRc,
) -> std::result::Result<String, String> {
    let sink = Rc::new(RefCell::new(None));
    let metadata_handles: Rc<
        RefCell<Vec<(pw::metadata::Metadata, pw::metadata::MetadataListener)>>,
    > = Rc::new(RefCell::new(Vec::new()));
    let registry = core
        .get_registry_rc()
        .map_err(|error| format!("create PipeWire registry: {error}"))?;
    let registry_weak = registry.downgrade();
    let sink_for_registry = sink.clone();
    let handles_for_registry = metadata_handles.clone();
    let mainloop_for_registry = mainloop.clone();
    let registry_listener = registry
        .add_listener_local()
        .global(move |object| {
            if object.type_ != ObjectType::Metadata
                || object
                    .props
                    .as_ref()
                    .and_then(|props| props.get("metadata.name"))
                    != Some(DEFAULT_METADATA)
            {
                return;
            }
            let Some(registry) = registry_weak.upgrade() else {
                return;
            };
            let Ok(metadata) = registry.bind::<pw::metadata::Metadata, _>(object) else {
                return;
            };
            let sink_for_property = sink_for_registry.clone();
            let loop_for_property = mainloop_for_registry.downgrade();
            let listener = metadata
                .add_listener_local()
                .property(move |subject, key, _, value| {
                    if subject == 0 && key == Some(DEFAULT_SINK_KEY) {
                        if let Some(name) = parse_sink(value) {
                            *sink_for_property.borrow_mut() = Some(name);
                            if let Some(mainloop) = loop_for_property.upgrade() {
                                mainloop.quit();
                            }
                        }
                    }
                    0
                })
                .register();
            handles_for_registry.borrow_mut().push((metadata, listener));
        })
        .register();
    let timeout_loop = mainloop.downgrade();
    let timeout = mainloop.loop_().add_timer(move |_| {
        if let Some(mainloop) = timeout_loop.upgrade() {
            mainloop.quit();
        }
    });
    timeout
        .update_timer(Some(RESOLVE_TIMEOUT), None)
        .into_result()
        .map_err(|error| format!("arm PipeWire default-sink timeout: {error}"))?;
    mainloop.run();
    drop(timeout);
    drop(registry_listener);
    drop(metadata_handles);
    let result = sink
        .borrow_mut()
        .take()
        .ok_or_else(|| "PipeWire did not report a default audio sink".to_string());
    result
}

#[cfg(test)]
mod tests {
    use super::parse_sink;

    #[test]
    fn default_sink_metadata_requires_a_nonempty_name() {
        assert_eq!(
            parse_sink(Some(r#"{"name":"alsa_output.demo"}"#)),
            Some("alsa_output.demo".into())
        );
        assert_eq!(parse_sink(Some(r#"{"name":""}"#)), None);
        assert_eq!(parse_sink(Some("not-json")), None);
    }
}
