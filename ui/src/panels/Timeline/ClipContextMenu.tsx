import { useState, type Dispatch, type SetStateAction } from 'react'
import type { Project } from '../../lib/client'
import { Icon } from '../../icons'
import { linkedSiblings, type LaidItem } from './layout'
import GeneratedOverlayContextMenu from './GeneratedOverlayContextMenu'
import CustomSpeedMenuEditor from './CustomSpeedMenuEditor'
import {
  adjacentGapSlot,
  assetBasename,
  assetMediaKind,
  clipContextMenuContract,
  isContiguousRun,
} from './ClipContextMenuModel'

export type ClipMenuState = { x: number; y: number; itemId: string; atMs: number }
export type AssetPickMode = 'replace' | 'fit'

interface ClipContextMenuProps {
  menu: ClipMenuState
  project: Project | null
  allItems: LaidItem[]
  selectedClipIds: string[]
  assetPick: AssetPickMode | null
  setAssetPick: Dispatch<SetStateAction<AssetPickMode | null>>
  clipboardHasContent: boolean
  onClose: () => void
  onCopyClip: (clipId: string) => boolean
  onCutClip: (clipId: string) => void
  onPasteClip: () => void
  onPasteAttributes: (targetIds: string[]) => void
  onOpenTrim: (itemId: string, x: number, y: number) => void
  onSelect: (clipIds: string[]) => void
  onSeek: (atMs: number) => void
  removeItemById: (itemId: string, ripple: boolean) => void | Promise<void>
  removeTrackById: (trackId: string) => void | Promise<void>
  splitItemAt: (itemId: string, atMs: number) => void
  fadeItem: (itemId: string, which: 'in' | 'out') => void
  trimItemTo: (itemId: string, edge: 'start' | 'end', atMs: number) => void
  reverseItem: (itemId: string) => void
  freezeItem: (itemId: string) => void
  stabilizeItem: (itemId: string) => void
  speedItem: (itemId: string, factor: number) => void
  crossfadeAdjacent: (itemId: string) => void
  muteItem: (itemId: string) => void
  cleanVoiceItem: (itemId: string) => void
  blurFacesItem: (itemId: string, atMs: number) => void
  detachAudioItem: (itemId: string) => void | Promise<void>
  splitEditItem: (itemId: string, kind: 'j' | 'l') => void | Promise<void>
  replaceClipSource: (itemId: string, asset: string) => void
  fitToFillAdjacent: (itemId: string, asset: string) => void | Promise<void>
  nestSelection: () => void | Promise<void>
}

function clampMenu(el: HTMLDivElement, x: number, y: number): void {
  const margin = 8
  const rect = el.getBoundingClientRect()
  const left = Math.max(margin, Math.min(x, window.innerWidth - rect.width - margin))
  const top = Math.max(margin, Math.min(y, window.innerHeight - rect.height - margin))
  el.style.left = `${left}px`
  el.style.top = `${top}px`
}

export default function ClipContextMenu({
  menu,
  project,
  allItems,
  selectedClipIds,
  assetPick,
  setAssetPick,
  clipboardHasContent,
  onClose,
  onCopyClip,
  onCutClip,
  onPasteClip,
  onPasteAttributes,
  onOpenTrim,
  onSelect,
  onSeek,
  removeItemById,
  removeTrackById,
  splitItemAt,
  fadeItem,
  trimItemTo,
  reverseItem,
  freezeItem,
  stabilizeItem,
  speedItem,
  crossfadeAdjacent,
  muteItem,
  cleanVoiceItem,
  blurFacesItem,
  detachAudioItem,
  splitEditItem,
  replaceClipSource,
  fitToFillAdjacent,
  nestSelection,
}: ClipContextMenuProps) {
  const [speedOpen, setSpeedOpen] = useState(false)
  const it = allItems.find((i) => i.id === menu.itemId)
  if (!it) return null
  const contract = clipContextMenuContract(it, project, allItems)
  if (contract.surface === 'none') return null
  const tid = it.trackId
  const firstVideo = project?.tracks.find((t) => t.kind === 'video')?.id
  const firstAudio = project?.tracks.find((t) => t.kind === 'audio')?.id
  const isOverlayTrack = tid !== firstVideo && tid !== firstAudio
  if (contract.contentClass === 'title' || contract.contentClass === 'shape') {
    return <GeneratedOverlayContextMenu
      item={it}
      menu={menu}
      onClose={onClose}
      onSelect={onSelect}
      removeItemById={removeItemById}
      removeTrackById={removeTrackById}
      splitItemAt={splitItemAt}
      isOverlayTrack={isOverlayTrack}
    />
  }
  const isCaption = contract.surface === 'caption'
  const isAudio = contract.contentClass === 'audio'
  const cutMs = menu.atMs
  const insideSpan = cutMs > it.startMs && cutMs < it.startMs + it.durMs
  const canMedia = contract.isMedia
  // J/L remains deliberately LAID/EDL-keyed (unlike edit.crossfade). Its
  // class gate is contract-owned; only this operation's own seam lookup uses
  // rendered coordinates.
  const hasVideoSeam = contract.allowsSplitEdit && allItems.some((a) =>
    a.trackId === it.trackId && a.kind === 'video' && a.id !== it.id &&
    (a.startMs === it.startMs + it.durMs || a.startMs + a.durMs === it.startMs),
  )
  const showSpeed = contract.allowsSpeedEdits
  const showFreeze = contract.allowsFreeze
  const showPicture = contract.allowsPictureTools
  const showStabilize = contract.allowsStabilize
  const showAudioGrp = contract.hasTimelineAudio || contract.detachAudio.visibility === 'visible'
  const showPrivacy = contract.allowsPrivacyTools
  // Remove propagates to the EXACT linked audio counterpart (removeItemById →
  // linkedSiblings, the c68b449c linked-A/V rule; ambiguous matches refuse).
  // Compute the same fact here so the Remove tooltips DISCLOSE the propagation
  // truthfully — exactly when it will actually happen.
  const removesLinkedAudio = contract.contentClass === 'video' && linkedSiblings(it, allItems).length === 1
  const sourceAssets = Object.entries(project?.assets ?? {}).filter(([id, asset]) => {
    if (id === it?.asset) return false
    const mediaKind = assetMediaKind(asset)
    if (!isAudio) return mediaKind === 'video' || mediaKind === 'image' || mediaKind === 'other'
    if (isAudio) return mediaKind === 'audio' || mediaKind === 'other'
    return false
  })
  const canReplace = canMedia && sourceAssets.length > 0
  const fitSlot = it ? adjacentGapSlot(it, allItems) : null
  const canFit = !!fitSlot && sourceAssets.length > 0
  const nestSel = allItems.filter((i) => selectedClipIds.includes(i.id) && (i.kind === 'video' || i.kind === 'audio'))
  const canNest = isContiguousRun(nestSel, allItems)

  if (isCaption) {
    return (
      <>
        <div className="tl-ctx-backdrop" data-cut-ctx-backdrop onMouseDown={onClose} onContextMenu={(e) => { e.preventDefault(); onClose() }} />
        <div className="tl-ctx" role="menu" data-cut-clip-menu data-cut-clip-kind="caption" style={{ left: menu.x, top: menu.y }}
          ref={(el) => { if (el) clampMenu(el, menu.x, menu.y) }}
        >
          <span className="tl-ctx__label" aria-hidden="true">Caption</span>
          <button className="tl-ctx__item" data-cut-ctx="caption-edit" role="menuitem"
            title="Edit this caption’s text & style in the Inspector"
            onClick={() => { onSelect([menu.itemId]); onClose() }}>
            <Icon name="captions" size={14} /> Edit text &amp; style…
          </button>
          <button className="tl-ctx__item" data-cut-ctx="caption-seek" role="menuitem"
            title="Move the playhead to the start of this caption"
            onClick={() => { if (it) onSeek(it.startMs); onClose() }}>
            <Icon name="marker" size={14} /> Seek to caption
          </button>
          <span className="tl-ctx__sep" aria-hidden="true" />
          <button className="tl-ctx__item tl-ctx__item--danger" data-cut-ctx="remove" role="menuitem"
            onClick={() => { void removeItemById(menu.itemId, true); onClose() }}>
            <Icon name="rippleDelete" size={14} /> Remove caption <kbd className="tl-ctx__kbd">Del</kbd>
          </button>
        </div>
      </>
    )
  }

  return (
    <>
      <div className="tl-ctx-backdrop" data-cut-ctx-backdrop onMouseDown={onClose} onContextMenu={(e) => { e.preventDefault(); onClose() }} />
      {/* Class tag mirrors the caption branch's data-cut-clip-kind so
          automation and the debug API can identify the rendered menu class. */}
      <div className="tl-ctx" role="menu" data-cut-clip-menu data-cut-clip-kind={contract.contentClass} style={{ left: menu.x, top: menu.y }}
        ref={(el) => { if (el) clampMenu(el, menu.x, menu.y) }}
      >
        <span className="tl-ctx__label" aria-hidden="true">Clipboard</span>
        <button className="tl-ctx__item" data-cut-ctx="copy" role="menuitem"
          disabled={!canMedia}
          title={canMedia ? 'Copy this clip (Ctrl/Cmd+C)' : 'Copy works on a video or audio clip'}
          onClick={() => { onCopyClip(menu.itemId); onClose() }}>
          <Icon name="copy" size={14} /> Copy <kbd className="tl-ctx__kbd">⌘C</kbd>
        </button>
        <button className="tl-ctx__item" data-cut-ctx="cut" role="menuitem"
          disabled={!canMedia}
          title={canMedia ? 'Cut this clip — copy + remove it (Ctrl/Cmd+X)' : 'Cut works on a video or audio clip'}
          onClick={() => { onCutClip(menu.itemId); onClose() }}>
          <Icon name="cut" size={14} /> Cut <kbd className="tl-ctx__kbd">⌘X</kbd>
        </button>
        <button className="tl-ctx__item" data-cut-ctx="paste" role="menuitem"
          disabled={!clipboardHasContent}
          title={clipboardHasContent ? 'Paste the copied clip at the playhead (Ctrl/Cmd+V)' : 'Copy or cut a clip first'}
          onClick={() => { onPasteClip(); onClose() }}>
          <Icon name="paste" size={14} /> Paste <kbd className="tl-ctx__kbd">⌘V</kbd>
        </button>
        <button className="tl-ctx__item" data-cut-ctx="paste-attributes" role="menuitem"
          disabled={!clipboardHasContent || !canMedia}
          title={clipboardHasContent
            ? 'Paste the copied clip\u2019s grade / transform / speed / volume / effects onto the selected clip(s) (Ctrl/Cmd+Alt+V)'
            : 'Copy a clip first, then paste its attributes onto others'}
          onClick={() => {
            const targets = selectedClipIds.includes(menu.itemId) ? selectedClipIds : [menu.itemId]
            onPasteAttributes(targets)
            onClose()
          }}>
          <Icon name="paste" size={14} /> Paste attributes… <kbd className="tl-ctx__kbd">⌘⌥V</kbd>
        </button>

        <span className="tl-ctx__sep" aria-hidden="true" />
        <span className="tl-ctx__label" aria-hidden="true">Edit</span>
        <button className="tl-ctx__item" data-cut-ctx="split" role="menuitem"
          title="Split this clip at the click point (S)"
          onClick={() => { splitItemAt(menu.itemId, menu.atMs); onClose() }}>
          <Icon name="split" size={14} /> Split here <kbd className="tl-ctx__kbd">S</kbd>
        </button>
        {/* J/L cuts stagger PICTURE vs SOUND at a seam — meaningless for
            title/shape renders (no audio stream), so they are class-filtered
            on top of the (deliberately laid, split_edit EDL-keyed) seam test. */}
        {contract.allowsSplitEdit && hasVideoSeam && (
          <>
            <button className="tl-ctx__item" data-cut-ctx="split-edit-j" role="menuitem"
              title="J-cut — start the next scene's audio before its picture. The audio must be detached at the cut."
              onClick={() => { void splitEditItem(menu.itemId, 'j'); onClose() }}>
              <Icon name="crossfade" size={14} /> J-cut (audio leads)
            </button>
            <button className="tl-ctx__item" data-cut-ctx="split-edit-l" role="menuitem"
              title="L-cut — continue this scene's audio after its picture cuts away. The audio must be detached at the cut."
              onClick={() => { void splitEditItem(menu.itemId, 'l'); onClose() }}>
              <Icon name="crossfade" size={14} /> L-cut (audio lags)
            </button>
          </>
        )}

        <span className="tl-ctx__sep" aria-hidden="true" />
        <span className="tl-ctx__label" aria-hidden="true">Trim</span>
        <button className="tl-ctx__item" data-cut-ctx="trim-tools" role="menuitem"
          disabled={!canMedia}
          title={canMedia ? 'Slip / slide / roll — frame-accurate trim steppers (pro trim tools)' : 'Trim tools work on a video or audio clip'}
          onClick={() => { onOpenTrim(menu.itemId, menu.x, menu.y); onClose() }}>
          <Icon name="split" size={14} /> Trim (slip / slide / roll)…
        </button>
        <button className="tl-ctx__item" data-cut-ctx="trim-start" role="menuitem"
          disabled={!insideSpan}
          title={!insideSpan ? 'Right-click inside the clip to choose a trim point' : 'Remove everything before the click point'}
          onClick={() => { trimItemTo(menu.itemId, 'start', menu.atMs); onClose() }}>
          <Icon name="collapseLeft" size={14} /> Trim start to here
        </button>
        <button className="tl-ctx__item" data-cut-ctx="trim-end" role="menuitem"
          disabled={!insideSpan}
          title={!insideSpan ? 'Right-click inside the clip to choose a trim point' : 'Remove everything after the click point'}
          onClick={() => { trimItemTo(menu.itemId, 'end', menu.atMs); onClose() }}>
          <Icon name="collapseRight" size={14} /> Trim end to here
        </button>

        <span className="tl-ctx__sep" aria-hidden="true" />
        <span className="tl-ctx__label" aria-hidden="true">Delete</span>
        {/* Both Remove paths propagate to the exact linked audio counterpart
            (removeItemById) — the tooltip discloses that whenever it will
            happen, the same trust-story pattern the rest of the app uses. */}
        <button className="tl-ctx__item tl-ctx__item--danger" data-cut-ctx="remove" role="menuitem"
          title={removesLinkedAudio
            ? 'Remove this clip AND its linked audio, closing the gap (Del)'
            : 'Remove this clip and close the gap (Del)'}
          onClick={() => { void removeItemById(menu.itemId, true); onClose() }}>
          <Icon name="rippleDelete" size={14} /> Remove clip <kbd className="tl-ctx__kbd">Del</kbd>
        </button>
        <button className="tl-ctx__item" data-cut-ctx="remove-gap" role="menuitem"
          title={removesLinkedAudio
            ? 'Remove this clip AND its linked audio, keeping the gap (⌥Del)'
            : 'Remove this clip but keep the gap (⌥Del)'}
          onClick={() => { void removeItemById(menu.itemId, false); onClose() }}>
          <Icon name="liftDelete" size={14} /> Remove, keep gap <kbd className="tl-ctx__kbd">⌥Del</kbd>
        </button>

        {/* Replace / Fit-to-fill / Nest swap or collapse the clip's SOURCE
            media — on a title/shape they would sever the title.update /
            shape.update editing identity (the render is regenerated from the
            title text, not swappable footage), so the section is
            class-filtered for title clips. */}
        {contract.allowsSourceEdits && (
          <>
        <span className="tl-ctx__sep" aria-hidden="true" />
        <span className="tl-ctx__label" aria-hidden="true">Replace</span>
        <button className="tl-ctx__item" data-cut-ctx="replace" role="menuitem"
          aria-expanded={assetPick === 'replace'}
          disabled={!canReplace}
          title={!canMedia ? 'Replace works on a video or audio clip'
            : !canReplace ? 'Import another compatible clip first'
            : 'Swap this clip’s source while keeping its slot, duration, and look'}
          onClick={() => setAssetPick((p) => (p === 'replace' ? null : 'replace'))}>
          <Icon name="import" size={14} /> Replace with…
          <Icon name={assetPick === 'replace' ? 'chevronUp' : 'chevronDown'} size={14} className="tl-ctx__caret" />
        </button>
        {assetPick === 'replace' && canReplace && (
          <div className="tl-ctx__sub" data-cut-ctx-replace-list role="group">
            {sourceAssets.map(([id, asset]) => (
              <button key={id} className="tl-ctx__item tl-ctx__item--sub" role="menuitem"
                data-cut-ctx-replace-asset={id}
                title={`Replace with ${assetBasename(asset)} (${id})`}
                onClick={() => { replaceClipSource(menu.itemId, id); onClose() }}>
                <Icon name={assetMediaKind(asset) === 'audio' ? 'audioClip' : assetMediaKind(asset) === 'image' ? 'image' : 'film'} size={14} /> {assetBasename(asset)}
              </button>
            ))}
          </div>
        )}
        <button className="tl-ctx__item" data-cut-ctx="fit-to-fill" role="menuitem"
          aria-expanded={assetPick === 'fit'}
          disabled={!canFit}
          title={!fitSlot ? 'Fit to fill needs an empty gap next to this clip; remove a neighbour to open one'
            : sourceAssets.length === 0 ? 'Import a clip to place in the gap'
            : 'Fill the adjacent gap and adjust speed to fit exactly'}
          onClick={() => setAssetPick((p) => (p === 'fit' ? null : 'fit'))}>
          <Icon name="fitTimeline" size={14} /> Fit to fill gap…
          <Icon name={assetPick === 'fit' ? 'chevronUp' : 'chevronDown'} size={14} className="tl-ctx__caret" />
        </button>
        {assetPick === 'fit' && canFit && (
          <div className="tl-ctx__sub" data-cut-ctx-fit-list role="group">
            {sourceAssets.map(([id, asset]) => (
              <button key={id} className="tl-ctx__item tl-ctx__item--sub" role="menuitem"
                data-cut-ctx-fit-asset={id}
                title={`Fit ${assetBasename(asset)} into the ${fitSlot?.duration_ms ?? 0}ms gap`}
                onClick={() => { void fitToFillAdjacent(menu.itemId, id); onClose() }}>
                <Icon name={assetMediaKind(asset) === 'audio' ? 'audioClip' : assetMediaKind(asset) === 'image' ? 'image' : 'film'} size={14} /> {assetBasename(asset)}
              </button>
            ))}
          </div>
        )}
        <button className="tl-ctx__item" data-cut-ctx="nest" role="menuitem"
          disabled={!canNest}
          title={!canNest ? 'Select two or more adjacent clips on one track first'
            : `Collapse the ${nestSel.length} selected clips into one nested clip`}
          onClick={() => { void nestSelection(); onClose() }}>
          <Icon name="layers" size={14} /> Nest selection{canNest ? ` (${nestSel.length})` : ''}
        </button>
          </>
        )}

        {showSpeed && (
          <>
            <span className="tl-ctx__sep" aria-hidden="true" />
            <button className="tl-ctx__item" data-cut-ctx="speed-time" role="menuitem"
              aria-expanded={speedOpen}
              title="Show playback speed, reverse, and freeze controls"
              onClick={() => setSpeedOpen((open) => !open)}>
              <Icon name="speed" size={14} /> Speed &amp; time…
              <Icon name={speedOpen ? 'chevronUp' : 'chevronDown'} size={14} className="tl-ctx__caret" />
            </button>
            {speedOpen && (
              <div className="tl-ctx__sub" data-cut-ctx-speed-list role="group">
                <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="speed-half" role="menuitem"
                  title="Slow to half speed"
                  onClick={() => { speedItem(menu.itemId, 0.5); onClose() }}>
                  <Icon name="speed" size={14} /> ½× speed
                </button>
                <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="speed-normal" role="menuitem"
                  title="Reset to normal speed"
                  onClick={() => { speedItem(menu.itemId, 1); onClose() }}>
                  <Icon name="speed" size={14} /> 1× (normal)
                </button>
                <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="speed-double" role="menuitem"
                  title="Speed up to twice normal speed"
                  onClick={() => { speedItem(menu.itemId, 2); onClose() }}>
                  <Icon name="speed" size={14} /> 2× speed
                </button>
                <CustomSpeedMenuEditor
                  current={it.speed ?? 1}
                  onApply={(factor) => speedItem(menu.itemId, factor)}
                  onClose={onClose}
                />
                <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="reverse" role="menuitem"
                  title="Play the clip backward"
                  onClick={() => { reverseItem(menu.itemId); onClose() }}>
                  <Icon name="flip" size={14} /> Reverse
                </button>
                {showFreeze && (
                  <button className="tl-ctx__item tl-ctx__item--sub" data-cut-ctx="freeze" role="menuitem"
                    title="Hold the first frame for the whole slot"
                    onClick={() => { freezeItem(menu.itemId); onClose() }}>
                    <Icon name="keyframe" size={14} /> Freeze frame
                  </button>
                )}
              </div>
            )}
          </>
        )}

        {showPicture && (
          <>
            <span className="tl-ctx__sep" aria-hidden="true" />
            <span className="tl-ctx__label" aria-hidden="true">Picture</span>
            {contract.allowsColorAndCrop && (
              <button className="tl-ctx__item" data-cut-ctx="color-grade" role="menuitem"
                title="Open the Color tab to grade this clip"
                onClick={() => { onSelect([menu.itemId]); document.dispatchEvent(new CustomEvent('cut:open-grade')); onClose() }}>
                <Icon name="grade" size={14} /> Color grade…
              </button>
            )}
            <button className="tl-ctx__item" data-cut-ctx="transform" role="menuitem"
              title="Open Transform / Layer for position, scale, and opacity"
              onClick={() => { onSelect([menu.itemId]); document.dispatchEvent(new CustomEvent('cut:open-layer')); onClose() }}>
              <Icon name="transform" size={14} /> Transform / crop…
            </button>
            {showStabilize && (
              <button className="tl-ctx__item" data-cut-ctx="stabilize" role="menuitem"
                title="Smooth out camera shake"
                onClick={() => { stabilizeItem(menu.itemId); onClose() }}>
                <Icon name="sliders" size={14} /> Stabilize
              </button>
            )}
            {showPrivacy && (
              <button className="tl-ctx__item" data-cut-ctx="blur-faces" role="menuitem"
                title="Detect and blur every face at the cursor frame"
                onClick={() => { blurFacesItem(menu.itemId, menu.atMs); onClose() }}>
                <Icon name="redact" size={14} /> Blur faces
              </button>
            )}
          </>
        )}

        <span className="tl-ctx__sep" aria-hidden="true" />
        <span className="tl-ctx__label" aria-hidden="true">Transitions &amp; fades</span>
        <button className="tl-ctx__item" data-cut-ctx="add-transition" role="menuitem"
          disabled={!contract.addTransition.enabled}
          title={contract.addTransition.reason}
          onClick={() => { crossfadeAdjacent(menu.itemId); onClose() }}>
          <Icon name="crossfade" size={14} /> Add transition
        </button>
        <button className="tl-ctx__item" data-cut-ctx="fade-in" role="menuitem"
          disabled={!canMedia} title={canMedia ? 'Fade in over 0.5 seconds' : 'Fade works on a video or audio clip'}
          onClick={() => { fadeItem(menu.itemId, 'in'); onClose() }}>
          <Icon name="effect" size={14} /> Fade in (0.5s)
        </button>
        <button className="tl-ctx__item" data-cut-ctx="fade-out" role="menuitem"
          disabled={!canMedia} title={canMedia ? 'Fade out over 0.5 seconds' : 'Fade works on a video or audio clip'}
          onClick={() => { fadeItem(menu.itemId, 'out'); onClose() }}>
          <Icon name="effect" size={14} /> Fade out (0.5s)
        </button>

        {showAudioGrp && (
          <>
            <span className="tl-ctx__sep" aria-hidden="true" />
            <span className="tl-ctx__label" aria-hidden="true">Audio</span>
            {contract.detachAudio.visibility === 'visible' && (
              <button className="tl-ctx__item" data-cut-ctx="detach-audio" role="menuitem"
                title="Move this clip's audio onto its own editable track"
                onClick={() => { void detachAudioItem(menu.itemId); onClose() }}>
                <Icon name="waveform" size={14} /> Detach audio
              </button>
            )}
            {contract.hasTimelineAudio && <>
            <button className="tl-ctx__item" data-cut-ctx="gain" role="menuitem"
              title="Open the Audio mixer to change this clip's level"
              onClick={() => { onSelect([menu.itemId]); document.dispatchEvent(new CustomEvent('cut:open-drawer', { detail: 'mixer' })); onClose() }}>
              <Icon name="mixer" size={14} /> Gain…
            </button>
            <button className="tl-ctx__item" data-cut-ctx="mute" role="menuitem"
              title="Silence this clip"
              onClick={() => { muteItem(menu.itemId); onClose() }}>
              <Icon name="mute" size={14} /> Mute
            </button>
            <button className="tl-ctx__item" data-cut-ctx="clean-voice" role="menuitem"
              title="Clean voice with noise reduction, gating, compression, and EQ"
              onClick={() => { cleanVoiceItem(menu.itemId); onClose() }}>
              <Icon name="waveform" size={14} /> Clean voice / EQ
            </button>
            </>}
          </>
        )}

        {isOverlayTrack && tid && (
          <>
            <span className="tl-ctx__sep" aria-hidden="true" />
            <button className="tl-ctx__item tl-ctx__item--danger" data-cut-ctx="remove-track" role="menuitem"
              onClick={() => { void removeTrackById(tid); onClose() }}>
              <Icon name="trash" size={14} /> Remove track “{tid}”
            </button>
          </>
        )}
      </div>
    </>
  )
}
