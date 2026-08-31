use super::WindowsPausePilotRefusal;
use record_recovery::RecordingStream;

const WINDOWS_MONITOR_ID_PREFIX: &str = "shellx-monitor-v1:windows:";
const MONITOR_ID_DIGEST_LEN: usize = 64;

/// The one accepted native profile: exact screen plus independently owned
/// microphone/system audio when selected. Every other live-recording surface
/// is a refusal, not a setting that the pilot silently drops.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsPausePilotProfile {
    exact_monitor_id: String,
    fps: u32,
    capture_input: bool,
    microphone: bool,
    system_audio: bool,
}

impl WindowsPausePilotProfile {
    /// Admit only one exact native monitor and an integer encoder cadence.
    pub fn admit(request: WindowsPausePilotRequest) -> Result<Self, WindowsPausePilotRefusal> {
        request.validate()
    }

    pub fn exact_monitor_id(&self) -> &str {
        &self.exact_monitor_id
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    #[cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]
    pub(crate) fn captures_input(&self) -> bool {
        self.capture_input
    }

    #[cfg_attr(
        not(all(windows, feature = "capture-windows")),
        allow(
            dead_code,
            reason = "the Windows-only native pause worker is not compiled on this host"
        )
    )]
    pub(crate) fn selected_audio_streams(&self) -> Vec<RecordingStream> {
        let mut streams = Vec::with_capacity(2);
        if self.microphone {
            streams.push(RecordingStream::MicrophoneAudio);
        }
        if self.system_audio {
            streams.push(RecordingStream::SystemAudio);
        }
        streams
    }

    #[cfg(test)]
    pub(crate) fn test_profile_with_passive_input(exact_monitor_id: String, fps: f64) -> Self {
        let mut profile = Self::admit(WindowsPausePilotRequest::screen_video_only(
            exact_monitor_id,
            fps,
        ))
        .expect("screen-only test profile is admitted");
        profile.capture_input = true;
        profile
    }
}

/// Internal admission input for a future pause-ready start path. It is not a
/// public request schema and mirrors unsupported surfaces so none are silently
/// dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsPausePilotRequest {
    pub exact_monitor_id: Option<String>,
    pub fps: f64,
    pub audio: bool,
    pub microphone: bool,
    pub system_audio: bool,
    pub input: bool,
    pub window: bool,
    pub camera: bool,
    pub studio: bool,
    pub autoedit: bool,
    pub polish: bool,
}

impl WindowsPausePilotRequest {
    pub fn screen_video_only(exact_monitor_id: String, fps: f64) -> Self {
        Self {
            exact_monitor_id: Some(exact_monitor_id),
            fps,
            audio: false,
            microphone: false,
            system_audio: false,
            input: false,
            window: false,
            camera: false,
            studio: false,
            autoedit: false,
            polish: false,
        }
    }

    /// Private Windows-only admission for one exact screen plus selected
    /// independently finalized audio owners; server sidecars remain required.
    pub fn screen_with_audio(
        exact_monitor_id: String,
        fps: f64,
        microphone: bool,
        system_audio: bool,
    ) -> Self {
        Self {
            exact_monitor_id: Some(exact_monitor_id),
            fps,
            audio: false,
            microphone,
            system_audio,
            input: false,
            window: false,
            camera: false,
            studio: false,
            autoedit: false,
            polish: false,
        }
    }

    fn validate(self) -> Result<WindowsPausePilotProfile, WindowsPausePilotRefusal> {
        let exact_monitor_id = self
            .exact_monitor_id
            .filter(|id| valid_exact_monitor_id(id))
            .ok_or(WindowsPausePilotRefusal::ExactMonitorRequired)?;
        let unsupported = [
            (self.audio, WindowsPausePilotRefusal::Audio),
            (self.input, WindowsPausePilotRefusal::Input),
            (self.window, WindowsPausePilotRefusal::Window),
            (self.camera, WindowsPausePilotRefusal::Camera),
            (self.studio, WindowsPausePilotRefusal::Studio),
            (self.autoedit, WindowsPausePilotRefusal::Autoedit),
            (self.polish, WindowsPausePilotRefusal::Polish),
        ];
        if let Some((_, refusal)) = unsupported.into_iter().find(|(requested, _)| *requested) {
            return Err(refusal);
        }
        if !self.fps.is_finite() || !(1.0..=240.0).contains(&self.fps) || self.fps.fract() != 0.0 {
            return Err(WindowsPausePilotRefusal::IntegerFpsRequired);
        }
        Ok(WindowsPausePilotProfile {
            exact_monitor_id,
            fps: self.fps as u32,
            capture_input: false,
            microphone: self.microphone,
            system_audio: self.system_audio,
        })
    }
}

fn valid_exact_monitor_id(id: &str) -> bool {
    id.strip_prefix(WINDOWS_MONITOR_ID_PREFIX)
        .is_some_and(|digest| {
            digest.len() == MONITOR_ID_DIGEST_LEN
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}
