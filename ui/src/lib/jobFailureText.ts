// The text a person reads when a background job fails: the plain reason the
// engine gave, followed by the next step it suggests. Every surface that shows
// a failed job (the status bar, Transcript Tools, Health and Recovery) uses
// this one function, so they all show the same words.

import type { JobRecord } from './jobModel'

function sentence(text: string): string {
  const trimmed = text.trim().replace(/\.{2,}$/, '.')
  if (!trimmed) return ''
  // Paths and command identifiers are case-sensitive, not sentence prose.
  const firstWord = trimmed.split(/\s/, 1)[0].replace(/[.!?]+$/, '')
  const capital = /[/\\_.:=]/.test(firstWord)
    ? trimmed
    : trimmed[0].toUpperCase() + trimmed.slice(1)
  return /[.!?…]$/.test(capital) ? capital : capital + '.'
}

export function jobFailureText(record: JobRecord, fallback: string): string {
  return errorText(record.error, fallback)
}

/** Presentation only: the original cause and suggested_action remain available
 * to agents. A failed analysis is not proof that tools are missing unless the
 * diagnostic actually names a missing dependency. */
export function captionsFailureText(error: { message?: string | null; cause?: string | null } | null | undefined): string | null {
  const message = error?.message ?? ''
  if (!/perception sidecar|Captions and transcription are not installed/i.test(message)) return null
  if (/not installed|not found|failed to run/i.test(message) || /ModuleNotFoundError|No module named/i.test(error?.cause ?? '')) {
    return 'The captions tools are not installed completely. Install or repair them in Settings > AI & transcription.'
  }
  return 'The captions tools could not finish. Check or repair them in Settings > AI & transcription, then try again.'
}

/** The same words for an error a verb returned before any job started. */
export function errorText(
  error: { message?: string | null; cause?: string | null; suggested_action?: string | null } | null | undefined,
  fallback: string,
): string {
  const captions = captionsFailureText(error)
  if (captions) return captions
  const message = error?.message?.trim() ?? ''
  const action = error?.suggested_action?.trim() ?? ''
  const reason = message ? sentence(message) : fallback
  if (!action) return reason
  const seen = new Set<string>()
  return (reason + ' ' + sentence(action)).split(/(?<=[.!?])\s+/).filter((part) => {
    const key = part.trim().replace(/[.!?]+$/, '').toLocaleLowerCase()
    if (seen.has(key)) return false
    seen.add(key)
    return true
  }).join(' ')
}
