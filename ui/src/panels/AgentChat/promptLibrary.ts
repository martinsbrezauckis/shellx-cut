export type AgentPromptCategory = 'Polish' | 'Repurpose' | 'Speech' | 'Review'

export interface AgentPromptPreset {
  id: string
  category: AgentPromptCategory
  label: string
  prompt: string
  verbs: string[]
  quick?: boolean
}

/** Curated, editable requests backed only by this Chat session's inspect/edit
 * policy. Selecting one only pre-fills Agent Chat; it never starts a turn or
 * grants a service, render, or verification capability. */
export const AGENT_PROMPT_LIBRARY: AgentPromptPreset[] = [
  {
    id: 'edit-for-clarity',
    category: 'Polish',
    label: 'Edit for clarity',
    prompt: 'Inspect the selected spoken section. Use only reversible transcript edits to remove filler words and repeated takes; explain each applied change in the turn result.',
    verbs: ['transcript.remove_fillers', 'transcript.remove_retakes', 'transcript.cut_words'],
  },
  {
    id: 'talking-head-polish',
    category: 'Polish',
    label: 'Polish talking head',
    prompt: 'Inspect the selected talking-head transcript and make only the contained transcript cleanup edits that improve pacing. Explain the changed ranges and leave rendering to the human.',
    verbs: ['transcript.remove_silences', 'transcript.remove_retakes', 'transcript.remove_fillers'],
  },
  {
    id: 'vertical-highlights',
    category: 'Repurpose',
    label: 'Repurpose as shorts',
    prompt: 'Inspect the transcript and identify self-contained highlight ranges with exact timestamps for short-form edits. Do not create clips or renders; open the Clips drawer for candidates and delivery.',
    verbs: ['transcript.timeline', 'transcript.search', 'transcript.chapters'],
    quick: true,
  },
  {
    id: 'social-package',
    category: 'Repurpose',
    label: 'Package for social',
    prompt: 'Inspect the current sequence and propose a social package for 9:16, 1:1, and 16:9 with concrete edit ranges. Do not render or deliver files; open the Clips drawer to create candidates and delivery outputs.',
    verbs: ['project.state', 'transcript.timeline', 'inspect.range'],
  },
  {
    id: 'add-captions',
    category: 'Speech',
    label: 'Add readable captions',
    prompt: 'Inspect the current spoken transcript, generate readable captions, then reflow them if the current timing needs it. Explain the caption changes in the turn result.',
    verbs: ['transcript.get', 'captions.generate', 'captions.reflow'],
  },
  {
    id: 'label-speakers',
    category: 'Speech',
    label: 'Prepare speaker labels',
    prompt: 'Inspect the current transcript and identify the source and ranges that need speaker labels. Do not run a service from Chat; open Transcript Tools > Label speakers to create them.',
    verbs: ['transcript.get', 'transcript.timeline'],
    quick: true,
  },
  {
    id: 'dub-latvian',
    category: 'Speech',
    label: 'Prepare Latvian dub',
    prompt: 'Inspect the current transcript and identify the source ready for a Latvian dubbed track. Do not run a service from Chat; open Transcript Tools > Dub audio to choose the language and create it.',
    verbs: ['transcript.get', 'transcript.timeline'],
    quick: true,
  },
  {
    id: 'preflight-review',
    category: 'Review',
    label: 'Check before export',
    prompt: 'Inspect the current project and summarize the visible edit risks to review before export. Do not claim a verification pass or run output checks; open Review > QC for paced, caption, delivery, and brand verification.',
    verbs: ['project.health', 'project.state', 'inspect.range'],
  },
]

export const AGENT_PROMPT_CATEGORIES: AgentPromptCategory[] = ['Polish', 'Repurpose', 'Speech', 'Review']
export const AGENT_QUICK_PROMPTS = AGENT_PROMPT_LIBRARY.filter((preset) => preset.quick)
