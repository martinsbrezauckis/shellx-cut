import generatedContent from './content.generated.json'

export interface ManualFeatureContent {
  id: string
  label: string
  group: string
  title: string
  where: string
  description: string
  requirement: string
  api: string
}

interface GeneratedManualContent {
  schema: 'shellx-cut/manual-content@1'
  appVersion: string
  featureCount: number
  features: ManualFeatureContent[]
  aliases: Record<string, string>
  unindexed: string[]
}

export const MANUAL_CONTENT = generatedContent as GeneratedManualContent
export const MANUAL_FEATURES = MANUAL_CONTENT.features
export const MANUAL_FEATURE_BY_ID = new Map(MANUAL_FEATURES.map((feature) => [feature.id, feature]))

export function resolveManualFeatureId(id: string): string | null {
  if (MANUAL_FEATURE_BY_ID.has(id)) return id
  const alias = MANUAL_CONTENT.aliases[id]
  return alias && MANUAL_FEATURE_BY_ID.has(alias) ? alias : null
}
