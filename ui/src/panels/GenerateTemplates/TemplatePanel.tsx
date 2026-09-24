import type {
  GenerateInsertResult,
  GenerateParam,
  GeneratePreviewResult,
  GenerateTemplateManifest,
  GenerateTemplateSummary,
} from '../../lib/client'
import { useState } from 'react'
import { Icon } from '../../icons'
import {
  colorValue,
  fieldLabel,
  isBlank,
  type KindFilter,
  type ParamValues,
} from './model'
import TemplateCatalog from './TemplateCatalog'

const COLOR_PRESETS = [
  { name: 'Gold', value: '#FFD24A' },
  { name: 'Blue', value: '#22AAFF' },
  { name: 'Mint', value: '#33CC99' },
  { name: 'Coral', value: '#FF6B6B' },
  { name: 'White', value: '#FFFFFF' },
  { name: 'Black', value: '#000000' },
] as const

function ColorParamField({
  name,
  value,
  common,
  onParam,
}: {
  name: string
  value: unknown
  common: { 'data-cut-generate-param': string; 'aria-invalid': boolean | undefined }
  onParam: (name: string, value: unknown) => void
}) {
  const [open, setOpen] = useState(false)
  const label = fieldLabel(name)
  return (
    <div className="gt-color">
      <button
        type="button"
        className="gt-color__swatch"
        {...common}
        data-cut-generate-param-control={name}
        aria-label={`Choose ${label} color`}
        aria-expanded={open}
        aria-controls={open ? `gt-color-palette-${name}` : undefined}
        style={{ backgroundColor: colorValue(value) }}
        onClick={() => setOpen((wasOpen) => !wasOpen)}
      />
      <input
        className="gt-input gt-input--mono"
        data-cut-generate-param-text={name}
        aria-label={`${label} color value`}
        value={String(value ?? '')}
        onChange={(e) => onParam(name, e.target.value)}
      />
      {open && (
        <div className="gt-color__palette" id={`gt-color-palette-${name}`} role="group" aria-label={`${label} color presets`}>
          {COLOR_PRESETS.map((preset) => (
            <button
              key={preset.value}
              type="button"
              className="gt-color__preset"
              data-cut-generate-param-control={`${name}-preset-${preset.value}`}
              aria-label={`${preset.name} ${preset.value}`}
              aria-pressed={colorValue(value).toUpperCase() === preset.value}
              title={`${preset.name} ${preset.value}`}
              style={{ backgroundColor: preset.value }}
              onClick={() => {
                onParam(name, preset.value)
                setOpen(false)
              }}
            />
          ))}
        </div>
      )}
    </div>
  )
}

interface TemplatePanelProps {
  kind: KindFilter
  query: string
  templates: GenerateTemplateSummary[]
  selectedId: string | null
  loadingList: boolean
  loadingManifest: boolean
  manifest: GenerateTemplateManifest | null
  params: ParamValues
  atMs: number
  projectReady: boolean
  canRun: boolean
  busy: 'preview' | 'insert' | null
  missing: string[]
  error: string | null
  preview: GeneratePreviewResult | null
  insertResult: GenerateInsertResult | null
  onKind: (kind: KindFilter) => void
  onQuery: (query: string) => void
  onSelected: (id: string) => void
  onParam: (name: string, value: unknown) => void
  onAtMs: (value: number) => void
  onPreview: () => void
  onInsert: () => void
}

export default function TemplatePanel({
  kind,
  query,
  templates,
  selectedId,
  loadingList,
  loadingManifest,
  manifest,
  params,
  atMs,
  projectReady,
  canRun,
  busy,
  missing,
  error,
  preview,
  insertResult,
  onKind,
  onQuery,
  onSelected,
  onParam,
  onAtMs,
  onPreview,
  onInsert,
}: TemplatePanelProps) {
  const renderField = (name: string, param: GenerateParam) => {
    const value = params[name]
    const requiredMissing = param.required && isBlank(value)
    const common = {
      'data-cut-generate-param': name,
      'aria-invalid': requiredMissing || undefined,
    }
    if (Array.isArray(param.enum) && param.enum.length > 0) {
      return (
        <select
          {...common}
          data-cut-generate-param-control={name}
          className="gt-input"
          value={String(value ?? '')}
          onChange={(e) => onParam(name, e.target.value)}
        >
          {param.enum.map((opt) => (
            <option key={String(opt)} value={String(opt)}>{String(opt)}</option>
          ))}
        </select>
      )
    }
    if (param.type === 'boolean') {
      return (
        <label className="gt-check">
          <input
            {...common}
            data-cut-generate-param-control={name}
            type="checkbox"
            checked={Boolean(value)}
            onChange={(e) => onParam(name, e.target.checked)}
          />
          <span>{Boolean(value) ? 'On' : 'Off'}</span>
        </label>
      )
    }
    if (param.type === 'color') {
      return (
        <ColorParamField name={name} value={value} common={common} onParam={onParam} />
      )
    }
    return (
      <input
        {...common}
        data-cut-generate-param-control={name}
        className="gt-input"
        type={param.type === 'integer' || param.type === 'number' ? 'number' : 'text'}
        min={param.minimum ?? undefined}
        max={param.maximum ?? undefined}
        step={param.step ?? (param.type === 'integer' ? 1 : undefined)}
        value={String(value ?? '')}
        onChange={(e) => onParam(name, param.type === 'integer' || param.type === 'number' ? Number(e.target.value) : e.target.value)}
      />
    )
  }

  return (
    <div className="gt-grid">
      <TemplateCatalog
        kind={kind}
        query={query}
        templates={templates}
        selectedId={selectedId}
        loadingList={loadingList}
        onKind={onKind}
        onQuery={onQuery}
        onSelected={onSelected}
      />

      <section className="gt-inspector" aria-label="Generate template parameters">
        {loadingManifest && <div className="gt-empty">Loading template...</div>}
        {!loadingManifest && !manifest && <div className="gt-empty">Select a template to edit its parameters.</div>}
        {manifest && (
          <>
            <div className="gt-template-head">
              <div>
                <h3>{manifest.title}</h3>
                <p>{manifest.summary}</p>
              </div>
            </div>
            <div className="gt-actions gt-actions--template-primary">
              <button
                className="gt-btn gt-btn--secondary"
                data-cut-generate-template-preview
                disabled={!canRun}
                onClick={() => onPreview()}
              >
                {busy === 'preview' ? 'Previewing...' : <><Icon name="eye" size={14} /> Preview</>}
              </button>
              <button
                className="gt-btn gt-btn--primary"
                data-cut-generate-template-insert
                disabled={!canRun}
                onClick={() => onInsert()}
              >
                {busy === 'insert' ? 'Inserting...' : <><Icon name="effect" size={14} /> Insert</>}
              </button>
            </div>

            {missing.length > 0 && (
              <div className="gt-note" data-cut-generate-template-missing>
                Fill required: {missing.map(fieldLabel).join(', ')}
              </div>
            )}
            {!projectReady && <div className="gt-note">Open a project to preview or insert templates.</div>}
            {error && <div className="gt-error" data-cut-generate-template-error role="alert">{error}</div>}
            {preview && (
              <div className="gt-note" data-cut-generate-template-preview-inline>
                Preview ready: {preview.preview_id}
              </div>
            )}
            {insertResult && (
              <div className="gt-note" data-cut-generate-template-insert-inline>
                Inserted {insertResult.clips.length} clip{insertResult.clips.length === 1 ? '' : 's'}.
              </div>
            )}
            <div className="gt-tags">
              {manifest.tags.map((tag) => <span key={tag}>{tag}</span>)}
            </div>

            <div className="gt-fields">
              {Object.entries(manifest.params).map(([name, param]) => {
                const content = <>
                  <span className="gt-field__label">
                    {fieldLabel(name)}
                    {param.required && <b>required</b>}
                  </span>
                  {renderField(name, param)}
                  {param.description && <span className="gt-field__hint">{param.description}</span>}
                </>
                return param.type === 'color'
                  ? <div className="gt-field" key={name}>{content}</div>
                  : <label className="gt-field" key={name}>{content}</label>
              })}
              <label className="gt-field">
                <span className="gt-field__label">timeline start</span>
                <input
                  className="gt-input"
                  data-cut-generate-at-ms
                  type="number"
                  min={0}
                  value={atMs}
                  onChange={(e) => onAtMs(Math.max(0, Math.round(Number(e.target.value) || 0)))}
                />
                <span className="gt-field__hint">Defaults to current playhead in milliseconds.</span>
              </label>
            </div>
          </>
        )}
      </section>

      <aside className="gt-evidence" aria-label="Generate evidence">
        <div className="gt-preview" data-cut-generate-template-preview-pane>
          {preview?.url ? (
            <img
              data-cut-generate-template-preview-img
              src={preview.url}
              alt={`${preview.id} preview`}
            />
          ) : preview ? (
            <div className="gt-note" data-cut-generate-template-preview-path>
              Preview wrote {preview.path}, but no browser URL was returned.
            </div>
          ) : (
            <div className="gt-empty">Preview evidence appears here.</div>
          )}
        </div>

        {preview && (
          <dl className="gt-result" data-cut-generate-template-preview-result>
            <dt>Preview</dt><dd>{preview.preview_id}</dd>
            <dt>Size</dt><dd>{preview.width}x{preview.height}</dd>
            <dt>Lowering</dt><dd>{preview.lowering.verb}</dd>
          </dl>
        )}

        {insertResult && (
          <dl className="gt-result" data-cut-generate-template-result>
            <dt>Checkpoint</dt><dd>{String(insertResult.checkpoint.id ?? '')}</dd>
            <dt>Ops</dt><dd>{insertResult.op_ids.join(', ')}</dd>
            <dt>Clips</dt><dd>{insertResult.clips.join(', ') || 'none'}</dd>
            <dt>Assets</dt><dd>{insertResult.assets.join(', ') || 'none'}</dd>
            <dt>Lowering</dt><dd>{insertResult.lowering.verb}</dd>
            <dt>Restore</dt><dd>{insertResult.restore_hint}</dd>
          </dl>
        )}
      </aside>
    </div>
  )
}
