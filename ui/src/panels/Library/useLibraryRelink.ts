import { useCallback, type Dispatch, type SetStateAction } from 'react'
import { availabilitySentence, useAvailability } from '../../lib/availability'
import { callVerb, type LibItem } from '../../lib/client'
import { pickLibraryRelinkMedia } from '../../lib/tauri'

interface LibraryRelinkOptions {
  busy: string | null
  setBusy: Dispatch<SetStateAction<string | null>>
  setError: Dispatch<SetStateAction<string | null>>
  flash: (message: string) => void
  reload: () => void
}

export function useLibraryRelink({
  busy,
  setBusy,
  setError,
  flash,
  reload,
}: LibraryRelinkOptions) {
  const relinkAvailability = useAvailability('shell.desktop')
  return useCallback(
    async (item: LibItem) => {
      if (!item.src_path || item.blob || item.media_ok !== false || busy) return
      if (relinkAvailability.state === 'absent') {
        setError(availabilitySentence('shell.desktop'))
        return
      }
      const path = await pickLibraryRelinkMedia()
      if (!path) return
      setBusy(item.id)
      setError(null)
      const result = await callVerb('library.relink', { id: item.id, path })
      setBusy(null)
      if (result.ok) {
        flash(`Relinked "${item.name}"`)
      } else {
        const recovery = result.error?.suggested_action
        setError(`${result.error?.message ?? 'Could not relink this media'}${recovery ? `. ${recovery}` : ''}`)
      }
      reload()
    },
    [busy, flash, relinkAvailability.state, reload, setBusy, setError],
  )
}
