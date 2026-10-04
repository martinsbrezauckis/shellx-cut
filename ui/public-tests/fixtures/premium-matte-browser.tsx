import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import MatteDrawer from '../../src/panels/Matte'
import EnvCardRow from '../../src/panels/Environment/EnvCardRow'
import type { DoctorCard } from '../../src/lib/doctor'
import type { Project } from '../../src/lib/client'

const project = (origin: string) => ({
  name: origin, project_identity: { project_name: origin, origin_path_sha256: origin },
  tracks: [{ kind: 'video', clips: [{ id: 'clip-1', asset: 'asset-1' }] }], assets: { 'asset-1': {} },
}) as unknown as Project
const cards: Record<string, DoctorCard> = {
  missing: { id: 'matte_premium', kind: 'matte', status: 'missing', details: {}, hint: 'Premium is missing' },
  hardware: { id: 'matte_premium', kind: 'matte', status: 'degraded', details: { installed: true, cuda_available: false }, hint: 'NVIDIA CUDA unavailable on this host' },
  unknown: { id: 'matte_premium', kind: 'matte', status: 'unknown', details: {}, hint: 'GPU probe timed out' },
  rejected: { id: 'matte_premium', kind: 'matte', status: 'degraded', details: { installed: false, prepared_runtime: { rejected: true } }, hint: 'Prepared runtime rejected' },
  ready: { id: 'matte_premium', kind: 'matte', status: 'ok', details: { installed: true, cuda_available: true } },
}

function Fixture() {
  const [origin, setOrigin] = useState('a')
  const [status, setStatus] = useState('missing')
  return <>
    {Object.keys(cards).map(key => <button key={key} data-test-env-status={key} onClick={() => setStatus(key)}>{key}</button>)}
    <button data-test-project-switch onClick={() => setOrigin(value => value === 'a' ? 'b' : 'a')}>Switch project</button>
    <EnvCardRow card={cards[status]} os="linux" arch="x86_64" onChanged={() => {}} />
    <MatteDrawer project={project(origin)} clipId="clip-1" playheadMs={0} onClose={() => {}} />
  </>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
