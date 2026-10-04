import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useVoiceoverPlayback } from '../../src/panels/Preview/useVoiceoverPlayback'
import type { Rate } from '../../src/panels/Preview/PreviewTransport'

function HookFixture() {
  const [playhead, setPlayhead] = useState(0)
  const [rate, setRate] = useState<Rate>(0)
  const [renders, setRenders] = useState(0)
  const { playback, outUnconfirmed } = useVoiceoverPlayback({
    playheadMs: playhead, durationMs: 10_000, rate, onSeek: setPlayhead, setRate,
  })
  Object.assign(window, {
    voiceoverFixtureRerender: () => setRenders(value => value + 1),
    voiceoverFixturePause: () => setRate(0),
  })
  return <div data-test-render={renders}>
    <output data-test-playhead>{playhead}</output>
    <output data-test-rate>{rate}</output>
    <output data-test-owner>{playback ? `${playback.requestId}:${playback.bridgeEpoch}` : ''}</output>
    {outUnconfirmed && <p data-cut-voiceover-out-unconfirmed role="status">Automatic stop could not be confirmed. Use Stop or Cancel on the voiceover track.</p>}
  </div>
}

function Fixture() {
  const [mounted, setMounted] = useState(true)
  const [scope, setScope] = useState(1)
  Object.assign(window, {
    voiceoverFixtureUnmount: () => setMounted(false),
    voiceoverFixtureProject: () => setScope(value => value + 1),
  })
  return <div>
    <output data-test-project-scope>{scope}</output>
    {mounted && <HookFixture key={scope} />}
  </div>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
