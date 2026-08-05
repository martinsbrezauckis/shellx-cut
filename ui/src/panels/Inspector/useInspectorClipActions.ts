import { useCallback, useEffect, useMemo, useState } from 'react'
import { callVerb, type Project, type TrackKind } from '../../lib/client'
import { replacementCandidates } from './model'

export interface InspectorClipActionSelection {
  clip: {
    id: string
    asset: string
  }
  trackKind: TrackKind
}

interface InspectorClipActionsArgs {
  project: Project | null
  sel: InspectorClipActionSelection | null
}

export function useInspectorClipActions({ project, sel }: InspectorClipActionsArgs) {
  const replaceOptions = useMemo(
    () => replacementCandidates(project, sel?.clip.asset ?? '', sel?.trackKind ?? 'caption'),
    [project, sel],
  )
  const [replaceAssetId, setReplaceAssetId] = useState('')
  const [clipActionNote, setClipActionNote] = useState<string | null>(null)

  useEffect(() => {
    setReplaceAssetId((current) => {
      if (current && replaceOptions.some((asset) => asset.id === current)) return current
      return replaceOptions[0]?.id ?? ''
    })
  }, [replaceOptions])

  const runReplaceSource = useCallback(async () => {
    if (!sel || !replaceAssetId) return
    setClipActionNote(null)
    const r = await callVerb('edit.replace', {
      target_clip: sel.clip.id,
      asset: replaceAssetId,
      rationale: `inspector: replace ${sel.clip.id} source with ${replaceAssetId}`,
    })
    if (r.ok) {
      setClipActionNote(`Source replaced with ${replaceAssetId}`)
      if (sel.trackKind === 'video') document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setClipActionNote(r.error?.message ?? r.error?.code ?? 'could not replace source')
    }
    setTimeout(() => setClipActionNote(null), 6000)
  }, [sel, replaceAssetId])

  const runDetachAudio = useCallback(async () => {
    if (!sel || sel.trackKind !== 'video') return
    setClipActionNote(null)
    const r = await callVerb('edit.detach_audio', {
      clip: sel.clip.id,
      rationale: `inspector: detach audio from ${sel.clip.id}`,
    })
    if (r.ok) {
      const result = r.result as { detached?: boolean; audio_track?: string; reason?: string } | undefined
      setClipActionNote(result?.detached
        ? `Audio detached to ${result.audio_track ?? 'a new audio track'}`
        : (result?.reason ?? 'Audio is already editable separately'))
    } else {
      setClipActionNote(r.error?.message ?? r.error?.code ?? 'could not detach audio')
    }
    setTimeout(() => setClipActionNote(null), 6000)
  }, [sel])

  return {
    replaceOptions,
    replaceAssetId,
    clipActionNote,
    setReplaceAssetId,
    runReplaceSource,
    runDetachAudio,
  }
}
