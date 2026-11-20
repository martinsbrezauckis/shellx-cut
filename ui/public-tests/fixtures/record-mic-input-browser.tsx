import { createRoot } from 'react-dom/client'
import { MicInputControl } from '../../src/panels/Record/MicInputControl'

createRoot(document.getElementById('root')!).render(
  <MicInputControl audio disabled={false} idle onAudioChange={() => {}} onTestingChange={() => {}} />,
)
