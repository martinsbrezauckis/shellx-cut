import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  STUDIO_BACKGROUND_PRESETS,
  backgroundLabel,
  defaultStudioState,
} from '../src/panels/Record/studioTypes'

const here = dirname(fileURLToPath(import.meta.url))
const read = (path: string) => readFileSync(resolve(here, path), 'utf8')
const record = read('../src/panels/Record/index.tsx')
const resultPanel = read('../src/panels/Record/RecordingResultPanel.tsx')
const session = read('../src/app/useRecordingSession.ts')
const controls = read('../src/panels/Record/StudioControls.tsx')
const preview = read('../src/panels/Record/StudioPreview.tsx')
const sourceControl = read('../src/panels/Record/RecordingSourceControl.tsx')
const studioCss = read('../src/panels/Record/record.css')
const app = read('../src/App.tsx')
const workspace = read('../src/app/AppWorkspace.tsx')
const recordingTopBar = read('../src/topbar/RecordingTopBar.tsx')
const modeTabs = read('../src/topbar/WorkspaceModeTabs.tsx')
const readiness = read('../src/panels/Record/RecordingReadinessSummary.tsx')
const exportHook = read('../src/panels/Record/useRecordingExport.ts')
const serverStudio = read('../../app/server/src/screen_record_studio.rs')

assert.deepEqual(
  STUDIO_BACKGROUND_PRESETS.map((preset) => preset.id),
  ['gradient', 'solid', 'blur_screen', 'none'],
  'the one preset catalog owns Studio background options in their editor order',
)
assert.equal(defaultStudioState().background, 'gradient', 'the default comes from the centralized preset catalog')
assert.equal(backgroundLabel('none'), 'None', 'None retains its distinct user-facing label')
assert.notEqual(
  STUDIO_BACKGROUND_PRESETS.find((preset) => preset.id === 'none')?.description,
  STUDIO_BACKGROUND_PRESETS.find((preset) => preset.id === 'solid')?.description,
  'None and Solid state different outcomes rather than duplicate labels',
)
assert.match(controls, /STUDIO_BACKGROUND_PRESETS\.map/, 'Studio controls render the centralized preset catalog')
assert.match(controls, /data-cut-studio-background-description/, 'Studio controls expose a stable preset-description selector')
assert.match(preview, /data-cut-studio-preset=\{preset\.id\}/, 'Studio preview exposes its selected preset for browser inspection')
assert.match(studioCss, /\.rec-studio-preview--gradient,[\s\S]*?\.rec-studio-preview--blur_screen \{ background: var\(--bg-deep/,
  'background choices never substitute a synthetic picture for native source pixels')
assert.match(serverStudio, /"none" => Ok\(record_core::Background::Transparent\)/,
  'None persists a transparent output plan instead of the Solid color')
assert.match(serverStudio, /if background == "none" \{[\s\S]*?plan\.frame\.enabled = false;/,
  'None disables the decorative frame so its transparent plan is full-bleed')
assert.match(session, /captureRef\.current = admitted\.captureId[\s\S]*?\{ source: 'background', kind: 'style', background: preset\.studio\.background \}[\s\S]*?appendInitialStudioEvent\(admitted\.captureId, event\)/,
  'Auto-edit journals the selected initial background only after Start supplies a capture identity')
assert.match(session, /if \(!preset\.raw && preset\.studio\)[\s\S]*?for \(const event of events\)[\s\S]*?publish\(\{ phase: 'recording'/,
  'the Start transition journals initial composition before publishing recording state')

assert.doesNotMatch(record, /data-cut-rec-output-path=/, 'ordinary output DOM attributes do not expose the chosen absolute path')
assert.doesNotMatch(record, /data-cut-rec-raw-path=/, 'raw completion DOM attributes do not expose the saved absolute path')
assert.doesNotMatch(record, /Raw recording saved → \{lastRaw\.path\}/, 'raw completion copy is path-light')
assert.doesNotMatch(record, /title=\{recordOutputPath/, 'ordinary output labels do not put the path in a browser tooltip')
assert.match(resultPanel, /\{rawSaved && \([\s\S]*?\{raw \? \([\s\S]*?data-cut-action="record-save-copy"[\s\S]*?data-cut-action="record-save-raw-copy"/,
  'copy actions appear only after a saved raw MP4 and distinguish raw from polished recordings')
assert.match(resultPanel, /data-cut-action="record-output-settings"[\s\S]*?Export folder settings/,
  'the saved result opens the shared default export folder setting')
assert.doesNotMatch(record, /data-cut-action="record-output-pick"/,
  'setup has no per-capture destination picker before Stop')
assert.match(record, /onSaveRawCopy=\{\(\) => \{ void rawCopy\.saveCopy\(\) \}\}/,
  'the result action delegates to the maintained raw-copy job')
assert.doesNotMatch(exportHook, /Saved \$\{job\.format\.toUpperCase\(\)\} →/, 'export completion copy does not echo the saved path')
assert.doesNotMatch(exportHook, /\$\{path\}\)/, 'export failure copy does not echo the chosen path')

assert.match(app, /layout\.workspaceMode === 'record'[\s\S]*<RecordingTopBar/,
  'Record replaces the editor toolbar with dedicated recording chrome')
assert.match(app, /hidden=\{layout\.workspaceMode !== 'edit'\}/,
  'the selected-clip Tools rail is absent outside Edit, including Record')
assert.match(workspace, /const recordTimelineDeferred = layout\.workspaceMode === 'record'/,
  'Record always owns the middle row instead of retaining a populated editor timeline')
assert.match(workspace, /commentsOpen && layout\.workspaceMode === 'edit'/,
  'review comments cannot obscure the Recording Studio workspace')
assert.match(recordingTopBar, /<WorkspaceModeTabs[\s\S]*mode="record"/,
  'focused recording chrome keeps the editor\'s shared mode navigation')
assert.match(modeTabs, /data-cut-mode="edit"[\s\S]*data-cut-action="record-back-edit"/,
  'the Record Edit mode keeps the registered explicit route back to Edit')
assert.doesNotMatch(recordingTopBar, /data-cut-recording-mode-active|className="[^"]*tb-recording-back(?:\s|")/,
  'recording chrome does not replace stable navigation with a passive mode pill or duplicate Back control')
assert.match(readiness, /data-cut-action="record-readiness-details-toggle"/,
  'the compact readiness summary keeps technical Doctor detail behind an addressable disclosure')
assert.match(preview, /'No source frame'[\s\S]*data-cut-rec-preview-unavailable/,
  'an unavailable source preview names the missing real frame without showing a fake capture')
assert.match(readiness, /'Choose what to share in the system picker when you start\.'/,
  'the portal-pending summary tells the user what to do without internal proof language')
assert.match(sourceControl, /Otherwise, choose what to share in the system picker when you start\./,
  'the empty source list gives the same compact system-picker instruction')
assert.match(readiness, /detail: 'Choose a source to continue\.'/,
  'the ready summary does not repeat the rehearsal and Record sequence')
assert.match(record, /<h1>Recording Studio<\/h1>[\s\S]*'Choose what to capture, check sound, then start\.'/,
  'the page header shows the approved Record title and one concise setup sequence')
assert.doesNotMatch(recordingTopBar, /data-cut-toolbar|data-cut-render-btn|data-cut-export-btn/,
  'recording chrome contains no inactive edit, render, or export toolbar')
assert.doesNotMatch(record, /<details className="rec__advanced"/,
  'Start, length & pause and quality no longer hide in a separate left-rail disclosure')
assert.match(record, /captureTimingControl=\{captureTimingControl\}[\s\S]*videoQualityControl=\{videoQualityControl\}/,
  'timing and quality are wired to the right settings pages')
assert.match(controls, /<RecordingSettingsTabs pages=\{pages\}/,
  'camera, background, timer, timing, and quality use one right-rail navigation')
assert.match(studioCss, /\.rec__body\s*\{[\s\S]*?grid-template-columns:\s*minmax\(290px, 330px\) minmax\(0, 1fr\) minmax\(290px, 330px\);[\s\S]*?"settings preview controls"/,
  'Record places setup, preview, and scene controls in the approved three-column workspace')
assert.match(studioCss, /\.rec-studio-controls\s*\{[\s\S]*?overflow:\s*visible/,
  'scene controls do not add a nested workspace scrollbar')
assert.match(record, /<RecordingReadinessSummary[\s\S]*?<div className="rec__body">[\s\S]*?<div className="rec__studio"[\s\S]*?<aside className="rec__settings"[\s\S]*?<\/div>\s*\n\s*\{\/\* Transport \/ HUD[\s\S]*?<div className="rec__transport"/,
  'the transport is a footer sibling of the body rather than a body grid row')
assert.match(studioCss, /data-cut-record-phase="recording"[\s\S]*\.rec__settings/,
  'active recording removes setup chrome so preview and live controls stay focused')

console.log('PASS Studio preset semantics and path-light Recorder output contract')
