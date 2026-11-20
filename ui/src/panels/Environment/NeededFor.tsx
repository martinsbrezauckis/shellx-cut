// NeededFor.tsx — under each tool in Settings, what in the app needs it,
// read from the doctor answer's requirements section (schema/requirements.json,
// the same registry the engine's refusals and the agent documents use).

import type { DoctorReport, RequirementTool } from '../../lib/doctor'

/** Requirement entries a doctor card reports the state of. */
export function toolsForCard(report: Pick<DoctorReport, 'requirements'>, cardId: string): RequirementTool[] {
  const tools = report.requirements?.tools ?? []
  return tools.filter((tool) => tool.doctor_card === cardId
    || (tool.doctor_cards_prefix !== undefined && cardId.startsWith(tool.doctor_cards_prefix)))
}

const SHOWN = 4

function list(names: string[]): string {
  if (names.length <= SHOWN) return names.join(', ')
  return `${names.slice(0, SHOWN).join(', ')} and ${names.length - SHOWN} more`
}

/** "Needed for" and "Makes better" lines for one card; unique names, in registry order. */
export function neededForLines(tools: RequirementTool[]): { needed: string[]; improves: string[] } {
  const needed = [...new Set(tools.flatMap((tool) => tool.needed_for ?? []))]
  const improves = [...new Set(tools.flatMap((tool) => tool.improves ?? []))].filter((name) => !needed.includes(name))
  return { needed, improves }
}

export default function NeededFor({ cardId, tools }: { cardId: string; tools: RequirementTool[] }) {
  const { needed, improves } = neededForLines(tools)
  if (needed.length === 0 && improves.length === 0) return null
  return (
    <div className="env-card-needed" data-cut-env-needed-for={cardId}>
      {needed.length > 0 && (
        <span title={needed.join(', ')} data-cut-env-needed-list>Needed for: {list(needed)}.</span>
      )}
      {improves.length > 0 && (
        <span title={improves.join(', ')} data-cut-env-improves-list> Makes better: {list(improves)}.</span>
      )}
    </div>
  )
}
