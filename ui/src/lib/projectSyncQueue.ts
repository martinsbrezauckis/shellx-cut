// Bounded scheduling primitives for project revision synchronization.

/** One coalesced project.state pull. `generation` is the caller's project
 * identity guard, while `targetRevision` lets a completed pull prove that it
 * already covered a burst's newest event without issuing an empty follow-up. */
export interface RevisionSyncRequest {
  generation: number
  forceSnapshot: boolean
  advertisedPrevious?: string
  targetRevision?: string
}

export interface RevisionSyncOutcome<T> {
  value: T
  generation: number
  projectRevision?: string | null
}

export interface RevisionSyncMetrics {
  pullsStarted: number
  requestsCoalesced: number
  pendingPullsSkippedAsCurrent: number
}

/** Safety reconciliation waits for a quiet period after enough real deltas;
 * a high-rate stream also has a deliberately large hard cap. This keeps a
 * 10k sustained agent run to a few snapshots rather than one per 32 ops. */
export const PROJECT_SYNC_IDLE_RECONCILE_MIN_DELTAS = 32
export const PROJECT_SYNC_IDLE_RECONCILE_MS = 2_000
export const PROJECT_SYNC_MAX_DELTAS_BEFORE_RECONCILE = 4_096

export function projectReconciliationDelay(deltaApplications: number): number | null {
  if (deltaApplications < PROJECT_SYNC_IDLE_RECONCILE_MIN_DELTAS) return null
  return deltaApplications >= PROJECT_SYNC_MAX_DELTAS_BEFORE_RECONCILE
    ? 0
    : PROJECT_SYNC_IDLE_RECONCILE_MS
}

/** Serialize revision pulls as one in-flight request plus at most one pending
 * request. A WebSocket burst therefore becomes one pull when its latest
 * revision is already covered, or one bounded follow-up when it is not. */
export class ProjectSyncCoalescer<T> {
  private inFlight: Promise<T> | null = null
  private pending: RevisionSyncRequest | null = null
  private readonly counters: RevisionSyncMetrics = {
    pullsStarted: 0,
    requestsCoalesced: 0,
    pendingPullsSkippedAsCurrent: 0,
  }

  constructor(
    private readonly pull: (request: RevisionSyncRequest) => Promise<RevisionSyncOutcome<T>>,
    private readonly alreadyCurrent: (request: RevisionSyncRequest, outcome: RevisionSyncOutcome<T>) => boolean,
  ) {}

  request(request: RevisionSyncRequest): Promise<T> {
    if (this.inFlight) {
      this.pending = this.pending ? mergeRevisionSyncRequests(this.pending, request) : request
      this.counters.requestsCoalesced += 1
      return this.inFlight
    }
    const task = this.drain(request)
    this.inFlight = task
    void task.then(
      () => this.release(task),
      () => this.release(task),
    )
    return task
  }

  metrics(): RevisionSyncMetrics {
    return { ...this.counters }
  }

  private async drain(first: RevisionSyncRequest): Promise<T> {
    let request = first
    for (;;) {
      let outcome: RevisionSyncOutcome<T>
      try {
        this.counters.pullsStarted += 1
        outcome = await this.pull(request)
      } catch (error) {
        const pending = this.pending
        this.pending = null
        if (pending) {
          request = pending
          continue
        }
        throw error
      }
      const pending = this.pending
      this.pending = null
      if (!pending) return outcome.value
      if (this.alreadyCurrent(pending, outcome)) {
        this.counters.pendingPullsSkippedAsCurrent += 1
        return outcome.value
      }
      request = pending
    }
  }

  private release(task: Promise<T>): void {
    if (this.inFlight !== task) return
    this.inFlight = null
    // An event cannot normally land between drain observing no pending work and
    // this microtask, but preserve the one-pending invariant even under a
    // re-entrant caller.
    const pending = this.pending
    this.pending = null
    if (pending) void this.request(pending)
  }
}

function mergeRevisionSyncRequests(current: RevisionSyncRequest, next: RevisionSyncRequest): RevisionSyncRequest {
  // A project switch starts a new identity generation. Never combine its
  // forced snapshot with an old project's advertised revision.
  if (current.generation !== next.generation) return next
  return {
    generation: next.generation,
    forceSnapshot: current.forceSnapshot || next.forceSnapshot,
    advertisedPrevious: next.advertisedPrevious ?? current.advertisedPrevious,
    targetRevision: next.targetRevision ?? current.targetRevision,
  }
}
