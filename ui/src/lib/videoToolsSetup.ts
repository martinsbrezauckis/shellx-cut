// Shared navigation for FFmpeg-dependent surfaces.
//
// Settings was split into explicit categories, so setup affordances must route
// to Video & performance before asking HighlightOverlay to find the FFmpeg card.

import { openCutManual } from './manual'

export function openVideoToolsSettings(): void {
  document.dispatchEvent(new CustomEvent('cut:open-ui-surface', {
    detail: { id: 'settings-video-performance' },
  }))
  window.setTimeout(() => {
    document.dispatchEvent(new CustomEvent('cut:local-highlight', {
      detail: {
        selector: '[data-cut-env-card="ffmpeg"]',
        label: 'Video processing',
        description: 'Install this first when preview, import, render, or export is unavailable.',
        duration_ms: 4500,
        scroll: true,
      },
    }))
  }, 260)
}

export function openVideoToolsGuide(): void {
  openCutManual('cut.preview.ffmpeg_setup')
}

export function recheckVideoTools(): void {
  document.dispatchEvent(new CustomEvent('cut:refresh-doctor'))
}
