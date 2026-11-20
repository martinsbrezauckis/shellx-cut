import { useEffect, useState } from 'react'
import { getRecordHotkeyCapability, onRecordHotkeyCapability, type RecordHotkeyCapability } from '../lib/tauri'

/** The desktop shell's report of how the global record shortcut is registered.
 * `null` in a browser build, where there is no shell. A pushed event is newer
 * than the initial read, so it wins when the two race. */
export function useRecordHotkeyCapability(): RecordHotkeyCapability | null {
  const [capability, setCapability] = useState<RecordHotkeyCapability | null>(null)
  useEffect(() => {
    let alive = true
    let pushed = false
    void getRecordHotkeyCapability().then((initial) => {
      if (alive && !pushed && initial) setCapability(initial)
    })
    const off = onRecordHotkeyCapability((next) => {
      if (alive) { pushed = true; setCapability(next) }
    })
    return () => { alive = false; off() }
  }, [])
  return capability
}
