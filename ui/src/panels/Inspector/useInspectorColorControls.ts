import { useCallback, useEffect, useState } from 'react'
import { callVerb, type ClipGrade, type ClipGradeWindow, type ColorSpace, type Project } from '../../lib/client'
import {
  GRADE_STACK_LAYERS,
  WINDOW_LOOKS,
  WINDOW_REGIONS,
  colorSpaceFromInput,
  toGradeLayer,
} from './model'

export interface InspectorColorSelection {
  clip: {
    id: string
    grade?: ClipGrade | null
    grade_stack?: ClipGrade[]
    grade_windows?: ClipGradeWindow[]
    input_color_space?: ColorSpace
  }
}

interface InspectorColorControlsArgs {
  project: Project | null
  sel: InspectorColorSelection | null
}

export function useInspectorColorControls({ project, sel }: InspectorColorControlsArgs) {
  const [colorBusy, setColorBusy] = useState(false)
  const [colorNote, setColorNote] = useState<string | null>(null)
  const [presets, setPresets] = useState<{ name: string }[]>([])
  const [selPreset, setSelPreset] = useState('')
  const [saveName, setSaveName] = useState('')
  const [stackLayer, setStackLayer] = useState('contrast')
  const [winRegion, setWinRegion] = useState('center')
  const [winLook, setWinLook] = useState('brighten')

  const clearColorNoteSoon = () => {
    setTimeout(() => setColorNote(null), 5000)
  }

  const setProjectSpace = async (field: 'working' | 'output', value: ColorSpace) => {
    setColorBusy(true)
    setColorNote(null)
    const base = field === 'working' ? { working: value } : { output: value }
    const r = await callVerb('project.color', { ...base, rationale: `inspector: ${field} space -> ${value}` })
    setColorBusy(false)
    if (r.ok) {
      setColorNote(`${field === 'working' ? 'Working' : 'Output'} space -> ${value}`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not set color space')
    }
    clearColorNoteSoon()
  }

  const setClipInputSpace = async (value: string) => {
    if (!sel) return
    const input = value ? colorSpaceFromInput(value) : null
    if (value && !input) {
      setColorNote('Unknown input color space')
      clearColorNoteSoon()
      return
    }
    setColorBusy(true)
    setColorNote(null)
    const args = input ? { clip: sel.clip.id, input } : { clip: sel.clip.id }
    const r = await callVerb('edit.color_space', { ...args, rationale: `inspector: input space -> ${value || 'clear'}` })
    setColorBusy(false)
    if (r.ok) {
      setColorNote(value ? `Input space -> ${value}` : 'Input space tag cleared')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not set input space')
    }
    clearColorNoteSoon()
  }

  const reloadPresets = useCallback(async () => {
    const r = await callVerb('grade.list', {})
    if (!r.ok) return
    const ps = ((r.result as { presets?: { name: string }[] } | undefined)?.presets ?? []).map((p) => ({ name: p.name }))
    setPresets(ps)
    setSelPreset((cur) => (cur && ps.some((p) => p.name === cur) ? cur : ps[0]?.name ?? ''))
  }, [])

  useEffect(() => {
    void reloadPresets()
  }, [reloadPresets, project?.name])

  const saveLook = async () => {
    if (!sel || colorBusy) return
    const name = saveName.trim() || `look ${presets.length + 1}`
    setColorBusy(true)
    setColorNote(null)
    const r = await callVerb('grade.save', { clip: sel.clip.id, name, rationale: `inspector: save look "${name}"` })
    setColorBusy(false)
    if (r.ok) {
      setSaveName('')
      setSelPreset(name)
      setColorNote(`Saved look "${name}"`)
      await reloadPresets()
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not save look')
    }
    clearColorNoteSoon()
  }

  const applyLook = async () => {
    if (!sel || !selPreset || colorBusy) return
    setColorBusy(true)
    setColorNote(null)
    const r = await callVerb('grade.apply', { clip: sel.clip.id, name: selPreset, rationale: `inspector: apply look "${selPreset}"` })
    setColorBusy(false)
    if (r.ok) {
      setColorNote(`Applied "${selPreset}"`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not apply look')
    }
    clearColorNoteSoon()
  }

  const stackBase = (): (ClipGrade | Partial<ClipGrade>)[] => {
    if (!sel) return []
    const st = sel.clip.grade_stack ?? []
    if (st.length) return st
    return sel.clip.grade ? [sel.clip.grade] : []
  }

  const addStackLayer = async () => {
    if (!sel || colorBusy) return
    const preset = GRADE_STACK_LAYERS.find((l) => l.key === stackLayer) ?? GRADE_STACK_LAYERS[0]
    const grades = [...stackBase(), preset.grade].map(toGradeLayer)
    setColorBusy(true)
    setColorNote(null)
    const r = await callVerb('edit.grade_stack', { clip: sel.clip.id, grades, rationale: `inspector: add grade layer (${preset.label})` })
    setColorBusy(false)
    if (r.ok) {
      setColorNote(`Added grade layer "${preset.label}" (${grades.length} total)`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not add grade layer')
    }
    clearColorNoteSoon()
  }

  const removeStackLayer = async (idx: number) => {
    if (!sel || colorBusy) return
    const grades = stackBase().filter((_, i) => i !== idx).map(toGradeLayer)
    setColorBusy(true)
    setColorNote(null)
    const r = await callVerb('edit.grade_stack', { clip: sel.clip.id, grades, rationale: `inspector: remove grade layer ${idx + 1}` })
    setColorBusy(false)
    if (r.ok) {
      setColorNote(grades.length ? `Removed layer (${grades.length} left)` : 'Cleared grade stack')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not remove grade layer')
    }
    clearColorNoteSoon()
  }

  const clipWindows: ClipGradeWindow[] = sel?.clip.grade_windows ?? []

  const addWindow = async () => {
    if (!sel || colorBusy) return
    const region = WINDOW_REGIONS.find((r) => r.key === winRegion) ?? WINDOW_REGIONS[0]
    const look = WINDOW_LOOKS.find((l) => l.key === winLook) ?? WINDOW_LOOKS[0]
    setColorBusy(true)
    setColorNote(null)
    const r = await callVerb('edit.grade_window', {
      clip: sel.clip.id,
      shape: region.shape,
      points: region.points,
      feather: 0.08,
      ...toGradeLayer(look.grade),
      rationale: `inspector: power window ${region.label} / ${look.label}`,
    })
    setColorBusy(false)
    if (r.ok) {
      setColorNote(`Added "${region.label}" window (${look.label})`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not add window')
    }
    clearColorNoteSoon()
  }

  const removeWindow = async (idx: number) => {
    if (!sel || colorBusy) return
    const remaining = clipWindows.filter((_, i) => i !== idx)
    setColorBusy(true)
    setColorNote(null)
    const r = await callVerb('edit.grade_window', {
      clip: sel.clip.id,
      remove_index: idx,
      rationale: `inspector: remove power window ${idx + 1}`,
    })
    setColorBusy(false)
    if (r.ok) {
      setColorNote(remaining.length ? `Removed window (${remaining.length} left)` : 'Cleared windows')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not remove window')
    }
    clearColorNoteSoon()
  }

  const clearWindows = async () => {
    if (!sel || colorBusy) return
    setColorBusy(true)
    setColorNote(null)
    const r = await callVerb('edit.grade_window', { clip: sel.clip.id, enabled: false, rationale: 'inspector: clear all power windows' })
    setColorBusy(false)
    if (r.ok) {
      setColorNote('Cleared all windows')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setColorNote(r.error?.message ?? r.error?.code ?? 'could not clear windows')
    }
    clearColorNoteSoon()
  }

  return {
    colorBusy,
    colorNote,
    presets,
    selPreset,
    saveName,
    stackLayer,
    winRegion,
    winLook,
    clipWindows,
    setSelPreset,
    setSaveName,
    setStackLayer,
    setWinRegion,
    setWinLook,
    setProjectSpace,
    setClipInputSpace,
    saveLook,
    applyLook,
    addStackLayer,
    removeStackLayer,
    addWindow,
    removeWindow,
    clearWindows,
  }
}
