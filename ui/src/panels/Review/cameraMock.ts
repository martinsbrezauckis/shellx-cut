import type { ClipTransform, Project, Keyframe } from '../../lib/client'

export function createCameraMock(project: Project, enabled: boolean, split = false, ordinary = false, keyed = false) {
  const original: ClipTransform = {
    x: .76, y: .604444, scale: .2, opacity: 1,
    shape: ordinary ? 'rounded' : 'circle', radius: ordinary ? .15 : undefined,
    ring: { width: .02, colour: '#ffffffe6' },
  }
  const keyframes: Keyframe[] = keyed ? [
    { param: 'pos_x', interp: 'hold', points: [{ t_ms: 0, value: .76 }, { t_ms: 10000, value: .04 }, { t_ms: 20000, value: .4 }] },
    { param: 'pos_y', interp: 'hold', points: [{ t_ms: 0, value: .6 }, { t_ms: 10000, value: .04 }, { t_ms: 20000, value: .3 }] },
    { param: 'scale', interp: 'hold', points: [{ t_ms: 0, value: 1 }, { t_ms: 10000, value: .5 }, { t_ms: 20000, value: .75 }] },
  ] : []
  const camera = { id: 'camera1', asset: 'a2', src_in_ms: 0, src_out_ms: 58000, transform: { ...original }, keyframes }
  const history: { transform: ClipTransform; keyframes: Keyframe[] }[] = []
  if (enabled) {
    project.recordings = ordinary ? [] : [{ camera_clip: camera.id, camera_look: { transform: original, keyframes } }]
    // Keep the Camera fixture unobscured by the demo's full-frame intro overlay.
    project.tracks.find((track) => track.id === 'v2')!.clips = []
    project.tracks.splice(1, 0, { id: 'v27', kind: 'video', clips: split
      ? [{ ...camera, src_out_ms: 1000 }, { ...camera, id: 'camera-right', src_in_ms: 1000,
        recording: { take: 'mock-take', camera: true } }]
      : [camera] })
  }
  return (name: string, args: Record<string, unknown>) => {
    if (!enabled) return null
    if (name === 'project.undo' && history.length) {
      Object.assign(camera, history.pop()!)
      return { transform: camera.transform }
    }
    if (args.clip !== camera.id || !['edit.transform', 'screen_record.camera_reset'].includes(name)) return null
    history.push(structuredClone({ transform: camera.transform, keyframes: camera.keyframes }))
    if (name === 'screen_record.camera_reset') {
      camera.transform = structuredClone(original)
      camera.keyframes = structuredClone(keyframes)
    } else {
      const { clip: _clip, ...transform } = args
      // Mirror the separate-camera engine hand-place rule; size keys survive.
      if (!ordinary && (args.x !== camera.transform.x || args.y !== camera.transform.y)) {
        camera.keyframes = camera.keyframes.filter(k => k.param !== 'pos_x' && k.param !== 'pos_y')
      }
      camera.transform = transform as unknown as ClipTransform
    }
    return { ...camera.transform }
  }
}
