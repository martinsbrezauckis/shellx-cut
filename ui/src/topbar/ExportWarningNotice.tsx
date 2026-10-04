export interface ExportWarningNoticeState {
  filename: string | null
  warnings: ReadonlyArray<{ code: string; message: string }>
}

/** A saved interchange file can be usable while still omitting edit state. */
export default function ExportWarningNotice({ filename, warnings }: ExportWarningNoticeState) {
  return (
    <aside className="tb-export-warnings" data-cut-export-warnings role="status" aria-live="polite">
      <strong>Exported with warnings</strong>
      {filename && <span className="tb-export-warnings__file" data-cut-export-warning-file>Saved {filename}</span>}
      <ul>
        {warnings.map((warning, index) => (
          <li key={`${warning.code}-${index}`} data-cut-export-warning>
            {warning.message}
          </li>
        ))}
      </ul>
    </aside>
  )
}
