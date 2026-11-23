import type { AvailabilityRule, AvailabilitySubject } from './types'

// Grade/transform/matte accept video-track media (core edit.rs); still, title
// and shape are video-track media too. An overlay is a track position, not a
// different content kind. GIF selects the same video-track media range.
const noPictureTarget = (_: unknown, subject: Readonly<AvailabilitySubject>) =>
  !!subject.selectedKinds?.length && !subject.selectedKinds.some(kind =>
    kind === 'video' || kind === 'still' || kind === 'title' || kind === 'shape')

// A missing selection, probe or target is unknown, not a definite exclusion.
export const editingRules: readonly AvailabilityRule[] = [
  { id: 'editing.speed', family: 'kind', sentence: 'Speed does not apply to captions.',
    absent: (_, s) => !!s.selectedKinds?.length && s.selectedKinds.every(kind => kind === 'caption') },
  { id: 'editing.picture-tools', family: 'kind', sentence: 'Picture tools need a clip with pictures.',
    absent: noPictureTarget },
  { id: 'editing.gif', family: 'kind', sentence: 'GIF needs a clip with pictures.',
    absent: noPictureTarget },
  { id: 'editing.detach-audio', family: 'kind', sentence: 'This source has no sound to detach.',
    absent: (_, s) => s.streams?.audio === false },
  { id: 'manual.toggle', family: 'system', sentence: 'The manual is already open in this editor.',
    absent: (_, s) => s.embeddedManual === true },
  { id: 'source.video-target', family: 'kind', sentence: 'This source has no picture to place on a video track.',
    absent: (_, s) => s.streams?.video === false },
  { id: 'source.audio-target', family: 'kind', sentence: 'This source has no sound to place on an audio track.',
    absent: (_, s) => s.streams?.audio === false },
  { id: 'export.project-settings', family: 'kind', sentence: 'These settings apply to exports at the project size.',
    absent: (_, s) => s.outputShape !== undefined && s.outputShape !== 'project' },
  { id: 'history.seek', family: 'kind', sentence: 'This edit has no position on the timeline.',
    absent: (_, s) => s.operation?.timelinePosition === null },
  { id: 'manual.editor-target', family: 'kind', sentence: 'This article has no control to show in the editor.',
    absent: (_, s) => s.article?.editorTarget === false && s.article.featureId !== 'cut.top.export.archive' },
  { id: 'manual.archive', family: 'system', sentence: 'Archive export is not available in this release.',
    absent: (_, s) => s.article?.featureId === 'cut.top.export.archive' },
]
