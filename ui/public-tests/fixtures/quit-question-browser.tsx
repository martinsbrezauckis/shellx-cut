// The close-time question as the desktop shell drives it: a stand-in for the
// shell's event bridge is installed before the app mounts, so QuitQuestion
// subscribes through its normal Tauri path. The test emits the same event the
// shell sends (`cut:quit-jobs-running`) and reads the answers the window gives.
import { createRoot } from 'react-dom/client'
import QuitQuestion from '../../src/app/QuitQuestion'
import '../../src/app/quitQuestion.css'

type Listener = (event: { payload: unknown }) => void
const listeners = new Map<string, Listener[]>()
const calls: Array<{ command: string; args?: Record<string, unknown> }> = []
const target = window as unknown as Record<string, unknown>
target.__TAURI__ = {
  core: {
    invoke: async (command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args })
      return true
    },
  },
  event: {
    listen: async (name: string, callback: Listener) => {
      listeners.set(name, [...(listeners.get(name) ?? []), callback])
      return () => listeners.set(name, (listeners.get(name) ?? []).filter((item) => item !== callback))
    },
  },
}
target.__quitCalls = calls
target.__quitListeners = (name: string) => (listeners.get(name) ?? []).length
target.__shellEmit = (name: string, payload: unknown) => {
  for (const callback of listeners.get(name) ?? []) callback({ payload })
}

createRoot(document.getElementById('root')!).render(
  <div style={{ width: 900, height: 600 }}>
    <p data-test-under-question>The editor under the question</p>
    <QuitQuestion />
  </div>,
)
