import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'

export type RecordingSettingsSection = 'camera' | 'background' | 'timer' | 'timing' | 'quality'

const SECTIONS: readonly { id: RecordingSettingsSection; label: string }[] = [
  { id: 'camera', label: 'Camera' },
  { id: 'background', label: 'Background' },
  { id: 'timer', label: 'Video timer' },
  { id: 'timing', label: 'Capture timing' },
  { id: 'quality', label: 'Video quality' },
]

interface RecordingSettingsTabsProps {
  pages: Record<RecordingSettingsSection, ReactNode>
}

/** Keep every page mounted so draft inputs survive a tab switch. */
export function RecordingSettingsTabs({ pages }: RecordingSettingsTabsProps) {
  const [active, setActive] = useState<RecordingSettingsSection>('camera')
  const id = useId()
  const tabs = useRef<(HTMLButtonElement | null)[]>([])

  const chooseFromKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const next = event.key === 'ArrowRight' || event.key === 'ArrowDown'
      ? (index + 1) % SECTIONS.length
      : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
        ? (index - 1 + SECTIONS.length) % SECTIONS.length
        : event.key === 'Home' ? 0 : event.key === 'End' ? SECTIONS.length - 1 : null
    if (next === null) return
    event.preventDefault()
    setActive(SECTIONS[next].id)
    tabs.current[next]?.focus()
  }

  return (
    <div className="rec-settings-tabs">
      <div className="rec-settings-tabs__list" role="tablist" aria-label="Recording option sections">
        {SECTIONS.map((section, index) => (
          <button
            key={section.id}
            ref={(element) => { tabs.current[index] = element }}
            type="button"
            role="tab"
            id={`${id}-${section.id}-tab`}
            aria-controls={`${id}-${section.id}-panel`}
            aria-selected={active === section.id}
            tabIndex={active === section.id ? 0 : -1}
            data-cut-rec-settings-tab={section.id}
            onClick={() => setActive(section.id)}
            onKeyDown={(event) => chooseFromKey(event, index)}
          >{section.label}</button>
        ))}
      </div>
      {SECTIONS.map((section) => (
        <section
          key={section.id}
          className="rec-settings-tabs__panel"
          role="tabpanel"
          id={`${id}-${section.id}-panel`}
          aria-labelledby={`${id}-${section.id}-tab`}
          tabIndex={0}
          hidden={active !== section.id}
          data-cut-rec-settings-panel={section.id}
        >{pages[section.id]}</section>
      ))}
    </div>
  )
}
