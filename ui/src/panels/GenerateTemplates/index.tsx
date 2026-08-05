import { useEffect, useMemo, useState } from 'react'
import {
  callVerb,
  type GenerateFromPromptResult,
  type GenerateInsertResult,
  type GeneratePreviewResult,
  type GenerateStoryboardResult,
  type GenerateTemplateManifest,
  type GenerateTemplateSummary,
  type Project,
} from '../../lib/client'
import GenerateAssetSurface from '../Generate'
import PromptPanel from './PromptPanel'
import StoryboardPanel from './StoryboardPanel'
import TemplatePanel from './TemplatePanel'
import WorkspaceTabs from './WorkspaceTabs'
import {
  fieldLabel,
  missingRequired,
  seedParams,
  serializeParams,
  templateListResultFrom,
  templateManifestFrom,
  type GenerateWorkspaceTab,
  type KindFilter,
  type ParamValues,
  type PromptAgent,
  type PromptPolicy,
  type StoryboardMode,
} from './model'
import './generateTemplates.css'

export type { GenerateWorkspaceTab } from './model'

interface GenerateTemplatesWorkspaceProps {
  project: Project | null
  playheadMs: number
  selectedClipId?: string | null
  onInserted?: () => void
  activeTab?: GenerateWorkspaceTab
  onTab?: (tab: GenerateWorkspaceTab) => void
}

export default function GenerateTemplatesWorkspace({ project, playheadMs, selectedClipId, onInserted, activeTab, onTab }: GenerateTemplatesWorkspaceProps) {
  const [localTab, setLocalTab] = useState<GenerateWorkspaceTab>('templates')
  const tab = activeTab ?? localTab
  const selectTab = (next: GenerateWorkspaceTab) => {
    if (onTab) onTab(next)
    else setLocalTab(next)
  }
  const [kind, setKind] = useState<KindFilter>('all')
  const [query, setQuery] = useState('')
  const [templates, setTemplates] = useState<GenerateTemplateSummary[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [manifest, setManifest] = useState<GenerateTemplateManifest | null>(null)
  const [params, setParams] = useState<ParamValues>({})
  const [atMs, setAtMs] = useState(Math.max(0, Math.round(playheadMs)))
  const [atTouched, setAtTouched] = useState(false)
  const [loadingList, setLoadingList] = useState(false)
  const [loadingManifest, setLoadingManifest] = useState(false)
  const [busy, setBusy] = useState<'preview' | 'insert' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [preview, setPreview] = useState<GeneratePreviewResult | null>(null)
  const [insertResult, setInsertResult] = useState<GenerateInsertResult | null>(null)
  const [promptText, setPromptText] = useState('')
  const [promptPolicy, setPromptPolicy] = useState<PromptPolicy>('preview')
  const [promptAgent, setPromptAgent] = useState<PromptAgent>('auto')
  const [promptBusy, setPromptBusy] = useState(false)
  const [promptResult, setPromptResult] = useState<GenerateFromPromptResult | null>(null)
  const [storyboardInput, setStoryboardInput] = useState('')
  const [storyboardMode, setStoryboardMode] = useState<StoryboardMode>('quick_prompt')
  const [storyboardAgent, setStoryboardAgent] = useState<PromptAgent>('auto')
  const [storyboardAnswers, setStoryboardAnswers] = useState<Record<string, string>>({})
  const [storyboardBusy, setStoryboardBusy] = useState<PromptPolicy | null>(null)
  const [storyboardResult, setStoryboardResult] = useState<GenerateStoryboardResult | null>(null)
  const [storyboardError, setStoryboardError] = useState<string | null>(null)

  useEffect(() => {
    if (!atTouched) setAtMs(Math.max(0, Math.round(playheadMs)))
  }, [playheadMs, atTouched])

  useEffect(() => {
    let alive = true
    const timer = window.setTimeout(() => {
      setLoadingList(true)
      setError(null)
      void callVerb('generate.list', {
        kind,
        source: 'all',
        query: query.trim() || undefined,
      }).then((r) => {
        if (!alive) return
        const listResult = r.ok ? templateListResultFrom(r.result) : null
        if (listResult) {
          const next = listResult.templates
          setTemplates(next)
          setSelectedId((current) => {
            if (current && next.some((t) => t.id === current)) return current
            return next.find((t) => t.id === 'builtin.lower-third.clean')?.id ?? next[0]?.id ?? null
          })
        } else if (r.ok) {
          setTemplates([])
          setSelectedId(null)
          setError('generate.list returned an invalid template list')
        } else {
          setTemplates([])
          setSelectedId(null)
          setError(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'could not load Generate templates'}`)
        }
      }).catch(() => {
        if (!alive) return
        setTemplates([])
        setSelectedId(null)
        setError('server unreachable while loading Generate templates')
      }).finally(() => {
        if (alive) setLoadingList(false)
      })
    }, 120)
    return () => {
      alive = false
      window.clearTimeout(timer)
    }
  }, [kind, query])

  useEffect(() => {
    if (!selectedId) {
      setManifest(null)
      setParams({})
      return
    }
    let alive = true
    setLoadingManifest(true)
    setError(null)
    setPreview(null)
    setInsertResult(null)
    void callVerb('generate.describe', { id: selectedId }).then((r) => {
      if (!alive) return
      const manifestResult = r.ok ? templateManifestFrom(r.result) : null
      if (manifestResult) {
          setManifest(manifestResult)
          setParams(seedParams(manifestResult))
      } else if (r.ok) {
        setManifest(null)
        setParams({})
        setError('generate.describe returned an invalid template manifest')
      } else {
        setManifest(null)
        setParams({})
        setError(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'could not describe template'}`)
      }
    }).catch(() => {
      if (!alive) return
      setManifest(null)
      setParams({})
      setError('server unreachable while describing Generate template')
    }).finally(() => {
      if (alive) setLoadingManifest(false)
    })
    return () => { alive = false }
  }, [selectedId])

  const missing = useMemo(() => missingRequired(manifest, params), [manifest, params])
  const selectedTemplate = templates.find((t) => t.id === selectedId) ?? null
  // Keep Preview/Insert clickable when required values are missing so the
  // validation path can reveal and focus the first field instead of leaving a
  // disabled command beside an off-screen hint.
  const canRun = !!manifest && !busy && !!project
  const canRunPrompt = !!project && promptText.trim().length > 0 && !promptBusy
  const canRunStoryboard = !!project && storyboardInput.trim().length > 0 && !storyboardBusy

  const setParam = (name: string, value: unknown) => {
    setParams((p) => ({ ...p, [name]: value }))
    setPreview(null)
    setInsertResult(null)
    setError(null)
  }

  const validateForAction = () => {
    if (!manifest) {
      setError('Select a Generate template first.')
      return null
    }
    if (!project) {
      setError('Create or open a project first.')
      return null
    }
    const miss = missingRequired(manifest, params)
    if (miss.length > 0) {
      setError(`Fill required field${miss.length === 1 ? '' : 's'}: ${miss.map(fieldLabel).join(', ')}`)
      window.requestAnimationFrame(() => {
        const field = Array.from(document.querySelectorAll<HTMLElement>('[data-cut-generate-param]'))
          .find((element) => element.dataset.cutGenerateParam === miss[0])
        field?.scrollIntoView({ block: 'center', behavior: 'smooth' })
        field?.focus({ preventScroll: true })
      })
      return null
    }
    return serializeParams(manifest, params)
  }

  const runPreview = async () => {
    const serialized = validateForAction()
    if (!manifest || !serialized) return
    setBusy('preview')
    setError(null)
    setPreview(null)
    try {
      const r = await callVerb('generate.preview', {
        id: manifest.id,
        params: serialized,
        width: 640,
        height: 360,
        frame_ms: atMs,
      })
      if (r.ok && r.result) setPreview(r.result)
      else if (r.ok) setError('preview returned no result')
      else setError(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'preview failed'}`)
    } catch {
      setError('server unreachable during Generate preview')
    } finally {
      setBusy(null)
    }
  }

  const runPrompt = async () => {
    const prompt = promptText.trim()
    if (!prompt || promptBusy) return
    if (!project) {
      setError('Create or open a project first.')
      return
    }
    setPromptBusy(true)
    setError(null)
    setPromptResult(null)
    try {
      const r = await callVerb('generate.from_prompt', {
        prompt,
        policy: promptPolicy,
        agent: promptAgent,
        template_id: selectedId ?? undefined,
        at_ms: atMs,
        width: 640,
        height: 360,
        rationale: 'human: generate prompt',
      })
      if (r.ok && r.result) {
        setPromptResult(r.result)
        if (r.result.insert) onInserted?.()
      } else if (r.ok) {
        setError('prompt generation returned no result')
      } else {
        setError(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'prompt generation failed'}`)
      }
    } catch {
      setError('server unreachable during Generate prompt')
    } finally {
      setPromptBusy(false)
    }
  }

  const runStoryboard = async (policy: PromptPolicy) => {
    const input = storyboardInput.trim()
    if (!input || storyboardBusy) return
    if (!project) {
      setStoryboardError('Create or open a project first.')
      return
    }
    const answers = Object.fromEntries(Object.entries(storyboardAnswers).filter(([, value]) => value.trim().length > 0))
    setStoryboardBusy(policy)
    setStoryboardError(null)
    try {
      const r = await callVerb('generate.storyboard', {
        input,
        mode: storyboardMode,
        policy,
        agent: storyboardAgent,
        answers: Object.keys(answers).length > 0 ? answers : undefined,
        context: {
          project_name: project.name,
          playhead_ms: atMs,
          selected_template_id: selectedId ?? undefined,
        },
        rationale: `human: generate storyboard ${policy}`,
      })
      if (r.ok && r.result) {
        setStoryboardResult(r.result)
        if (r.result.insert) onInserted?.()
      } else if (r.ok) {
        setStoryboardError('storyboard generation returned no result')
      } else {
        setStoryboardError(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'storyboard generation failed'}`)
      }
    } catch {
      setStoryboardError('server unreachable during Generate storyboard')
    } finally {
      setStoryboardBusy(null)
    }
  }

  const runInsert = async () => {
    const serialized = validateForAction()
    if (!manifest || !serialized) return
    setBusy('insert')
    setError(null)
    try {
      const r = await callVerb('generate.insert', {
        id: manifest.id,
        params: serialized,
        at_ms: atMs,
        rationale: 'human: generate template insert',
      })
      if (r.ok && r.result) {
        setInsertResult(r.result)
        onInserted?.()
      } else if (r.ok) {
        setError('insert returned no result')
      } else {
        setError(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'insert failed'}`)
      }
    } catch {
      setError('server unreachable during Generate insert')
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="gt" data-cut-panel="generate-templates" aria-label="Generate templates">
      <header className="gt-head">
        <div>
          <h2>Generate</h2>
          <p>
            {loadingList ? 'Loading templates' : `${templates.length} template${templates.length === 1 ? '' : 's'}`}
            {selectedTemplate ? ` · ${selectedTemplate.title}` : ''}
            {project ? ` · ${project.name}.cutproj` : ' · no project open'}
          </p>
        </div>
        <div className="gt-head__facts" data-cut-generate-template-status>
          <span>{project ? 'Project ready' : 'Project required'}</span>
          <span>{manifest?.lowering.verb ?? 'No template'}</span>
        </div>
      </header>

      <WorkspaceTabs tab={tab} onTab={selectTab} />

      {tab === 'templates' ? (
        <TemplatePanel
          kind={kind}
          query={query}
          templates={templates}
          selectedId={selectedId}
          loadingList={loadingList}
          loadingManifest={loadingManifest}
          manifest={manifest}
          params={params}
          atMs={atMs}
          projectReady={!!project}
          canRun={canRun}
          busy={busy}
          missing={missing}
          error={error}
          preview={preview}
          insertResult={insertResult}
          onKind={setKind}
          onQuery={setQuery}
          onSelected={setSelectedId}
          onParam={setParam}
          onAtMs={(value) => {
            setAtTouched(true)
            setAtMs(value)
          }}
          onPreview={() => void runPreview()}
          onInsert={() => void runInsert()}
        />
      ) : tab === 'prompt' ? (
        <PromptPanel
          projectReady={!!project}
          selectedId={selectedId}
          promptText={promptText}
          promptPolicy={promptPolicy}
          promptAgent={promptAgent}
          atMs={atMs}
          canRunPrompt={canRunPrompt}
          promptBusy={promptBusy}
          promptResult={promptResult}
          error={error}
          onPromptText={(value) => {
            setPromptText(value)
            setPromptResult(null)
            setError(null)
          }}
          onPromptPolicy={setPromptPolicy}
          onPromptAgent={setPromptAgent}
          onAtMs={(value) => {
            setAtTouched(true)
            setAtMs(value)
          }}
          onRun={() => void runPrompt()}
        />
      ) : tab === 'storyboard' ? (
        <StoryboardPanel
          projectReady={!!project}
          storyboardInput={storyboardInput}
          storyboardMode={storyboardMode}
          storyboardAgent={storyboardAgent}
          storyboardAnswers={storyboardAnswers}
          atMs={atMs}
          canRunStoryboard={canRunStoryboard}
          storyboardBusy={storyboardBusy}
          storyboardResult={storyboardResult}
          storyboardError={storyboardError}
          onStoryboardInput={(value) => {
            setStoryboardInput(value)
            setStoryboardResult(null)
            setStoryboardError(null)
          }}
          onStoryboardMode={(value) => {
            setStoryboardMode(value)
            setStoryboardResult(null)
            setStoryboardError(null)
          }}
          onStoryboardAgent={setStoryboardAgent}
          onAnswer={(questionId, value) => setStoryboardAnswers((answers) => ({ ...answers, [questionId]: value }))}
          onAtMs={(value) => {
            setAtTouched(true)
            setAtMs(value)
          }}
          onRun={(policy) => void runStoryboard(policy)}
        />
      ) : (
        <div className="gt-media" data-cut-generate-media-panel>
          <div className="gt-media__intro" data-cut-generate-media-intro>
            <strong>AI media</strong>
            <span>Creates image or video assets through the selected generation provider, then inserts them into the Cut project.</span>
          </div>
          <GenerateAssetSurface
            project={project}
            playheadMs={playheadMs}
            selectedClipId={selectedClipId}
            onGenerated={onInserted}
          />
        </div>
      )}
    </section>
  )
}
