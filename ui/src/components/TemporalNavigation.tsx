import type { MouseEventHandler, ReactNode } from 'react'

type TemporalFormat = (atMs: number) => string
type DataAttributes = Record<`data-${string}`, string | undefined>

interface TemporalPresentationProps {
  /** The supplied time is owned by the caller's authoritative occurrence. */
  atMs: number
  format?: TemporalFormat
  className?: string
  title?: string
  ariaLabel?: string
  role?: string
  children?: ReactNode
  data?: DataAttributes
}

interface TemporalPointProps extends TemporalPresentationProps {
  onActivate: (atMs: number) => void
  disabled?: boolean
  onClick?: MouseEventHandler<HTMLButtonElement>
}

interface TemporalRangeProps extends Omit<TemporalPresentationProps, 'atMs'> {
  /** This range is displayed exactly as supplied; it is never normalized here. */
  rangeMs: readonly [number, number]
  onActivate?: (atMs: number) => void
  disabled?: boolean
  onClick?: MouseEventHandler<HTMLButtonElement>
  separator?: string
}

/** Compact exact time for temporal editor controls. It deliberately has no
 * knowledge of source/media/EDL ownership; consumers provide those values. */
export function formatTemporalTime(atMs: number): string {
  const total = Math.max(0, Math.round(atMs))
  const minutes = Math.floor(total / 60_000)
  const seconds = Math.floor((total % 60_000) / 1_000)
  const milliseconds = total % 1_000
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(milliseconds).padStart(3, '0')}`
}

function pointValue(atMs: number): string {
  return String(Math.round(atMs))
}

/** A point seek whose caller retains the authoritative time/occurrence route. */
export function TemporalPoint({
  atMs,
  format = formatTemporalTime,
  className,
  title,
  ariaLabel,
  role,
  children,
  data,
  onActivate,
  disabled = false,
  onClick,
}: TemporalPointProps) {
  const text = children ?? format(atMs)
  return (
    <button
      type="button"
      className={className}
      title={title}
      aria-label={ariaLabel ?? `Seek ${format(atMs)}`}
      role={role}
      {...data}
      data-cut-temporal-point={pointValue(atMs)}
      onClick={(event) => {
        onClick?.(event)
        if (!event.defaultPrevented) onActivate(atMs)
      }}
      disabled={disabled}
    >
      {text}
    </button>
  )
}

/** An exact range display that becomes a seek control only when the owning
 * consumer supplies a navigation callback. */
export function TemporalRange({
  rangeMs,
  format = formatTemporalTime,
  className,
  title,
  ariaLabel,
  role,
  children,
  data,
  onActivate,
  disabled = false,
  onClick,
  separator = ' → ',
}: TemporalRangeProps) {
  const [startMs, endMs] = rangeMs
  const text = children ?? `${format(startMs)}${startMs === endMs ? '' : `${separator}${format(endMs)}`}`
  const rangeValue = `${pointValue(startMs)}-${pointValue(endMs)}`
  if (!onActivate) {
    return (
      <span className={className} title={title} aria-label={ariaLabel} role={role} {...data} data-cut-temporal-range={rangeValue}>
        {text}
      </span>
    )
  }
  return (
    <button
      type="button"
      className={className}
      title={title}
      aria-label={ariaLabel ?? `Seek ${format(startMs)}${startMs === endMs ? '' : ` to ${format(endMs)}`}`}
      role={role}
      {...data}
      data-cut-temporal-range={rangeValue}
      onClick={(event) => {
        onClick?.(event)
        if (!event.defaultPrevented) onActivate(startMs)
      }}
      disabled={disabled}
    >
      {text}
    </button>
  )
}
