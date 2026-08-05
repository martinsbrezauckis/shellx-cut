// custom.ts — the 6 editor-specific glyphs Lucide doesn't cover, authored on
// Lucide's EXACT spec so they sit in the same family as native icons:
//   24×24 viewBox · 2px stroke · currentColor · round caps/joins · ≥1px padding.
// Built via `createLucideIcon`, so each is a real `LucideIcon` component that
// honours the same `size` / `strokeWidth` / `absoluteStrokeWidth` props as native
// Lucide icons — the <Icon> wrapper can't tell them apart. Gaps verified against
// the icon coverage matrix: mask, matte/bg-removal, ripple-delete,
// crossfade/transition, scrub/playhead, stabilize.
//
// Two conventions copied verbatim from native Lucide iconNodes (or React warns):
//   • every element carries a unique `key` (createLucideIcon does NOT auto-key).
//   • SVG attrs are React/camelCase — `strokeDasharray`, not `stroke-dasharray`.
// Drawn from Lucide primitives where possible so the line language matches; all 6
// were render-verified at 14/16/20px against native icons on a dark sheet.
import { createLucideIcon } from "lucide-react";

/** Playhead / scrub — a flat-top pentagon marker over a vertical stem (NLE cursor). */
export const PlayheadScrub = createLucideIcon("PlayheadScrub", [
  ["path", { d: "M7 3h10v5l-5 4-5-4z", key: "ph-head" }], // pentagon head, point at (12,12)
  ["line", { x1: "12", y1: "12", x2: "12", y2: "21", key: "ph-stem" }], // stem
]);

/** Ripple-delete — a dashed removed seam with two arrows closing the gap. */
export const RippleDelete = createLucideIcon("RippleDelete", [
  ["line", { x1: "12", y1: "3", x2: "12", y2: "21", strokeDasharray: "2 2", key: "rd-seam" }],
  ["path", { d: "M3 12h5m-2-2 2 2-2 2", key: "rd-l" }], // left arrow → into the seam
  ["path", { d: "M21 12h-5m2-2-2 2 2 2", key: "rd-r" }], // right arrow ← into the seam
]);

/** Crossfade / transition — two wedges meeting at centre (the classic bowtie). */
export const Crossfade = createLucideIcon("Crossfade", [
  ["path", { d: "M3 5 11 12 3 19Z", key: "xf-l" }], // left wedge ▷
  ["path", { d: "M21 5 13 12 21 19Z", key: "xf-r" }], // right wedge ◁
]);

/** Mask — one shape clipping another (rounded rect + overlapping circle). */
export const Mask = createLucideIcon("Mask", [
  ["rect", { x: "3", y: "3", width: "13", height: "13", rx: "1.5", key: "mk-r" }],
  ["circle", { cx: "16", cy: "16", r: "5", key: "mk-c" }],
]);

/** Matte / background-removal — subject kept (solid), background removed (dashed). */
export const Matte = createLucideIcon("Matte", [
  ["rect", { x: "3", y: "3", width: "18", height: "18", rx: "2", strokeDasharray: "3 3", key: "mt-f" }],
  ["circle", { cx: "12", cy: "10", r: "3", key: "mt-h" }], // subject head
  ["path", { d: "M7 19a5 5 0 0 1 10 0", key: "mt-s" }], // subject shoulders
]);

/** Stabilize — a locked frame: corner brackets + centre crosshair. */
export const Stabilize = createLucideIcon("Stabilize", [
  ["path", { d: "M4 8V5a1 1 0 0 1 1-1h3", key: "st-tl" }], // top-left bracket
  ["path", { d: "M16 4h3a1 1 0 0 1 1 1v3", key: "st-tr" }], // top-right
  ["path", { d: "M20 16v3a1 1 0 0 1-1 1h-3", key: "st-br" }], // bottom-right
  ["path", { d: "M8 20H5a1 1 0 0 1-1-1v-3", key: "st-bl" }], // bottom-left
  ["path", { d: "M12 10v4M10 12h4", key: "st-x" }], // centre crosshair
]);
