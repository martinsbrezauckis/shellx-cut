function proxyAddress() {
  let base = process.env.CUTD_PROXY_ADDR || process.env.SWEEP_CUTD || ''
  if (!base) throw new Error('CUTD_PROXY_ADDR is not set for agent fixture')
  if (!/^https?:\/\//i.test(base)) base = `http://${base}`
  return base.replace(/\/+$/, '')
}

async function postVerb(name, body) {
  const res = await fetch(`${proxyAddress()}/api/verb/${name}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-cut-actor': process.env.CUTD_PROXY_ACTOR || 'agent:fixture',
    },
    body: JSON.stringify(body || {}),
  })
  return res.json()
}

async function projectState() {
  const response = await postVerb('project.state', {})
  return response.result || {}
}

function firstClip(state, wantKind = 'video') {
  for (const track of state.tracks || []) {
    if (wantKind && track.kind !== wantKind) continue
    for (const clip of track.clips || []) {
      if (clip?.asset && clip.id) return { track: track.id, clip: clip.id }
    }
  }
  for (const track of state.tracks || []) {
    for (const clip of track.clips || []) {
      if (clip?.asset && clip.id) return { track: track.id, clip: clip.id }
    }
  }
  return null
}

export async function applyAgentEdit(text, provider = 'fixture') {
  const request = (text.split(/User request:\s*/i).pop() || text).toLowerCase()
  const state = await projectState()
  const video = firstClip(state, 'video')
  const media = video || firstClip(state, '')
  if (/split/.test(request)) {
    if (!video) throw new Error('no video clip to split')
    const response = await postVerb('edit.split', {
      track: video.track,
      at_ms: 2000,
      rationale: `fcv ${provider} fixture: split`,
    })
    return { verb: 'edit.split', ok: response.ok, detail: response.error?.message || '' }
  }
  if (/fade/.test(request)) {
    if (!media) throw new Error('no media clip to fade')
    const response = await postVerb('edit.fade', {
      clip: media.clip,
      in_ms: 500,
      kind: 'both',
      rationale: `fcv ${provider} fixture: fade`,
    })
    return { verb: 'edit.fade', ok: response.ok, detail: response.error?.message || '' }
  }
  if (/speed|2x|2 x/.test(request)) {
    if (!media) throw new Error('no media clip to retime')
    const response = await postVerb('edit.speed', {
      clip: media.clip,
      factor: 2,
      rationale: `fcv ${provider} fixture: speed`,
    })
    return { verb: 'edit.speed', ok: response.ok, detail: response.error?.message || '' }
  }
  const requestedSeconds = Number(
    request.match(/marker\s+at\s+(\d+(?:\.\d+)?)\s*seconds?/)?.[1] || 3,
  )
  const requestedLabel = request.match(/\bnamed\s+([^\n.]+)/)?.[1]?.trim()
    || `FCV ${provider} marker`
  const response = await postVerb('edit.add_marker', {
    at_ms: Math.max(0, Math.round(requestedSeconds * 1000)),
    label: requestedLabel,
    rationale: `fcv ${provider} fixture: marker`,
  })
  return { verb: 'edit.add_marker', ok: response.ok, detail: response.error?.message || '' }
}
