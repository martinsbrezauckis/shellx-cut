import { mediaBasename } from '../lib/mediaPath'
import { Availability, useAvailability } from '../lib/availability'
import { pickFolder } from '../lib/tauri'

type FolderPickerKind = 'stock' | 'assemble' | 'portable'

interface NativeFolderPickerProps {
  kind: FolderPickerKind
  label: string
  dialogTitle: string
  value: string
  disabled?: boolean
  onChooseStart?: () => void
  onDesktopRequired: () => void
  onSelected: (path: string) => void
}

/** Native-only folder selection with path-light status text. */
export default function NativeFolderPicker({
  kind,
  label,
  dialogTitle,
  value,
  disabled = false,
  onChooseStart,
  onDesktopRequired,
  onSelected,
}: NativeFolderPickerProps) {
  const folderAvailability = useAvailability('shell.desktop')
  const wrapperProps = kind === 'stock'
    ? { 'data-cut-stock-dir-picker': '' }
    : kind === 'assemble'
      ? { 'data-cut-assemble-dir-picker': '' }
      : { 'data-cut-portable-dir-picker': '' }
  const buttonProps = kind === 'stock'
    ? { 'data-cut-stock-dir-choose': '' }
    : kind === 'assemble'
      ? { 'data-cut-assemble-dir-choose': '' }
      : { 'data-cut-portable-dir-choose': '' }
  const statusProps = kind === 'stock'
    ? { 'data-cut-stock-dir-status': '' }
    : kind === 'assemble'
      ? { 'data-cut-assemble-dir-status': '' }
      : { 'data-cut-portable-dir-status': '' }

  const choose = async () => {
    onChooseStart?.()
    if (folderAvailability.state === 'absent') {
      onDesktopRequired()
      return
    }
    const selected = await pickFolder({ title: dialogTitle })
    if (selected) onSelected(selected)
  }

  return (
    <div className="cd-field" {...wrapperProps}>
      <span className="cd-field-label">{label}</span>
      <div className="cd-row">
        <Availability rule="shell.desktop">
          <button className="cd-btn cd-btn--ghost" data-cut-action="native-folder-choose" {...buttonProps} type="button" disabled={disabled}
            onClick={() => void choose()}>{value ? 'Change folder…' : 'Choose folder…'}</button>
        </Availability>
        <span className="cd-note" {...statusProps}>
          {value ? mediaBasename(value) : (folderAvailability.state === 'absent' ? 'Desktop app required' : 'No folder selected')}
        </span>
      </div>
    </div>
  )
}
