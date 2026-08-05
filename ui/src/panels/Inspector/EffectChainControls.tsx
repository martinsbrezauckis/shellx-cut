import { useEffect, useMemo, useRef, useState } from 'react'
import { callVerb, type ClipEffect } from '../../lib/client'
import { Icon } from '../../icons'
import { clipEffectsOf } from './model'
import {
  effectParameterSummary,
  effectType,
  moveClipEffect,
  toggleClipEffect,
} from './effectChainModel'

export interface EffectOption {
  eff: ClipEffect
  label: string
  description?: string
}

interface EffectChainControlsProps {
  clipId: string
  effects: unknown
  kind: 'video' | 'audio'
  options: EffectOption[]
  extraOptions?: EffectOption[]
  externallyBusy?: boolean
  onBusyChange?: (busy: boolean) => void
  onApplied?: () => void
}

interface QueuedChain {
  next: ClipEffect[]
  action: string
}

export default function EffectChainControls({
  clipId,
  effects,
  kind,
  options,
  extraOptions = [],
  externallyBusy = false,
  onBusyChange,
  onApplied,
}: EffectChainControlsProps) {
  const canonical = clipEffectsOf(effects)
  const canonicalKey = JSON.stringify(canonical)
  const [displayEffects, setDisplayEffects] = useState<ClipEffect[]>(canonical)
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [showMore, setShowMore] = useState(false)

  const selectedClipRef = useRef(clipId)
  const confirmedRef = useRef<ClipEffect[]>(canonical)
  const projectedRef = useRef<ClipEffect[]>(canonical)
  const queueRef = useRef<QueuedChain[]>([])
  const drainRef = useRef<symbol | null>(null)
  const aliveRef = useRef(true)

  useEffect(() => {
    aliveRef.current = true
    return () => {
      aliveRef.current = false
      drainRef.current = null
      queueRef.current = []
      onBusyChange?.(false)
    }
  }, [])

  useEffect(() => {
    if (selectedClipRef.current !== clipId) {
      selectedClipRef.current = clipId
      drainRef.current = null
      queueRef.current = []
      setBusy(false)
      setPending(0)
      setError(null)
      setShowMore(false)
      onBusyChange?.(false)
    }
    if (drainRef.current === null) {
      confirmedRef.current = canonical
      projectedRef.current = canonical
      setDisplayEffects(canonical)
    }
  // canonicalKey is the stable semantic dependency; canonical is rebuilt on render.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clipId, canonicalKey])

  const labels = useMemo(() => {
    const map = new Map<string, string>()
    for (const option of [...options, ...extraOptions]) map.set(effectType(option.eff), option.label)
    return map
  }, [options, extraOptions])

  const beginDrain = (targetClip: string) => {
    if (drainRef.current !== null) return
    const drainId = Symbol('effect-chain')
    drainRef.current = drainId
    setBusy(true)
    onBusyChange?.(true)

    void (async () => {
      while (queueRef.current.length > 0) {
        const queued = queueRef.current[0]
        if (!queued) break
        let result
        try {
          result = await callVerb('edit.effect', {
            clip: targetClip,
            effects: queued.next,
            rationale: `inspector: ${queued.action}`,
          })
        } catch (cause) {
          result = {
            ok: false,
            error: { code: 'request_failed', message: String(cause), cause: 'effect chain request' },
          }
        }

        if (!aliveRef.current || drainRef.current !== drainId || selectedClipRef.current !== targetClip) return
        if (!result.ok) {
          queueRef.current = []
          projectedRef.current = confirmedRef.current
          setDisplayEffects(confirmedRef.current)
          setPending(0)
          setError(result.error?.message ?? result.error?.code ?? 'Could not update effects')
          break
        }

        confirmedRef.current = queued.next
        queueRef.current.shift()
        setPending(queueRef.current.length)
        onApplied?.()
      }

      if (!aliveRef.current || drainRef.current !== drainId || selectedClipRef.current !== targetClip) return
      drainRef.current = null
      projectedRef.current = confirmedRef.current
      setDisplayEffects(confirmedRef.current)
      setBusy(false)
      onBusyChange?.(false)
    })()
  }

  const enqueue = (next: ClipEffect[], action: string) => {
    if (externallyBusy || next === projectedRef.current) return
    projectedRef.current = next
    queueRef.current.push({ next, action })
    setDisplayEffects(next)
    setPending(queueRef.current.length)
    setError(null)
    beginDrain(clipId)
  }

  const toggle = (option: EffectOption) => {
    const type = effectType(option.eff)
    const removing = projectedRef.current.some((effect) => effectType(effect) === type)
    enqueue(
      toggleClipEffect(projectedRef.current, option.eff),
      `${removing ? 'remove' : 'add'} ${type}`,
    )
  }

  const removeAt = (index: number) => {
    const effect = projectedRef.current[index]
    if (!effect) return
    enqueue(
      projectedRef.current.filter((_, effectIndex) => effectIndex !== index),
      `remove ${effectType(effect)}`,
    )
  }

  const moveAt = (index: number, delta: -1 | 1) => {
    const effect = projectedRef.current[index]
    if (!effect) return
    enqueue(
      moveClipEffect(projectedRef.current, index, delta),
      `move ${effectType(effect)} ${delta < 0 ? 'up' : 'down'}`,
    )
  }

  const renderChip = (option: EffectOption, extra = false) => {
    const type = effectType(option.eff)
    const on = displayEffects.some((effect) => effectType(effect) === type)
    const selector = kind === 'video'
      ? { 'data-cut-inspector-effect': type }
      : { 'data-cut-inspector-audio-effect': type }
    return (
      <button
        key={`${extra ? 'extra' : 'primary'}:${type}`}
        type="button"
        className={`insp__chip${on ? ' insp__chip--on' : ''}`}
        data-cut-effect-on={on ? 'true' : 'false'}
        disabled={externallyBusy}
        title={option.description}
        onClick={() => toggle(option)}
        {...selector}
      >
        {option.label}
      </button>
    )
  }

  return (
    <div
      className="insp__effect-chain"
      data-cut-effect-chain={kind}
      data-cut-effect-chain-busy={busy ? 'true' : 'false'}
      data-cut-effect-chain-pending={pending}
      aria-busy={busy}
    >
      <div className="insp__group-title insp__group-title--sub">Current chain</div>
      {displayEffects.length > 0 ? (
        <ul className="insp__list" data-cut-effect-chain-list>
          {displayEffects.map((effect, index) => {
            const type = effectType(effect)
            const label = labels.get(type) ?? type.replaceAll('_', ' ')
            return (
              <li className="insp__list-row" data-cut-effect-chain-item={type} key={`${type}:${index}`}>
                <span className="insp__chain-copy">
                  <span className="insp__list-label">{index + 1}. {label}</span>
                  <span className="insp__chain-params">{effectParameterSummary(effect)}</span>
                </span>
                <span className="insp__chain-actions">
                  <button type="button" className="insp__chain-action" data-cut-effect-chain-move-up={type} aria-label={`Move ${label} up`}
                    title={`Move ${label} up`} disabled={externallyBusy || index === 0}
                    onClick={() => moveAt(index, -1)}><Icon name="chevronUp" size={14} /></button>
                  <button type="button" className="insp__chain-action" data-cut-effect-chain-move-down={type} aria-label={`Move ${label} down`}
                    title={`Move ${label} down`} disabled={externallyBusy || index === displayEffects.length - 1}
                    onClick={() => moveAt(index, 1)}><Icon name="chevronDown" size={14} /></button>
                  <button type="button" className="insp__chain-action" data-cut-effect-chain-remove={type} aria-label={`Remove ${label}`}
                    title={`Remove ${label}`} disabled={externallyBusy}
                    onClick={() => removeAt(index)}><Icon name="trash" size={14} /></button>
                </span>
              </li>
            )
          })}
        </ul>
      ) : (
        <p className="insp__hint" data-cut-effect-chain-empty>No clip effects</p>
      )}

      <div className="insp__group-title insp__group-title--sub">Add effects</div>
      <div className="insp__effects" data-cut-inspector-effects={kind === 'video' ? '' : undefined}
        data-cut-inspector-audio-effects={kind === 'audio' ? '' : undefined}>
        {options.map((option) => renderChip(option))}
        {extraOptions.length > 0 && (
          <button type="button" className="insp__chip insp__chip--more" data-cut-inspector-effects-more
            disabled={externallyBusy} title={`${extraOptions.length} more engine effects`}
            onClick={() => setShowMore((value) => !value)}>
            {showMore ? 'Fewer effects' : `More effects… (${extraOptions.length})`}
          </button>
        )}
      </div>
      {showMore && extraOptions.length > 0 && (
        <div className="insp__effects" data-cut-inspector-effects-extra>
          {extraOptions.map((option) => renderChip(option, true))}
        </div>
      )}
      {busy && <p className="insp__hint" data-cut-effect-chain-status>Applying {pending} change{pending === 1 ? '' : 's'}…</p>}
      {error && <p className="insp__hint insp__hint--error" role="alert" data-cut-effect-chain-error>{error}</p>}
    </div>
  )
}
