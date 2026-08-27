/** Whether this page explicitly selected Cut's deterministic offline runtime. */
export function isMockRuntime(search = window.location.search): boolean {
  return new URLSearchParams(search).get('mock') === '1'
}

/** Load the mock adapter without putting it on the normal editor startup path. */
export async function loadMockRuntime(): Promise<void> {
  await import('./panels/Review/mock')
}
