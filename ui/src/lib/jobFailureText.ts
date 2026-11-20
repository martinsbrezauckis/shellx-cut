// The text a person reads when a background job fails: the plain reason the
// engine gave, followed by the next step it suggests. Every surface that shows
// a failed job (the status bar, Transcript Tools, Health and Recovery) uses
// this one function, so they all show the same words.

import type { JobRecord } from './jobModel'

function sentence(text: string): string {
  const trimmed = text.trim()
  if (!trimmed) return ''
  const capital = trimmed[0].toUpperCase() + trimmed.slice(1)
  return /[.!?]$/.test(capital) ? capital : capital + '.'
}

export function jobFailureText(record: JobRecord, fallback: string): string {
  return errorText(record.error, fallback)
}

/** The same words for an error a verb returned before any job started. */
export function errorText(
  error: { message?: string | null; suggested_action?: string | null } | null | undefined,
  fallback: string,
): string {
  const message = error?.message?.trim() ?? ''
  const action = error?.suggested_action?.trim() ?? ''
  const reason = message ? sentence(message) : fallback
  return action ? reason + ' ' + sentence(action) : reason
}
