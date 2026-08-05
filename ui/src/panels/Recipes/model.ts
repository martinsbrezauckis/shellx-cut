import type { RecipeManifest } from '../../lib/client'

export const RECIPE_ACTIONS_REQUIRING_PREVIEW = new Set([
  'transcript.remove_silences',
  'transcript.remove_fillers',
  'transcript.remove_retakes',
  'audio.cleanup_voice',
  'captions.generate',
  'edit.add_mask',
  'edit.trim_edges',
  'render.final',
  'render.bundle',
  'export.publish',
])

export function recipeNeedsPreview(recipe: Pick<RecipeManifest, 'stages'>): boolean {
  return recipe.stages.some((stage) => RECIPE_ACTIONS_REQUIRING_PREVIEW.has(stage.verb))
}
