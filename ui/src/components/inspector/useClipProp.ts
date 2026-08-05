// components/inspector/useClipProp — the seed-and-commit hook behind the Transform
// section. It owns the bridge between
// a clip's stored transform and the verb: SEED each property from `sel.clip.*`,
// and COMMIT-ON-RELEASE → `callVerb('edit.transform', …)` + the composed-preview
// refresh the Inspector already uses after a visual verb.
//
// ── WHY A MULTI-FIELD VECTOR, NOT ONE FIELD PER ROW ───────────────────────────
// `edit.transform` is a SET verb over the whole transform {x,y,scale,opacity}:
// every call replaces the clip's transform, and identity (0,0,1,1) CLEARS it
// (verified `dispatch.rs:4250` + `schema/verbs.json`). So committing one row must
// send the OTHER three current values too, or they'd snap to their defaults. This
// hook therefore holds the full transform vector, lets a row commit ONE field, and
// fires the verb with the merged vector. (PropertyRow still owns the smooth-drag
// local draft; this hook only handles the committed snapshot + the verb.)
//
// ── REFRESH ───────────────────────────────────────────────────────────────────
// After a successful verb it dispatches `cut:show-composed` — the SAME event the
// Inspector's `applyVisual` (Inspector/index.tsx:108) and GradeDrawer use to flip
// the Preview to the composed (actually-transformed) frame. A transform only shows
// on the composed frame, never the raw proxy, so this is load-bearing for feedback.
//
// Deps: react (useMemo/useState/useEffect/useCallback), ../../lib/client (callVerb
// + ClipTransform). Callers: panels/Inspector (Transform section).

import { useCallback, useEffect, useMemo, useState } from 'react'
import { callVerb, type ClipCrop, type ClipTransform } from '../../lib/client'

/** The identity transform — what a cleared transform reads as, and what each row's
 *  reset returns to. Mirrors cut-core defaults (0,0,1,1). opacity defaults to 1
 *  (full-frame, opaque); a missing stored opacity is treated as 1. */
export const TRANSFORM_IDENTITY: Required<ClipTransform> = { x: 0, y: 0, scale: 1, opacity: 1 }

/** The four transform fields a row can drive. */
export type TransformField = 'x' | 'y' | 'scale' | 'opacity'

/** What useClipTransform returns to the Transform section. */
export interface ClipTransformState {
  /** The current committed transform vector (seeded from the clip, updated on each
   *  commit). Always fully populated (opacity coerced to 1 when absent). */
  transform: Required<ClipTransform>
  /** Commit ONE field → fires edit.transform with the merged vector + refreshes the
   *  composed preview. The other three fields ride along at their current values so
   *  the SET verb doesn't reset them. Returns the promise so callers can chain. */
  commitField: (field: TransformField, value: number) => Promise<void>
  /** Reset the WHOLE section to identity (0,0,1,1) → clears the clip's transform. */
  resetAll: () => Promise<void>
  /** True while a transform verb is in flight (lets the section disable rows). */
  busy: boolean
}

/**
 * Seed + commit hook for a clip's overlay transform.
 *
 * @param clipId The selected clip id (null when nothing applicable is selected).
 * @param stored The clip's stored transform (`sel.clip.transform`) or null/undefined
 *   when identity — the hook seeds from it and coerces a missing opacity to 1.
 * @returns {ClipTransformState} the current vector + commit/reset actions.
 *
 * Side effects: `commitField`/`resetAll` call `edit.transform` and dispatch the
 * `cut:show-composed` DOM event on success. The hook re-seeds whenever `clipId` or
 * the stored vector changes (selection change, verb result snapshot).
 */
export function useClipTransform(
  clipId: string | null,
  stored: ClipTransform | null | undefined,
): ClipTransformState {
  // Seed the vector from the clip (or identity). Coerce a missing opacity to 1 so
  // the Opacity row always shows a concrete value.
  const seed = useMemo<Required<ClipTransform>>(
    () =>
      stored
        ? {
            x: stored.x ?? TRANSFORM_IDENTITY.x,
            y: stored.y ?? TRANSFORM_IDENTITY.y,
            scale: stored.scale ?? TRANSFORM_IDENTITY.scale,
            opacity: stored.opacity ?? TRANSFORM_IDENTITY.opacity,
          }
        : { ...TRANSFORM_IDENTITY },
    [stored],
  )

  const [transform, setTransform] = useState<Required<ClipTransform>>(seed)
  const [busy, setBusy] = useState(false)

  // Re-seed when the selection or the stored vector changes (a new clip, or a
  // verb result landing a fresh transform). clipId in deps so re-selecting the
  // SAME stored shape on a different clip still re-seeds.
  useEffect(() => {
    setTransform(seed)
  }, [seed, clipId])

  /** Fire edit.transform with `next` and refresh the composed preview on success.
   *  On failure we keep the optimistic local vector (the next snapshot corrects
   *  it); the row just won't show a composed frame. */
  const fire = useCallback(
    async (next: Required<ClipTransform>, rationale: string) => {
      if (!clipId) return
      setBusy(true)
      try {
        const r = await callVerb('edit.transform', {
          clip: clipId,
          x: next.x,
          y: next.y,
          scale: next.scale,
          opacity: next.opacity,
          rationale,
        })
        if (r.ok) {
          // Same refresh the Inspector's applyVisual + GradeDrawer use: a transform
          // only shows on the COMPOSED frame, so flip the Preview to composed.
          document.dispatchEvent(new CustomEvent('cut:show-composed'))
        }
      } finally {
        setBusy(false)
      }
    },
    [clipId],
  )

  const commitField = useCallback(
    async (field: TransformField, value: number) => {
      // Merge the changed field onto the current vector → the full SET verb.
      const next = { ...transform, [field]: value }
      setTransform(next)
      await fire(next, `inspector: transform ${field} = ${value}`)
    },
    [transform, fire],
  )

  const resetAll = useCallback(async () => {
    const next = { ...TRANSFORM_IDENTITY }
    setTransform(next)
    await fire(next, 'inspector: reset transform')
  }, [fire])

  return { transform, commitField, resetAll, busy }
}

// ─────────────────────────────────────────────────────────────────────────────
// useClipCrop — the seed-and-commit hook behind the Cropping section.
//
// ── WHY A FULL-RECT SET VERB (mirrors useClipTransform's vector pattern) ──────
// `edit.crop` is a SET verb over the WHOLE rectangle: it REQUIRES {clip,x,y,w,h}
// every call (verified schema/verbs.json + dispatch.rs). Values are SOURCE PIXELS
// — the rect of the source frame to KEEP (NOT normalized, NOT L/R/T/B insets):
// x,y = top-left of the kept rect, w,h = its size. An IDENTITY crop (origin 0,0 +
// full source size) CLEARS the stored crop (engine stores no crop). So committing
// one edge must carry the other three current values, exactly like the transform
// vector. This hook holds the full {x,y,w,h}, clamps it inside the source geometry
// (the engine hard-errors on out-of-bounds), commits one field at a time, and
// fires the verb with the merged + clamped rect.
//
// ── SOURCE GEOMETRY GATE ──────────────────────────────────────────────────────
// Crop lives in source space, so the rows need the asset's source pixel
// dimensions (from `project.assets[asset].probe.{width,height}`). Until the asset
// is probed those are unknown — the caller passes `dims=null` and the Cropping
// section shows a "probe pending" hint instead of unbounded sliders (the same
// guard the Layer drawer uses, panels/Layer/index.tsx:139).

/** Source pixel geometry the crop rect must stay inside. */
export interface SourceDims { w: number; h: number }

/** The four crop fields a row can drive (source px). */
export type CropField = 'x' | 'y' | 'w' | 'h'

/** What useClipCrop returns to the Cropping section. */
export interface ClipCropState {
  /** Current committed crop rect (source px), seeded from the clip or identity
   *  (whole source frame). Always fully populated against `dims`. */
  crop: Required<ClipCrop>
  /** Commit ONE field → clamp the merged rect inside `dims` and fire edit.crop
   *  (+ composed-preview refresh). The other three ride along. */
  commitField: (field: CropField, value: number) => Promise<void>
  /** Reset to the whole source frame (identity = clears the stored crop). */
  resetAll: () => Promise<void>
  /** True while a crop verb is in flight. */
  busy: boolean
}

/** Clamp a candidate rect inside [0,dims] keeping w/h >= 1 and x+w<=W, y+h<=H —
 *  the engine's bounds (x+w<=width, y+h<=height); out-of-bounds is a hard error. */
function clampRect(r: Required<ClipCrop>, dims: SourceDims): Required<ClipCrop> {
  const w = Math.max(1, Math.min(Math.round(r.w), dims.w))
  const x = Math.max(0, Math.min(Math.round(r.x), dims.w - w))
  const h = Math.max(1, Math.min(Math.round(r.h), dims.h))
  const y = Math.max(0, Math.min(Math.round(r.y), dims.h - h))
  return { x, y, w, h }
}

/**
 * Seed + commit hook for a clip's SOURCE crop rectangle.
 *
 * @param clipId The selected clip id (null when nothing applicable is selected).
 * @param stored The clip's stored crop (`sel.clip.crop`) or null/undefined (= whole
 *   frame). Seeds w/h from the source dims when no crop is stored.
 * @param dims The asset's source pixel geometry, or null until the asset is probed
 *   (the caller hides the rows + shows a "probe pending" hint when null).
 * @returns {ClipCropState} the current rect + commit/reset actions.
 *
 * Side effects: `commitField`/`resetAll` call `edit.crop` and dispatch
 * `cut:show-composed` on success. Re-seeds when clipId / stored / dims change.
 */
export function useClipCrop(
  clipId: string | null,
  stored: ClipCrop | null | undefined,
  dims: SourceDims | null,
): ClipCropState {
  // Seed the rect from the clip, or the whole source frame when no crop is stored.
  // Falls back to a 0×0 placeholder when dims are unknown (rows are hidden then).
  const seed = useMemo<Required<ClipCrop>>(() => {
    const W = dims?.w ?? 0
    const H = dims?.h ?? 0
    return stored
      ? { x: stored.x, y: stored.y, w: stored.w, h: stored.h }
      : { x: 0, y: 0, w: W, h: H }
  }, [stored, dims])

  const [crop, setCrop] = useState<Required<ClipCrop>>(seed)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    setCrop(seed)
  }, [seed, clipId])

  const fire = useCallback(
    async (next: Required<ClipCrop>, rationale: string) => {
      if (!clipId) return
      setBusy(true)
      try {
        const r = await callVerb('edit.crop', {
          clip: clipId,
          x: next.x,
          y: next.y,
          w: next.w,
          h: next.h,
          rationale,
        })
        if (r.ok) document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } finally {
        setBusy(false)
      }
    },
    [clipId],
  )

  const commitField = useCallback(
    async (field: CropField, value: number) => {
      if (!dims) return
      const next = clampRect({ ...crop, [field]: value }, dims)
      setCrop(next)
      await fire(next, `inspector: crop ${field} = ${next[field]}`)
    },
    [crop, dims, fire],
  )

  const resetAll = useCallback(async () => {
    if (!dims) return
    // Identity = whole source frame → clears the stored crop server-side.
    const next: Required<ClipCrop> = { x: 0, y: 0, w: dims.w, h: dims.h }
    setCrop(next)
    await fire(next, 'inspector: reset crop')
  }, [dims, fire])

  return { crop, commitField, resetAll, busy }
}
