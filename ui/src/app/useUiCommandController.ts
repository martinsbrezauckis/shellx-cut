import { useEffect, type Dispatch, type MutableRefObject, type SetStateAction } from 'react'
import type { Project } from '../lib/client'
import { events, type CutEvent, type UiCommandResult, type UiCommandVerb } from '../lib/events'
import { VERB_BEHAVIOR, VERB_NAMES } from '../lib/generatedVerbBehavior'
import type { HighlightSpec } from '../HighlightOverlay'
import type { UiObservableState } from './uiControlState'
import { uiSurface } from './uiSurfaceRegistry'

interface UiCommandControllerArgs {
  stateRef: MutableRefObject<UiObservableState>
  project: Project | null
  setPlayheadMs: Dispatch<SetStateAction<number>>
  setSelectedClipIds: Dispatch<SetStateAction<string[]>>
  setHighlight: Dispatch<SetStateAction<HighlightSpec | null>>
  highlightNonce: MutableRefObject<number>
  openSurface: (id: string) => boolean
}

type UiCommand = Extract<CutEvent, { type: 'ui_command' }> & { request_id: number }
type UiCommandError = NonNullable<UiCommandResult['error']>

const equalStrings = (left: string[], right: string[]) =>
  left.length === right.length && left.every((value, index) => value === right[index])

const waitForCommittedState = async (
  stateRef: MutableRefObject<UiObservableState>,
  previousRevision: number,
  predicate: (state: UiObservableState) => boolean,
  timeoutMs = 1_500,
): Promise<UiObservableState | null> => {
  const deadline = performance.now() + timeoutMs
  while (performance.now() < deadline) {
    await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()))
    const state = stateRef.current
    if (state.state_revision > previousRevision && predicate(state)) return state
  }
  return null
}

function highlightSelector(args: Record<string, unknown>): string | null {
  if (typeof args.selector === 'string' && args.selector.trim()) return args.selector
  if (typeof args.clip === 'string' && args.clip.trim()) return `[data-cut-clip="${CSS.escape(args.clip)}"]`
  if (typeof args.panel === 'string' && args.panel.trim()) return `[data-cut-panel="${CSS.escape(args.panel)}"]`
  return null
}

function queryTarget(selector: string): HTMLElement | null {
  try {
    return document.querySelector<HTMLElement>(selector)
  } catch {
    return null
  }
}

/** Applies relayed view commands and answers only after a later state revision
 * proves the requested outcome. Unknown, unavailable, and no-op requests are
 * explicit applied:false results. */
export function useUiCommandController({
  stateRef,
  project,
  setPlayheadMs,
  setSelectedClipIds,
  setHighlight,
  highlightNonce,
  openSurface,
}: UiCommandControllerArgs) {
  useEffect(() => {
    const answer = (
      command: UiCommand,
      applied: boolean,
      requested: Record<string, unknown>,
      state: UiObservableState,
      extra: Pick<UiCommandResult, 'surface' | 'selector' | 'error'> = {},
    ) => {
      events.answerUiCommand({
        request_id: command.request_id,
        verb: command.verb,
        applied,
        requested,
        state,
        ...extra,
      })
    }
    const reject = (
      command: UiCommand,
      requested: Record<string, unknown>,
      error: UiCommandError,
      extra: Pick<UiCommandResult, 'surface' | 'selector'> = {},
    ) => answer(command, false, requested, stateRef.current, { ...extra, error })

    const handle = async (command: UiCommand) => {
      const before = stateRef.current
      switch (command.verb) {
        case 'ui.playhead': {
          const raw = command.args.at_ms
          if (!Number.isSafeInteger(raw) || Number(raw) < 0) {
            reject(command, {}, { code: 'invalid_args', message: 'at_ms must be a non-negative safe integer' })
            return
          }
          const atMs = Number(raw)
          const requested = { at_ms: atMs }
          if (before.playhead_ms === atMs) {
            reject(command, requested, { code: 'conflict', message: `playhead is already at ${atMs} ms` })
            return
          }
          setPlayheadMs(atMs)
          const state = await waitForCommittedState(
            stateRef,
            before.state_revision,
            (current) => current.playhead_ms === atMs,
          )
          if (state) answer(command, true, requested, state)
          else reject(command, requested, { code: 'conflict', message: 'playhead state did not commit before the confirmation deadline' })
          return
        }
        case 'ui.select': {
          const raw = command.args.clip_ids
          if (!Array.isArray(raw) || raw.some((id) => typeof id !== 'string')) {
            reject(command, {}, { code: 'invalid_args', message: 'clip_ids must be an array of strings' })
            return
          }
          const ids = [...raw] as string[]
          const requested = { clip_ids: ids }
          const available = new Set(
            project?.tracks.flatMap((track) =>
              track.clips.flatMap((clip) => ('id' in clip ? [clip.id] : [])),
            ) ?? [],
          )
          const missing = ids.find((id) => !available.has(id))
          if (missing) {
            reject(command, requested, { code: 'not_found', message: `clip '${missing}' is not present in the active sequence` })
            return
          }
          if (equalStrings(before.selected_clip_ids, ids)) {
            reject(command, requested, { code: 'conflict', message: 'the requested clips are already selected' })
            return
          }
          setSelectedClipIds(ids)
          const state = await waitForCommittedState(
            stateRef,
            before.state_revision,
            (current) => equalStrings(current.selected_clip_ids, ids),
          )
          if (state) answer(command, true, requested, state)
          else reject(command, requested, { code: 'conflict', message: 'selection state did not commit before the confirmation deadline' })
          return
        }
        case 'ui.open': {
          const panel = typeof command.args.panel === 'string' ? command.args.panel : ''
          const requested = { panel }
          const definition = uiSurface(panel)
          if (!definition?.action) {
            reject(command, requested, { code: 'not_found', message: `surface '${panel || '(empty)'}' is not agent-openable` }, { surface: panel || undefined })
            return
          }
          if (definition.action.kind !== 'focus' && before.open_surface_ids.includes(panel)) {
            reject(command, requested, { code: 'conflict', message: `surface '${panel}' is already open` }, { surface: panel, selector: definition.selector })
            return
          }
          if (!openSurface(panel)) {
            reject(command, requested, { code: 'not_found', message: `surface '${panel}' has no available opener` }, { surface: panel, selector: definition.selector })
            return
          }
          const state = await waitForCommittedState(
            stateRef,
            before.state_revision,
            (current) => {
              if (!current.open_surface_ids.includes(panel)) return false
              const target = queryTarget(definition.selector)
              if (!target) return false
              if (definition.action?.kind !== 'focus') return true
              return document.activeElement === target || target.contains(document.activeElement)
            },
          )
          if (state) answer(command, true, requested, state, { surface: panel, selector: definition.selector })
          else reject(command, requested, { code: 'conflict', message: `surface '${panel}' did not become observable before the confirmation deadline` }, { surface: panel, selector: definition.selector })
          return
        }
        case 'ui.highlight': {
          const clear = command.args.clear === true
          const selector = highlightSelector(command.args)
          const requested = Object.fromEntries(
            Object.entries(command.args).filter(([, value]) => value !== undefined),
          )
          if (clear || !selector) {
            if (!before.overlays.highlight) {
              reject(command, requested, { code: 'conflict', message: 'there is no active highlight to clear' })
              return
            }
            setHighlight(null)
            const state = await waitForCommittedState(
              stateRef,
              before.state_revision,
              (current) => current.overlays.highlight === null,
            )
            if (state) answer(command, true, requested, state)
            else reject(command, requested, { code: 'conflict', message: 'highlight did not clear before the confirmation deadline' })
            return
          }
          if (!queryTarget(selector)) {
            reject(command, requested, { code: 'not_found', message: `highlight target '${selector}' is not visible` }, { selector })
            return
          }
          highlightNonce.current += 1
          setHighlight({
            selector: typeof command.args.selector === 'string' ? command.args.selector : undefined,
            clip: typeof command.args.clip === 'string' ? command.args.clip : undefined,
            panel: typeof command.args.panel === 'string' ? command.args.panel : undefined,
            label: typeof command.args.label === 'string' ? command.args.label : undefined,
            description: typeof command.args.description === 'string' ? command.args.description : undefined,
            duration_ms: typeof command.args.duration_ms === 'number' ? command.args.duration_ms : undefined,
            scroll: typeof command.args.scroll === 'boolean' ? command.args.scroll : undefined,
            n: highlightNonce.current,
          })
          const state = await waitForCommittedState(
            stateRef,
            before.state_revision,
            (current) => current.overlays.highlight !== null && document.querySelector('[data-cut-highlight]') !== null,
          )
          if (state) answer(command, true, requested, state, { selector })
          else reject(command, requested, { code: 'conflict', message: 'highlight did not become observable before the confirmation deadline' }, { selector })
        }
      }
    }

    return events.subscribe((event) => {
      if (event.type !== 'ui_command' || typeof event.request_id !== 'number') return
      void handle(event as UiCommand)
    })
  }, [
    highlightNonce,
    openSurface,
    project,
    setHighlight,
    setPlayheadMs,
    setSelectedClipIds,
    stateRef,
  ])
}

export const UI_COMMAND_VERBS = VERB_NAMES.filter((verb) =>
  VERB_BEHAVIOR[verb].facets.includes('ui_command'),
) as readonly UiCommandVerb[]
