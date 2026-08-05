import type { VerbResult } from '../../lib/client'
import { userVerbFailureMessage } from '../../lib/userActionFeedback'

export function timelineEditFailureMessage(result: VerbResult, fallback: string): string | null {
  return userVerbFailureMessage(result, fallback)
}
