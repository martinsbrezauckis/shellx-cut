import type { Dispatch, SetStateAction } from 'react'
import type { Project } from '../../lib/client'
import ContextMenuFrame from '../../components/ContextMenuFrame'
import { Icon } from '../../icons'
import { linkedSiblings, type LaidItem } from './layout'
import GeneratedOverlayContextMenu from './GeneratedOverlayContextMenu'
import {
  adjacentGapSlot,
  assetMediaKind,
  type AssetPickMode,
  clipContextMenuContract,
  isContiguousRun,
} from './ClipContextMenuModel'
import { AudioSection, ClipboardSection, SourceSection, SpeedSection, TransitionsSection } from './ClipContextMenuSections'

export type ClipMenuState = { x: number; y: number; itemId: string; atMs: number }
export type { AssetPickMode } from './ClipContextMenuModel'

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
  onPasteAttributes: (targetIds: string[]) => void
  onOpenTrim: (itemId: string, x: number, y: number) => void
  onSelect: (clipIds: string[]) => void
  onSeek: (atMs: number) => void
  removeItemById: (itemId: string, ripple: boolean) => void | Promise<void>
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
  onPasteAttributes,
  onOpenTrim,
  onSelect,
  onSeek,
  removeItemById,
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
  const it = allItems.find((i) => i.id === menu.itemId)
  if (!it) return null
  const contract = clipContextMenuContract(it, project, allItems)
  if (contract.surface === 'none') return null
  if (contract.contentClass === 'title' || contract.contentClass === 'shape') {
    return <GeneratedOverlayContextMenu
      item={it}
      menu={menu}
      onClose={onClose}
      onSelect={onSelect}
      removeItemById={removeItemById}
      splitItemAt={splitItemAt}
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
    return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-clip-menu" backdropId="data-cut-ctx-backdrop" menuAttributes={{ 'data-cut-clip-kind': 'caption' }} ariaLabel="Caption menu" onClose={onClose}>
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
    </ContextMenuFrame>
  }

  return (
      <ContextMenuFrame
        x={menu.x}
        y={menu.y}
        menuId="data-cut-clip-menu"
        backdropId="data-cut-ctx-backdrop"
        menuAttributes={{ 'data-cut-clip-kind': contract.contentClass }}
        ariaLabel={`${contract.contentClass} clip menu`}
        onClose={onClose}
      >
        <ClipboardSection
          canMedia={canMedia}
          clipboardHasContent={clipboardHasContent}
          itemId={menu.itemId}
          selectedClipIds={selectedClipIds}
          onCopyClip={onCopyClip}
          onCutClip={onCutClip}
          onPasteAttributes={onPasteAttributes}
          onClose={onClose}
        />

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

        <SourceSection
          allowsSourceEdits={contract.allowsSourceEdits}
          canMedia={canMedia}
          canReplace={canReplace}
          canFit={canFit}
          canNest={canNest}
          nestCount={nestSel.length}
          fitDurationMs={fitSlot?.duration_ms ?? null}
          itemId={menu.itemId}
          sourceAssets={sourceAssets}
          assetPick={assetPick}
          setAssetPick={setAssetPick}
          onReplace={replaceClipSource}
          onFit={fitToFillAdjacent}
          onNest={nestSelection}
          onClose={onClose}
        />

        {showSpeed && <SpeedSection
          current={it.speed ?? 1}
          canFreeze={showFreeze}
          onSpeed={(factor) => speedItem(menu.itemId, factor)}
          onReverse={() => reverseItem(menu.itemId)}
          onFreeze={() => freezeItem(menu.itemId)}
          onClose={onClose}
        />}

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

        <TransitionsSection
          transition={contract.addTransition}
          canMedia={canMedia}
          onTransition={() => crossfadeAdjacent(menu.itemId)}
          onFade={(which) => fadeItem(menu.itemId, which)}
          onClose={onClose}
        />
        {showAudioGrp && <AudioSection
          hasTimelineAudio={contract.hasTimelineAudio}
          detachAudio={contract.detachAudio}
          onDetach={() => detachAudioItem(menu.itemId)}
          onGain={() => { onSelect([menu.itemId]); document.dispatchEvent(new CustomEvent('cut:open-drawer', { detail: 'mixer' })) }}
          onMute={() => muteItem(menu.itemId)}
          onCleanVoice={() => cleanVoiceItem(menu.itemId)}
          onClose={onClose}
        />}

      </ContextMenuFrame>
  )
}
