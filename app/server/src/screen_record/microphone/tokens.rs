use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
struct Issued {
    reference: record_capture::MicrophoneEndpointRef,
    expires_at: Instant,
}

fn issued() -> &'static Mutex<HashMap<String, Issued>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, Issued>>> = OnceLock::new();
    TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn issue(reference: &record_capture::MicrophoneEndpointRef) -> Option<String> {
    let mut random = [0_u8; 24];
    getrandom::fill(&mut random).ok()?;
    let token = format!("mic_cap_{}", hex::encode(random));
    let mut entries = issued()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    entries.retain(|_, issued| issued.expires_at > Instant::now());
    entries.insert(
        token.clone(),
        Issued {
            reference: reference.clone(),
            expires_at: Instant::now() + TTL,
        },
    );
    Some(token)
}

pub(super) fn consume(token: &str) -> Option<record_capture::MicrophoneEndpointRef> {
    let mut entries = issued()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    entries.retain(|_, issued| issued.expires_at > Instant::now());
    entries.remove(token).map(|issued| issued.reference)
}
