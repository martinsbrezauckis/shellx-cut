import { strict as assert } from 'node:assert'
import { assetMediaKind, assetReadiness, summarizeMediaReadiness, type MediaReadinessAsset } from '../src/panels/Assets/mediaReadiness'

// Windows installed run F8: a project with a TITLE showed "Source playback" and
// "Proxy 1/2 ... Editing uses the original files". A title is a movie Cut draws:
// it plays as drawn and never has a proxy, so it is not footage missing one.

const row = (id: string, path: string, probe: MediaReadinessAsset['probe'], proxy?: string): MediaReadinessAsset => ({
  id, path, probe, proxy, offline: false, used: 1,
})
const footage = row('a1', 'C:\\CutPre3\\media\\cnt20_a1.mp4', { kind: 'video', width: 1920, height: 1080, fps: 30 }, 'proxies/a1.mp4')
const title = row('a2', 'C:\\CutPre3\\proj\\p147.cutproj\\titles\\title_8b3629235aed3bf3.mov', { kind: 'video', width: 1920, height: 1080 })
const shapeUnix = row('a3', '/home/p/x.cutproj/titles/title_0011223344556677.mov', { kind: 'video' })
const generated = row('a4', '/home/p/x.cutproj/assets/generated/clip.mp4', { kind: 'video', width: 1280, height: 720, fps: 24 })
const image = row('a5', '/m/still.png', { kind: 'image' })
const audio = row('a6', '/m/music.wav', { kind: 'audio' })
const ownFileNamedTitle = row('a7', '/m/title_intro.mov', { kind: 'video', width: 1920, height: 1080, fps: 30 })

// What each kind is.
assert.equal(assetMediaKind(footage), 'footage')
assert.equal(assetMediaKind(title), 'title')
assert.equal(assetMediaKind(shapeUnix), 'title')
assert.equal(assetMediaKind(generated), 'generated')
assert.equal(assetMediaKind(image), 'image')
assert.equal(assetMediaKind(audio), 'audio')
assert.equal(assetMediaKind(ownFileNamedTitle), 'footage', "a person's own file named title_* is footage")

// What each label says.
assert.equal(assetReadiness(title).label, 'Title')
assert.ok(!assetReadiness(title).badges.some((b) => b.label === 'Source playback'), 'a title never reads "Source playback"')
assert.equal(assetReadiness(footage).label, 'Proxy ready')
assert.equal(assetReadiness(generated).label, 'Source playback', 'a generated video without a proxy plays its file, like footage')
assert.match(assetReadiness(image).hint, /still image.*No proxy is needed/)
assert.match(assetReadiness(audio).hint, /Sound only.*No proxy is needed/)

// The panel's summary: footage + title -> every video has its proxy, nothing plays from source.
const summary = summarizeMediaReadiness([footage, title])
assert.equal(summary.dimensions.proxy.value, '1/1', 'the title is not counted as a video missing a proxy')
assert.equal(summary.dimensions.proxy.state, 'ready')
assert.equal(summary.sourceOnly, 0)
assert.ok(!/original files/.test(summary.hint), `no "Editing uses the original files": ${summary.hint}`)
assert.equal(summary.videos, 1)
// A title needs no speech or picture analysis either.
assert.equal(summary.dimensions.speech.value === '1/2', false)

// A title whose movie is missing is not a missing source to relink: Cut draws it again.
const missingTitle = { ...title, offline: true }
assert.equal(assetReadiness(missingTitle).label, 'Title')
assert.match(assetReadiness(missingTitle).hint, /draws it again/)
assert.equal(summarizeMediaReadiness([footage, missingTitle]).offline, 0, 'no "source missing" for a title')

// Footage really without a proxy still says so.
const noProxy = summarizeMediaReadiness([{ ...footage, proxy: undefined }, title])
assert.equal(noProxy.sourceOnly, 1)
assert.equal(noProxy.dimensions.proxy.value, 'Source')

console.log('PASS each asset kind reads as what it is; a title never counts as footage without a proxy')
