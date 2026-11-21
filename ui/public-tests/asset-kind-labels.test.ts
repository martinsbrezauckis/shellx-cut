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

// High frame rate warns about preview without calling HD footage 4K.
for (const fps of [60, 59.94]) {
  const hd = row('hd', '/m/hd.mp4', { kind: 'video', width: 1920, height: 1080, fps })
  const readiness = assetReadiness(hd)
  assert.equal(readiness.label, 'High frame rate', '1080p60 is high frame rate, not 4K')
  assert.equal(readiness.level, 'large-source')
  assert.equal(readiness.needsAction, true)
  assert.equal(readiness.hint, 'This clip may stutter during preview. Turn on proxies for future imports, or re-import it with proxies if playback lags.')
  assert.deepEqual(readiness.badges, [{ label: 'High frame rate', tone: 'warn', title: 'High-frame-rate source playback can stutter without a proxy.' }])
  assert.ok(!readiness.badges.some((badge) => badge.label.includes('4K')))
  assert.equal(assetReadiness({ ...hd, proxy: 'proxies/hd.mp4' }).label, 'Proxy ready')
  assert.equal(assetReadiness({ ...hd, offline: true }).label, 'Missing source')
}
assert.equal(assetReadiness(row('hd30', '/m/hd.mp4', { kind: 'video', width: 1920, height: 1080, fps: 30 })).label, 'Source playback')
for (const probe of [
  { kind: 'video' as const, width: 3840, height: 2160, fps: 30 },
  { kind: 'video' as const, width: 3840, height: 2160, fps: 60 },
  { kind: 'video' as const, width: 2160, height: 3840 },
]) {
  const readiness = assetReadiness(row('large', '/m/large.mp4', probe))
  assert.equal(readiness.label, '4K source')
  assert.deepEqual(readiness.badges, [{ label: '4K source', tone: 'warn', title: 'High-resolution source playback can stutter without a proxy.' }])
}
assert.equal(assetReadiness({ ...title, probe: { kind: 'video', width: 3840, height: 2160, fps: 60 } }).label, 'Title')
for (const asset of [image, audio]) {
  const readiness = assetReadiness({ ...asset, probe: { ...asset.probe, width: 3840, height: 2160, fps: 60 } })
  assert.ok(!['4K source', 'High frame rate'].includes(readiness.label))
  assert.ok(!readiness.badges.some((badge) => ['4K source', 'High frame rate'].includes(badge.label)))
}

console.log('PASS each asset kind reads as what it is; a title never counts as footage without a proxy')
