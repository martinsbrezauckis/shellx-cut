import type { Comment, Project, ProjectIdentity } from "./clientModel";
import { resolveCommentTime, timelineClipSpans } from "./commentAnchors";
export const CHAT_TIMELINE_TARGET_SCHEMA =
  "shellx-cut/chat-timeline-target/1" as const;
export type TargetKind = "range" | "selection" | "comment" | "position";
export interface ChatTimelineTargetClip {
  track_id: string;
  clip_id: string;
  /** The clip's current full timeline span when this request was prepared. */
  timeline_range_ms: [number, number];
  /** The exact part of this clip that defined the user's request context. */
  target_range_ms: [number, number];
}
/** Immutable timeline context attached to one Agent Chat request. The server
 * treats this as data, verifies its project identity/revision/spans before it
 * launches a provider, and returns the same snapshot in the turn receipt. */
export interface ChatTimelineTarget {
  schema: typeof CHAT_TIMELINE_TARGET_SCHEMA;
  kind: TargetKind;
  project_identity: ProjectIdentity;
  project_revision: string;
  /** Present only for a request launched from a persisted review comment. */
  comment_id?: string;
  /** Exact timeline span. A `comment` can retain its saved point or range;
   * `position_ms` always names its start. A `position` target is an equal point. */
  range_ms: [number, number];
  position_ms?: number;
  clips: ChatTimelineTargetClip[];
  /** Human-readable, client-only context label. The server never treats it as authority. */
  label: string;
}
interface TimelineTargetArgs {
  project: Project | null;
  comment: Comment;
  selectedClipIds: string[];
  selectedRange: [number, number] | null;
}
const finiteRange = (range: readonly number[]): range is [number, number] =>
  range.length === 2 &&
  range.every((value) => Number.isFinite(value)) &&
  range[0] >= 0 &&
  range[1] >= range[0];
const roundedRange = (range: readonly number[]): [number, number] => [
  Math.round(range[0]),
  Math.round(range[1]),
];
const timecode = (ms: number): string => {
  const normalized = Math.max(0, Math.round(ms));
  const seconds = Math.floor(normalized / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}.${String(normalized % 1000).padStart(3, "0")}`;
};
function targetClip(
  span: ReturnType<typeof timelineClipSpans>[number],
  range: [number, number],
): ChatTimelineTargetClip {
  return {
    track_id: span.trackId,
    clip_id: span.clipId,
    timeline_range_ms: [span.startMs, span.endMs],
    target_range_ms: range,
  };
}
function intersectingClips(
  project: Project,
  range: [number, number],
): ChatTimelineTargetClip[] {
  return timelineClipSpans(project)
    .filter((span) => span.startMs < range[1] && span.endMs > range[0])
    .map((span) =>
      targetClip(span, [
        Math.max(range[0], span.startMs),
        Math.min(range[1], span.endMs),
      ]),
    );
}
/** Snapshot an editor selection/range/playhead without consulting a comment. */
export function createTimelineChatTarget({
  project,
  selectedClipIds,
  selectedRange,
  positionMs,
}: Omit<TimelineTargetArgs, "comment"> & {
  positionMs: number;
}): ChatTimelineTarget | null {
  const identity = project?.project_identity;
  const revision = project?.project_revision;
  if (!project || !identity || !revision || !Number.isFinite(positionMs))
    return null;
  if (
    selectedRange &&
    finiteRange(selectedRange) &&
    selectedRange[1] > selectedRange[0]
  ) {
    const range = roundedRange(selectedRange);
    const clips = intersectingClips(project, range);
    return {
      schema: CHAT_TIMELINE_TARGET_SCHEMA,
      kind: "range",
      project_identity: identity,
      project_revision: revision,
      range_ms: range,
      clips,
      label: `Selected range ${timecode(range[0])}–${timecode(range[1])}${clips.length ? ` · ${clips.length} clip${clips.length === 1 ? "" : "s"}` : ""}`,
    };
  }
  const selected = new Set(selectedClipIds);
  const spans = timelineClipSpans(project).filter((span) =>
    selected.has(span.clipId),
  );
  if (spans.length) {
    const clips = spans.map((span) =>
      targetClip(span, [span.startMs, span.endMs]),
    );
    const range: [number, number] = [
      Math.min(...clips.map((clip) => clip.target_range_ms[0])),
      Math.max(...clips.map((clip) => clip.target_range_ms[1])),
    ];
    return {
      schema: CHAT_TIMELINE_TARGET_SCHEMA,
      kind: "selection",
      project_identity: identity,
      project_revision: revision,
      range_ms: range,
      clips,
      label: `Selected ${clips.length} clip${clips.length === 1 ? "" : "s"} · ${timecode(range[0])}–${timecode(range[1])}`,
    };
  }
  const position = Math.max(0, Math.round(positionMs));
  const clips = timelineClipSpans(project)
    .filter((span) => span.startMs <= position && span.endMs >= position)
    .map((span) => targetClip(span, [position, position]));
  return {
    schema: CHAT_TIMELINE_TARGET_SCHEMA,
    kind: "position",
    project_identity: identity,
    project_revision: revision,
    range_ms: [position, position],
    position_ms: position,
    clips,
    label: `Playhead at ${timecode(position)}`,
  };
}

/**
 * Snapshot the exact context the editor shows at the moment the user asks the
 * agent. A marked timeline range wins; then explicit selected clips; otherwise
 * the comment's current anchored (or saved stale) position is used.
 */
export function createCommentChatTimelineTarget({
  project,
  comment,
  selectedClipIds,
  selectedRange,
}: TimelineTargetArgs): ChatTimelineTarget | null {
  const identity = project?.project_identity;
  const revision = project?.project_revision;
  if (!project || !identity || !revision) return null;

  if (
    selectedRange &&
    finiteRange(selectedRange) &&
    selectedRange[1] > selectedRange[0]
  ) {
    const range = roundedRange(selectedRange);
    const clips = intersectingClips(project, range);
    return {
      schema: CHAT_TIMELINE_TARGET_SCHEMA,
      kind: "range",
      project_identity: identity,
      project_revision: revision,
      comment_id: comment.id,
      range_ms: range,
      clips,
      label: `Selected range ${timecode(range[0])}–${timecode(range[1])}${clips.length ? ` · ${clips.length} clip${clips.length === 1 ? "" : "s"}` : ""}`,
    };
  }

  const selected = new Set(selectedClipIds);
  const spans = timelineClipSpans(project).filter((span) =>
    selected.has(span.clipId),
  );
  if (spans.length) {
    const clips = spans.map((span) =>
      targetClip(span, [span.startMs, span.endMs]),
    );
    const range: [number, number] = [
      Math.min(...clips.map((clip) => clip.target_range_ms[0])),
      Math.max(...clips.map((clip) => clip.target_range_ms[1])),
    ];
    return {
      schema: CHAT_TIMELINE_TARGET_SCHEMA,
      kind: "selection",
      project_identity: identity,
      project_revision: revision,
      comment_id: comment.id,
      range_ms: range,
      clips,
      label: `Selected ${clips.length} clip${clips.length === 1 ? "" : "s"} · ${timecode(range[0])}–${timecode(range[1])}`,
    };
  }

  const resolved = resolveCommentTime(project, comment);
  const start = Math.max(0, Math.round(resolved.atMs));
  const end = Math.max(start, Math.round(resolved.endMs ?? resolved.atMs));
  const clips = end > start
    ? intersectingClips(project, [start, end])
    : timelineClipSpans(project)
      .filter((span) => span.startMs <= start && span.endMs >= start)
      .map((span) => targetClip(span, [start, start]));
  return {
    schema: CHAT_TIMELINE_TARGET_SCHEMA,
    kind: "comment",
    project_identity: identity,
    project_revision: revision,
    comment_id: comment.id,
    range_ms: [start, end],
    position_ms: start,
    clips,
    label:
      end > start
        ? `Comment range ${timecode(start)}–${timecode(end)}`
        : `Comment at ${timecode(start)}`,
  };
}

/** Rebind a stored request to the current revision without consulting the
 * current selection. It succeeds only when the same project and exact target
 * identities still exist and cover the original target ranges. */
export function rebaseChatTimelineTarget(
  project: Project | null,
  target: ChatTimelineTarget,
): ChatTimelineTarget | null {
  if (!project?.project_identity || !project.project_revision) return null;
  if (
    project.project_identity.origin_path_sha256 !==
      target.project_identity.origin_path_sha256 ||
    project.project_identity.project_name !==
      target.project_identity.project_name
  )
    return null;
  // Sending immediately after the user selected a range must preserve that
  // exact snapshot, including deliberate empty timeline space around clips.
  if (project.project_revision === target.project_revision) {
    return { ...target, project_identity: project.project_identity };
  }

  const live = timelineClipSpans(project);
  const targetClipsRemain = (allowMovedAnchors: boolean): boolean => target.clips.every((requested) => {
    const current = live.find(
      (span) =>
        span.trackId === requested.track_id &&
        span.clipId === requested.clip_id,
    );
    return Boolean(
      current &&
      (allowMovedAnchors || (
        current.startMs <= requested.target_range_ms[0] &&
        current.endMs >= requested.target_range_ms[1]
      )),
    );
  });

  if (target.comment_id && !project.comments?.some((comment) => comment.id === target.comment_id)) {
    return null;
  }

  if (target.kind === "comment") {
    const comment = target.comment_id
      ? project.comments?.find(
          (candidate) => candidate.id === target.comment_id,
        )
      : null;
    // A comment target follows its original semantic anchor across the exact
    // Step back revision. It must never fall back to a stale saved time when
    // that anchored clip was deleted or replaced.
    if (!comment || !targetClipsRemain(true)) return null;
    const rebased = createCommentChatTimelineTarget({
      project,
      comment,
      selectedClipIds: [],
      selectedRange: null,
    });
    if (!rebased || !target.clips.every((requested) => rebased.clips.some((current) =>
      current.track_id === requested.track_id && current.clip_id === requested.clip_id,
    ))) return null;
    return rebased;
  }

  if (!targetClipsRemain(true)) return null;

  const rebasedClips = target.clips.map((requested) => {
    const current = live.find((span) =>
      span.trackId === requested.track_id && span.clipId === requested.clip_id,
    );
    if (!current) return null;
    if (target.kind === "selection") {
      return targetClip(current, [current.startMs, current.endMs]);
    }
    const startOffset = requested.target_range_ms[0] - requested.timeline_range_ms[0];
    const endOffset = requested.target_range_ms[1] - requested.timeline_range_ms[0];
    if (
      startOffset < 0 || endOffset < startOffset ||
      current.startMs + endOffset > current.endMs
    ) return null;
    return targetClip(current, [
      current.startMs + startOffset,
      current.startMs + endOffset,
    ]);
  });
  if (rebasedClips.some((clip) => clip === null)) return null;
  const clips = rebasedClips as ChatTimelineTargetClip[];
  const first = clips.length
    ? Math.min(...clips.map((clip) => clip.target_range_ms[0]))
    : target.range_ms[0];
  const last = clips.length
    ? Math.max(...clips.map((clip) => clip.target_range_ms[1]))
    : target.range_ms[1];
  const originalFirst = target.clips.length
    ? Math.min(...target.clips.map((clip) => clip.target_range_ms[0]))
    : target.range_ms[0];
  const originalLast = target.clips.length
    ? Math.max(...target.clips.map((clip) => clip.target_range_ms[1]))
    : target.range_ms[1];
  const range: [number, number] = target.kind === "range"
    ? [
      Math.max(0, first - Math.max(0, originalFirst - target.range_ms[0])),
      last + Math.max(0, target.range_ms[1] - originalLast),
    ]
    : [first, last];
  if (target.kind === "position" && (
    clips.some((clip) => clip.target_range_ms[0] !== clip.target_range_ms[1]) ||
    new Set(clips.map((clip) => clip.target_range_ms[0])).size > 1
  )) return null;
  const position = target.kind === "position" ? range[0] : undefined;
  const label = target.kind === "range"
    ? `Selected range ${timecode(range[0])}–${timecode(range[1])}${clips.length ? ` · ${clips.length} clip${clips.length === 1 ? "" : "s"}` : ""}`
    : target.kind === "selection"
      ? `Selected ${clips.length} clip${clips.length === 1 ? "" : "s"} · ${timecode(range[0])}–${timecode(range[1])}`
      : `Playhead at ${timecode(position ?? range[0])}`;
  return {
    ...target,
    project_identity: project.project_identity,
    project_revision: project.project_revision,
    range_ms: range,
    ...(position == null ? { position_ms: undefined } : { position_ms: position }),
    clips,
    label,
  };
}
