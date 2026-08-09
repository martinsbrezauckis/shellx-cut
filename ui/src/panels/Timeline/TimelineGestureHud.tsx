export interface TimelineGestureHudState {
  x: number
  y: number
  label: string
  sub?: string
  tone: 'info' | 'warn'
}

interface TimelineGestureHudProps {
  hud: TimelineGestureHudState | null
}

export default function TimelineGestureHud({ hud }: TimelineGestureHudProps) {
  if (!hud) return null

  return (
    <div
      className={`tl-hud tl-hud--${hud.tone}`}
      data-cut-hud
      data-cut-hud-tone={hud.tone}
      style={{ left: hud.x + 14, top: hud.y + 16 }}
    >
      <span className="tl-hud__label">{hud.label}</span>
      {hud.sub && <span className="tl-hud__sub">{hud.sub}</span>}
    </div>
  )
}
