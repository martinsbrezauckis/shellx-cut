// lib/themePref.ts — persisted UI colour theme (dark default, light optional).
//
// ShellX Cut ships a warm near-black dark theme (theme.css :root). A LIGHT theme is
// an additive `[data-theme="light"]` token override on <html>: the whole UI is
// token-driven (var(--bg/--ink/--surface/…)), so flipping the attribute re-skins
// every surface without touching component markup. This module is the single
// source of truth for that choice, mirroring proxyPref.ts / chatAgentPref.ts:
//   • DARK is the default (and the absence of the attribute = the :root dark theme).
//   • the choice is localStorage-backed so it survives reloads.
//   • applyTheme() reflects the choice onto document.documentElement so CSS picks
//     it up; initTheme() is called from main.tsx BEFORE React renders so there is
//     no dark→light flash on a light-theme reload.
//
// Callers: main.tsx (initTheme on boot), components/ThemeToggle.tsx (get/set).

export type ThemeName = 'dark' | 'light'

const KEY = 'cut.theme'
const DEFAULT: ThemeName = 'dark'
export const THEME_CHANGE_EVENT = 'cut:theme-changed'

/** The persisted theme, or dark if none/invalid/storage-unavailable. */
export function getTheme(): ThemeName {
  try {
    const v = localStorage.getItem(KEY)
    if (v === 'light' || v === 'dark') return v
  } catch {
    /* storage unavailable (private mode / quota) — use the default */
  }
  return DEFAULT
}

/** Persist the choice (best-effort) AND apply it immediately. */
export function setTheme(name: ThemeName): void {
  try {
    localStorage.setItem(KEY, name)
  } catch {
    /* storage unavailable — still apply for this session, just don't remember */
  }
  applyTheme(name)
  document.dispatchEvent(new CustomEvent<ThemeName>(THEME_CHANGE_EVENT, { detail: name }))
}

/**
 * Reflect the theme on <html data-theme>. Dark = NO attribute (the :root default),
 * so the dark path is the absence of any override and stays pixel-identical.
 */
export function applyTheme(name: ThemeName): void {
  const el = document.documentElement
  if (name === 'light') el.setAttribute('data-theme', 'light')
  else el.removeAttribute('data-theme')
}

/** Read + apply the stored theme. Call once on boot (main.tsx) before first paint. */
export function initTheme(): ThemeName {
  const t = getTheme()
  applyTheme(t)
  return t
}
