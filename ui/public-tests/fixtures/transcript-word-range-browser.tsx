import { useEffect, useMemo, useState } from 'react'
import { createRoot } from 'react-dom/client'
import AssetWords from '../../src/panels/Transcript/AssetWords'
import Transcript from '../../src/panels/Transcript'
import { activeCutSpans } from '../../src/panels/Review/shared'
import type { OpRecord, Project, Transcript as TranscriptData, WordSpan } from '../../src/lib/client'
import '../../src/panels/Transcript/transcript.css'

const op = (op_id: string, word_range: unknown): OpRecord => ({
  op_id, verb: 'transcript.cut_words', status: 'applied',
  args: { asset: 'fixture', word_range }, ts: '', actor: { kind: 'human', name: 'fixture', via: 'fixture' },
})
const malformed = [
  op('nonprogress', [2 ** 53, 2 ** 53]), op('infinity', [0, Infinity]),
  op('negative', [-1, 9]), op('fractional', [0.5, 9]), op('reversed', [9, 2]),
  op('shape', [0, 9, 10]),
  { ...op('effects', null), args: null, effects: [null, { asset: 'fixture', word_range: [0, NaN] }] } as unknown as OpRecord,
]
const wordsAt = (indices: number[]): WordSpan[] => indices.map((idx, position) => ({
  idx, word: `word-${idx}`, start_ms: position * 100, end_ms: position * 100 + 80,
}))
const panelProject: Project = {
  schema: 'fixture', name: 'word-range-panel', tracks: [], assets: { fixture: { path: 'fixture', hash: 'fixture' } },
  settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48000 }, markers: [], caption_styles: {}, checkpoints: [],
}
const panelTranscripts = { fixture: { asset: 'fixture', model: 'fixture', words: wordsAt([2, 3]) } } as Record<string, TranscriptData>
const panelInitialOps: OpRecord[] = [
  { ...op('prototype', [0, 1]), args: { asset: '__proto__', word_range: [0, 1] } },
  { ...op('constructor', null), args: {}, effects: [{ asset: 'constructor', word_range: [2, 3] }] },
  op('ordinary', [2, 2]),
]

function Fixture() {
  const [ops, setOps] = useState(malformed)
  const [words, setWords] = useState(() => wordsAt([2, 9, Number.MAX_SAFE_INTEGER]))
  const [ticks, setTicks] = useState(0)
  const [restored, setRestored] = useState('')
  const [panelOps, setPanelOps] = useState(panelInitialOps)
  const cuts = useMemo(() => activeCutSpans(ops), [ops])
  useEffect(() => {
    const timer = setInterval(() => setTicks(value => value + 1), 20)
    return () => clearInterval(timer)
  }, [])
  return <>
    <output data-heartbeat>{ticks}</output>
    <output data-restored>{restored}</output>
    <button data-fixture-cuts onClick={() => setOps([
      ...malformed, op('first', [2, 9]), op('huge', [0, Number.MAX_SAFE_INTEGER]),
    ])}>Load cuts</button>
    <button data-fixture-words onClick={() => setWords(wordsAt([3, 10, Number.MAX_SAFE_INTEGER]))}>Load words</button>
    <AssetWords assetId="fixture" words={words} cuts={cuts} muted={[]} ignored={[]}
      activeIdx={-1} sel={null} pending={null} onWordDown={() => {}} onWordEnter={() => {}}
      onWordActivate={() => {}} onRestore={opId => {
        setRestored(opId)
        setOps(current => [...current, {
          ...op(`restore-${opId}`, null), verb: 'edit.restore', args: { op_id: opId },
        }])
      }} />
    <div data-panel-fixture>
      <Transcript project={panelProject} ops={panelOps} transcripts={panelTranscripts} playheadMs={0}
        onCutWords={() => {}} onSeek={() => {}} onRestore={opId => setPanelOps(current => [...current, {
          ...op(`panel-restore-${opId}`, null), verb: 'edit.restore', args: { op_id: opId },
        }])} />
    </div>
  </>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
