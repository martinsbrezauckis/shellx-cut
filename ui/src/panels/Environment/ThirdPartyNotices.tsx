// ThirdPartyNotices.tsx — Settings > About "Third-party notices": shows the bundled THIRD-PARTY-NOTICES.txt.
//
// The file lists every library compiled into ShellX Cut (Rust crates, npm packages) with copyright lines and the
// full licence texts. It is large, so it is fetched only when the person opens it. A failure is shown in the panel
// with what to do next; nothing is silent. Callers: About.tsx. Deps: lib/thirdPartyNotices.

import { useState } from 'react'
import { loadThirdPartyNotices } from '../../lib/thirdPartyNotices'
import './third-party-notices.css'

export default function ThirdPartyNotices() {
  const [open, setOpen] = useState(false)
  const [loading, setLoading] = useState(false)
  const [text, setText] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const toggle = async () => {
    if (open) {
      setOpen(false)
      return
    }
    setOpen(true)
    if (text !== null || loading) return
    setLoading(true)
    setError(null)
    const result = await loadThirdPartyNotices()
    setLoading(false)
    if (result.ok) setText(result.text)
    else setError(result.message)
  }

  return (
    <div className="env-notices" data-cut-about-notices-panel>
      <button
        type="button"
        className="env-btn env-btn--secondary env-btn--sm"
        data-cut-about-notices
        aria-expanded={open}
        aria-busy={loading}
        title="Open the bundled list of the libraries ShellX Cut is built from, with their copyright lines and licence texts"
        onClick={() => void toggle()}
      >
        {open ? 'Hide third-party notices' : 'Third-party notices'}
      </button>
      {open && loading && <p className="env-about-status env-about-status--muted" role="status">Loading the notices…</p>}
      {open && error && <p className="env-about-status env-about-status--error" data-cut-about-notices-error role="alert">{error}</p>}
      {open && text !== null && (
        <pre className="env-notices-text" data-cut-about-notices-text tabIndex={0} aria-label="Third-party notices">{text}</pre>
      )}
    </div>
  )
}
