// Closing the app during a recording: the desktop shell holds the close and
// asks this window to save the recording first, exactly like a normal Stop
// (raw MP4, then the editable clip on the timeline). The window tells the
// shell it took over, shows that it is saving, and lets the shell close when
// done. If this window cannot answer, the shell stops the recording itself.

import { useEffect, useRef } from 'react'
import { acknowledgeCloseRequest, finishCloseRequest, onCloseRequested } from '../lib/tauri'

export const SAVING_BEFORE_CLOSE = 'Saving your recording before ShellX Cut closes…'

export interface CloseRequestOwner {
  /** True while a capture id is held by this window. */
  isRecording: () => boolean
  /** Stop and save as after a normal Stop, with reason app_exit. */
  stopForClose: () => Promise<void>
  /** Show the saving message. */
  announce: (message: string) => void
}

/** The close sequence, separated from React for tests. */
export async function handleCloseRequest(
  owner: CloseRequestOwner,
  shell: { acknowledge: () => Promise<boolean>; finish: () => Promise<boolean> },
): Promise<void> {
  await shell.acknowledge()
  try {
    if (owner.isRecording()) {
      owner.announce(SAVING_BEFORE_CLOSE)
      await owner.stopForClose()
    }
  } finally {
    await shell.finish()
  }
}

export function useRecordingCloseRequest(owner: CloseRequestOwner): void {
  const ownerRef = useRef(owner)
  ownerRef.current = owner
  useEffect(() => onCloseRequested(() => {
    void handleCloseRequest(ownerRef.current, { acknowledge: acknowledgeCloseRequest, finish: finishCloseRequest })
  }), [])
}
