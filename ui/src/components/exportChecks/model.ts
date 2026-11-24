// What a person sees of an export check list, as plain data: which rows, in which
// order, with which words. No React here, so it is easy to test and to reuse.
//
// The list arrives as untrusted JSON typed as ExportChecks, so every read below
// tolerates a missing or odd field. An entry that cannot be drawn is skipped; an
// entry in a state we do not know is drawn as "not checked", never as fine.
import type { ExportCheckAction, ExportCheckEntry, ExportCheckPhase, ExportChecks } from '../../lib/exportChecks'

/** The four marks. Each has its own shape in the list, so colour is never the only signal. */
export type CheckMark = 'found' | 'error' | 'not_checked' | 'fine'

export const MARK_WORDS: Record<CheckMark, string> = {
  found: 'Found something',
  error: 'Could not run',
  not_checked: 'Not checked',
  fine: 'Passed',
}

export interface CheckRow {
  entry: ExportCheckEntry
  id: string
  name: string
  /** The engine's result, as sent. The row's spoken name always carries all of it. */
  result: string
  /** What the row draws after the name: empty when the result only repeats the words of the line it sits under. */
  shownResult: string
  mark: CheckMark
  stateWords: string
  /** The one sentence shown when the row opens. */
  sentence: string
  /** One short extra line for a found row whose files were only partly checked. */
  extraLine?: string
  /** The one usable action, or nothing: never a button that cannot work. */
  action?: ExportCheckAction
}

export interface ExportCheckView {
  found: CheckRow[]
  errors: CheckRow[]
  suggestions: CheckRow[]
  passed: CheckRow[]
  onRequest: CheckRow[]
  /** Found plus errors: what needs a look. */
  lookCount: number
  header: string
}

const KINDS = new Set(['seek', 'open_tools', 'copy_details', 'command'])

export const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`
const foundWords = (count: number) => `${plural(count, 'thing')} to look at`
const errorWords = (count: number) => `${plural(count, 'check')} could not run`

const nonEmpty = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0

function markOf(state: unknown): CheckMark | null {
  if (state === 'found') return 'found'
  if (state === 'error') return 'error'
  if (state === 'checked') return 'fine'
  if (state === 'not_applicable') return null
  return 'not_checked'
}

function drawable(value: unknown): value is ExportCheckEntry {
  const entry = value as Partial<ExportCheckEntry> | null
  return !!entry && typeof entry === 'object' && nonEmpty(entry.id) && nonEmpty(entry.name)
}

/** An action is shown only when it has what it needs to work. */
export function usableAction(entry: ExportCheckEntry): ExportCheckAction | undefined {
  const action = entry.action
  if (!action || typeof action !== 'object' || !KINDS.has(action.kind) || !nonEmpty(action.label)) return undefined
  // Output does not exist yet: there is nothing to act on.
  if (entry.reason_code === 'after_export') return undefined
  if (action.kind === 'seek') return Number.isFinite(action.at_ms) ? action : undefined
  if (action.kind === 'open_tools') return nonEmpty(action.tool_id) ? action : undefined
  if (action.kind === 'copy_details') return nonEmpty(action.text) ? action : undefined
  const args = action.args
  return nonEmpty(action.verb) && !!args && typeof args === 'object' && !Array.isArray(args) ? action : undefined
}

/** The first sentence of a longer explanation, with a full stop. An opened row says one sentence, never a paragraph. */
function firstSentence(text: string): string {
  const first = text.trim().split(/(?<=[.!?])\s+/)[0] ?? ''
  return /[.!?]$/.test(first) ? first : `${first}.`
}

function sentenceFor(entry: ExportCheckEntry, action: ExportCheckAction | undefined): string {
  // The state decides first, so an engine explanation (which can hold tool or error words)
  // is shown only for a finding, where it is the sentence about the finding itself.
  if (entry.state === 'found') return nonEmpty(entry.reason) ? firstSentence(entry.reason) : 'Something was found here.'
  if (entry.state === 'error') return action?.kind === 'copy_details' ? 'This check could not run; copy the details to report it.' : 'This check could not run.'
  if (entry.state === 'checked') return 'Checked, nothing found.'
  switch (entry.reason_code) {
    case 'optional_tool_missing': return 'Not checked because its tools are not installed.'
    case 'not_run_yet': return 'Not checked yet.'
    case 'unknown_older_analysis': return 'Not checked: the saved results are from an older version.'
    case 'after_export': return 'This is checked once the file is made.'
    case 'on_request': return 'This runs only when you ask.'
    default: return 'Not checked.'
  }
}

/** A result that only says again what the folded line above it says ("fine" under "11 checks passed",
 *  "on request" under "6 more checks on request") is not drawn: the rows stay names only. */
function repeatsItsFold(mark: CheckMark, entry: ExportCheckEntry, result: string): boolean {
  const said = result.toLowerCase().replace(/[.\s]+$/, '')
  return (mark === 'fine' && said === 'fine') || (mark === 'not_checked' && entry.reason_code === 'on_request' && said === 'on request')
}

function rowFor(entry: ExportCheckEntry, mark: CheckMark): CheckRow {
  const action = usableAction(entry)
  const unchecked = Array.isArray(entry.coverage?.unchecked_assets) ? entry.coverage.unchecked_assets.length : 0
  const result = nonEmpty(entry.result) ? entry.result.trim() : ''
  return {
    entry,
    id: entry.id,
    name: entry.name.trim(),
    result,
    shownResult: repeatsItsFold(mark, entry, result) ? '' : result,
    mark,
    stateWords: MARK_WORDS[mark],
    sentence: sentenceFor(entry, action),
    // A found result wins the row; files that were not checked are said in one short line.
    extraLine: mark === 'found' && unchecked > 0 ? `${plural(unchecked, 'file')} not checked` : undefined,
    action,
  }
}

export function buildExportCheckView(checks: ExportChecks | null | undefined, phase?: ExportCheckPhase): ExportCheckView {
  const view: ExportCheckView = { found: [], errors: [], suggestions: [], passed: [], onRequest: [], lookCount: 0, header: '' }
  const entries = Array.isArray(checks?.entries) ? checks.entries : []
  const at = phase ?? checks?.phase
  for (const value of entries) {
    if (!drawable(value)) continue
    const mark = markOf(value.state)
    if (!mark) continue
    if (mark === 'not_checked' && value.reason_code === 'after_export' && at === 'before_export') continue
    const row = rowFor(value, mark)
    if (mark === 'found') view.found.push(row)
    else if (mark === 'error') view.errors.push(row)
    else if (mark === 'fine') view.passed.push(row)
    else if (value.reason_code === 'on_request') view.onRequest.push(row)
    else view.suggestions.push(row)
  }
  view.lookCount = view.found.length + view.errors.length
  // One header line, in the same words as the summary line, so the two never give two different counts.
  if (view.found.length > 0 && view.errors.length > 0) view.header = `${foundWords(view.found.length)}, ${errorWords(view.errors.length)}`
  else if (view.found.length > 0) view.header = `${foundWords(view.found.length)} before exporting`
  else if (view.errors.length > 0) view.header = errorWords(view.errors.length)
  else if (view.suggestions.length > 0) view.header = 'Nothing found, some checks did not run'
  else if (view.passed.length > 0) view.header = 'All checks passed'
  else view.header = 'No checks to show'
  return view
}

/** True when the export warning should open: something found, or a check that failed. Never for not-checked alone. */
export function exportChecksOpenWarning(checks: ExportChecks | null | undefined): boolean {
  const view = buildExportCheckView(checks)
  return view.lookCount > 0
}

export interface ExportCheckSummary { text: string; mark: CheckMark }

/** Longest summary, in characters, that stays on one line at the width of the export warning. */
const ONE_LINE = 52

const toolsPhrase = (rows: CheckRow[]) => {
  const tools = new Set(rows.map((row) => row.action?.kind === 'open_tools' ? row.action.tool_id : undefined))
  return tools.size === 1 && tools.has('perception') ? 'install the captions tools' : 'install the missing tools'
}

/** One quiet line for the export panel and the finished file. */
export function summarizeExportChecks(checks: ExportChecks | null | undefined, phase?: ExportCheckPhase): ExportCheckSummary | null {
  const view = buildExportCheckView(checks, phase)
  const missing = view.suggestions.filter((row) => row.entry.reason_code === 'optional_tool_missing')
  const notRun = view.suggestions.length - missing.length
  const pieces: string[] = []
  if (view.found.length) pieces.push(foundWords(view.found.length))
  if (view.errors.length) pieces.push(errorWords(view.errors.length))
  if (missing.length) pieces.push(`${plural(missing.length, 'check')} not available: ${toolsPhrase(missing)}`)
  if (notRun) pieces.push(`${plural(notRun, 'check')} not run yet`)
  const mark: CheckMark = view.found.length ? 'found' : view.errors.length ? 'error' : view.suggestions.length ? 'not_checked' : 'fine'
  if (pieces.length) {
    // The two things that matter most, but only while they stay on one line at the warning width;
    // the rows say the rest.
    const both = pieces.slice(0, 2).join(' · ')
    return { text: both.length <= ONE_LINE ? both : pieces[0], mark }
  }
  const passed = view.passed.length
  if (!passed) return null
  return { text: passed === 1 ? '1 check passed' : `All ${passed} checks passed`, mark }
}
