// lib/dnd.ts — the drag contract between the Assets tray and the Timeline.
//
// We do NOT use HTML5 drag-and-drop: this app runs inside a Tauri WebView whose
// native OS drag-drop handler (dragDropEnabled, used for "drop a file from
// Explorer to import") INTERCEPTS drag events and suppresses the DOM
// dragstart/dragover/drop the HTML5 API needs — so a real mouse drag from an
// asset card never reached the timeline (only synthetic JS-dispatched DragEvents
// did, which masked the bug in testing). Instead the Assets tray runs a custom
// pointer drag (with a mouse-event compatibility lane for WKWebView) and
// broadcasts its position via these DOM CustomEvents; the Timeline listens,
// draws the insertion indicator on move, and dispatches ONE edit.insert on
// drop. These input events are not touched by Tauri's drag-drop layer.

/** Fired on every pointermove while an asset card is being dragged. */
export const ASSET_DRAG_MOVE = 'cut:asset-dragmove'
/** Fired on pointerup that ends an asset-card drag (the drop). */
export const ASSET_DRAG_DROP = 'cut:asset-drop'

/** detail payload for both events. clientX/clientY are viewport coords. */
export interface AssetDragDetail {
  asset: string
  /** Probe kind ("video" | "audio" | "image") — picks the default track. */
  kind: string
  clientX: number
  clientY: number
  /** Alt held at drop → place as an OVERLAY/new separate track. Normal drops
   *  insert into the base track unless the cursor is over an existing overlay
   *  lane. Carries the live modifier from pointerup. */
  alt?: boolean
}
