import { Icon } from '../../icons'
import type { BrollPlaced, RepurposeClip, ScriptSegment, ShortsItem } from './assemblePlanModel'

const fmtTc = (ms: number) => {
  const s = Math.max(0, Math.round(ms / 1000))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}
const fmtScore = (value: number) => Math.abs(value) <= 1 ? `${Math.round(value * 100)}%` : `${Math.round(value)}`

export function AssembleResults({
  shorts, repurposeClips, segments, placed, aspect, onSeek,
}: {
  shorts: ShortsItem[] | null
  repurposeClips: RepurposeClip[] | null
  segments: ScriptSegment[] | null
  placed: BrollPlaced[] | null
  aspect: string
  onSeek?: (atMs: number) => void
}) {
  return <>
    {shorts && shorts.length > 0 && <div className="cd-results" data-cut-assemble-results="shorts">
      {shorts.map((item) => <div className="cd-result-row" key={item.rank} data-cut-assemble-result={item.rank}>
        <div className="cd-result-head"><span className="cd-result-rank">#{item.rank}</span><span className="cd-result-tc">{fmtTc(item.range_ms[0])}–{fmtTc(item.range_ms[1])}</span><span className="cd-result-score">{fmtScore(item.score)}</span>
          {onSeek && <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={item.rank} onClick={() => onSeek(item.range_ms[0])}>Jump</button>}</div>
        <p className="cd-result-text">{item.title}</p><p className="cd-result-reason">{Math.round(item.duration_ms / 1000)}s · {item.reframe?.aspect ?? aspect}{item.has_captions ? ' · captions' : ''}{item.factors ? ` · ${Object.entries(item.factors).map(([key, value]) => `${key.split('_')[0]} ${Math.round(value * 100)}`).join(' / ')}` : ''}</p>
      </div>)}
    </div>}
    {repurposeClips && repurposeClips.length > 0 && <div className="cd-results" data-cut-assemble-results="repurpose">
      {repurposeClips.map((clip) => <div className="cd-result-row" key={clip.rank} data-cut-assemble-result={clip.rank}>
        <div className="cd-result-head"><span className="cd-result-rank">#{clip.rank}</span><span className="cd-result-tc">{fmtTc(clip.range_ms[0])}–{fmtTc(clip.range_ms[1])}</span><span className="cd-result-score">{fmtScore(clip.score)}</span>
          {onSeek && <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={clip.rank} onClick={() => onSeek(clip.range_ms[0])}>Jump</button>}</div>
        <p className="cd-result-text">{clip.text}</p>{clip.reason && <p className="cd-result-reason">{clip.reason}</p>}
      </div>)}
    </div>}
    {segments && segments.length > 0 && <div className="cd-results" data-cut-assemble-results="from_script">
      {segments.map((segment) => <div className={`cd-result-row ${segment.matched ? '' : 'cd-result-row--unmatched'}`} key={segment.line_idx} data-cut-assemble-result={segment.line_idx} data-cut-assemble-matched={String(segment.matched)}>
        <div className="cd-result-head"><span className="cd-result-rank">{segment.matched ? <Icon name="check" size={14} tone="success" /> : '—'}</span><span className="cd-result-line">{segment.script_line}</span>{segment.matched && <span className="cd-result-score">{fmtScore(segment.score)}</span>}
          {segment.matched && segment.range_ms && onSeek && <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={segment.line_idx} onClick={() => onSeek(segment.range_ms![0])}>Jump</button>}</div>
        {segment.matched && segment.text && <p className="cd-result-text">{segment.text}</p>}
      </div>)}
    </div>}
    {placed && placed.length > 0 && <div className="cd-results" data-cut-assemble-results="broll">
      {placed.map((item, index) => <div className="cd-result-row" key={index} data-cut-assemble-result={index}>
        <div className="cd-result-head"><span className="cd-result-tc">@ {fmtTc(item.at_ms)}</span><span className="cd-result-line">{item.query}</span>
          {onSeek && <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={index} onClick={() => onSeek(item.at_ms)}>Jump</button>}</div>
      </div>)}
    </div>}
  </>
}
