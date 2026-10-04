import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { DoctorCard } from '../../lib/doctor'

type ProbeState = 'probing' | 'error' | 'absent' | 'ready'

/** Own this drawer's Doctor reads, including late replies after a project switch. */
export function useMatteDoctor(projectOrigin: string | null) {
  const [readState, setReadState] = useState<ProbeState>('probing')
  const [readPremiumCard, setPremiumCard] = useState<DoctorCard | null>(null)
  const [docHint, setDocHint] = useState<string | null>(null)
  const [resultOwner, setResultOwner] = useState<{ origin: string | null; generation: number } | null>(null)
  const mountedRef = useRef(false)
  const originRef = useRef(projectOrigin)
  const generationRef = useRef(0)
  originRef.current = projectOrigin

  const currentScope = useCallback((origin: string | null) => (
    mountedRef.current && originRef.current === origin
  ), [])

  const probe = useCallback(async (refresh = false) => {
    const origin = originRef.current
    const generation = ++generationRef.current
    if (mountedRef.current) setReadState('probing')
    const current = () => currentScope(origin) && generationRef.current === generation
    try {
      const reply = await callVerb('system.doctor', refresh ? { refresh: true } : {})
      if (!current()) return
      if (!reply.ok) {
        setPremiumCard(null)
        setResultOwner({ origin, generation })
        setReadState('error')
        return
      }
      const cards = (reply.result as { cards?: DoctorCard[] } | undefined)?.cards ?? []
      const matte = cards.find((card) => card.id === 'matte')
      const premium = cards.find((card) => card.id === 'matte_premium')
      if (!matte) {
        setPremiumCard(null)
        setResultOwner({ origin, generation })
        setReadState('error')
        return
      }
      setPremiumCard(premium ?? null)
      setDocHint(matte.hint ?? null)
      setResultOwner({ origin, generation })
      setReadState(matte.status === 'ok' ? 'ready' : 'absent')
    } catch {
      if (!current()) return
      setPremiumCard(null)
      setResultOwner({ origin, generation })
      setReadState('error')
    }
  }, [currentScope])

  useEffect(() => {
    mountedRef.current = true
    void probe()
    return () => {
      mountedRef.current = false
      generationRef.current += 1
    }
  }, [projectOrigin, probe])

  const resultCurrent = resultOwner !== null && resultOwner.origin === projectOrigin
    && resultOwner.generation === generationRef.current
  return {
    probeState: resultCurrent ? readState : 'probing',
    premiumReady: resultCurrent && readPremiumCard?.status === 'ok',
    premiumCard: resultCurrent ? readPremiumCard : null,
    docHint: resultCurrent ? docHint : null,
    probe,
    currentScope,
  }
}
