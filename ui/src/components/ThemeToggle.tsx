// components/ThemeToggle.tsx — light/dark colour-theme switch.
//
// Two presentations of the SAME control (both drive lib/themePref):
//   • variant="icon"  — a compact topbar icon button (sun when light is active,
//     moon when dark), for at-a-glance switching.
//   • variant="row"   — a labelled segmented control for the Setup/Environment hub.
//
// The theme is a global side effect (an attribute on <html>), so the only React
// state here is the icon/label reflection; setTheme() does the persistence + apply.
// Relay-drivable: data-cut-theme-toggle + data-cut-theme expose state to the API.
//
// Callers: topbar/index.tsx (icon), panels/Environment/index.tsx (row).
import { useEffect, useState } from 'react'
import { Icon } from '../icons'
import { getTheme, setTheme, THEME_CHANGE_EVENT, type ThemeName } from '../lib/themePref'

export default function ThemeToggle({ variant = 'icon' }: { variant?: 'icon' | 'row' }) {
  const [theme, setLocal] = useState<ThemeName>(getTheme)

  useEffect(() => {
    const sync = (event: Event) => {
      const next = (event as CustomEvent<ThemeName>).detail
      setLocal(next === 'light' || next === 'dark' ? next : getTheme())
    }
    document.addEventListener(THEME_CHANGE_EVENT, sync)
    return () => document.removeEventListener(THEME_CHANGE_EVENT, sync)
  }, [])

  const apply = (next: ThemeName) => {
    setTheme(next)
  }
  const toggle = () => apply(theme === 'light' ? 'dark' : 'light')

  if (variant === 'row') {
    // Labelled segmented control for the settings drawer.
    return (
      <div className="theme-toggle-row" data-cut-theme={theme}>
        <span className="theme-toggle-label">Appearance</span>
        <div className="theme-seg" role="group" aria-label="Interface theme">
          <button
            type="button"
            className={`theme-seg__btn${theme === 'dark' ? ' theme-seg__btn--on' : ''}`}
            data-cut-theme-set="dark"
            aria-pressed={theme === 'dark'}
            onClick={() => apply('dark')}
          >
            <Icon name="themeDark" size={14} /> Dark
          </button>
          <button
            type="button"
            className={`theme-seg__btn${theme === 'light' ? ' theme-seg__btn--on' : ''}`}
            data-cut-theme-set="light"
            aria-pressed={theme === 'light'}
            onClick={() => apply('light')}
          >
            <Icon name="themeLight" size={14} /> Light
          </button>
        </div>
      </div>
    )
  }

  // Compact icon button (topbar). Shows the theme you'd switch TO is implied by the
  // CURRENT-state glyph: sun = light active, moon = dark active.
  return (
    <button
      type="button"
      className="tb-btn tb-btn--secondary tb-icon-btn tb-nav"
      data-cut-theme-toggle
      data-cut-theme={theme}
      aria-label={theme === 'light' ? 'Switch to dark theme' : 'Switch to light theme'}
      title={theme === 'light' ? 'Light theme — click for dark' : 'Dark theme — click for light'}
      onClick={(e) => {
        e.currentTarget.blur()
        toggle()
      }}
    >
      <Icon name={theme === 'light' ? 'themeLight' : 'themeDark'} size={16} />
    </button>
  )
}
