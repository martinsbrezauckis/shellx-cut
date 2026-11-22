import type { RecordingZoomSegment } from '../../lib/client'
export function createZoomMock(jobs = new Map<string, unknown>()) {
 let missingVideo = typeof location !== 'undefined' && new URLSearchParams(location.search).has('missingRecordingVideo')
 const automatic = (): RecordingZoomSegment[] => [1,2,3].map(i => ({id:`z${i}`,start_ms:i*3000,end_ms:i*3000+2000,scale:2,cx:0.5,cy:0.5,ramp_in_ms:550,ramp_out_ms:500,origin:'auto'}))
 let segments = automatic(), stale = false, sequence = 3
 return (name: string, args: Record<string, unknown>): {ok:true;result:unknown} | null => {
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
   case 'screen_record.take_refresh': {
    stale=false; missingVideo=false
    const job_id='mock-zoom-refresh'
    jobs.set(job_id,{job_id,kind:name,state:'done',progress:1,result:{clip},created_ts:new Date().toISOString(),updated_ts:new Date().toISOString()})
    return {ok:true,result:{job_id,clip,asset:'a1',baked:'mock',cached:false,applied_plan:'applied'}}
   }
   default: return null
  }
 }
}
