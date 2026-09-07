import { useCallback, useEffect, type Dispatch, type MutableRefObject, type SetStateAction } from 'react'
import type { GenerateWorkspaceTab } from '../panels/GenerateTemplates'
import { isSettingsCategoryId, type SettingsCategoryId } from '../panels/Environment/settingsModel'
import type { AppDrawer } from './AppDrawerStack'
import type { AgentChatPrefill, ChatEvidenceAttachment } from '../lib/evidenceAttachments'
import { normalizeGenerateTab } from './model'
import { uiSurface } from './uiSurfaceRegistry'
import type { RequestLayout } from './useRecordingWorkspaceNavigation'

interface AppSurfaceEventsArgs {
  setLayout: RequestLayout
  setCommentsOpen: Dispatch<SetStateAction<boolean>>
  setFocusComment: Dispatch<SetStateAction<{ id: string; n: number } | null>>
  setActiveDrawer: Dispatch<SetStateAction<AppDrawer | null>>
  setGenerateTab: Dispatch<SetStateAction<GenerateWorkspaceTab>>
  setWizardOpen: Dispatch<SetStateAction<boolean>>
  setEnvOpen: Dispatch<SetStateAction<boolean>>
  setEnvCategory: Dispatch<SetStateAction<SettingsCategoryId>>
  onRefreshDoctor: () => void | Promise<unknown>
  onOpenManual?: (featureId?: string) => void
  agentChatPromptSeq: MutableRefObject<number>
  setAgentChatPrefill: Dispatch<SetStateAction<AgentChatPrefill | null>>
}

/** Bridges document-level app events to the shared surface registry. Returns
 * the same opener used by ui.open, so human and agent routes cannot drift. */
export function useAppSurfaceEvents({
  setLayout,
  setCommentsOpen,
  setFocusComment,
  setActiveDrawer,
  setGenerateTab,
  setWizardOpen,
  setEnvOpen,
  setEnvCategory,
  onRefreshDoctor,
  onOpenManual,
  agentChatPromptSeq,
  setAgentChatPrefill,
}: AppSurfaceEventsArgs) {
  const openSurface = useCallback((id: string): boolean => {
    const entry = uiSurface(id)
    const action = entry?.action
    if (!entry || !action) return false
    const showEditor = (): boolean => {
      if (!setLayout((layout) => ({ ...layout, workspaceMode: 'edit' }))) return false
      setWizardOpen(false)
      setEnvOpen(false)
      return true
    }
    switch (action.kind) {
      case 'focus':
        if (!showEditor()) return false
        window.requestAnimationFrame(() => {
          const element = document.querySelector<HTMLElement>(entry.selector)
          element?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
          element?.focus()
        })
        return true
      case 'left':
        if (!showEditor()) return false
        setLayout((layout) => ({ ...layout, workspaceMode: 'edit', leftTab: action.tab, leftCollapsed: false }))
        return true
      case 'find':
        if (!showEditor()) return false
        setLayout((layout) => ({
          ...layout,
          workspaceMode: 'edit',
          leftTab: 'find',
          findSurface: action.surface,
          leftCollapsed: false,
        }))
        return true
      case 'generate':
        if (!showEditor()) return false
        setGenerateTab(action.tab)
        setLayout((layout) => ({ ...layout, workspaceMode: 'edit', leftTab: 'generate', leftCollapsed: false }))
        return true
      case 'workspace':
        if (!setLayout((layout) => ({ ...layout, workspaceMode: action.workspace }))) return false
        setWizardOpen(false)
        setEnvOpen(false)
        return true
      case 'right':
        if (!showEditor()) return false
        setLayout((layout) => ({
          ...layout,
          workspaceMode: 'edit',
          rightTab: action.tab,
          railCollapsed: false,
        }))
        return true
      case 'review':
        if (!showEditor()) return false
        setLayout((layout) => ({
          ...layout,
          workspaceMode: 'edit',
          railCollapsed: false,
          railPinned: true,
        }))
        document.dispatchEvent(new CustomEvent('cut:open-review-tab', { detail: action.tab }))
        return true
      case 'settings':
        if (!isSettingsCategoryId(action.category)) return false
        setWizardOpen(false)
        setEnvCategory(action.category)
        setEnvOpen(true)
        return true
      case 'overlay':
        if (action.overlay === 'wizard') {
          setEnvOpen(false)
          setWizardOpen(true)
        } else {
          if (!showEditor()) return false
          setCommentsOpen(true)
        }
        return true
      case 'drawer':
        if (!showEditor()) return false
        setActiveDrawer(action.drawer)
        return true
    }
  }, [
    setActiveDrawer,
    setCommentsOpen,
    setEnvCategory,
    setEnvOpen,
    setGenerateTab,
    setLayout,
    setWizardOpen,
  ])

  useEffect(() => {
    const onOpenUiSurface = (event: Event) => {
      const detail = (event as CustomEvent<{ id?: unknown } | string>).detail
      const id = typeof detail === 'string' ? detail : detail?.id
      if (typeof id === 'string') openSurface(id)
    }
    const onOpenReceipts = () => openSurface('receipts')
    const onOpenLeftTab = (e: Event) => {
      const tab = (e as CustomEvent).detail as 'transcript' | 'assets' | 'generate' | 'projects' | 'library'
      if (tab === 'generate') openSurface('generate')
      else if (tab === 'library' || tab === 'transcript' || tab === 'assets' || tab === 'projects') openSurface(tab)
    }
    const onOpenEnvironment = () => openSurface('environment')
    const onOpenWizard = () => openSurface('wizard')
    const onRefreshDoctorEvent = () => {
      void onRefreshDoctor()
    }
    const onOpenManualEvent = (e: Event) => {
      const feature = (e as CustomEvent<{ feature?: string } | string | undefined>).detail
      onOpenManual?.(typeof feature === 'string' ? feature : feature?.feature)
    }
    const onOpenComment = (e: Event) => {
      const id = (e as CustomEvent<{ id?: string }>).detail?.id
      openSurface('comments')
      if (id) setFocusComment((prev) => ({ id, n: (prev?.n ?? 0) + 1 }))
    }
    const onKinetic = () => openSurface('kinetic')
    const onGrade = () => openSurface('color')
    const onLayer = () => openSurface('layer')
    const onMatte = () => openSurface('matte')
    const onStock = () => openSurface('find-media')
    const onShape = () => openSurface('shape')
    const onSearch = () => openSurface('find-moment')
    const onGenerate = (e: Event) => {
      const detail = (e as CustomEvent<{ tab?: GenerateWorkspaceTab } | GenerateWorkspaceTab | undefined>).detail
      const requestedTab = typeof detail === 'string' ? detail : detail?.tab
      const tab = normalizeGenerateTab(requestedTab)
      openSurface(tab === 'templates' ? 'generate' : `generate-${tab}`)
    }
    const onOpenChat = (e: Event) => {
      openSurface('chat')
      const detail = (e as CustomEvent<string | { prompt?: string; evidence?: ChatEvidenceAttachment[] }>).detail
      const prompt = typeof detail === 'string' ? detail : detail?.prompt
      if (prompt?.trim()) {
        agentChatPromptSeq.current += 1
        setAgentChatPrefill({
          prompt,
          nonce: agentChatPromptSeq.current,
          evidence: typeof detail === 'string' ? undefined : detail?.evidence,
        })
      }
    }
    const onOpenDrawer = (e: Event) => {
      const name = (e as CustomEvent).detail as AppDrawer | 'grade' | 'mixer' | 'stock' | 'search' | 'generate'
      const mapped = name === 'grade'
        ? 'color'
        : name === 'mixer'
          ? 'audio'
          : name === 'stock'
            ? 'find-media'
            : name === 'search'
              ? 'find-moment'
              : name
      openSurface(mapped)
    }

    document.addEventListener('cut:open-ui-surface', onOpenUiSurface)
    document.addEventListener('cut:open-receipts', onOpenReceipts)
    document.addEventListener('cut:open-left-tab', onOpenLeftTab)
    document.addEventListener('cut:open-environment', onOpenEnvironment)
    document.addEventListener('cut:open-wizard', onOpenWizard)
    document.addEventListener('cut:refresh-doctor', onRefreshDoctorEvent)
    document.addEventListener('cut:open-manual', onOpenManualEvent)
    document.addEventListener('cut:open-comment', onOpenComment)
    document.addEventListener('cut:open-kinetic', onKinetic)
    document.addEventListener('cut:open-grade', onGrade)
    document.addEventListener('cut:open-layer', onLayer)
    document.addEventListener('cut:open-matte', onMatte)
    document.addEventListener('cut:open-stock', onStock)
    document.addEventListener('cut:open-shape', onShape)
    document.addEventListener('cut:open-search', onSearch)
    document.addEventListener('cut:open-generate', onGenerate)
    document.addEventListener('cut:open-chat', onOpenChat)
    document.addEventListener('cut:open-drawer', onOpenDrawer)
    return () => {
      document.removeEventListener('cut:open-ui-surface', onOpenUiSurface)
      document.removeEventListener('cut:open-receipts', onOpenReceipts)
      document.removeEventListener('cut:open-left-tab', onOpenLeftTab)
      document.removeEventListener('cut:open-environment', onOpenEnvironment)
      document.removeEventListener('cut:open-wizard', onOpenWizard)
      document.removeEventListener('cut:refresh-doctor', onRefreshDoctorEvent)
      document.removeEventListener('cut:open-manual', onOpenManualEvent)
      document.removeEventListener('cut:open-comment', onOpenComment)
      document.removeEventListener('cut:open-kinetic', onKinetic)
      document.removeEventListener('cut:open-grade', onGrade)
      document.removeEventListener('cut:open-layer', onLayer)
      document.removeEventListener('cut:open-matte', onMatte)
      document.removeEventListener('cut:open-stock', onStock)
      document.removeEventListener('cut:open-shape', onShape)
      document.removeEventListener('cut:open-search', onSearch)
      document.removeEventListener('cut:open-generate', onGenerate)
      document.removeEventListener('cut:open-chat', onOpenChat)
      document.removeEventListener('cut:open-drawer', onOpenDrawer)
    }
  }, [
    agentChatPromptSeq,
    setAgentChatPrefill,
    setFocusComment,
    openSurface,
    onOpenManual,
    onRefreshDoctor,
  ])
  return openSurface
}
