import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { noTranscriptKind, noTranscriptSentence, transcriptActionFailure, transcriptEmptyState } from '../src/lib/noTranscript'

// "No transcript yet" is a normal state read from the refusal's structure
// (error.requirement), never from its wording.
const captionsMissing = { requirement: { id: 'captions_runtime', name: 'Captions' } }
const notTranscribed = { requirement: { id: 'transcript', name: 'Transcript' } }
assert.equal(noTranscriptKind(captionsMissing), 'captions_missing')
assert.equal(noTranscriptKind(notTranscribed), 'not_transcribed')
assert.equal(noTranscriptKind({ requirement: { id: 'ffmpeg' } }), null, 'another missing tool is not this')
assert.equal(noTranscriptKind({}), null, 'a real read error has no requirement')
assert.equal(noTranscriptKind(undefined), null)

// Each state says the true thing.
assert.match(noTranscriptSentence('captions_missing'), /^Captions and transcription are not installed\. Choose Install captions/)
assert.equal(noTranscriptSentence('not_transcribed'), 'No transcript yet. One is made after a clip is imported.')
assert.doesNotMatch(noTranscriptSentence('not_transcribed'), /media\./, 'a person cannot run a verb')

// What the pane shows while no asset has a transcript.
assert.equal(transcriptEmptyState({ captionsReady: null, transcribingNow: false }), 'checking', 'until the doctor answers')
assert.equal(transcriptEmptyState({ captionsReady: null, transcribingNow: true }), 'checking', 'never claims transcription before it knows')
assert.equal(transcriptEmptyState({ captionsReady: false, transcribingNow: false }), 'install')
assert.equal(transcriptEmptyState({ captionsReady: false, transcribingNow: true }), 'install', 'nothing can transcribe without captions')
assert.equal(transcriptEmptyState({ captionsReady: true, transcribingNow: true }), 'transcribing')
assert.equal(transcriptEmptyState({ captionsReady: true, transcribingNow: false }), 'empty')

// One line for a failed action: the real reason, or the engine's own message.
assert.equal(
  transcriptActionFailure('filler pass', { code: 'sidecar', message: 'Captions and transcription are not installed', cause: 'x', ...captionsMissing }),
  `filler pass: ${noTranscriptSentence('captions_missing')}`,
)
assert.equal(transcriptActionFailure('filler pass', { code: 'io', message: 'disk full', cause: 'x' }), 'filler pass: disk full')

// The panel wires it: the timeline read treats the refusal as a state, a real
// error still shows "Could not load timeline words", and the doctor-pending
// state no longer says Transcribing.
const panel = readFileSync(new URL('../src/panels/Transcript/index.tsx', import.meta.url), 'utf8')
assert.match(panel, /if \(!r\.ok && noTranscriptKind\(r\.error\)\) \{[\s\S]*?error: false/)
assert.match(panel, /Could not load timeline words\./)
assert.match(panel, /data-cut-transcript-checking/)
assert.match(panel, /data-cut-transcript-none/)
assert.match(panel, /emptyState === 'install'/)
assert.doesNotMatch(panel, /Engine ready \(or still probing\)/, 'probing no longer reads as transcribing')
// Other callers of the refused verbs read the structure too.
for (const file of ['Clips/index.tsx', 'Kinetic/index.tsx', 'Review/QC.tsx', 'Timeline/TimelineGlobalTools.tsx']) {
  assert.match(readFileSync(new URL(`../src/panels/${file}`, import.meta.url), 'utf8'), /noTranscriptKind/, file)
}
console.log('PASS transcript empty states')
