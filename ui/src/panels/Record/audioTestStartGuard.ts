// The microphone test and the system-audio test hold the same native capture
// resources a recording needs. Every start route (the Start button, the F9
// shortcut, the global hotkey and the countdown hand-off) goes through the
// recorder's start guard, so this refusal lives there and gives the person the
// same visible message the other setup refusals use.
export const AUDIO_TEST_RUNNING_START_ERROR = 'Finish the audio test before recording.'

export function audioTestStartError(tests: { audioProbeRunning: boolean; micTestRunning: boolean }): string | null {
  return tests.audioProbeRunning || tests.micTestRunning ? AUDIO_TEST_RUNNING_START_ERROR : null
}
