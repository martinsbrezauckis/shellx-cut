import { useState, type Dispatch, type SetStateAction } from 'react'
import type { Asset } from '../../lib/client'
import { Icon } from '../../icons'
import { assetBasename, assetMediaKind, type AssetPickMode, type ContextMenuActionState } from './ClipContextMenuModel'
import CustomSpeedMenuEditor from './CustomSpeedMenuEditor'

interface ClipboardSectionProps {
  canMedia: boolean
  clipboardHasContent: boolean
  itemId: string
  selectedClipIds: string[]
  onCopyClip: (clipId: string) => boolean
  onCutClip: (clipId: string) => void
  onPasteAttributes: (targetIds: string[]) => void
  onClose: () => void
}

export function ClipboardSection({
  canMedia,
  clipboardHasContent,
  itemId,
  selectedClipIds,
  onCopyClip,
  onCutClip,
  onPasteAttributes,
  onClose,
}: ClipboardSectionProps) {
  const pasteAttributesTitle = clipboardHasContent
    ? 'Paste the copied clip’s grade / transform / speed / volume / effects onto the selected clip(s) (Ctrl/Cmd+Alt+V)'
    : 'Copy a clip first, then paste its attributes onto others'
  return <>
    <span className="tl-ctx__label" aria-hidden="true">Clipboard</span>
    <button className="tl-ctx__item" data-cut-ctx="copy" role="menuitem"
      disabled={!canMedia}
      title={canMedia ? 'Copy this clip (Ctrl/Cmd+C)' : 'Copy works on a video or audio clip'}
      aria-description={!canMedia ? 'Copy works on a video or audio clip' : undefined}
      onClick={() => { onCopyClip(itemId); onClose() }}>
      <Icon name="copy" size={14} /> Copy <kbd className="tl-ctx__kbd">⌘C</kbd>
    </button>
    <button className="tl-ctx__item" data-cut-ctx="cut" role="menuitem"
      disabled={!canMedia}
      title={canMedia ? 'Cut this clip — copy + remove it (Ctrl/Cmd+X)' : 'Cut works on a video or audio clip'}
      aria-description={!canMedia ? 'Cut works on a video or audio clip' : undefined}
      onClick={() => { onCutClip(itemId); onClose() }}>
      <Icon name="cut" size={14} /> Cut <kbd className="tl-ctx__kbd">⌘X</kbd>
    </button>
    <button className="tl-ctx__item" data-cut-ctx="paste-attributes" role="menuitem"
      disabled={!clipboardHasContent || !canMedia}
      title={pasteAttributesTitle}
      aria-description={!clipboardHasContent || !canMedia ? pasteAttributesTitle : undefined}
      onClick={() => {
        onPasteAttributes(selectedClipIds.includes(itemId) ? selectedClipIds : [itemId])
        onClose()
      }}>
      <Icon name="paste" size={14} /> Paste attributes… <kbd className="tl-ctx__kbd">⌘⌥V</kbd>
    </button>
  </>
}

interface SourceSectionProps {
  allowsSourceEdits: boolean
  canMedia: boolean
  canReplace: boolean
  canFit: boolean
  canNest: boolean
  nestCount: number
  fitDurationMs: number | null
  itemId: string
  sourceAssets: Array<[string, Asset]>
  assetPick: AssetPickMode | null
  setAssetPick: Dispatch<SetStateAction<AssetPickMode | null>>
  onReplace: (itemId: string, assetId: string) => void
  onFit: (itemId: string, assetId: string) => void | Promise<void>
  onNest: () => void | Promise<void>
  onClose: () => void
}

/** Source replacement stays isolated from editorial/media tools so generated
 * overlays cannot accidentally regain a footage-source operation. */
export function SourceSection({
  allowsSourceEdits,
  canMedia,
  canReplace,
  canFit,
  canNest,
  nestCount,
  fitDurationMs,
  itemId,
  sourceAssets,
  assetPick,
  setAssetPick,
  onReplace,
  onFit,
  onNest,
  onClose,
}: SourceSectionProps) {
  if (!allowsSourceEdits) return null
  const replaceTitle = !canMedia ? 'Replace works on a video or audio clip'
    : !canReplace ? 'Import another compatible clip first'
      : 'Swap this clip’s source while keeping its slot, duration, and look'
  const fitTitle = !canFit
    ? 'Fit to fill needs an adjacent gap and compatible imported source'
    : 'Fill the adjacent gap and adjust speed to fit exactly'
  const nestTitle = !canNest ? 'Select two or more adjacent clips on one track first'
    : `Collapse the ${nestCount} selected clips into one nested clip`
  return <>
    <span className="tl-ctx__sep" aria-hidden="true" />
    <span className="tl-ctx__label" aria-hidden="true">Source</span>
    <button className="tl-ctx__item" data-cut-ctx="replace" role="menuitem"
      aria-expanded={assetPick === 'replace'}
      disabled={!canReplace}
      title={replaceTitle}
      aria-description={!canReplace ? replaceTitle : undefined}
      onClick={() => setAssetPick((current) => current === 'replace' ? null : 'replace')}>
      <Icon name="import" size={14} /> Replace with…
      <Icon name={assetPick === 'replace' ? 'chevronUp' : 'chevronDown'} size={14} className="tl-ctx__caret" />
    </button>
    {assetPick === 'replace' && canReplace && <div className="tl-ctx__sub" data-cut-ctx-replace-list role="group">
      {sourceAssets.map(([id, asset]) => <button key={id} className="tl-ctx__item tl-ctx__item--sub" role="menuitem"
        data-cut-ctx-replace-asset={id}
        title={`Replace with ${assetBasename(asset)} (${id})`}
        onClick={() => { onReplace(itemId, id); onClose() }}>
        <Icon name={assetMediaKind(asset) === 'audio' ? 'audioClip' : assetMediaKind(asset) === 'image' ? 'image' : 'film'} size={14} /> {assetBasename(asset)}
      </button>)}
    </div>}
    <button className="tl-ctx__item" data-cut-ctx="fit-to-fill" role="menuitem"
      aria-expanded={assetPick === 'fit'}
      disabled={!canFit}
      title={fitTitle}
      aria-description={!canFit ? fitTitle : undefined}
      onClick={() => setAssetPick((current) => current === 'fit' ? null : 'fit')}>
      <Icon name="fitTimeline" size={14} /> Fit to fill gap…
      <Icon name={assetPick === 'fit' ? 'chevronUp' : 'chevronDown'} size={14} className="tl-ctx__caret" />
    </button>
    {assetPick === 'fit' && canFit && <div className="tl-ctx__sub" data-cut-ctx-fit-list role="group">
      {sourceAssets.map(([id, asset]) => <button key={id} className="tl-ctx__item tl-ctx__item--sub" role="menuitem"
        data-cut-ctx-fit-asset={id}
        title={`Fit ${assetBasename(asset)} into the ${fitDurationMs ?? 0}ms gap`}
        onClick={() => { void onFit(itemId, id); onClose() }}>
        <Icon name={assetMediaKind(asset) === 'audio' ? 'audioClip' : assetMediaKind(asset) === 'image' ? 'image' : 'film'} size={14} /> {assetBasename(asset)}
      </button>)}
    </div>}
    <button className="tl-ctx__item" data-cut-ctx="nest" role="menuitem"
      disabled={!canNest}
      title={nestTitle}
      aria-description={!canNest ? nestTitle : undefined}
      onClick={() => { void onNest(); onClose() }}>
      <Icon name="layers" size={14} /> Nest selection{canNest ? ` (${nestCount})` : ''}
    </button>
  </>
}

interface TransitionsSectionProps {
  transition: ContextMenuActionState
  canMedia: boolean
  onTransition: () => void
  onFade: (which: 'in' | 'out') => void
  onClose: () => void
}

export function TransitionsSection({ transition, canMedia, onTransition, onFade, onClose }: TransitionsSectionProps) {
  const fadeReason = 'Fade works on a video or audio clip'
  return <>
    <span className="tl-ctx__sep" aria-hidden="true" />
    <span className="tl-ctx__label" aria-hidden="true">Transitions &amp; fades</span>
    <button className="tl-ctx__item" data-cut-ctx="add-transition" role="menuitem"
      disabled={!transition.enabled}
      title={transition.reason}
      aria-description={!transition.enabled ? transition.reason : undefined}
      onClick={() => { onTransition(); onClose() }}>
      <Icon name="crossfade" size={14} /> Add transition
    </button>
    <button className="tl-ctx__item" data-cut-ctx="fade-in" role="menuitem"
      disabled={!canMedia}
      title={canMedia ? 'Fade in over 0.5 seconds' : fadeReason}
      aria-description={!canMedia ? fadeReason : undefined}
      onClick={() => { onFade('in'); onClose() }}>
      <Icon name="effect" size={14} /> Fade in (0.5s)
    </button>
    <button className="tl-ctx__item" data-cut-ctx="fade-out" role="menuitem"
      disabled={!canMedia}
      title={canMedia ? 'Fade out over 0.5 seconds' : fadeReason}
      aria-description={!canMedia ? fadeReason : undefined}
      onClick={() => { onFade('out'); onClose() }}>
      <Icon name="effect" size={14} /> Fade out (0.5s)
    </button>
  </>
}

interface AudioSectionProps {
  hasTimelineAudio: boolean
  detachAudio: ContextMenuActionState
  onDetach: () => void | Promise<void>
  onGain: () => void
  onMute: () => void
  onCleanVoice: () => void
  onClose: () => void
}

/** Linked A/V remains available from its video owner, while an audio clip
 * resolves itself as the exact audio target. The compact parent prevents the
 * media menu from listing four audio operations at first level. */
export function AudioSection({ hasTimelineAudio, detachAudio, onDetach, onGain, onMute, onCleanVoice, onClose }: AudioSectionProps) {
  const [open, setOpen] = useState(false)
  if (!hasTimelineAudio && detachAudio.visibility !== 'visible') return null
  return <>
    <span className="tl-ctx__sep" aria-hidden="true" />
    <button className="tl-ctx__item" data-cut-ctx="audio" role="menuitem" aria-expanded={open}
      title="Show linked-audio and clip-audio controls"
      onClick={() => setOpen((current) => !current)}>
      <Icon name="waveform" size={14} /> Audio…
      <Icon name={open ? 'chevronUp' : 'chevronDown'} size={14} className="tl-ctx__caret" />
    </button>
    {open && <div className="tl-ctx__sub" data-cut-ctx-audio-list role="group">
      {detachAudio.visibility === 'visible' && <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="detach-audio" role="menuitem"
        title="Move this clip's audio onto its own editable track"
        onClick={() => { void onDetach(); onClose() }}>
        <Icon name="waveform" size={14} /> Detach audio
      </button>}
      {hasTimelineAudio && <>
        <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="gain" role="menuitem"
          title="Open the Audio mixer to change this clip's level"
          onClick={() => { onGain(); onClose() }}><Icon name="mixer" size={14} /> Gain…</button>
        <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="mute" role="menuitem"
          title="Silence this clip"
          onClick={() => { onMute(); onClose() }}><Icon name="mute" size={14} /> Mute</button>
        <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="clean-voice" role="menuitem"
          title="Clean voice with noise reduction, gating, compression, and EQ"
          onClick={() => { onCleanVoice(); onClose() }}><Icon name="waveform" size={14} /> Clean voice / EQ</button>
      </>}
    </div>}
  </>
}

interface SpeedSectionProps {
  current: number
  canFreeze: boolean
  onSpeed: (factor: number) => void
  onReverse: () => void
  onFreeze: () => void
  onClose: () => void
}

export function SpeedSection({ current, canFreeze, onSpeed, onReverse, onFreeze, onClose }: SpeedSectionProps) {
  const [open, setOpen] = useState(false)
  return <>
    <span className="tl-ctx__sep" aria-hidden="true" />
    <button className="tl-ctx__item" data-cut-ctx="speed-time" role="menuitem" aria-expanded={open}
      title="Show playback speed, reverse, and freeze controls"
      onClick={() => setOpen((currentOpen) => !currentOpen)}>
      <Icon name="speed" size={14} /> Speed &amp; time…
      <Icon name={open ? 'chevronUp' : 'chevronDown'} size={14} className="tl-ctx__caret" />
    </button>
    {open && <div className="tl-ctx__sub" data-cut-ctx-speed-list role="group">
      <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="speed-half" role="menuitem" title="Slow to half speed" onClick={() => { onSpeed(0.5); onClose() }}><Icon name="speed" size={14} /> ½× speed</button>
      <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="speed-normal" role="menuitem" title="Reset to normal speed" onClick={() => { onSpeed(1); onClose() }}><Icon name="speed" size={14} /> 1× (normal)</button>
      <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="speed-double" role="menuitem" title="Speed up to twice normal speed" onClick={() => { onSpeed(2); onClose() }}><Icon name="speed" size={14} /> 2× speed</button>
      <CustomSpeedMenuEditor current={current} onApply={onSpeed} onClose={onClose} />
      <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="reverse" role="menuitem" title="Play the clip backward" onClick={() => { onReverse(); onClose() }}><Icon name="flip" size={14} /> Reverse</button>
      {canFreeze && <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="freeze" role="menuitem" title="Hold the first frame for the whole slot" onClick={() => { onFreeze(); onClose() }}><Icon name="keyframe" size={14} /> Freeze frame</button>}
    </div>}
  </>
}
