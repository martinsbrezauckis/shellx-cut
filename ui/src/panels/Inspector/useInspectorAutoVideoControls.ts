import { useMemo, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { layoutTrack } from '../Timeline/layout'
import { ADJ_LOOKS } from './model'

export interface InspectorAutoVideoSelection {
  clip: {
    id: string
  }
  trackId: string
}

interface InspectorAutoVideoControlsArgs {
  project: Project | null
  sel: InspectorAutoVideoSelection | null
}

export function useInspectorAutoVideoControls({ project, sel }: InspectorAutoVideoControlsArgs) {
  const [matchRef, setMatchRef] = useState('')
  const [zoomIntensity, setZoomIntensity] = useState(0.12)
  const [adjLook, setAdjLook] = useState('vignette')
  const [autoNote, setAutoNote] = useState<string | null>(null)
  const [autoBusy, setAutoBusy] = useState<'' | 'balance' | 'match' | 'zoom' | 'adjust'>('')

  const clearAutoNoteSoon = () => {
    setTimeout(() => setAutoNote(null), 5000)
  }

  const refCandidates = useMemo(() => {
    if (!project || !sel) return [] as { id: string }[]
    const out: { id: string }[] = []
    for (const t of project.tracks ?? []) {
      if (t.kind !== 'video') continue
      for (const c of t.clips ?? []) {
        if ('asset' in c && c.id !== sel.clip.id) out.push({ id: c.id })
      }
    }
    return out
  }, [project, sel])

  const autoBalance = async () => {
    if (!sel || autoBusy) return
    setAutoBusy('balance')
    setAutoNote(null)
    const r = await callVerb('edit.auto_balance', {
      clip: sel.clip.id,
      strength: 1.0,
      rationale: 'inspector: auto balance',
    })
    setAutoBusy('')
    if (r.ok) {
      setAutoNote('Auto-balanced')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setAutoNote(r.error?.message ?? r.error?.code ?? 'could not auto-balance')
    }
    clearAutoNoteSoon()
  }

  const colorMatch = async () => {
    if (!sel || !matchRef || autoBusy) return
    setAutoBusy('match')
    setAutoNote(null)
    const r = await callVerb('edit.color_match', {
      clip: sel.clip.id,
      reference: matchRef,
      strength: 1.0,
      rationale: `inspector: match colour to ${matchRef}`,
    })
    setAutoBusy('')
    if (r.ok) {
      setAutoNote(`Matched colour to ${matchRef}`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setAutoNote(r.error?.message ?? r.error?.code ?? 'could not match colour')
    }
    clearAutoNoteSoon()
  }

  const autoZoom = async () => {
    if (!sel || autoBusy) return
    setAutoBusy('zoom')
    setAutoNote(null)
    const r = await callVerb('edit.auto_zoom', {
      clip: sel.clip.id,
      intensity: zoomIntensity,
      rationale: `inspector: auto zoom ${zoomIntensity}`,
    })
    setAutoBusy('')
    if (r.ok) {
      const n = (r.result as { count?: number } | undefined)?.count
      setAutoNote(typeof n === 'number' ? (n > 0 ? `Added ${n} punch-in${n === 1 ? '' : 's'}` : 'No clear emphasis beats - no zoom added') : 'Auto-zoom applied')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setAutoNote(r.error?.message ?? r.error?.code ?? 'could not auto-zoom')
    }
    clearAutoNoteSoon()
  }

  const addAdjustment = async () => {
    if (!sel || !project || autoBusy) return
    const track = (project.tracks ?? []).find((t) => t.id === sel.trackId)
    if (!track) {
      setAutoNote('could not resolve the clip track')
      return
    }
    const item = layoutTrack(track).find((i) => i.id === sel.clip.id)
    if (!item) {
      setAutoNote('could not resolve the clip span')
      return
    }
    const range_ms: [number, number] = [Math.round(item.startMs), Math.round(item.startMs + item.durMs)]
    const preset = ADJ_LOOKS.find((l) => l.key === adjLook) ?? ADJ_LOOKS[0]
    setAutoBusy('adjust')
    setAutoNote(null)
    const r = await callVerb('edit.adjustment', {
      range_ms,
      ...preset.look,
      rationale: `inspector: adjustment layer (${adjLook})`,
    })
    setAutoBusy('')
    if (r.ok) {
      setAutoNote(`Added "${preset.label}" adjustment over this clip`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setAutoNote(r.error?.message ?? r.error?.code ?? 'could not add adjustment')
    }
    clearAutoNoteSoon()
  }

  return {
    matchRef,
    zoomIntensity,
    adjLook,
    autoNote,
    autoBusy,
    refCandidates,
    setMatchRef,
    setZoomIntensity,
    setAdjLook,
    autoBalance,
    colorMatch,
    autoZoom,
    addAdjustment,
  }
}
