// BrandMark.tsx — the canonical ShellX CUT brand lockup (NOT a registry icon).
//
// This is the app's identity mark, not a UI glyph: the traced ShellX X (path
// shared with branding/shellx-cut-icon.svg, DO NOT redraw) rendered in
// --ink, struck by the editor-blue (--cut) NLE playhead (top handle + vertical
// line). The strike is widened so it
// survives 20px: at 20px / 512 viewBox a 1px screen line ≈ 26 viewBox units.
//
// Lives under ui/src/icons/ (exempt from the icon tripwire) because it is a
// one-off SVG with brand-specific paths + the --ink/--cut fills — it does not
// belong in the semantic <Icon> registry (which is uniform-stroke Lucide
// glyphs). "Reuse verbatim, never redraw": the paths below are unchanged from
// the original topbar inline SVG.
//
// Caller: topbar/index.tsx (the 54px app top bar brand slot).

/** The ShellX CUT brand mark — 20px inline SVG, fixed paths, theme-token fills. */
export function BrandMark() {
  return (
    <svg width="20" height="20" viewBox="0 0 512 512" aria-hidden="true" data-cut-brand>
      <path
        d="M74 90 L200 90 L256 168 L312 90 L438 90 L330 256 L438 422 L312 422 L256 344 L200 422 L74 422 L182 256 Z"
        fill="var(--ink)"
      />
      {/* playhead strike: NLE top handle + vertical line, editor blue */}
      <path d="M218 10 L294 10 L294 52 L256 92 L218 52 Z" fill="var(--cut)" />
      <rect x="243" y="52" width="26" height="450" fill="var(--cut)" />
    </svg>
  )
}
