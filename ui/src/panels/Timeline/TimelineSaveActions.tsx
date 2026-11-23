interface TimelineSaveActionsProps {
  canSaveRange: boolean
  savingRange: boolean
  saveNote: string | null
  onSaveRange: () => void
}

/** Fixed save action; GIF follows the fixed controls in the toolbar's changing tail. */
export default function TimelineSaveActions({ canSaveRange, savingRange, saveNote, onSaveRange }: TimelineSaveActionsProps) {
  return (
    <>
      <button
        type="button"
        className="tl-tool"
        data-cut-action="save-range"
        disabled={savingRange || !canSaveRange}
        title="Render the selected timeline span as a reusable asset"
        onClick={() => void onSaveRange()}
      >
        {savingRange ? 'Saving…' : 'Save to Assets'}
      </button>
      {saveNote && <span className="tl-save-note" data-cut-save-note>{saveNote}</span>}
    </>
  )
}

export function TimelineGifAction({ canSaveGif, savingGif, onSaveGif }: {
  canSaveGif: boolean
  savingGif: boolean
  onSaveGif: () => void
}) {
  return <button
    type="button"
    className="tl-tool"
    data-cut-action="save-gif"
    disabled={savingGif || !canSaveGif}
    title="Export the selected video clips as a looping GIF, up to 30 seconds"
    onClick={() => void onSaveGif()}
  >
    {savingGif ? 'GIF…' : 'GIF'}
  </button>
}
