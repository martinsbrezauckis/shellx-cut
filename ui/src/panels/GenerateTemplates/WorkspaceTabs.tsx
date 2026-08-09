import type { GenerateWorkspaceTab } from './model'

interface WorkspaceTabsProps {
  tab: GenerateWorkspaceTab
  onTab: (tab: GenerateWorkspaceTab) => void
}

const TABS: Array<{ id: GenerateWorkspaceTab; label: string }> = [
  { id: 'templates', label: 'Templates' },
  { id: 'prompt', label: 'Native prompt' },
  { id: 'storyboard', label: 'Storyboard' },
  { id: 'media', label: 'AI media' },
]

export default function WorkspaceTabs({ tab, onTab }: WorkspaceTabsProps) {
  return (
    <div className="gt-tabs" role="tablist" aria-label="Generate workspace">
      {TABS.map((item) => (
        <button
          key={item.id}
          type="button"
          role="tab"
          aria-selected={tab === item.id}
          className={tab === item.id ? 'gt-tab gt-tab--on' : 'gt-tab'}
          data-cut-generate-tab={item.id}
          onClick={() => onTab(item.id)}
        >
          {item.label}
        </button>
      ))}
    </div>
  )
}
