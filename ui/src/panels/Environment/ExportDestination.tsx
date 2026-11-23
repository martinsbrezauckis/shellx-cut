import { useEffect, useState } from 'react'
import { applyExportOutputDir, folderTail, refreshOutputFolder } from '../../lib/exportDestination'
import { useOutputFolderStatus } from '../../lib/useOutputFolder'
import { pickFolder } from '../../lib/tauri'
import { Availability, availabilitySentence, useAvailability } from '../../lib/availability'

export default function ExportDestination() {
  // The engine holds the destination; this panel only shows it and changes it.
  const chooser = useAvailability('shell.desktop')
  const status = useOutputFolderStatus()
  const dir = status?.dir ?? null
  const [note, setNote] = useState('')

  useEffect(() => { void refreshOutputFolder() }, [])

  const choose = async () => {
    setNote('')
    if (chooser.state === 'absent') {
      setNote(availabilitySentence('shell.desktop'))
      return
    }
    const picked = await pickFolder()
    if (!picked) return
    if (await applyExportOutputDir(picked)) {
      await refreshOutputFolder()
      setNote('Default export folder updated.')
    } else {
      setNote('Could not use that folder.')
    }
  }

  const clear = async () => {
    if (await applyExportOutputDir(null)) {
      await refreshOutputFolder()
      setNote('Using each project exports folder.')
    } else {
      setNote('Could not switch to the project exports folder.')
    }
  }

  return (
    <section className="env-export-destination" data-cut-export-default-folder data-cut-output-dir={dir ?? ''}>
      <div className="env-export-main">
        <div className="env-export-title" data-cut-export-default-heading>Default save folder</div>
        <div className="env-export-role">Exports and recordings use this by default. Save As can override one file.</div>
        <code className="env-export-path" title={dir ?? undefined}>
          {dir ? folderTail(dir) : 'Each project /exports folder'}
        </code>
        {status && !status.usable && (
          <div className="env-export-note" data-cut-export-default-problem role="alert">
            {status.problem} {status.suggested_action}
          </div>
        )}
      </div>
      <div className="env-export-actions">
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
        <Availability rule="shell.desktop">
          <button type="button" className="env-btn env-btn--secondary" data-cut-action="settings-export-folder" data-cut-export-default-pick onClick={() => void choose()}>
            Choose export folder
          </button>
        </Availability>
      </div>
      {note && <div className="env-export-note" data-cut-export-default-note>{note}</div>}
    </section>
  )
}
