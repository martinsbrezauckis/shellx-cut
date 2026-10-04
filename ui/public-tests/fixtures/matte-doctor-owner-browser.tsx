import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useMatteDoctor } from '../../src/panels/Matte/useMatteDoctor'

function Owner() {
  const [origin, setOrigin] = useState('origin-a')
  const { probeState, premiumReady, probe } = useMatteDoctor(origin)
  const ready = probeState === 'ready' || (probeState === 'absent' && premiumReady)
  return <>
    <button data-test-refresh onClick={() => void probe(true)}>Re-check</button>
    <button data-test-switch onClick={() => setOrigin('origin-b')}>Switch</button>
    <output data-test-owner-state={probeState} data-test-owner-ready={ready}>{probeState}</output>
  </>
}

createRoot(document.getElementById('root')!).render(<Owner />)
