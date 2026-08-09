import { useEffect, useState } from 'react'
import { applyExportOutputDir, folderTail, getStoredOutputDir, setStoredOutputDir } from '../../lib/exportDestination'
import { isTauri, pickFolder } from '../../lib/tauri'

export default function ExportDestination() {
  const [dir, setDir] = useState<string | null>(() => getStoredOutputDir())
  const [note, setNote] = useState('')

  useEffect(() => {
    if (dir) void applyExportOutputDir(dir)
  }, [dir])

  const choose = async () => {
    setNote('')
    if (!isTauri()) {
      setNote('Use the desktop app to choose a folder.')
      return
    }
    const picked = await pickFolder()
    if (!picked) return
    if (await applyExportOutputDir(picked)) {
      setStoredOutputDir(picked)
      setDir(picked)
      setNote('Default export folder updated.')
    } else {
      setNote('Could not use that folder.')
    }
  }

  const clear = async () => {
    await applyExportOutputDir(null)
    setStoredOutputDir(null)
    setDir(null)
    setNote('Using each project exports folder.')
  }

  return (
    <section className="env-export-destination" data-cut-export-default-folder data-cut-output-dir={dir ?? ''}>
      <div className="env-export-main">
        <div className="env-export-title" data-cut-export-default-heading>Default save folder</div>
        <div className="env-export-role">Exports and recordings use this by default. Save As can override one file.</div>
        <code className="env-export-path" title={dir ?? undefined}>
          {dir ? folderTail(dir) : 'Each project /exports folder'}
        </code>
      </div>
      <div className="env-export-actions">
        <button type="button" className="env-btn env-btn--secondary" data-cut-export-default-pick onClick={() => void choose()}>
          Choose export folder
        </button>
        <button
          type="button"
          className="env-btn env-btn--ghost"
          data-cut-export-default-clear
          disabled={!dir}
          title={dir ? 'Use each project exports folder' : 'Already using each project exports folder'}
          onClick={() => void clear()}
        >
          Clear
        </button>
      </div>
      {note && <div className="env-export-note" data-cut-export-default-note>{note}</div>}
    </section>
  )
}
