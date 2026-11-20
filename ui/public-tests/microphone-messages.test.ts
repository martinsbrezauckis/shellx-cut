import assert from 'node:assert/strict'
import test from 'node:test'
import { microphoneProblemText, voiceoverFailureText } from '../src/app/microphoneMessages'

// The engine words the Windows block; the controls show it unchanged and in order.
const desktopAppsBlocked = {
  reason: 'windows_desktop_apps_microphone_off',
  message: 'Windows is blocking microphone access for desktop apps',
  suggested_action: 'Turn on Settings > Privacy & security > Microphone > Let desktop apps access your microphone, then try again',
}

test('a Windows microphone block reads as what is missing and where to turn it on', () => {
  const text = microphoneProblemText(desktopAppsBlocked)
  assert.equal(
    text,
    'Windows is blocking microphone access for desktop apps. Turn on Settings > Privacy & security > Microphone > Let desktop apps access your microphone, then try again',
  )
})

test('no problem and an empty problem say nothing', () => {
  assert.equal(microphoneProblemText(null), null)
  assert.equal(microphoneProblemText(undefined), null)
  assert.equal(microphoneProblemText({ reason: 'x' }), null)
})

test('a failed take names its reason instead of a generic failure', () => {
  const shown = voiceoverFailureText({ ...desktopAppsBlocked, detail: 'mic config: Access is denied. (0x80070005)' })
  assert.match(shown, /^Voiceover failed\. Windows is blocking microphone access for desktop apps/)
  assert.match(shown, /Let desktop apps access your microphone/)
  assert.doesNotMatch(shown, /0x80070005/, 'the system text stays in technical detail')
})

test('a failed take without a reason keeps the recovery note', () => {
  assert.equal(voiceoverFailureText(undefined), 'Voiceover failed; its private recovery state is retained.')
})

test('no microphone device and a busy device are told apart', () => {
  const none = microphoneProblemText({ reason: 'no_microphone_device', message: 'No microphone was found', suggested_action: 'Connect a microphone, then try again' })
  const busy = microphoneProblemText({ reason: 'microphone_in_use', message: 'The microphone is in use by another recording or app', suggested_action: 'Close the app that uses it, then try again' })
  assert.match(none ?? '', /^No microphone was found\./)
  assert.match(busy ?? '', /^The microphone is in use/)
})
