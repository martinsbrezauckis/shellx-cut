// components/inspector/InspectorSection — a collapsible titled section for the
// "section stack" container. Each
// Inspector tool group (Transform, Cropping, Speed, Effects…) lives in one of
// these so the Video tab reads as a uniform stack of collapsible sections.
//
// Anatomy: a header row with a disclosure caret (▾/▸), the title, an optional
// BYPASS dot (●) that toggles the whole section's effect on/off, and a section
// RESET (↺). The body collapses on click. Purely PRESENTATIONAL + structural:
// bypass and reset are OPTIONAL callbacks the caller wires to verbs — the section
// itself fires nothing (agent-first: verbs stay the only writers).
//
// This is a cosmetic wrapper around the existing preset groups (no
// behavior change). The bypass/reset wiring is opt-in per section as later phases
// hook real verbs (e.g. a Stabilization bypass → edit.stabilize{enabled}).
//
// MODULE SCOPE: like every Inspector primitive, declared at top level so it never
// remounts its children mid-interaction (matches PropertyRow's invariant).
//
// Deps: react (useState). Styling via the shared `is-*` classes in inspector.css.
// Callers: panels/Inspector.

import { useState, type ReactNode } from 'react'
import { Icon } from '../../icons'

/** Props for one collapsible Inspector section. */
export interface InspectorSectionProps {
  /** Section title (e.g. "Transform"). */
  title: string
  /** Optional hover explanation for a domain-specific section title. */
  titleHint?: string
  /** Section body — the rows/chips it groups. */
  children: ReactNode
  /** Start collapsed? Default expanded (the common tools are open by default). */
  defaultCollapsed?: boolean
  /** Concise current-state/capability text that remains visible when collapsed. */
  summary?: ReactNode
  /** Optional semantic tone for an applied or blocked summary. */
  summaryTone?: 'neutral' | 'active' | 'warning'
  /** When provided, render the BYPASS dot (●). `bypassed` is its current state;
   *  `onToggleBypass` flips it. Omit both to hide the dot (sections with no
   *  on/off concept, e.g. Effects). */
  bypassed?: boolean
  /** Toggle the section's bypass (the caller maps this to the relevant verb). */
  onToggleBypass?: () => void
  /** When provided, render the section RESET (↺) — clears every property in the
   *  section back to its default (the caller fires the clearing verb(s)). */
  onReset?: () => void
  /** data-cut-* selector stem for the gate + agent (e.g. "transform"). Stamps
   *  `data-cut-section`, `-toggle`, `-bypass`, `-reset`. */
  sectionKey: string
}

/**
 * A collapsible Inspector section with an optional bypass dot + reset. MODULE
 * SCOPE (see header). Holds only its own collapsed state; bypass/reset are the
 * caller's verbs.
 */
export default function InspectorSection({
  title,
  titleHint,
  children,
  defaultCollapsed = false,
  summary,
  summaryTone = 'neutral',
  bypassed,
  onToggleBypass,
  onReset,
  sectionKey,
}: InspectorSectionProps) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed)
  const hasBypass = typeof bypassed === 'boolean' && !!onToggleBypass

  return (
    <section
      className={`is${collapsed ? ' is--collapsed' : ''}`}
      data-cut-section={sectionKey}
      data-cut-section-collapsed={collapsed ? 'true' : 'false'}
    >
      <header className="is__head">
        {/* Disclosure: caret + title toggles the body. */}
        <button
          type="button"
          className="is__disclosure"
          data-cut-section-toggle={sectionKey}
          aria-expanded={!collapsed}
          title={titleHint}
          onClick={() => setCollapsed((v) => !v)}
        >
          <span className="is__caret" aria-hidden="true">
            <Icon name={collapsed ? 'chevronRight' : 'chevronDown'} size={14} />
          </span>
          <span className="is__heading">
            <span className="is__title">{title}</span>
            {summary && (
              <span
                className={`is__summary is__summary--${summaryTone}`}
                data-cut-section-summary={sectionKey}
                data-cut-section-summary-tone={summaryTone}
              >
                {summary}
              </span>
            )}
          </span>
        </button>

        <div className="is__head-actions">
          {/* Bypass dot — filled when ACTIVE (not bypassed), hollow when bypassed. */}
          {hasBypass && (
            <button
              type="button"
              className={`is__bypass${bypassed ? ' is__bypass--off' : ''}`}
              data-cut-section-bypass={sectionKey}
              data-cut-section-bypassed={bypassed ? 'true' : 'false'}
              aria-label={bypassed ? `Enable ${title}` : `Bypass ${title}`}
              aria-pressed={!bypassed}
              title={bypassed ? `Enable ${title}` : `Bypass ${title}`}
              onClick={onToggleBypass}
            >
              {bypassed ? '○' : '●'}
            </button>
          )}
          {/* Section reset — clears the section's properties (caller fires verbs). */}
          {onReset && (
            <button
              type="button"
              className="is__reset"
              data-cut-section-reset={sectionKey}
              aria-label={`Reset ${title}`}
              title={`Reset ${title}`}
              onClick={onReset}
            >
              <Icon name="reset" size={14} />
            </button>
          )}
        </div>
      </header>

      {!collapsed && <div className="is__body">{children}</div>}
    </section>
  )
}
