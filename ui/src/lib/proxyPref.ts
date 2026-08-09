// lib/proxyPref.ts — persisted "Generate editing proxies" preference.
//
// Default ON. When OFF, media.import is called with proxy:false so HEAVY files
// (large FHD / multi-GB raw) import INSTANTLY instead of blocking minutes on the
// 960×540 transcode. The asset stays usable — transcript edits + the composed-frame
// poster scrub from the SOURCE — and the final render uses the source too, so output
// quality is unaffected; only smooth proxy <video> playback is unavailable until a
// proxy exists. Server side: dispatch.rs spawn_import_chain (the make_proxy flag).
//
// localStorage-backed so the choice survives reloads; falls back to ON if storage
// is unavailable (private mode / quota).

const KEY = 'cut.generateProxies'

/** True unless the user explicitly turned proxies OFF. */
export function getGenerateProxies(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'false'
  } catch {
    return true
  }
}

export function setGenerateProxies(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? 'true' : 'false')
  } catch {
    /* storage unavailable — the session still works, just not persisted */
  }
}
