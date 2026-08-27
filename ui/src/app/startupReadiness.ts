// startupReadiness.ts — one-shot signal from the committed Cut app root.

/**
 * Return a React ref callback that reports only a real committed Cut app root.
 * The callback is intentionally synchronous and local: the actual WebSocket
 * write is best-effort, so first paint never waits on server or network work.
 */
export function createCutAppRootMountReporter(reportMounted: () => void) {
  let reported = false
  return (node: HTMLDivElement | null): void => {
    if (!node || reported || !node.hasAttribute('data-cut-app-root')) return
    reported = true
    reportMounted()
  }
}
