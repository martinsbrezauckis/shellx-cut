import assert from 'node:assert/strict'
import type { Project } from '../src/lib/client'
import {
  createCommentChatTimelineTarget,
  createTimelineChatTarget,
  rebaseChatTimelineTarget,
} from '../src/lib/chatTimelineTarget'

const identity = {
  schema: 'shellx-cut/project-identity/1',
  origin_path_sha256: `sha256:${'a'.repeat(64)}`,
  project_name: 'timeline-target-fixture',
}

function project(revision = 'op_000004'): Project {
  return {
    schema: 'shellx-cut/project/1',
    name: identity.project_name,
    project_identity: identity,
    project_revision: revision,
    settings: { width: 1920, height: 1080, fps: 30 },
    assets: {},
    tracks: [{
      id: 'v1',
      kind: 'video',
      clips: [{
        kind: 'media',
        id: 'c1',
        asset: 'a1',
        src_in_ms: 0,
        src_out_ms: 10_000,
        speed: 1,
      }],
    }],
    comments: [{
      id: 'cm1',
      at_ms: 1_000,
      end_ms: 2_000,
      text: 'Tighten this section',
      author: 'reviewer',
      status: 'open',
      anchor: { track_id: 'v1', clip_id: 'c1', offset_ms: 1_000 },
    }],
  } as Project
}

function projectWithOuterRangeGaps(revision = 'op_000004', leadGapMs = 2_000): Project {
  const value = project(revision)
  value.tracks[0]!.clips = [
    { kind: 'gap', duration_ms: leadGapMs },
    { kind: 'media', id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: 2_000, speed: 1 },
    { kind: 'gap', duration_ms: 2_000 },
    { kind: 'media', id: 'c2', asset: 'a2', src_in_ms: 0, src_out_ms: 2_000, speed: 1 },
    { kind: 'gap', duration_ms: 2_000 },
  ]
  return value
}

const fixture = project()
const range = createTimelineChatTarget({
  project: fixture,
  selectedClipIds: [],
  selectedRange: [1_200, 3_400],
  positionMs: 2_000,
})
assert.deepEqual(range && {
  kind: range.kind,
  range: range.range_ms,
  clips: range.clips.map((clip) => [clip.track_id, clip.clip_id, clip.target_range_ms]),
}, {
  kind: 'range',
  range: [1_200, 3_400],
  clips: [['v1', 'c1', [1_200, 3_400]]],
}, 'a selected timeline range snapshots the exact intersected span and clip identity')

const selection = createTimelineChatTarget({
  project: fixture,
  selectedClipIds: ['c1'],
  selectedRange: null,
  positionMs: 2_000,
})
assert.deepEqual(selection && {
  kind: selection.kind,
  range: selection.range_ms,
  position: selection.position_ms,
}, {
  kind: 'selection',
  range: [0, 10_000],
  position: undefined,
}, 'a clip selection snapshots the selected clip rather than the current playhead')

const position = createTimelineChatTarget({
  project: fixture,
  selectedClipIds: [],
  selectedRange: null,
  positionMs: 2_222,
})
assert.deepEqual(position && {
  kind: position.kind,
  range: position.range_ms,
  position: position.position_ms,
}, {
  kind: 'position',
  range: [2_222, 2_222],
  position: 2_222,
}, 'an unselected playhead uses the explicit position variant with an equal range')
assert.match(position?.label ?? '', /0:02\.222/, 'timeline target labels retain millisecond precision')

const movedProject = project('op_000005')
movedProject.tracks[0]!.clips.unshift({
  kind: 'media',
  id: 'lead-in',
  asset: 'a0',
  src_in_ms: 0,
  src_out_ms: 3_000,
  speed: 1,
})
const movedRange = range && rebaseChatTimelineTarget(movedProject, range)
assert.deepEqual(movedRange?.clips[0], {
  track_id: 'v1',
  clip_id: 'c1',
  timeline_range_ms: [3_000, 13_000],
  target_range_ms: [4_200, 6_400],
}, 'Replace moves a range with its retained clip-relative span and refreshes the full clip span')
assert.equal(movedRange?.project_revision, 'op_000005')

const outerGapProject = projectWithOuterRangeGaps()
const outerGapRange = createTimelineChatTarget({
  project: outerGapProject,
  selectedClipIds: [],
  selectedRange: [0, 10_000],
  positionMs: 3_000,
})
assert.deepEqual(
  outerGapRange && rebaseChatTimelineTarget(outerGapProject, outerGapRange),
  outerGapRange,
  'Send preserves an unchanged selected range byte-for-byte, including its outer empty timeline space',
)
const movedOuterGapRange = outerGapRange && rebaseChatTimelineTarget(
  projectWithOuterRangeGaps('op_000005', 5_000),
  outerGapRange,
)
assert.deepEqual(movedOuterGapRange && {
  range: movedOuterGapRange.range_ms,
  clips: movedOuterGapRange.clips.map((clip) => [clip.clip_id, clip.target_range_ms]),
}, {
  range: [3_000, 13_000],
  clips: [['c1', [5_000, 7_000]], ['c2', [9_000, 11_000]]],
}, 'replacement retains requested outer range offsets and both intersected clips after they move together')

const movedPosition = position && rebaseChatTimelineTarget(movedProject, position)
assert.deepEqual(movedPosition && {
  range: movedPosition.range_ms,
  position: movedPosition.position_ms,
}, { range: [5_222, 5_222], position: 5_222 }, 'Replace moves a retained playhead position with its exact clip-relative offset')

const extendedProject = project('op_000005')
extendedProject.tracks[0]!.clips[0]!.src_out_ms = 12_000
const extendedSelection = selection && rebaseChatTimelineTarget(extendedProject, selection)
assert.deepEqual(extendedSelection?.clips[0]?.target_range_ms, [0, 12_000], 'a selected clip resolves to its current full span after an extension')

const trimmedProject = project('op_000005')
trimmedProject.tracks[0]!.clips[0]!.src_out_ms = 2_000
assert.equal(
  range && rebaseChatTimelineTarget(trimmedProject, range),
  null,
  'Replace refuses a clip trimmed shorter than its retained exact target span',
)

const comment = fixture.comments[0]!
const commentTarget = createCommentChatTimelineTarget({
  project: fixture,
  comment,
  selectedClipIds: [],
  selectedRange: null,
})
assert.equal(commentTarget?.kind, 'comment', 'a Comment shortcut preserves its own semantic anchor')
assert.equal(commentTarget?.comment_id, 'cm1')

const rangedCommentProject = project()
rangedCommentProject.tracks[0]!.clips.push({
  kind: 'media',
  id: 'c2',
  asset: 'a2',
  src_in_ms: 0,
  src_out_ms: 5_000,
  speed: 1,
})
rangedCommentProject.comments[0] = {
  ...comment,
  at_ms: 9_000,
  end_ms: 11_000,
  anchor: { track_id: 'v1', clip_id: 'c1', offset_ms: 9_000 },
}
const rangedCommentTarget = createCommentChatTimelineTarget({
  project: rangedCommentProject,
  comment: rangedCommentProject.comments[0]!,
  selectedClipIds: [],
  selectedRange: null,
})
assert.deepEqual(
  rangedCommentTarget?.clips.map((clip) => [clip.clip_id, clip.target_range_ms]),
  [['c1', [9_000, 10_000]], ['c2', [10_000, 11_000]]],
  'a ranged comment retains every clip intersecting its exact requested span',
)

const commentOnRange = createCommentChatTimelineTarget({
  project: fixture,
  comment,
  selectedClipIds: [],
  selectedRange: [1_200, 2_400],
})
assert.equal(commentOnRange?.kind, 'range', 'an explicit range remains the target while retaining the comment link')
assert.equal(commentOnRange?.comment_id, 'cm1')

const afterStepBack = project('op_000006')
const rebased = commentTarget && rebaseChatTimelineTarget(afterStepBack, commentTarget)
assert.equal(rebased?.project_revision, 'op_000006', 'Replace rebinds a retained Comment target to the revision after Step back')
assert.equal(rebased?.clips[0]?.clip_id, 'c1', 'Replace retains the anchored clip identity instead of consulting a later UI selection')

const deletedAnchor = project('op_000006')
deletedAnchor.tracks[0]!.clips = []
assert.equal(
  commentTarget && rebaseChatTimelineTarget(deletedAnchor, commentTarget),
  null,
  'Replace refuses a deleted anchored clip instead of silently falling back to its saved timestamp',
)

const otherProject = project('op_000006')
otherProject.project_identity = { ...identity, project_name: 'other-project' }
assert.equal(
  commentTarget && rebaseChatTimelineTarget(otherProject, commentTarget),
  null,
  'Replace refuses a target from another immutable project identity even when revisions coincide',
)

const deletedLinkedComment = project('op_000006')
deletedLinkedComment.comments = []
assert.equal(
  commentOnRange && rebaseChatTimelineTarget(deletedLinkedComment, commentOnRange),
  null,
  'Replace refuses a selected-range request whose retained comment was deleted',
)

console.log('PASS Agent Chat timeline target snapshots and replacement rebasing')
