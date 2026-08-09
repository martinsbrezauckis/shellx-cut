import type { ReactNode } from 'react'
import type { DoctorReport } from '../../lib/doctor'
import { SETTINGS_CATEGORIES, searchSettings, settingsCategory, type SettingsCategoryId } from './settingsModel'
import './settings-shell.css'

interface SettingsShellProps {
  active: SettingsCategoryId
  onActive: (category: SettingsCategoryId) => void
  query: string
  onQuery: (query: string) => void
  onRefresh: () => Promise<DoctorReport | null>
  onClose: () => void
  children: ReactNode
}

export default function SettingsShell({
  active,
  onActive,
  query,
  onQuery,
  onRefresh,
  onClose,
  children,
}: SettingsShellProps) {
  const current = settingsCategory(active)
  const results = searchSettings(query)
  const choose = (category: SettingsCategoryId) => {
    onActive(category)
    onQuery('')
  }

  return (
    <>
      <header className="settings-head">
        <div className="settings-heading">
          <h2 className="env-modal-title">Settings</h2>
          <p className="env-modal-sub">{current.description}</p>
        </div>
        <div className="settings-search-wrap">
          <label className="settings-search-label" htmlFor="cut-settings-search">Find a setting</label>
          <input
            id="cut-settings-search"
            className="settings-search"
            type="search"
            value={query}
            placeholder="Search settings"
            data-cut-settings-search
            onChange={(event) => onQuery(event.currentTarget.value)}
          />
        </div>
        <div className="env-drawer-actions settings-head-actions">
          <button className="env-btn env-btn--ghost" data-cut-environment-refresh onClick={onRefresh} title="Check tools and services again">
            Re-scan
          </button>
          <button className="env-btn env-btn--secondary" data-cut-environment-close onClick={onClose}>
            Close
          </button>
        </div>
      </header>

      <div className="settings-layout">
        <div className="settings-category-select-wrap">
          <label htmlFor="cut-settings-category">Settings category</label>
          <select
            id="cut-settings-category"
            value={active}
            data-cut-settings-category-select
            onChange={(event) => choose(event.currentTarget.value as SettingsCategoryId)}
          >
            {SETTINGS_CATEGORIES.map((category) => (
              <option key={category.id} value={category.id}>{category.label}</option>
            ))}
          </select>
        </div>
        <nav className="settings-nav" aria-label="Settings categories" data-cut-settings-categories>
          {SETTINGS_CATEGORIES.map((category) => (
            <button
              key={category.id}
              type="button"
              className={`settings-nav-item${active === category.id ? ' settings-nav-item--active' : ''}`}
              data-cut-settings-category={category.id}
              aria-current={active === category.id ? 'page' : undefined}
              onClick={() => choose(category.id)}
            >
              <span>{category.label}</span>
            </button>
          ))}
        </nav>

        <main
          className="settings-body"
          id={`settings-${active}`}
          data-cut-settings-body={active}
          tabIndex={-1}
        >
          {query.trim() ? (
            <section className="settings-search-results" aria-labelledby="settings-search-results-title">
              <h3 id="settings-search-results-title">Search results</h3>
              <p>{results.length} {results.length === 1 ? 'destination' : 'destinations'} for “{query.trim()}”</p>
              {results.length > 0 ? (
                <div className="settings-search-result-list">
                  {results.map(({ category, matched }) => (
                    <button
                      key={category.id}
                      type="button"
                      data-cut-settings-search-result={category.id}
                      onClick={() => choose(category.id)}
                    >
                      <strong>{category.label}</strong>
                      <span>{category.description}</span>
                      {matched !== category.label && matched !== category.description && <small>Matches “{matched}”</small>}
                    </button>
                  ))}
                </div>
              ) : (
                <div className="settings-empty-search">No setting matches that search.</div>
              )}
            </section>
          ) : children}
        </main>
      </div>
    </>
  )
}
