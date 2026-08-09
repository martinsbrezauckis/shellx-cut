// lib/ops — schema-derived op classification shared by Review + Timeline undo.
// Unknown names remain conservative so a stale UI artifact cannot hide a
// timeline edit from undo/rebase.

import { VERB_BEHAVIOR } from './generatedVerbBehavior'

/** True when an op changes tracks, markers, or caption styles and is therefore
 * undoable / rebasable. */
export function mutatesTimeline(verb: string): boolean {
  const behavior = VERB_BEHAVIOR[verb]
  return behavior === undefined || behavior.mutation_class === 'timeline'
}
