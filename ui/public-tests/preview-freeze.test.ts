import assert from 'node:assert/strict'
import { activeVideo, canPlayPreviewVideo } from '../src/panels/Preview/model'
import { resolveOverlays } from '../src/panels/Preview/composite'
import { sourceMsAtTimelinePosition } from '../src/lib/mediaTime'
import { sourceAtPlayhead } from '../src/panels/Timeline/layout'
const clip = {id:'freeze',asset:'a',src_in_ms:20000,src_out_ms:24000,freeze:{at_ms:750}}
const p:any = {assets:{a:{proxy:'/a.mp4'}},tracks:[{id:'v1',kind:'video',clips:[clip]}]}
for (const time of [0, 1000, 3999]) {
  const v = activeVideo(p,time,new Set())!
  assert.equal(v.srcMs,20750)
  assert.equal(v.srcMs,sourceAtPlayhead(p,time)?.srcMs)
  assert.equal(v.freezeAtMs,750)
}
const overlay:any = {assets:p.assets,tracks:[{id:'v1',kind:'video',clips:[{...clip,id:'base',freeze:undefined}]},{id:'v2',kind:'video',clips:[{...clip,freeze:{at_ms:0},reverse:true,speed:2}]}]}
const layer = resolveOverlays(overlay,1000,'v1',new Set()).overlays[0]
assert.equal(layer.freezeAtMs,0, 'zero freeze offset is retained')
assert.equal(sourceMsAtTimelinePosition(layer,1000),20000, 'freeze holds despite speed and reverse')
assert.equal(sourceMsAtTimelinePosition(layer,1999),20000)
assert.equal(canPlayPreviewVideo(1, false, 0), false)
assert.equal(canPlayPreviewVideo(1, false, 750), false)
assert.equal(canPlayPreviewVideo(1, true), false)
assert.equal(canPlayPreviewVideo(-1, false), false)
assert.equal(canPlayPreviewVideo(1, false), true)
console.log('PASS frozen base and overlay source mapping')
for (const offset of [0, 750]) {
  for (const speed of [1, 2]) {
    for (const reverse of [false, true]) {
      const frozen:any = {assets:p.assets,tracks:[{id:'v1',kind:'video',clips:[{...clip,freeze:{at_ms:offset},speed,reverse}]}]}
      const duration = 4000 / speed
      for (const at of [0, Math.floor(duration / 2), duration - 1]) {
        const video = activeVideo(frozen,at,new Set())!
        assert.equal(video.srcMs,20000 + offset)
        assert.equal(canPlayPreviewVideo(1,video.reverse,video.freezeAtMs),false)
        assert.equal(canPlayPreviewVideo(2,video.reverse,video.freezeAtMs),false)
        assert.equal(canPlayPreviewVideo(-1,video.reverse,video.freezeAtMs),false)
      }
    }
  }
}
