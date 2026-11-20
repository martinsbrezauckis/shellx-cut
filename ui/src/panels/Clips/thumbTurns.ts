// The engine renders at most two uncached preview frames at a time and answers
// the rest with "busy" (HTTP 429). A list of suggestions asks for one frame per
// card at once, so every card past the second would lose its picture. Cards
// take turns instead: each asks for its frame only after the previous card's
// frame has loaded, failed, or given up.

export interface ThumbTurn {
  /** Resolves when it is this card's turn to request its frame. */
  ready: Promise<void>
  /** Hand the turn to the next card. Safe to call more than once. */
  release: () => void
}

let tail: Promise<void> = Promise.resolve()

export function takeThumbTurn(timeoutMs = 8000): ThumbTurn {
  let release!: () => void
  const mine = new Promise<void>((resolve) => { release = resolve })
  const ready = tail
  tail = ready.then(() => mine)
  const timer = setTimeout(release, timeoutMs)
  return { ready, release: () => { clearTimeout(timer); release() } }
}
