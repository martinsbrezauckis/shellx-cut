export type AgentPromptCategory = 'Polish' | 'Repurpose' | 'Speech' | 'Review'

export interface AgentPromptPreset {
  id: string
  category: AgentPromptCategory
  label: string
  prompt: string
  verbs: string[]
  quick?: boolean
}

/** Curated, editable requests backed only by existing Cut verbs and recipes.
 * Selecting one pre-fills Agent Chat; it never launches a turn by itself. */
export const AGENT_PROMPT_LIBRARY: AgentPromptPreset[] = [
  {
    id: 'edit-for-clarity',
    category: 'Polish',
    label: 'Edit for clarity',
    prompt: 'Clean this spoken edit with the Edit for Clarity recipe at Natural intensity. Show the exact plan before applying it.',
    verbs: ['recipe.run'],
  },
  {
    id: 'talking-head-polish',
    category: 'Polish',
    label: 'Polish talking head',
    prompt: 'Polish this talking-head edit with the talking-head cleanup recipe. Show the exact plan before applying it.',
    verbs: ['recipe.run'],
  },
  {
    id: 'vertical-highlights',
    category: 'Repurpose',
    label: 'Repurpose as shorts',
    prompt: 'Find the strongest self-contained moments and repurpose them as vertical shorts for TikTok and Reels. Show the plan before changing the timeline.',
    verbs: ['clip.candidates', 'render.bundle'],
    quick: true,
  },
  {
    id: 'social-package',
    category: 'Repurpose',
    label: 'Package for social',
    prompt: 'Create a social package from the current sequence in 9:16, 1:1, and 16:9. Show the output plan before rendering.',
    verbs: ['recipe.run', 'render.bundle'],
  },
  {
    id: 'add-captions',
    category: 'Speech',
    label: 'Add readable captions',
    prompt: 'Transcribe the current spoken clip and add readable captions with the add-captions recipe. Show the plan before applying it.',
    verbs: ['recipe.run'],
  },
  {
    id: 'label-speakers',
    category: 'Speech',
    label: 'Label speakers',
    prompt: 'Label the speakers in this video and show who is talking in each section.',
    verbs: ['media.diarize'],
    quick: true,
  },
  {
    id: 'dub-latvian',
    category: 'Speech',
    label: 'Dub to Latvian',
    prompt: 'Dub the timeline audio into Latvian while preserving the original timing.',
    verbs: ['audio.dub'],
    quick: true,
  },
  {
    id: 'preflight-review',
    category: 'Review',
    label: 'Check before export',
    prompt: 'Run pre-render checks for pacing, captions, delivery, and brand. Report the issues without changing the timeline.',
    verbs: ['verify.pregate', 'verify.pacing', 'verify.captions', 'verify.delivery', 'verify.brand'],
  },
]

export const AGENT_PROMPT_CATEGORIES: AgentPromptCategory[] = ['Polish', 'Repurpose', 'Speech', 'Review']
export const AGENT_QUICK_PROMPTS = AGENT_PROMPT_LIBRARY.filter((preset) => preset.quick)
