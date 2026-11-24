// The same rows for the finished file: every applicable check, as it stood.
import ExportCheckList, { type ExportCheckListProps } from './ExportCheckList'

export type ExportCheckReceiptListProps = Omit<ExportCheckListProps, 'label' | 'phase'>

export default function ExportCheckReceiptList(props: ExportCheckReceiptListProps) {
  return (
    <div className="ecw ecw--receipt" data-cut-export-check-receipt>
      <ExportCheckList {...props} phase="finished_file" label="Checks on this file" />
    </div>
  )
}
