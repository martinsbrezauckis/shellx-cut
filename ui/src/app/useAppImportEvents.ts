import { useEffect, useRef, type Dispatch, type SetStateAction } from 'react'
import { callVerb, type Project } from '../lib/client'
import { getGenerateProxies } from '../lib/proxyPref'
import { availabilitySentence, useAvailability } from '../lib/availability'
import { pickMedia } from '../lib/tauri'
import { publishUserActionFailure, publishUserActionMessage, TOOL_MISSING } from '../lib/userActionFeedback'
import type { LayoutState } from '../layout/useLayout'

interface AppImportEventsArgs {
  project: Project | null
  onChanged?: () => void
  setLayout: Dispatch<SetStateAction<LayoutState>>
}

/** Bridges every cut:open-import affordance to the same real media.import path. */
export function useAppImportEvents({ project, onChanged, setLayout }: AppImportEventsArgs) {
  const importAvailability = useAvailability('shell.desktop')
  const projectRef = useRef(project)
  projectRef.current = project
  const onChangedRef = useRef(onChanged)
  onChangedRef.current = onChanged

  useEffect(() => {
    const openAssets = () => setLayout((l) => ({ ...l, leftTab: 'assets', leftCollapsed: false }))
    const onOpenImport = async () => {
      openAssets()
      if (importAvailability.state === 'absent') {
        publishUserActionMessage(availabilitySentence('shell.desktop'))
        return
      }
      if (!projectRef.current) {
        publishUserActionMessage('Create a project in Projects to begin.')
        return
      }
      const paths = (await pickMedia()).map((p) => p.trim()).filter(Boolean)
      if (!paths.length) return

      let imported = 0
      for (const path of paths) {
        const r = await callVerb('media.import', {
          path,
          proxy: getGenerateProxies(),
          rationale: 'user import from app-wide Import media command',
        })
        if (r.ok) {
          imported++
          continue
        }
        // Never fail silently: the shared notice explains it and, for missing
        // video processing, offers the setup.
        publishUserActionFailure('media.import', r, 'Import failed')
        if (r.error?.code === TOOL_MISSING) break
      }
      if (imported > 0) onChangedRef.current?.()
    }

    document.addEventListener('cut:open-import', onOpenImport)
    return () => document.removeEventListener('cut:open-import', onOpenImport)
  }, [importAvailability.state, setLayout])
}
