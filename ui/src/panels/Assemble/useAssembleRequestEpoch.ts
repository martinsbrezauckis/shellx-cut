import { useCallback, useEffect, useRef } from 'react'

export type AssembleProjectChange = 'unchanged' | 'foreign-project' | 'stale-plan' | 'apply-pending' | 'apply-published' | 'broll-pending' | 'broll-published' | 'awaiting-project-refresh'
export type AssembleApplyCompletion = 'accepted' | 'failed' | 'stale' | 'cancelled'
type InvalidatingProjectChange = 'foreign-project' | 'stale-plan'

type PendingApply = {
  token: number
  expectedRevision: string
  observedRevision?: string
}

type AcceptedApply = {
  previousRevision: string
  resultRevision: string
}

type PendingIdentityRequest = {
  token: number
}

/** Tracks identity-bound request lifetime and revision-bound reviewed plans. */
export class AssembleRequestEpoch {
  #identity: string
  #revision: string
  #version = 0
  #active = true
  #pendingApply: PendingApply | null = null
  #acceptedApply: AcceptedApply | null = null
  #pendingIdentityRequest: PendingIdentityRequest | null = null
  #completedIdentityRequest = false
  #pendingInvalidation: InvalidatingProjectChange | null = null

  constructor(identity: string, revision: string) {
    this.#identity = identity
    this.#revision = revision
  }

  reconcileProject(identity: string, revision: string): AssembleProjectChange {
    if (this.#identity !== identity) {
      this.#identity = identity
      this.#revision = revision
      this.#pendingApply = null
      this.#acceptedApply = null
      this.#pendingIdentityRequest = null
      this.#completedIdentityRequest = false
      this.#pendingInvalidation = 'foreign-project'
      this.#version += 1
      return 'foreign-project'
    }
    if (this.#completedIdentityRequest) {
      this.#revision = revision
      return 'broll-published'
    }
    if (this.#acceptedApply?.resultRevision === revision) {
      this.#revision = revision
      this.#acceptedApply = null
      return 'apply-published'
    }
    if (this.#revision === revision) return this.#pendingInvalidation ?? 'unchanged'
    if (this.#acceptedApply?.previousRevision === revision) return 'awaiting-project-refresh'
    const previousRevision = this.#revision
    this.#revision = revision
    if (this.#pendingIdentityRequest) {
      return 'broll-pending'
    }
    if (this.#pendingApply?.expectedRevision === previousRevision) {
      this.#pendingApply.observedRevision = revision
      return 'apply-pending'
    }
    this.#pendingApply = null
    this.#pendingInvalidation = 'stale-plan'
    this.#version += 1
    return 'stale-plan'
  }

  /** Commit an invalidation seen during render after its effect has run. */
  acknowledgeProjectChange(change: InvalidatingProjectChange) {
    if (this.#pendingInvalidation === change) this.#pendingInvalidation = null
  }

  start(): number {
    this.#pendingApply = null
    this.#pendingIdentityRequest = null
    this.#completedIdentityRequest = false
    return ++this.#version
  }

  startApply(expectedRevision: string): number | null {
    if (!this.#active || expectedRevision !== this.#revision) return null
    const token = ++this.#version
    this.#pendingApply = { token, expectedRevision }
    this.#pendingIdentityRequest = null
    this.#completedIdentityRequest = false
    return token
  }

  /** B-roll owns its checkpoint and child-operation revisions until its request completes. */
  startIdentityRequest(): number | null {
    if (!this.#active) return null
    const token = ++this.#version
    this.#pendingApply = null
    this.#acceptedApply = null
    this.#completedIdentityRequest = false
    this.#pendingIdentityRequest = { token }
    return token
  }

  completeIdentityRequest(token: number): boolean {
    const pending = this.#pendingIdentityRequest
    if (!pending || !this.current(token) || pending.token !== token) return false
    this.#pendingIdentityRequest = null
    this.#completedIdentityRequest = true
    return true
  }

  acceptApply(token: number, resultRevision: string): AssembleApplyCompletion {
    const pending = this.#pendingApply
    if (!pending || !this.current(token) || pending.token !== token) return 'cancelled'
    this.#pendingApply = null
    if (pending.observedRevision && pending.observedRevision !== resultRevision) {
      this.#version += 1
      return 'stale'
    }
    this.#acceptedApply = pending.observedRevision === resultRevision
      ? null
      : { previousRevision: pending.expectedRevision, resultRevision }
    this.#revision = resultRevision
    return 'accepted'
  }

  failApply(token: number): AssembleApplyCompletion {
    const pending = this.#pendingApply
    if (!pending || !this.current(token) || pending.token !== token) return 'cancelled'
    this.#pendingApply = null
    if (pending.observedRevision) {
      this.#version += 1
      return 'stale'
    }
    return 'failed'
  }

  invalidate() {
    this.#pendingApply = null
    this.#acceptedApply = null
    this.#pendingIdentityRequest = null
    this.#completedIdentityRequest = false
    this.#pendingInvalidation = null
    this.#version += 1
  }

  current(version: number): boolean {
    return this.#active && version === this.#version
  }

  activate() {
    this.#active = true
  }

  dispose() {
    this.#active = false
    this.#pendingApply = null
    this.#acceptedApply = null
    this.#pendingIdentityRequest = null
    this.#completedIdentityRequest = false
    this.#pendingInvalidation = null
    this.#version += 1
  }
}

/** Guards deferred work across project events, local edits, and unmount. */
export function useAssembleRequestEpoch(identity: string, revision: string) {
  const epoch = useRef<AssembleRequestEpoch | null>(null)
  if (!epoch.current) epoch.current = new AssembleRequestEpoch(identity, revision)
  const projectChange = epoch.current.reconcileProject(identity, revision)

  useEffect(() => {
    epoch.current?.activate()
    return () => epoch.current?.dispose()
  }, [])

  return {
    start: useCallback(() => epoch.current!.start(), []),
    startApply: useCallback((expectedRevision: string) => epoch.current!.startApply(expectedRevision), []),
    startIdentityRequest: useCallback(() => epoch.current!.startIdentityRequest(), []),
    completeIdentityRequest: useCallback((token: number) => epoch.current!.completeIdentityRequest(token), []),
    acceptApply: useCallback((token: number, resultRevision: string) => epoch.current!.acceptApply(token, resultRevision), []),
    failApply: useCallback((token: number) => epoch.current!.failApply(token), []),
    acknowledgeProjectChange: useCallback((change: InvalidatingProjectChange) => epoch.current!.acknowledgeProjectChange(change), []),
    current: useCallback((version: number) => epoch.current!.current(version), []),
    invalidate: useCallback(() => epoch.current!.invalidate(), []),
    projectChange,
  }
}
