import { useEffect, useMemo, useState } from 'react'
import { callVerb, type CaptionStyle, type Project } from '../../lib/client'
import { pickSubtitle } from '../../lib/tauri'
import {
  CAPTION_CARD_MS,
  CAPTION_POSITIONS,
  TRANSLATE_LANGS,
  captionPositionFromInput,
  type CaptionPosition,
} from './model'

interface ProjectCaptionsSectionProps {
  project: Project | null
  playheadMs: number
}

export default function ProjectCaptionsSection({ project, playheadMs }: ProjectCaptionsSectionProps) {
  const [capText, setCapText] = useState('')
  const [capPos, setCapPos] = useState<CaptionPosition>('bottom')
  const [capColor, setCapColor] = useState('#FFFFFF')
  const [capSize, setCapSize] = useState(64)
  const [capNote, setCapNote] = useState<string | null>(null)
  const [transTargetLang, setTransTargetLang] = useState('es')
  const [transBusy, setTransBusy] = useState<'' | 'captions' | 'transcript'>('')
  const [transNote, setTransNote] = useState<string | null>(null)

  const hasCaptionSource = useMemo(
    () => (project?.tracks ?? []).some((t) => t.kind === 'caption' && t.id !== 'txt1' && (t.clips?.length ?? 0) > 0),
    [project],
  )

  const transcribedAssetIds = useMemo(
    () => Object.entries(project?.assets ?? {})
      .filter(([, a]) => !!(a as { transcript?: unknown }).transcript)
      .map(([id]) => id),
    [project],
  )
  const translateTranscriptAsset = transcribedAssetIds[0] ?? ''
  const hasTranscript = translateTranscriptAsset !== ''
  const transcriptTitle = hasTranscript
    ? `Translate transcript for ${translateTranscriptAsset}${transcribedAssetIds.length > 1 ? ' (first transcribed asset)' : ''}`
    : 'Transcribe a clip first — there is no transcript to translate'

  const addCaptionText = async () => {
    const text = capText.trim()
    if (!text) { setCapNote('Type some caption text first'); return }
    const start = Math.max(0, Math.round(playheadMs))
    const r = await callVerb('captions.add_text', {
      text,
      range_ms: [start, start + CAPTION_CARD_MS],
      position: capPos,
      rationale: 'inspector: add caption text card',
    })
    if (r.ok) {
      setCapText('')
      setCapNote(`Added caption at ${(start / 1000).toFixed(1)}s (${capPos})`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setCapNote(r.error?.message ?? r.error?.code ?? 'could not add caption')
    }
    setTimeout(() => setCapNote(null), 4000)
  }

  const applyCaptionStyle = async () => {
    const r = await callVerb('captions.set_style', {
      ref: `txt_${capPos}`,
      style: { font: 'Inter', size: capSize, color: capColor, pos: capPos },
      rationale: 'inspector: set caption style',
    })
    setCapNote(r.ok ? `Style set (${capSize}px ${capColor})` : (r.error?.message ?? 'could not set style'))
    setTimeout(() => setCapNote(null), 4000)
  }

  // Caption STYLE GALLERY (the grade-gallery pattern): saved looks +
  // built-in catalog from captions.list_styles; Save snapshots the CURRENT
  // panel style (inline — no need to Set style first); Apply restyles every
  // caption in the project (apply_style with ref omitted).
  const [lookName, setLookName] = useState('')
  const [lookPick, setLookPick] = useState('')
  const [looks, setLooks] = useState<Array<{ name: string; builtin: boolean; style: CaptionStyle }>>([])
  const reloadLooks = async () => {
    const r = await callVerb('captions.list_styles', {})
    if (r.ok) setLooks((r.result as { presets?: typeof looks })?.presets ?? [])
  }
  useEffect(() => {
    if (project) void reloadLooks()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.name])

  const saveLook = async () => {
    const name = lookName.trim()
    if (!name) return
    const r = await callVerb('captions.save_style', {
      name,
      style: { font: 'Inter', size: capSize, color: capColor, pos: capPos },
      rationale: 'inspector: save caption look',
    })
    setCapNote(r.ok ? `Saved look "${name}"` : (r.error?.message ?? 'could not save look'))
    if (r.ok) {
      setLookName('')
      void reloadLooks()
    }
    setTimeout(() => setCapNote(null), 4000)
  }

  const applyLook = async () => {
    if (!lookPick) return
    const r = await callVerb('captions.apply_style', { name: lookPick, rationale: 'inspector: apply caption look' })
    if (r.ok) {
      const refs = (r.result as { refs_updated?: string[] })?.refs_updated ?? []
      setCapNote(`Applied "${lookPick}" to ${refs.length} style${refs.length === 1 ? '' : 's'}`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setCapNote(r.error?.message ?? r.error?.code ?? 'could not apply look')
    }
    setTimeout(() => setCapNote(null), 4000)
  }

  const importCaptions = async () => {
    const path = await pickSubtitle()
    if (!path) return
    setCapNote('Importing captions…')
    const r = await callVerb('captions.import', { path, rationale: 'inspector: import subtitle file' })
    if (r.ok) {
      const n = (r.result as { caption_count?: number } | undefined)?.caption_count
      setCapNote(typeof n === 'number' ? `Imported ${n} captions` : 'Captions imported')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setCapNote(r.error?.message ?? r.error?.code ?? 'could not import captions')
    }
    setTimeout(() => setCapNote(null), 4000)
  }

  const translateCaptions = async () => {
    if (transBusy) return
    setTransBusy('captions')
    setTransNote(`Translating captions to ${transTargetLang}…`)
    const r = await callVerb('captions.translate', { target_lang: transTargetLang, mode: 'track', position: 'top', rationale: 'inspector: translate captions' })
    setTransBusy('')
    if (r.ok) {
      const n = (r.result as { cues_translated?: number } | undefined)?.cues_translated
      setTransNote(typeof n === 'number' ? `Translated ${n} cue${n === 1 ? '' : 's'} → ${transTargetLang}` : `Captions translated to ${transTargetLang}`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setTransNote(`Could not translate captions: ${r.error?.message ?? r.error?.code ?? 'unknown error'}`)
    }
    setTimeout(() => setTransNote(null), 8000)
  }

  const translateTranscript = async () => {
    if (transBusy) return
    if (!transcribedAssetIds.length) {
      setTransNote('Could not translate transcript: transcribe a clip first')
      setTimeout(() => setTransNote(null), 8000)
      return
    }
    setTransBusy('transcript')
    setTransNote(`Translating transcript to ${transTargetLang}…`)
    let lastError = 'unknown error'
    for (const asset of transcribedAssetIds) {
      const r = await callVerb('transcript.translate', { asset, target_lang: transTargetLang, rationale: 'inspector: translate transcript' })
      if (r.ok) {
        setTransBusy('')
        const n = (r.result as { segments_translated?: number } | undefined)?.segments_translated
        setTransNote(typeof n === 'number' ? `Translated ${n} segment${n === 1 ? '' : 's'} → ${transTargetLang}` : `Transcript translated to ${transTargetLang}`)
        setTimeout(() => setTransNote(null), 8000)
        return
      }
      lastError = r.error?.message ?? r.error?.code ?? 'unknown error'
      if (!/(has no words|nothing to translate|silent footage|0-word transcript)/i.test(lastError)) break
    }
    setTransBusy('')
    setTransNote(`Could not translate transcript: ${lastError}`)
    setTimeout(() => setTransNote(null), 8000)
  }

  return (
    <div className="insp__group" data-cut-inspector-group="captions">
      <div className="insp__group-title">Captions</div>
      <div className="insp__field">
        <input
          type="text"
          className="insp__text"
          data-cut-caption-text
          placeholder="Caption text…"
          value={capText}
          disabled={!project}
          onChange={(e) => setCapText(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') void addCaptionText() }}
        />
      </div>
      <div className="insp__row">
        <select
          className="insp__select"
          data-cut-caption-position
          value={capPos}
          disabled={!project}
          title="Where the caption card sits"
          onChange={(e) => setCapPos(captionPositionFromInput(e.target.value, capPos))}
        >
          {CAPTION_POSITIONS.map(({ pos, label }) => (<option key={pos} value={pos}>{label}</option>))}
        </select>
        <button
          type="button"
          className="insp__btn"
          data-cut-caption-add
          disabled={!project || !capText.trim()}
          title="Place this text as a caption card at the playhead"
          onClick={() => void addCaptionText()}
        >Add caption at playhead</button>
      </div>
      <div className="insp__row">
        <button
          type="button"
          className="insp__btn"
          data-cut-caption-import
          disabled={!project}
          title="Import an existing SRT, VTT, or ASS subtitle file as caption clips"
          onClick={() => void importCaptions()}
        >Import captions (SRT/VTT)…</button>
      </div>
      <div className="insp__group-title insp__group-title--sub">Style</div>
      <div className="insp__row">
        <label className="insp__inline" title="Caption text color">
          <input type="color" data-cut-caption-color value={capColor} disabled={!project}
            onChange={(e) => setCapColor(e.target.value)} />
        </label>
        <label className="insp__inline" title="Caption text size (px at project height)">
          <input type="number" className="insp__num" data-cut-caption-size min={12} max={200} step={2}
            value={capSize} disabled={!project}
            onChange={(e) => setCapSize(Math.max(12, Math.min(200, Number(e.target.value) || 64)))} />
        </label>
        <button type="button" className="insp__btn" data-cut-caption-style
          disabled={!project}
          title="Update this caption position with the color and size above"
          onClick={() => void applyCaptionStyle()}>Set style</button>
      </div>
      <div className="insp__group-title insp__group-title--sub">Style gallery</div>
      <div className="insp__row" data-cut-caption-gallery>
        <input
          type="text"
          className="insp__text insp__text--grow"
          data-cut-caption-save-name
          placeholder="look name…"
          value={lookName}
          disabled={!project}
          onChange={(e) => setLookName(e.target.value)}
        />
        <button
          type="button"
          className="insp__btn"
          data-cut-action="caption-style-save"
          disabled={!project || !lookName.trim()}
          title="Save the current color, size, and position as a reusable caption look"
          onClick={() => void saveLook()}
        >Save look</button>
      </div>
      <div className="insp__row">
        <select
          className="insp__select"
          data-cut-caption-preset
          value={lookPick}
          disabled={!project || looks.length === 0}
          title="Saved and built-in caption looks"
          onChange={(e) => setLookPick(e.target.value)}
        >
          <option value="">— pick a look —</option>
          {looks.map((l) => (
            <option key={l.name} value={l.name}>{l.builtin ? `${l.name} (built-in)` : l.name}</option>
          ))}
        </select>
        <button
          type="button"
          className="insp__btn"
          data-cut-action="caption-style-apply"
          disabled={!project || !lookPick}
          title="Apply this look to every caption in the project"
          onClick={() => void applyLook()}
        >Apply</button>
      </div>
      <div className="insp__group-title insp__group-title--sub">Translate</div>
      <div className="insp__row" data-cut-inspector-translate>
        <select
          className="insp__select"
          data-cut-translate-lang
          value={transTargetLang}
          disabled={!project || transBusy !== ''}
          title="Language to translate INTO"
          onChange={(e) => setTransTargetLang(e.target.value)}
        >
          {TRANSLATE_LANGS.map(({ code, label }) => (<option key={code} value={code}>{label}</option>))}
        </select>
        <button
          type="button"
          className="insp__btn"
          data-cut-action="translate-captions"
          disabled={!project || !hasCaptionSource || transBusy !== ''}
          title={hasCaptionSource
            ? 'Translate captions into the chosen language as a new track; timing is preserved'
            : 'Generate or import captions first — there are no caption cues to translate'}
          onClick={() => void translateCaptions()}
        >{transBusy === 'captions' ? 'Translating…' : 'Translate captions'}</button>
        <button
          type="button"
          className="insp__btn"
          data-cut-action="translate-transcript"
          data-cut-translate-transcript-asset={translateTranscriptAsset}
          disabled={!project || !hasTranscript || transBusy !== ''}
          title={transcriptTitle}
          onClick={() => void translateTranscript()}
        >{transBusy === 'transcript' ? 'Translating…' : 'Translate transcript'}</button>
      </div>
      {!hasCaptionSource && (
        <p className="insp__hint" data-cut-translate-hint>Generate captions (Transcript → Tools → Generate captions) to enable caption translation.</p>
      )}
      {transNote && <p className="insp__hint" data-cut-translate-note>{transNote}</p>}
      {capNote && <p className="insp__hint" data-cut-caption-note>{capNote}</p>}
    </div>
  )
}
