// Small request-state controller for Find media.
//
// Search and import calls can settle after a user changes source. Tokens make
// those late responses inert, while invalidation immediately returns the visible
// controls to an idle state instead of leaving Search or Import disabled.

export interface StockRequestState {
  searchEpoch: number
  fetchEpoch: number
  searching: boolean
  fetchingId: string | null
}

export const initialStockRequestState: StockRequestState = {
  searchEpoch: 0,
  fetchEpoch: 0,
  searching: false,
  fetchingId: null,
}

export function invalidateStockRequests(state: StockRequestState): StockRequestState {
  return {
    searchEpoch: state.searchEpoch + 1,
    fetchEpoch: state.fetchEpoch + 1,
    searching: false,
    fetchingId: null,
  }
}

export function beginStockSearch(state: StockRequestState): StockRequestState {
  return {
    searchEpoch: state.searchEpoch + 1,
    fetchEpoch: state.fetchEpoch + 1,
    searching: true,
    fetchingId: null,
  }
}

export function finishStockSearch(state: StockRequestState, epoch: number): StockRequestState {
  return epoch === state.searchEpoch ? { ...state, searching: false } : state
}

/**
 * Admit one import at a time. This is deliberately synchronous because two
 * different result buttons can be activated before React has rendered the
 * first disabled state; callers must not dispatch the rejected second fetch.
 */
export function beginStockFetch(state: StockRequestState, id: string): StockRequestState | null {
  if (state.fetchingId !== null) return null
  return { ...state, fetchEpoch: state.fetchEpoch + 1, fetchingId: id }
}

export function finishStockFetch(state: StockRequestState, epoch: number): StockRequestState {
  return epoch === state.fetchEpoch ? { ...state, fetchingId: null } : state
}

export function isCurrentStockSearch(state: StockRequestState, epoch: number): boolean {
  return epoch === state.searchEpoch
}

export function isCurrentStockFetch(state: StockRequestState, epoch: number): boolean {
  return epoch === state.fetchEpoch
}
