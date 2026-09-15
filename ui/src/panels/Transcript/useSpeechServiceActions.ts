import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb, type JobRecord, type ProjectIdentity, type VerbResult } from '../../lib/client'
import type { DoctorCard } from '../../lib/doctor'
import { runUserVerb } from '../../lib/userActionFeedback'
import {
  sameSpeechProject,
  settleDiarization,
  speechServiceAvailability,
  startDiarization,
  startDubbing,
  type SpeechActionDispatcher,
  type SpeechRun,
} from './speechServiceModel'

export type SpeechActionPhase = 'idle' | 'running' | 'success' | 'error' | 'stale'

export interface SpeechActionState {
  phase: SpeechActionPhase
  message: string
}

export interface TranscriptSpeechServices {
  diarize: DoctorCard | null
  dub: DoctorCard | null
}

export interface UseSpeechServiceActionsOptions {
  asset: string | null
  projectIdentity: ProjectIdentity | null
  services: TranscriptSpeechServices
  onProjectChanged?: () => void
}

const idleState: SpeechActionState = { phase: 'idle', message: '' }

function errorMessage(result: VerbResult | null): string {
  return result?.error?.message?.trim() || 'Could not read the speaker-label job status.'
}

type DiarizationRun = Extract<SpeechRun, { action: 'diarize' }>
type TimerHandle = number

export interface DiarizationPollerOptions {
  readStatus: (jobId: string) => Promise<VerbResult<JobRecord>>
  currentIdentity: () => ProjectIdentity | null
  onState: (state: SpeechActionState) => void
  onSuccess: () => void
  schedule?: (callback: () => void, delayMs: number) => TimerHandle
  clearSchedule?: (timer: TimerHandle) => void
}

/** Owns one diarization status loop. A run is current only while its exact
 * object is active; every async boundary rechecks that identity before it can
 * update UI state or schedule another poll. */
export function createDiarizationPoller({
  readStatus,
  currentIdentity,
  onState,
  onSuccess,
  schedule = (callback, delayMs) => window.setTimeout(callback, delayMs),
  clearSchedule = (timer) => window.clearTimeout(timer),
}: DiarizationPollerOptions) {
  let activeRun: DiarizationRun | null = null
  let timer: TimerHandle | null = null
  let generation = 0

  const clearTimer = () => {
    if (timer !== null) clearSchedule(timer)
    timer = null
  }

  const isCurrent = (run: DiarizationRun, token: number) => (
    generation === token && activeRun === run
  )

  const scheduleCheck = (run: DiarizationRun, token: number, delayMs: number) => {
    clearTimer()
    timer = schedule(() => {
      timer = null
      void check(run, token)
    }, delayMs)
  }

  const check = async (run: DiarizationRun, token: number) => {
    if (!isCurrent(run, token)) return
    try {
      const response = await readStatus(run.jobId)
      if (!isCurrent(run, token)) return
      if (!response.ok || !response.result) {
        activeRun = null
        clearTimer()
        onState({ phase: 'error', message: errorMessage(response) })
        return
      }
      const completion = settleDiarization(run, response.result, currentIdentity())
      if (!isCurrent(run, token)) return
      if (completion.state === 'running') {
        onState({ phase: 'running', message: completion.message })
        if (isCurrent(run, token)) scheduleCheck(run, token, 1_200)
        return
      }
      activeRun = null
      clearTimer()
      onState({ phase: completion.state, message: completion.message })
      if (completion.state === 'success') onSuccess()
    } catch {
      // A short reconnect must not manufacture a terminal job failure. An
      // invalidated request must also not schedule a new timer.
      if (isCurrent(run, token)) scheduleCheck(run, token, 1_200)
    }
  }

  return {
    start(run: DiarizationRun, delayMs = 650) {
      generation += 1
      activeRun = run
      scheduleCheck(run, generation, delayMs)
    },
    cancel() {
      generation += 1
      activeRun = null
      clearTimer()
    },
  }
}

export function useSpeechServiceActions({
  asset,
  projectIdentity,
  services,
  onProjectChanged,
}: UseSpeechServiceActionsOptions) {
  const [state, setState] = useState<SpeechActionState>(idleState)
  const [targetLang, setTargetLang] = useState('lv')
  const request = useRef(0)
  const identityRef = useRef(projectIdentity)
  const changedRef = useRef(onProjectChanged)
  identityRef.current = projectIdentity
  changedRef.current = onProjectChanged
  const identityKey = projectIdentity
    ? projectIdentity.origin_path_sha256 + ':' + projectIdentity.project_name
    : ''

  const diarizationPoller = useRef<ReturnType<typeof createDiarizationPoller> | null>(null)
  if (diarizationPoller.current === null) {
    diarizationPoller.current = createDiarizationPoller({
      readStatus: (jobId) => callVerb('jobs.status', { job_id: jobId }),
      currentIdentity: () => identityRef.current,
      onState: setState,
      onSuccess: () => changedRef.current?.(),
    })
  }

  useEffect(() => {
    request.current += 1
    diarizationPoller.current?.cancel()
    setState(idleState)
  }, [identityKey])

  useEffect(() => () => {
    request.current += 1
    diarizationPoller.current?.cancel()
  }, [])

  const diarizeAvailability = useMemo(
    () => speechServiceAvailability(services.diarize, asset, projectIdentity, 'diarize'),
    [asset, projectIdentity, services.diarize],
  )
  const dubAvailability = useMemo(
    () => speechServiceAvailability(services.dub, asset, projectIdentity, 'dub'),
    [asset, projectIdentity, services.dub],
  )

  const dispatch = useCallback<SpeechActionDispatcher>(async (request) => {
    if (request.verb === 'media.diarize') {
      return runUserVerb('media.diarize', request.args, request.fallback)
    }
    return runUserVerb('audio.dub', request.args, request.fallback)
  }, [])

  const start = useCallback((action: 'diarize' | 'dub') => {
    const availability = action === 'diarize' ? diarizeAvailability : dubAvailability
    const identity = identityRef.current
    if (!availability.ready || !asset || !identity) {
      setState({ phase: 'error', message: availability.message })
      return
    }
    const token = ++request.current
    diarizationPoller.current?.cancel()
    setState({ phase: 'running', message: action === 'diarize' ? 'Starting speaker labels…' : 'Creating dubbed track…' })
    const begin = action === 'diarize'
      ? startDiarization(dispatch, asset, identity)
      : startDubbing(dispatch, asset, targetLang, identity)
    void begin.then((started) => {
      if (request.current !== token) return
      if (!sameSpeechProject(identity, identityRef.current)) {
        setState({
          phase: 'stale',
          message: action === 'diarize'
            ? 'Speaker-label result was discarded after the project changed.'
            : 'Dub result was discarded after the project changed.',
        })
        return
      }
      if (started.state === 'error') {
        setState({ phase: 'error', message: started.message })
        return
      }
      if (started.state === 'success') {
        setState({ phase: 'success', message: started.message })
        changedRef.current?.()
        return
      }
      setState({ phase: 'running', message: 'Labeling speakers…' })
      diarizationPoller.current?.start(started.run)
    }).catch(() => {
      if (request.current === token) setState({ phase: 'error', message: 'Could not start the selected speech service.' })
    })
  }, [asset, diarizeAvailability, dispatch, dubAvailability, targetLang])

  const openServiceSetup = useCallback(() => {
    document.dispatchEvent(new CustomEvent('cut:open-ui-surface', {
      detail: { id: 'settings-services-integrations' },
    }))
  }, [])

  return {
    state,
    targetLang,
    setTargetLang,
    diarizeAvailability,
    dubAvailability,
    startDiarize: () => start('diarize'),
    startDub: () => start('dub'),
    openServiceSetup,
  }
}
