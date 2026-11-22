import { useEffect, useState } from 'react'
import InspectorSection from '../../components/inspector/InspectorSection'
import { callVerb, type RecordingZoomListResult } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import RecordingZoomControls, { type ZoomPatch } from './RecordingZoomControls'
import { RecordingZoomList, RecordingZoomRedo, RecordingZoomRefresh } from './RecordingZoomParts'
import './recording-zoom.css'

interface RecordingZoomSectionProps {
  clip: string
  revision?: string | null
  timelineAtPlayhead: number | null
}

export default function RecordingZoomSection({ clip, revision, timelineAtPlayhead }: RecordingZoomSectionProps) {
  const [data, setData] = useState<RecordingZoomListResult | null>(null)
  const [selected, setSelected] = useState('')
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState('')
  const [confirm, setConfirm] = useState(false)
  const [job, setJob] = useState<string | null>(null)
  const [progress, setProgress] = useState(0)

  useEffect(() => {
    if (busy) return
    let live = true
    void callVerb('screen_record.zoom_list', { clip }).then((response) => {
      if (!live) return
      if (response.ok && response.result) {
        const result = response.result
        setData(result)
        setSelected((id) => result.segments.some((segment) => segment.id === id)
          ? id : result.segments[0]?.id ?? '')
      } else {
        setNote(response.error?.message ?? 'Could not read zooms. Select the clip again to retry.')
      }
    })
    return () => { live = false }
  }, [clip, revision, busy])

  useEffect(() => {
    if (!job) return
    let live = true
    const timer = setInterval(() => {
      void callVerb('jobs.status', { job_id: job }).then((response) => {
        if (!live) return
        if (!response.ok || response.result?.state === 'failed') {
          setNote(response.error?.message ?? response.result?.error?.message
            ?? 'Update failed. Review the recording and try again.')
          setJob(null)
          setBusy(false)
        } else if (response.result) {
          setProgress(response.result.progress)
          if (response.result.state === 'done') {
            setJob(null)
            setBusy(false)
          }
        }
      })
    }, 600)
    return () => { live = false; clearInterval(timer) }
  }, [job])

  const perform = async (action: () => Promise<unknown>) => {
    if (busy) return
    setBusy(true)
    setNote('')
    try {
      const result = await action()
      if (result && typeof result === 'object' && 'ok' in result && !result.ok) {
        const error = 'error' in result ? result.error : null
        setNote(error && typeof error === 'object' && 'message' in error
          ? String(error.message) : 'Could not change zooms. Try again.')
      }
    } catch {
      setNote('Could not change zooms. Try again.')
    } finally {
      setBusy(false)
    }
  }

  const update = (patch: ZoomPatch) => void perform(async () => {
    const response = await runUserVerb('screen_record.zoom_update', {
      clip, id: selected, ...patch,
    }, 'Could not change this zoom.')
    if (!response?.ok) setNote(response?.error?.message ?? 'Could not change this zoom.')
  })

  const refresh = () => {
    if (busy) return
    setBusy(true)
    setNote('')
    void runUserVerb('screen_record.take_refresh', { clip }, 'Could not update video.')
      .then((response) => {
        if (response?.ok && response.result) {
          setJob(response.result.job_id)
        } else {
          setBusy(false)
          setNote(response?.error?.message
            ?? 'Could not update video. Check the recording duration and try again.')
        }
      }).catch(() => {
        setBusy(false)
        setNote('Could not update video. Try again.')
      })
  }

  const segment = data?.segments.find((item) => item.id === selected)

  return (
    <InspectorSection title="Zoom" sectionKey="recording-zoom">
      <div className="insp__group">
        {note && (
          <p role="status" className="insp__hint">
            {note} Update video only after resolving this issue.
          </p>
        )}
        <RecordingZoomRefresh
          stale={data?.stale ?? false}
          missingVideo={data?.missing_video ?? false}
          busy={busy}
          job={job}
          progress={progress}
          refresh={refresh}
        />
        {busy && <p className="insp__hint">Wait for the current zoom change to finish.</p>}
        {!data && !busy && <p className="insp__hint">Zoom controls are waiting for recording data.</p>}
        <fieldset className="insp__zoom-fields insp__group" disabled={busy || !data}>
          <RecordingZoomList segments={data?.segments ?? []} selected={selected} select={setSelected} />
          {segment && (
            <RecordingZoomControls
              key={segment.id}
              segment={segment}
              durationMs={data!.duration_ms}
              update={update}
              remove={() => void perform(() => runUserVerb('screen_record.zoom_remove', {
                clip, id: selected,
              }, 'Could not delete zoom.'))}
            />
          )}
          <button
            data-cut-action="recording-zoom-add"
            className="insp__btn"
            disabled={timelineAtPlayhead === null}
            title={timelineAtPlayhead === null
              ? 'Place the playhead inside this clip.' : 'Add a two-second pointer-target zoom'}
            onClick={() => void perform(() => runUserVerb('screen_record.zoom_add', {
              clip, timeline_ms: timelineAtPlayhead!, duration_ms: 2000, scale: 2,
            }, 'Could not add zoom.'))}
          >
            Add zoom here
          </button>
          <RecordingZoomRedo
            confirm={confirm}
            setConfirm={setConfirm}
            redo={() => void perform(() => runUserVerb('screen_record.zoom_auto', {
              clip, keep_manual: true,
            }, 'Could not redo zooms.'))}
          />
        </fieldset>
      </div>
    </InspectorSection>
  )
}
