import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { VoiceoverTrackControl } from '../../src/panels/Timeline/VoiceoverTrackControl'

function Fixture() {
  const [mounted, setMounted] = useState(true)
  Object.assign(window, { voiceoverDoctorMount: () => setMounted(true), voiceoverDoctorUnmount: () => setMounted(false) })
  return mounted
    ? <VoiceoverTrackControl trackId="audio-1" locked={false} projectRevision="revision-1" playheadMs={0} exportRange={null} />
    : <div data-test-unmounted />
}

createRoot(document.getElementById('root')!).render(<Fixture />)
