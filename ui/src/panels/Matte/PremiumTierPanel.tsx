import type { PremiumMatteAvailability } from '../../lib/doctor'

const fallback: Record<Exclude<PremiumMatteAvailability, 'missing' | 'ready'>, string> = {
  'hardware-unavailable': 'Premium is installed, but no NVIDIA CUDA GPU is available. Use Standard (RVM), or run Premium on a supported NVIDIA machine.',
  unverified: 'Premium hardware could not be verified. Re-check before using this tier.',
  unavailable: 'Premium could not be prepared. Check the diagnostic hint, then re-check.',
}

export function PremiumTierPanel({ availability, hint, installing, error, context, onInstall, onRecheck }: {
  availability: PremiumMatteAvailability
  hint: string | null
  installing: string | null
  error: string | null
  context: 'requirements' | 'controls'
  onInstall: () => void
  onRecheck: () => void
}) {
  if (availability === 'ready') return null
  const missing = availability === 'missing'
  return (
    <div className={context === 'controls' ? 'cd-result' : undefined}
      data-cut-matte-premium-consent={context === 'controls' && missing ? '' : undefined}
      data-cut-matte-premium-status={availability}>
      <div className="cd-result-head">Premium — MatAnyone2 (pick the subject)</div>
      {missing ? (
        <>
          <p className="cd-note">Cleaner edges + temporal stability + click-to-pick WHICH subject. NVIDIA GPU, ~135 MB,
            <strong> non-commercial license (NTU S-Lab 1.0)</strong> — installing accepts it.</p>
          <button className="cd-btn" data-cut-matte-install-premium disabled={installing !== null} onClick={onInstall}>
            {installing === 'matanyone' ? 'Installing…' : 'Install Premium (accept non-commercial)'}
          </button>
        </>
      ) : (
        <p className="cd-note">{hint || fallback[availability]}
          {availability === 'hardware-unavailable' && hint && ' Use Standard (RVM), or run Premium on a supported NVIDIA machine.'}
        </p>
      )}
      {context === 'controls' && (
        <>
          <div style={{ height: 8 }} />
          <button className="cd-btn cd-btn--ghost" data-cut-matte-premium-recheck
            disabled={installing !== null} onClick={onRecheck}>Re-check</button>
        </>
      )}
      {context === 'controls' && error && <div className="cd-err" data-cut-matte-error role="alert">{error}</div>}
    </div>
  )
}
