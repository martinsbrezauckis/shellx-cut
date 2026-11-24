import type { RecordingZoomSegment } from '../../lib/client'
export function createZoomMock(jobs = new Map<string, unknown>()) {
 let missingVideo = typeof location !== 'undefined' && new URLSearchParams(location.search).has('missingRecordingVideo')
 const automatic = (): RecordingZoomSegment[] => [1,2,3].map(i => ({id:`z${i}`,start_ms:i*3000,end_ms:i*3000+2000,scale:2,cx:0.5,cy:0.5,ramp_in_ms:550,ramp_out_ms:500,origin:'auto'}))
 let segments = automatic(), stale = false, sequence = 3
 return (name: string, args: Record<string, unknown>): {ok:boolean;result?:unknown;error?:unknown} | null => {
  const clip = String(args.clip)
  switch (name) {
   case 'screen_record.zoom_list': return {ok:true,result:{clip,take:'mock-take',source:{w:1920,h:1080},duration_ms:58000,missing_video:missingVideo,missing_video_reason:missingVideo?'The baked recording was pruned.':null,next_action:missingVideo?'Update video rebuilds it (screen_record.take_refresh).':null,segments:segments.map(s => ({...s,timeline:{start_ms:s.start_ms,end_ms:s.end_ms}})),stale,applied_plan:'applied',pending_plan:stale?'pending':'applied'}}
   case 'screen_record.zoom_add': {
    const s: RecordingZoomSegment = {id:`z${++sequence}`,start_ms:Number(args.timeline_ms ?? args.at_ms),end_ms:Number(args.timeline_ms ?? args.at_ms)+Number(args.duration_ms ?? 2000),scale:Number(args.scale ?? 2),cx:Number(args.cx ?? 0.5),cy:Number(args.cy ?? 0.5),ramp_in_ms:550,ramp_out_ms:500,origin:'manual'}
    segments.push(s); stale=true; return {ok:true,result:{segment:s,stale}}
   }
   case 'screen_record.zoom_update': {
    const s = segments.find(s => s.id === args.id)
    if (s) { const {clip:_,id:__,...patch} = args; Object.assign(s,patch,args.target === 'pointer' ? {cx:0.5,cy:0.5,origin:'manual'} : {origin:'manual'}) }
    stale=true; return {ok:true,result:{segment:s,stale}}
   }
   case 'screen_record.zoom_remove': segments=segments.filter(s => s.id !== args.id); stale=true; return {ok:true,result:{removed:[args.id],stale}}
   case 'screen_record.zoom_auto': {
    const manual = segments.filter(s => s.origin === 'manual')
    segments=[...manual,...automatic().filter(s => !manual.some(m => m.start_ms<s.end_ms && m.end_ms>s.start_ms))]; stale=true
    return {ok:true,result:{segments,stale}}
   }
   case 'screen_record.make_editable': {
    const mode = typeof location !== 'undefined' ? new URLSearchParams(location.search).get('oldRecording') : null
    if (mode === 'missing') {
     return { ok: false, error: { code: 'not_found', message: 'Recording files could not be found', cause: 'events.json is missing from the recording capture folder', suggested_action: 'Restore events.json from backup, or keep using the existing video as-is.' } }
    }
    if (mode === 'older') {
     return { ok: false, error: { code: 'invalid_args', message: 'This recording was made with an older version and cannot be made editable', cause: 'recordings made before 0.6.116 did not keep a per-take plan snapshot', suggested_action: 'Keep using this video clip as-is, or record a new take.' } }
    }
    return { ok: true, result: { clip, take: 'mock-take', stale, segments, camera_clip: null, camera_asset: null } }
   }
   case 'screen_record.take_refresh': {
    stale=false; missingVideo=false
    const job_id='mock-zoom-refresh'
    const earlier_render={status:'kept',reason:'kept so Undo can bring it back',message:'Earlier video kept so Undo brings it back at once. To remove older recording videos, open Settings > Old recording renders.'}
    jobs.set(job_id,{clip,asset:'a1',baked:'mock',cached:false,applied_plan:'applied',earlier_render})
    return {ok:true,result:{job_id,clip,asset:'a1',baked:'mock',cached:false,applied_plan:'applied',earlier_render}}
   }
   default: return null
  }
 }
}
