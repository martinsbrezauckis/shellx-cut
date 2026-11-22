import { test } from 'node:test'
import assert from 'node:assert/strict'
import { zoomLabel, speedPatch, timelineToSource } from '../src/panels/Inspector/recordingZoomModel'
test('labels use timeline time and ownership', () => {
 assert.equal(zoomLabel({scale:2,origin:'manual',timeline:{start_ms:1500,end_ms:3500}}), '1.5–3.5 s · 2× · Mine')
})
test('speed presets preserve normal ramps', () => {
 assert.deepEqual(speedPatch('Normal'), {ramp_in_ms:550,ramp_out_ms:500})
})
test('playhead mapping uses the server segment mapping', () => {
 assert.equal(timelineToSource(2000,{start_ms:5000,end_ms:9000,timeline:{start_ms:1000,end_ms:3000}}),7000)
})
