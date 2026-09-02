import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { recordingCaptureSafetyPresentation } from '../src/panels/Record/recordingCaptureSafetyModel'
import type { ScreenRecordStatusResult } from '../src/lib/clientResults'

type SourceLifecycle = ScreenRecordStatusResult['source_lifecycle']
type ControllerPlacement = ScreenRecordStatusResult['controller_placement']

const source = (state: SourceLifecycle['state']): SourceLifecycle => ({ state, reason: 'Observed by the native capture owner.' })
const placement = (state: ControllerPlacement['state']): ControllerPlacement => ({ state, reason: 'Observed by a native platform path.' })

assert.equal(
  recordingCaptureSafetyPresentation(source('active'), placement('excluded')).controllerTone,
  'safe',
  'only an observed native exclusion earns the safe controller treatment',
)
assert.equal(
  recordingCaptureSafetyPresentation(source('active'), placement('auto_hidden')).controllerTone,
  'safe',
  'an observed auto-hide may be presented as safe without claiming exclusion',
)
assert.equal(
  recordingCaptureSafetyPresentation(source('active'), placement('refused')).controllerTone,
  'warning',
  'a native refusal remains visibly distinct from a successful exclusion',
)
assert.equal(
  recordingCaptureSafetyPresentation(source('terminal'), placement('unavailable')).controllerTone,
  'muted',
  'unverified placement never becomes a success indication',
)
assert.equal(
  recordingCaptureSafetyPresentation(source('source_lost'), placement('unavailable')).sourceLabel,
  'Selected source closed',
  'only an observed selected-source close gets the source-loss label',
)
assert.equal(
  recordingCaptureSafetyPresentation(source('unavailable'), placement('unavailable')).sourceLabel,
  'Source-loss monitoring unavailable',
  'a backend without a close signal stays visibly unavailable rather than inferred',
)

const component = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/RecordingCaptureSafetyStatus.tsx', import.meta.url)),
  'utf8',
)
const hook = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/useRecordingAudioMeters.ts', import.meta.url)),
  'utf8',
)
const windows = readFileSync(
  fileURLToPath(new URL('../../app/recorder/record-capture/src/windows.rs', import.meta.url)),
  'utf8',
)
const windowsWgcHandler = readFileSync(
  fileURLToPath(new URL('../../app/recorder/record-capture/src/windows_wgc_handler.rs', import.meta.url)),
  'utf8',
)
const windowsPlacement = readFileSync(
  fileURLToPath(new URL('../../app/recorder/record-capture/src/windows_controller_exclusion.rs', import.meta.url)),
  'utf8',
)
const windowsShellPlacement = readFileSync(
  fileURLToPath(new URL('../../app/desktop/src-tauri/src/windows_controller_placement.rs', import.meta.url)),
  'utf8',
)
const macosTarget = readFileSync(
  fileURLToPath(new URL('../../app/recorder/record-capture/src/macos_capture_target.rs', import.meta.url)),
  'utf8',
)
const macosPicker = readFileSync(
  fileURLToPath(new URL('../../app/recorder/record-capture/src/macos.rs', import.meta.url)),
  'utf8',
)
const macosOwner = readFileSync(
  fileURLToPath(new URL('../../app/desktop/src-tauri/src/macos_controller_owner.rs', import.meta.url)),
  'utf8',
)
const linux = readFileSync(
  fileURLToPath(new URL('../../app/recorder/record-capture/src/linux.rs', import.meta.url)),
  'utf8',
)

assert.match(component, /data-cut-rec-source-lifecycle/, 'source lifecycle remains inspectable in the compact live HUD')
assert.match(component, /data-cut-rec-controller-placement=/, 'controller placement exposes only a bounded outcome')
assert.match(hook, /sourceLifecycle: status\.source_lifecycle/, 'one bounded status poll carries source lifecycle')
assert.match(hook, /controllerPlacement: status\.controller_placement/, 'one bounded status poll carries controller placement')
assert.match(windows, /windows_controller_exclusion::admit_controller_placement/, 'display capture consumes a controller outcome without changing the selected source')
assert.match(windowsWgcHandler, /fn on_closed[\s\S]*self\.readiness[\s\S]*mark_terminal\(\)[\s\S]*selected_source_closed[\s\S]*self\.stop\.store\(true, Ordering::Release\)/,
  'every Windows WGC close revokes readiness before ending the shared wait while only its lifecycle CAS can label source loss')
assert.match(windowsShellPlacement, /SetWindowDisplayAffinity[\s\S]*GetWindowDisplayAffinity[\s\S]*WDA_EXCLUDEFROMCAPTURE/, 'the owning Tauri shell requires affinity readback before reporting success')
assert.doesNotMatch(windowsPlacement, /SetWindowDisplayAffinity|GetWindowDisplayAffinity|HWND/, 'cutd never calls affinity APIs against the shell process window')
assert.doesNotMatch(windowsPlacement, /SHELLX_CUT_CONTROLLER_HWND/, 'cutd receives no raw controller handle')
assert.match(windowsPlacement, /SHELLX_CUT_CONTROLLER_PLACEMENT_OWNER/, 'cutd accepts only a redacted shell-owned placement conclusion')
assert.match(macosTarget, /content\.applications\(\)[\s\S]*with_excluding_applications\(&\[owner\], &\[\]\)/,
  'macOS excludes the admitted owning application so later controller windows remain excluded')
assert.match(macosTarget, /selected-window capture preserves its exact source semantics/, 'macOS never changes exact window capture into display exclusion')
assert.match(macosPicker, /StreamCallbacks::new\(\)\.on_error[\s\S]*callback_readiness[\s\S]*mark_terminal\(\)[\s\S]*source_stop\.store\(true, Ordering::Release\)/,
  'every macOS SCK error revokes readiness before ending the shared wait while typed source loss remains separately guarded')
assert.match(macosPicker, /admitted_controller_owner_pid/, 'macOS picker uses the same explicit owner admission as capture exclusion')
assert.doesNotMatch(macosPicker, /getppid/, 'macOS picker never hides an arbitrary adopted-engine parent')
assert.match(macosOwner, /env_remove\(ENV_CONTROLLER_OWNER\)/, 'the Tauri child spawn scrubs inherited macOS owner markers')
assert.match(linux, /does not provide a verifiable controller-exclusion or safe auto-hide state/, 'Linux portal stays unavailable when it cannot prove controller safety')

console.log('PASS recording capture lifecycle and controller-placement truth')
