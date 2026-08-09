import type { Project } from '../../lib/client'

export const MAX_CHAT_ATTACHMENTS = 8

export interface ChatAttachmentOption {
  id: string
  label: string
}

export function chatAttachmentLabel(path: string, id: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || id
}

export function chatAttachmentOptions(project: Project | null): ChatAttachmentOption[] {
  if (!project) return []
  return Object.entries(project.assets)
    .map(([id, asset]) => ({ id, label: chatAttachmentLabel(asset.path, id) }))
    .sort((a, b) => a.label.localeCompare(b.label) || a.id.localeCompare(b.id))
}

export function toggleChatAttachment(
  selected: string[],
  id: string,
  max = MAX_CHAT_ATTACHMENTS,
): string[] {
  if (selected.includes(id)) return selected.filter((candidate) => candidate !== id)
  if (selected.length >= max) return selected
  return [...selected, id]
}
