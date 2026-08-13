import type { TimelineProps } from './index'
import ClipContextMenu from './ClipContextMenu'
import TimelineSurfaceContextMenu from './TimelineSurfaceContextMenu'
import TimelineTrackContextMenu from './TimelineTrackContextMenu'
import type { LaidItem } from './layout'
import type { useTimelineClipActions } from './useTimelineClipActions'
import type { useTimelineContextMenus } from './useTimelineContextMenus'

type TimelineMenuActions = Pick<ReturnType<typeof useTimelineClipActions>,
  | 'addTrack'
  | 'removeItemById'
  | 'removeTrackById'
  | 'splitItemAt'
  | 'fadeItem'
  | 'trimItemTo'
  | 'reverseItem'
  | 'freezeItem'
  | 'stabilizeItem'
  | 'speedItem'
  | 'crossfadeAdjacent'
  | 'muteItem'
  | 'cleanVoiceItem'
  | 'blurFacesItem'
  | 'detachAudioItem'
  | 'splitEditItem'
  | 'replaceClipSource'
  | 'fitToFillAdjacent'
  | 'nestSelection'
>

interface TimelineContextMenuLayerProps {
  timeline: TimelineProps
  allItems: LaidItem[]
  durationMs: number
  menus: ReturnType<typeof useTimelineContextMenus>
  actions: TimelineMenuActions
  onPasteAttributes: (targetIds: string[]) => void
  onOpenTrim: (itemId: string, x: number, y: number) => void
}

/** Render-only owner for context surfaces. Timeline keeps gesture state and
 * action dispatch; this layer keeps their menu-specific prop wiring bounded. */
export default function TimelineContextMenuLayer({
  timeline,
  allItems,
  durationMs,
  menus,
  actions,
  onPasteAttributes,
  onOpenTrim,
}: TimelineContextMenuLayerProps) {
  const { clipMenu, surfaceMenu, assetPick, setAssetPick } = menus
  return <>
    {clipMenu && <ClipContextMenu
      menu={clipMenu}
      project={timeline.project}
      allItems={allItems}
      selectedClipIds={timeline.selectedClipIds}
      assetPick={assetPick}
      setAssetPick={setAssetPick}
      clipboardHasContent={timeline.clipboardHasContent}
      onPasteAttributes={onPasteAttributes}
      onOpenTrim={onOpenTrim}
      onClose={() => menus.setClipMenu(null)}
      onCopyClip={timeline.onCopyClip}
      onCutClip={timeline.onCutClip}
      onSelect={timeline.onSelect}
      onSeek={timeline.onSeek}
      removeItemById={actions.removeItemById}
      splitItemAt={actions.splitItemAt}
      fadeItem={actions.fadeItem}
      trimItemTo={actions.trimItemTo}
      reverseItem={actions.reverseItem}
      freezeItem={actions.freezeItem}
      stabilizeItem={actions.stabilizeItem}
      speedItem={actions.speedItem}
      crossfadeAdjacent={actions.crossfadeAdjacent}
      muteItem={actions.muteItem}
      cleanVoiceItem={actions.cleanVoiceItem}
      blurFacesItem={actions.blurFacesItem}
      detachAudioItem={actions.detachAudioItem}
      splitEditItem={actions.splitEditItem}
      replaceClipSource={actions.replaceClipSource}
      fitToFillAdjacent={actions.fitToFillAdjacent}
      nestSelection={actions.nestSelection}
    />}

    {surfaceMenu && (surfaceMenu.kind === 'track' || surfaceMenu.kind === 'locked' ? (
      <TimelineTrackContextMenu
        menu={surfaceMenu}
        project={timeline.project}
        allItems={allItems}
        onSelect={timeline.onSelect}
        onRemoveTrack={actions.removeTrackById}
        onClose={() => menus.setSurfaceMenu(null)}
      />
    ) : (
      <TimelineSurfaceContextMenu
        menu={surfaceMenu}
        project={timeline.project}
        allItems={allItems}
        clipboardClipId={timeline.clipboardClipId}
        clipboardKind={timeline.clipboardKind}
        durationMs={durationMs}
        onSeek={timeline.onSeek}
        onExportRange={timeline.onExportRange}
        onAddTrack={actions.addTrack}
        onPasteAt={(atMs, trackId) => timeline.onPasteClip({ atMs, trackId })}
        onClose={() => menus.setSurfaceMenu(null)}
      />
    ))}
  </>
}
