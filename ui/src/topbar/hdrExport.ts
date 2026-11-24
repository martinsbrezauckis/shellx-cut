// Project HDR timeline detection and 10-bit Keep HDR export eligibility
// for the TopBar render options menu (no dead controls; honest notices).

import type { Asset, ClipGrade, Project } from '../lib/client'
import type { DoctorReport } from '../lib/doctor'
import type { FileFormat } from './model'

export type ExportColorMode = 'sdr' | 'keep_hdr'

export interface HdrTimelineSummary {
  hasHdr: boolean
  targetLabel: 'HLG' | 'PQ' | null
  sdrFallbackNotices: string[]
}

export interface KeepHdrStatus {
  available: boolean
  reason: string | null
  summary: HdrTimelineSummary
  previewNote: string | null
  fallbackNotice: string | null
}

function asRecord(v: unknown): Record<string, unknown> | null {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null
}

function assetHdrKind(asset: Asset | undefined): 'hlg' | 'pq' | null {
  const probe = asRecord(asset?.probe)
  if (!probe) return null
  const raw = asRecord(probe.raw)
  const streams = Array.isArray(raw?.streams) ? raw.streams : []
  const video = streams.find((s) => {
    const rec = asRecord(s)
    const disp = asRecord(rec?.disposition)
    return rec?.codec_type === 'video' && (typeof disp?.attached_pic !== 'number' || disp.attached_pic === 0)
  })
  const vRec = asRecord(video)
  const trc = typeof vRec?.color_transfer === 'string'
    ? vRec.color_transfer.trim().toLowerCase()
    : typeof probe.color_transfer === 'string'
      ? probe.color_transfer.trim().toLowerCase()
      : ''
  if (trc === 'arib-std-b67') return 'hlg'
  if (trc === 'smpte2084') return 'pq'
  return null
}

function gradeIsIdentity(g: ClipGrade | null | undefined): boolean {
  if (!g) return true
  const approx = (a: number, b: number) => Math.abs(a - b) < 1e-6
  return (
    approx(g.contrast, 1)
    && approx(g.brightness, 0)
    && approx(g.saturation, 1)
    && approx(g.gamma, 1)
    && g.temperature_k == null
    && (g.lut == null || g.lut === '')
  )
}

function clipSdrOnlyFeatures(clip: {
  grade?: ClipGrade | null
  grade_stack?: ClipGrade[]
  grade_windows?: Array<{ grade: ClipGrade }>
  effects?: unknown[]
}): string[] {
  const features: string[] = []
  const add = (label: string) => {
    if (!features.includes(label)) features.push(label)
  }
  if (!gradeIsIdentity(clip.grade) || (clip.grade_stack ?? []).some((g) => !gradeIsIdentity(g))) {
    add('color grade')
  }
  if ((clip.grade_windows ?? []).some((w) => !gradeIsIdentity(w.grade))) {
    add('power-window grade')
  }
  for (const fx of clip.effects ?? []) {
    const rec = asRecord(fx)
    switch (rec?.type) {
      case 'vignette':
        add('vignette')
        break
      case 'sepia':
        add('sepia')
        break
      case 'auto_color':
        add('auto-color')
        break
      case 'posterize':
        add('posterize')
        break
    }
  }
  return features
}

function fileBase(path: string): string {
  const parts = path.split(/[/\\]/)
  return parts[parts.length - 1] || path
}

/** Inspect active video tracks for untagged HLG/PQ clips and SDR-only grades/effects. */
export function inspectTimelineHdr(project: Project | null): HdrTimelineSummary {
  if (!project) return { hasHdr: false, targetLabel: null, sdrFallbackNotices: [] }
  const activeVideoTracks = project.tracks.filter((t) => {
    if (t.kind !== 'video' || t.clips.length === 0) return false
    return t.visible !== false
  })
  let firstHdr: 'hlg' | 'pq' | null = null
  const notices: string[] = []
  for (const track of activeVideoTracks) {
    for (const clip of track.clips) {
      if (!('asset' in clip)) continue
      const asset = project.assets[clip.asset]
      const hdr = clip.input_color_space ? null : assetHdrKind(asset)
      if (hdr && !firstHdr) firstHdr = hdr
      const sdrFeatures = clipSdrOnlyFeatures(clip)
      if (sdrFeatures.length > 0 && asset) {
        const name = fileBase(asset.path)
        const entry = `${name} (${sdrFeatures.join(', ')})`
        if (!notices.includes(entry)) notices.push(entry)
      }
    }
  }
  const hasHdr = firstHdr !== null
  return {
    hasHdr,
    targetLabel: firstHdr === 'hlg' ? 'HLG' : firstHdr === 'pq' ? 'PQ' : null,
    sdrFallbackNotices: hasHdr ? notices : [],
  }
}

export function keepHdrAvailability(
  project: Project | null,
  format: FileFormat,
  doctor: DoctorReport | null,
): KeepHdrStatus {
  const summary = inspectTimelineHdr(project)
  const ffmpegDetails = doctor?.cards.find((c) => c.id === 'ffmpeg')?.details
  let reason: string | null = null
  if (!summary.hasHdr) {
    reason = 'Needs an HDR (PQ or HLG) clip on the timeline'
  } else if (format !== 'hevc' && format !== 'av1') {
    reason = 'Keep HDR needs H.265 (HEVC) or AV1 10-bit'
  } else if (ffmpegDetails?.zscale === false) {
    reason = 'Keep HDR needs an FFmpeg that can convert HDR (zimg); see Settings → Environment'
  } else if (format === 'hevc' && ffmpegDetails?.can_export_hevc_10bit === false) {
    reason = 'Keep HDR needs an FFmpeg that encodes 10-bit H.265 (HEVC)'
  } else if (format === 'av1' && ffmpegDetails?.can_export_av1_10bit === false) {
    reason = 'Keep HDR needs an FFmpeg that encodes 10-bit AV1'
  }
  const available = reason === null
  const previewNote = summary.hasHdr ? 'Preview shows HDR clips tone-mapped' : null
  const fallbackNotice = summary.hasHdr && summary.sdrFallbackNotices.length > 0
    ? `Grades/effects run in SDR and convert back to ${summary.targetLabel ?? 'HDR'}: ${summary.sdrFallbackNotices.join('; ')}.`
    : null
  return { available, reason, summary, previewNote, fallbackNotice }
}
