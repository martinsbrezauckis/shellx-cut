// Compact presentation for project.health's rebuildable editing-cache facts.

import type { ProjectEditingCache } from '../../lib/clientResults'
import type { HealthRow } from './healthRecoveryModel'

export function formatCacheBytes(bytes: number): string {
  const safe = Math.max(0, Number.isFinite(bytes) ? bytes : 0)
  if (safe < 1024) return `${Math.round(safe)} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = safe / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

export function formatCacheChange(modifiedMs: number | undefined, nowMs = Date.now()): string | null {
  if (modifiedMs == null || !Number.isFinite(modifiedMs)) return null
  const ageSeconds = Math.max(0, Math.floor((nowMs - modifiedMs) / 1000))
  if (ageSeconds < 60) return 'Latest cache change was just now.'
  const minutes = Math.floor(ageSeconds / 60)
  if (minutes < 60) return `Latest cache change was ${minutes} minute${minutes === 1 ? '' : 's'} ago.`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `Latest cache change was ${hours} hour${hours === 1 ? '' : 's'} ago.`
  const days = Math.floor(hours / 24)
  return `Latest cache change was ${days} day${days === 1 ? '' : 's'} ago.`
}

export function editingCacheRow(
  cache: ProjectEditingCache | undefined,
  hasProject: boolean,
  scanFailed: boolean,
  nowMs = Date.now(),
): HealthRow {
  if (!hasProject) return { id: 'cache', label: 'Editing cache', state: 'unsupported', summary: 'Open a project to inspect its rebuildable editing cache.', action: null }
  if (scanFailed) return { id: 'cache', label: 'Editing cache', state: 'attention', summary: 'The cache inventory did not complete. Check again.', action: null }
  if (!cache) return { id: 'cache', label: 'Editing cache', state: 'attention', summary: 'This engine did not report a cache inventory.', action: null }
  if (cache.files === 0 && cache.status === 'ready') {
    return { id: 'cache', label: 'Editing cache', state: 'healthy', summary: 'No rebuildable proxies or thumbnails are stored.', action: null }
  }
  const size = formatCacheBytes(cache.bytes)
  const latest = formatCacheChange(cache.latest_modified_ms, nowMs)
  const reclaimable = cache.reclaimable_files > 0
    ? `${cache.status === 'partial' ? 'At least ' : ''}${formatCacheBytes(cache.reclaimable_bytes)} across ${cache.reclaimable_files} file${cache.reclaimable_files === 1 ? '' : 's'} appears unreferenced and rebuildable.`
    : 'No unreferenced rebuildable cache files were found.'
  const minimumHours = Math.max(1, Math.round(cache.cleanup_preview.minimum_age_ms / 3_600_000))
  const retention = `${minimumHours}-hour safety window`
  let cleanupPreview: string
  if (cache.cleanup_preview.status === 'blocked') {
    cleanupPreview = 'Cleanup preview is blocked until the bounded scan completes without skipped entries.'
  } else if (cache.cleanup_preview.aged_unreferenced_files > 0) {
    const agedFiles = cache.cleanup_preview.aged_unreferenced_files
    cleanupPreview = `${formatCacheBytes(cache.cleanup_preview.aged_unreferenced_bytes)} across ${agedFiles} file${agedFiles === 1 ? '' : 's'} ${agedFiles === 1 ? 'has' : 'have'} not changed for at least ${minimumHours} hours. Active work must still be rechecked before any future removal.`
  } else if (cache.cleanup_preview.recent_unreferenced_files > 0) {
    const recentFiles = cache.cleanup_preview.recent_unreferenced_files
    cleanupPreview = recentFiles === 1
      ? `The unreferenced file remains inside the ${retention}.`
      : `All ${recentFiles} unreferenced files remain inside the ${retention}.`
  } else {
    cleanupPreview = `No files are waiting beyond the ${retention}.`
  }
  const detail = `${cache.files} cached file${cache.files === 1 ? '' : 's'}. ${reclaimable} ${cleanupPreview} ${latest ?? 'No file-change time was reported.'} Nothing is removed from this page.`
  if (cache.status === 'partial') {
    return { id: 'cache', label: 'Editing cache', state: 'attention', summary: `At least ${size} of rebuildable proxies and thumbnails was found.`, detail: `The bounded scan skipped or truncated entries. ${detail}`, action: null }
  }
  return { id: 'cache', label: 'Editing cache', state: 'healthy', summary: `${size} of rebuildable proxies and thumbnails.`, detail, action: null }
}
