// The body of the export warning: one header line that counts what needs a look,
// then the list. The caller decides whether the warning opens at all, with
// exportChecksOpenWarning (found or error, never not-checked alone).
import { buildExportCheckView, exportChecksOpenWarning } from './model'
import ExportCheckList, { type ExportCheckListProps } from './ExportCheckList'

export { exportChecksOpenWarning }

export type ExportCheckWarningBodyProps = Omit<ExportCheckListProps, 'label' | 'phase'>

export default function ExportCheckWarningBody(props: ExportCheckWarningBodyProps) {
  const view = buildExportCheckView(props.checks, 'before_export')
  return (
    <div className="ecw" data-cut-export-check-warning data-look-count={view.lookCount}>
      <p className="ecw-head" data-cut-export-check-header>{view.header}</p>
      <ExportCheckList {...props} phase="before_export" label="Export checks" />
    </div>
  )
}
