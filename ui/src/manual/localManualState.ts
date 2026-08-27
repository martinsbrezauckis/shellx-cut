import { resolveManualFeatureId } from './content'

export interface LocalManualState {
  open: boolean
  requestedFeatureId?: string
  requestId: number
}

export const INITIAL_LOCAL_MANUAL_STATE: LocalManualState = {
  open: false,
  requestId: 0,
}

/**
 * Opens the bundled manual and records a new article request. This is UI state
 * only: selecting documentation must never reveal, open, or execute Cut UI.
 */
export function openLocalManualArticle(
  state: LocalManualState,
  featureId?: string,
): LocalManualState {
  const requestedFeatureId = featureId
    ? resolveManualFeatureId(featureId) ?? state.requestedFeatureId
    : state.requestedFeatureId
  return {
    open: true,
    requestedFeatureId,
    requestId: state.requestId + 1,
  }
}

export function closeLocalManual(state: LocalManualState): LocalManualState {
  return { ...state, open: false }
}

export function toggleLocalManual(state: LocalManualState): LocalManualState {
  return state.open ? closeLocalManual(state) : openLocalManualArticle(state)
}
