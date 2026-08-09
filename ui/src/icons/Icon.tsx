// Icon.tsx — the ONLY icon entry point in ShellX Cut.
//
// Every glyph in the app renders through <Icon name=… />. This is what makes the
// icon system normalized by construction: the wrapper
// bakes in the family discipline so no surface can drift —
//   • strokeWidth 2 + absoluteStrokeWidth  → hairlines stay visually identical at
//     14/16/18/20px (a 14px timeline icon and a 20px toolbar icon match).
//   • size constrained to the type 14|16|18|20 → no rogue 13px / 22px icons.
//   • aria-hidden by default (icons are decorative); pass `label` to make an
//     icon-only control accessible (role=img + aria-label).
// Native Lucide + the 6 custom NLE glyphs resolve through one typed registry, so
// callers reference intent ("split", "record"), never a vendor component.
import type { CSSProperties } from "react";
import { REGISTRY, type IconName } from "./registry";

/** Cut's allowed icon sizes — anything else is a type error (kept uniform on purpose). */
export type IconSize = 14 | 16 | 18 | 20;

/**
 * Semantic colour tone: without a restrained colour
 * colouring it's super hard to see icons in daylight, and some feel generic").
 *
 * The palette is otherwise monochrome (white is the accent), so a thin-stroke grey
 * icon on a bright screen reads as invisible AND undifferentiated. A small, RESTRAINED
 * set of tones colours icons by FUNCTION so they're legible at a glance and distinct
 * (Library is no longer "another grey glyph"). `default` keeps the inherited
 * currentColor — unchanged behaviour for the dense timeline glyphs where colour would
 * be noise. Tones resolve to CSS vars in theme.css (`.cut-ic--<tone>`).
 */
export type IconTone =
  | "default"
  | "brand" // primary / agent / active — Cut blue
  | "media" // video / film / clips
  | "audio" // mixer / music / mic
  | "asset" // library / assets / import — TEAL (cool, distinct from media/audio; not gold)
  | "record" // capture / live
  | "success"
  | "warn"
  | "danger";

export interface IconProps {
  /** Semantic glyph name from the registry (autocompleted + build-checked). */
  name: IconName;
  /** One of Cut's four sizes. Default 16 (the dense-UI workhorse). */
  size?: IconSize;
  /** Semantic colour tone — default keeps the inherited currentColor. */
  tone?: IconTone;
  /** When set, the icon is meaningful (icon-only button): exposes an accessible label. */
  label?: string;
  className?: string;
  style?: CSSProperties;
  /** Optional click for icon-only affordances (the wrapper stays presentational otherwise). */
  onClick?: () => void;
}

/**
 * Render a registry glyph with Cut's locked stroke/size discipline.
 *
 * @example <Icon name="split" />                    // 16px decorative
 * @example <Icon name="record" size={18} label="Record" />  // accessible icon-button
 */
export function Icon({ name, size = 16, tone = "default", label, className, style, onClick }: IconProps) {
  const Glyph = REGISTRY[name];
  const a11y = label
    ? { role: "img" as const, "aria-label": label }
    : { "aria-hidden": true as const };
  // `cut-ic` is the base class (a small daylight-contrast lift for every glyph);
  // `cut-ic--<tone>` adds the semantic colour. Both resolve in theme.css.
  const cls = ["cut-ic", tone !== "default" && `cut-ic--${tone}`, className]
    .filter(Boolean)
    .join(" ");
  return (
    <Glyph
      size={size}
      strokeWidth={2}
      absoluteStrokeWidth
      className={cls}
      style={style}
      onClick={onClick}
      {...a11y}
    />
  );
}

export type { IconName };
