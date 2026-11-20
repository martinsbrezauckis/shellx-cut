// requirementsModel.ts — the `requirements` section of system.doctor and the
// `error.requirement` of a refusal (schema/requirements.json: what each verb
// and feature needs, with the state of each tool and condition on this machine).

export type RequirementState = 'ready' | 'missing' | 'needs_attention' | 'not_checked' | 'per_asset' | 'not_applicable'

/** Where to get a tool or condition: the Settings panel (a ui.open id), the
 *  button, the verb that sets it up and what only a person can do. */
export interface RequirementGet {
  settings?: string
  button?: string
  verb?: string
  args?: Record<string, unknown>
  person?: string
}

export interface RequirementTool {
  id: string
  kind: 'tool' | 'condition'
  name: string
  what: string
  state: RequirementState
  reason?: string
  check?: { verb: string; args?: Record<string, unknown> }
  get: RequirementGet
  needed_for?: string[]
  improves?: string[]
  doctor_card?: string
  doctor_cards_prefix?: string
  platforms?: string[]
  per_asset?: boolean
}

export interface RequirementFeature {
  id: string
  name: string
  verbs: string[]
  requires: string[]
  open_project: 'all' | 'some' | 'none'
  can_run: 'yes' | 'no' | 'depends'
  blocked_by: unknown[]
  depends_on: unknown[]
  optional: Array<{ id: string; name: string; state: RequirementState; effect: string }>
  note?: string
}

export interface RequirementsSection {
  schema: string
  tools: RequirementTool[]
  features: RequirementFeature[]
}

/** `error.requirement` of a refusal about a missing tool or condition. */
export interface MissingRequirement {
  id: string
  name?: string
  get?: RequirementGet
}
