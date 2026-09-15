import type { ControlledVerbArgs } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import InspectorSection from '../../components/inspector/InspectorSection'
import PropertyRow from '../../components/inspector/PropertyRow'

export interface FadesSectionProps {
  clipId: string
  fadeInMs: number
  fadeOutMs: number
  isVideo: boolean
  clipDurMs: number
}

export default function FadesSection({ clipId, fadeInMs, fadeOutMs, isVideo, clipDurMs }: FadesSectionProps) {
  const fire = async (args: ControlledVerbArgs<'edit.fade'>) => {
    const result = await runUserVerb('edit.fade', args, 'Could not change the clip fades.')
    if (isVideo && result?.ok) document.dispatchEvent(new CustomEvent('cut:show-composed'))
  }
  const durS = clipDurMs > 0 ? clipDurMs / 1000 : 5
  const inMaxS = Math.max(0.1, durS - fadeOutMs / 1000)
  const outMaxS = Math.max(0.1, durS - fadeInMs / 1000)

  return (
    <InspectorSection
      title="Fades"
      sectionKey="fades"
      defaultCollapsed
      summary={fadeInMs === 0 && fadeOutMs === 0
        ? 'No fades'
        : `${(fadeInMs / 1000).toFixed(1)}s in · ${(fadeOutMs / 1000).toFixed(1)}s out`}
      summaryTone={fadeInMs === 0 && fadeOutMs === 0 ? 'neutral' : 'active'}
      bypassed={fadeInMs === 0 && fadeOutMs === 0}
      onToggleBypass={() => { if (fadeInMs !== 0 || fadeOutMs !== 0) return fire({ clip: clipId, in_ms: 0, out_ms: 0, rationale: 'inspector: clear fades' }) }}
      onReset={() => fire({ clip: clipId, in_ms: 0, out_ms: 0, rationale: 'inspector: reset fades' })}
    >
      <PropertyRow
        label="Fade in" propKey="fade-in" unit="s"
        value={fadeInMs / 1000} min={0} max={inMaxS} step={0.1} default={0}
        onCommit={(v) => fire({ clip: clipId, in_ms: Math.round(v * 1000), rationale: `inspector: fade in ${v}s` })}
      />
      <PropertyRow
        label="Fade out" propKey="fade-out" unit="s"
        value={fadeOutMs / 1000} min={0} max={outMaxS} step={0.1} default={0}
        onCommit={(v) => fire({ clip: clipId, out_ms: Math.round(v * 1000), rationale: `inspector: fade out ${v}s` })}
      />
    </InspectorSection>
  )
}
