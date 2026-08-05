// cdp-cut-verify-0650-uiux.mjs — focused installed Windows ShellX Cut UI/UX
// UI/UX verification over the real WebView2 CDP endpoint.
//
// Scope:
//   - Environment: compact cards, Dub/Diarize model-runtime cards, STT Canary tier,
//     Agent Chat handoff, no obvious visible horizontal clipping.
//   - Generate: Library-adjacent Generate workspace, Templates/Native prompt/Storyboard/AI media
//     tabs, template preview + insert mutation, media paid-generation guard.
//   - Library: visible card/list controls plus real library mutations and project import.
//
// Usage:
//   CUT_RECEIPT_DIR=/path/to/private/receipt node scripts/windows/cdp-cut-verify-0650-uiux.mjs
//
// The app must already be running with WebView2 CDP exposed on :9223.
// Preferred launcher:
//   node scripts/windows/launch-installed-cdp.mjs --cdp-port 9223

import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { withBooleanReceiptSummary } from '../lib/receipt-summary.mjs'
import { base64ToBuffer } from '../lib/safe-data.mjs'

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const CDP = process.env.CUT_CDP || `${'http'}://127.0.0.1:9223`

function windowsEnvironmentPath(name, fallback) {
  if (!/^[A-Z_]+$/.test(name)) throw new Error(`Invalid Windows environment name: ${name}`)
  const result = spawnSync(
    'powershell.exe',
    ['-NoProfile', '-Command', `[Environment]::GetEnvironmentVariable('${name}')`],
    { encoding: 'utf8' },
  )
  const value = result.status === 0 ? String(result.stdout || '').trim() : ''
  return value || fallback
}

function joinWindowsPath(base, leaf) {
  return `${String(base).replace(/[\\/]+$/, '')}\\${leaf}`
}

const WINDOWS_USER_PROFILE = windowsEnvironmentPath('USERPROFILE', 'C:\\Users\\Public')
const WINDOWS_TEMP = process.env.CUT_WINDOWS_TEMP || windowsEnvironmentPath('TEMP', 'C:\\Windows\\Temp')
const MEDIA = process.env.CUT_TEST_MEDIA || joinWindowsPath(WINDOWS_USER_PROFILE, 'Downloads\\talkinghead_hq.mp4')

function readRepoExpectedVersion() {
  const confPath = join(REPO_ROOT, 'app/desktop/src-tauri/tauri.conf.json')
  const conf = JSON.parse(readFileSync(confPath, 'utf8'))
  if (!/^\d+\.\d+\.\d+/.test(String(conf.version || ''))) {
    throw new Error(`Could not read app version from ${confPath}`)
  }
  return String(conf.version)
}

const EXPECTED_VERSION = process.env.CUT_EXPECTED_VERSION || readRepoExpectedVersion()
const VERSION_SLUG = EXPECTED_VERSION.replace(/\D+/g, '')
const RECEIPT_DIR = process.env.CUT_RECEIPT_DIR || join('/tmp', `shellx-cut-${VERSION_SLUG}-uiux-${Date.now()}`)
const UIUX_TITLE = `${EXPECTED_VERSION} UIUX`

mkdirSync(RECEIPT_DIR, { recursive: true })

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const winToWslPath = (path) => String(path).replace(/^([A-Za-z]):[\\/]/, (_match, drive) => `/mnt/${drive.toLowerCase()}/`).replace(/\\/g, '/')
const psLiteral = (value) => `'${String(value).replace(/'/g, "''")}'`

function removeWindowsDir(path) {
  const script = [
    '$ErrorActionPreference = "Stop"',
    `$p = ${psLiteral(path)}`,
    'if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction Stop }',
    'if (Test-Path -LiteralPath $p) { exit 1 }',
  ].join('; ')
  const result = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {
    encoding: 'utf8',
  })
  return {
    ok: result.status === 0,
    status: result.status,
    stderr: String(result.stderr || '').trim().slice(0, 500),
  }
}

function windowsPathExists(path) {
  const script = `if (Test-Path -LiteralPath ${psLiteral(path)}) { exit 0 } else { exit 1 }`
  const result = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {
    encoding: 'utf8',
  })
  return {
    ok: result.status === 0,
    status: result.status,
    stderr: String(result.stderr || '').trim().slice(0, 500),
  }
}

const targets = await (await fetch(`${CDP}/json/list`)).json()
const pageTarget = targets.find((target) => target.type === 'page' && /127\.0\.0\.1:\d+/.test(target.url))
if (!pageTarget) {
  console.log(`FAIL no WebView2 page on ${CDP}`)
  process.exit(1)
}

const ws = new WebSocket(pageTarget.webSocketDebuggerUrl)
let seq = 0
const pending = new Map()
ws.addEventListener('message', (event) => {
  let msg
  try {
    msg = JSON.parse(String(event.data))
  } catch {
    return
  }
  if (msg?.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg)
    pending.delete(msg.id)
  }
})
await new Promise((resolve) => ws.addEventListener('open', resolve, { once: true }))

function cmd(method, params = {}) {
  return new Promise((resolve) => {
    const id = ++seq
    pending.set(id, resolve)
    ws.send(JSON.stringify({ id, method, params }))
  })
}

async function ev(expression) {
  const response = await cmd('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  })
  if (response.result?.exceptionDetails) {
    return { __exception: String(response.result.exceptionDetails.text || 'exception') }
  }
  return response.result?.result?.value
}

async function post(verb, args = {}) {
  return ev(`fetch('/api/verb/${verb}',{method:'POST',headers:{'content-type':'application/json'},body:${JSON.stringify(JSON.stringify(args))}}).then(r=>r.json()).catch(e=>({ok:false,error:{message:String(e)}}))`)
}

async function qs(selector) {
  return ev(`document.querySelectorAll(${JSON.stringify(selector)}).length`)
}

async function waitFor(predicate, timeoutMs = 10000, stepMs = 250) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const value = await predicate()
    if (value) return value
    await sleep(stepMs)
  }
  return null
}

async function waitSel(selector, timeoutMs = 10000) {
  return waitFor(async () => (await qs(selector)) > 0, timeoutMs)
}

async function setValue(selector, value) {
  return ev(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;const p=e.tagName==='TEXTAREA'?HTMLTextAreaElement:HTMLInputElement;const d=Object.getOwnPropertyDescriptor(p.prototype,'value');d.set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return true})()`)
}

async function setSelect(selector, value) {
  return ev(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;const d=Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value');d.set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return e.value})()`)
}

async function key(selector, keyName) {
  return ev(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;e.dispatchEvent(new KeyboardEvent('keydown',{key:${JSON.stringify(keyName)},bubbles:true,cancelable:true}));return true})()`)
}

async function box(selector) {
  return ev(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return null;e.scrollIntoView({block:'center',inline:'center'});const r=e.getBoundingClientRect();if(r.width<1||r.height<1)return null;return {x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2),w:Math.round(r.width),h:Math.round(r.height),text:(e.textContent||'').trim().slice(0,120)}})()`)
}

async function mouse(selector) {
  const b = await box(selector)
  if (!b) return false
  await cmd('Input.dispatchMouseEvent', { type: 'mousePressed', x: b.x, y: b.y, button: 'left', buttons: 1, clickCount: 1 })
  await cmd('Input.dispatchMouseEvent', { type: 'mouseReleased', x: b.x, y: b.y, button: 'left', buttons: 1, clickCount: 1 })
  return true
}

async function screenshot(name) {
  const shot = await cmd('Page.captureScreenshot', { format: 'png' })
  if (!shot.result?.data) return null
  const path = join(RECEIPT_DIR, `${name}.png`)
  writeFileSync(path, base64ToBuffer(shot.result.data))
  return path
}

async function state() {
  return (await post('project.state', {}))?.result ?? null
}

function assetCount(projectState) {
  const assets = projectState?.assets
  if (!assets || typeof assets !== 'object' || Array.isArray(assets)) return 0
  let count = 0
  for (const key in assets) {
    if (Object.prototype.hasOwnProperty.call(assets, key)) count += 1
  }
  return count
}

async function uiState() {
  return (await post('ui.state', {}))?.result ?? null
}

async function openPanel(panel) {
  const opened = await post('ui.open', { panel })
  await sleep(650)
  return opened
}

const results = []
const evidence = {
  cdp: CDP,
  pageUrl: pageTarget.url,
  receiptDir: RECEIPT_DIR,
  media: MEDIA,
  screenshots: {},
  metrics: {},
}

function check(name, ok, detail = '') {
  results.push({ name, ok: !!ok, detail })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`)
}

await cmd('Runtime.enable')
await cmd('Page.enable')
await cmd('DOM.enable')

const doctor = await post('system.doctor', { refresh: true })
evidence.metrics.doctorVersion = doctor?.result?.app_version
check('installed-app-version', doctor?.result?.app_version === EXPECTED_VERSION, `app_version=${doctor?.result?.app_version} expected=${EXPECTED_VERSION}`)

const projectName = `uiux_${VERSION_SLUG}_${Date.now()}`
const projectDir = joinWindowsPath(WINDOWS_TEMP, `${projectName}.cutproj`)
const created = await post('project.create', {
  name: projectName,
  dir: projectDir,
  settings: { width: 1280, height: 720, fps: 30 },
})
check('project.create', created?.ok === true, created?.ok ? projectName : JSON.stringify(created?.error ?? created).slice(0, 180))
const imported = await post('media.import', { path: MEDIA, rationale: `${EXPECTED_VERSION} installed UI/UX verifier seed` })
check('media.import', imported?.ok === true, imported?.ok ? String(imported.result?.asset_id ?? '') : JSON.stringify(imported?.error ?? imported).slice(0, 180))
await sleep(1800)

// Environment
const outputChipMetrics = await ev(`(()=>{
  const chip=document.querySelector('[data-cut-output-chip]');
  if(!chip)return {present:false};
  const r=chip.getBoundingClientRect();
  const cs=getComputedStyle(chip);
  return {
    present:true,
    visible:r.width>8&&r.height>8&&cs.visibility!=='hidden'&&cs.display!=='none',
    text:(chip.textContent||'').replace(/\\s+/g,' ').trim().slice(0,120),
    dir:chip.getAttribute('data-cut-output-dir')||''
  };
})()`)
evidence.metrics.outputChip = outputChipMetrics
check('statusbar-output-chip-visible', !!outputChipMetrics?.present && !!outputChipMetrics?.visible && /export folder:/i.test(outputChipMetrics.text || ''), JSON.stringify(outputChipMetrics))
const outputChipClicked = await mouse('[data-cut-output-chip]')
const outputChipOpenedEnvironment = await waitFor(
  async () => await ev(`!!document.querySelector('[data-cut-environment] [data-cut-export-default-folder]')`),
  12000,
)
check('statusbar-output-chip-opens-environment', outputChipClicked && outputChipOpenedEnvironment === true, `clicked=${outputChipClicked} opened=${outputChipOpenedEnvironment}`)
await ev(`document.querySelector('[data-cut-environment-close]')?.click()`)
await sleep(350)
const settingsClicked = await mouse('[data-cut-settings-btn]')
const settingsOpenedExportFolder = await waitFor(
  async () => await ev(`!!document.querySelector('[data-cut-environment] [data-cut-export-default-folder]')`),
  12000,
)
check('settings-button-opens-export-folder', settingsClicked && settingsOpenedExportFolder === true, `clicked=${settingsClicked} opened=${settingsOpenedExportFolder}`)
await ev(`document.querySelector('[data-cut-environment-close]')?.click()`)
await sleep(350)

await openPanel('environment')
await waitSel('[data-cut-environment]', 8000)
evidence.screenshots.environment = await screenshot('environment')
const envMetrics = await ev(`(()=>{
  const root=document.querySelector('[data-cut-environment]');
  const visible=(el)=>{const r=el.getBoundingClientRect();const cs=getComputedStyle(el);return r.width>1&&r.height>1&&cs.visibility!=='hidden'&&cs.display!=='none'};
  const cards=[...document.querySelectorAll('[data-cut-env-card]')].map(c=>({
    id:c.getAttribute('data-cut-env-card'),
    status:c.getAttribute('data-cut-env-status'),
    title:(c.querySelector('[data-cut-env-title]')?.textContent||'').trim(),
    text:(c.textContent||'').replace(/\\s+/g,' ').trim().slice(0,500)
  }));
  const service=['dub','diarize'].map(id=>{
    const card=document.querySelector('[data-cut-env-card="'+id+'"]');
    const detail=card?.querySelector('[data-cut-env-service="'+id+'"]');
    return {
      id,
      open: !!detail?.querySelector('details[open]'),
      text:(card?.textContent||'').replace(/\\s+/g,' ').trim().slice(0,500),
      model:(detail?.querySelector('[data-cut-env-service-model="'+id+'"]')?.textContent||'').replace(/\\s+/g,' ').trim(),
      runner:(detail?.querySelector('[data-cut-env-service-runner="'+id+'"]')?.textContent||'').replace(/\\s+/g,' ').trim(),
      steps:detail?.querySelectorAll('[data-cut-env-service-setup-step="'+id+'"]').length || 0,
      primary:!!card?.querySelector('[data-cut-env-service-primary="'+id+'"]'),
      connect:!!card?.querySelector('[data-cut-env-service-connect="'+id+'"]'),
      rescan:!!card?.querySelector('[data-cut-env-service-rescan="'+id+'"]'),
      chat:!!card?.querySelector('[data-cut-env-service-chat="'+id+'"]')
    };
  });
  const stt=[...document.querySelectorAll('[data-cut-env-stt-model] option')].map(o=>({value:o.value,text:o.textContent.trim()}));
  const exportDefault=document.querySelector('[data-cut-export-default-folder]');
  const exportFolder={
    present:!!exportDefault,
    pick:!!exportDefault?.querySelector('[data-cut-export-default-pick]'),
    clear:!!exportDefault?.querySelector('[data-cut-export-default-clear]'),
    text:(exportDefault?.textContent||'').replace(/\\s+/g,' ').trim().slice(0,220)
  };
  const clipped=[...root.querySelectorAll('button,span,p,div,label,summary,select,code,dd')]
    .filter(visible)
    .filter(el=>!el.closest('details:not([open])'))
    .filter(el=>el.scrollWidth>el.clientWidth+3 && getComputedStyle(el).overflowX==='visible')
    .map(el=>({tag:el.tagName.toLowerCase(),cls:String(el.className||''),text:(el.textContent||'').replace(/\\s+/g,' ').trim().slice(0,140),client:el.clientWidth,scroll:el.scrollWidth}))
    .slice(0,20);
  return {cards,service,stt,exportFolder,clipped,drawer:{w:Math.round(root.getBoundingClientRect().width),h:Math.round(root.getBoundingClientRect().height)}};
})()`)
evidence.metrics.environment = envMetrics
const dub = envMetrics.service.find((row) => row.id === 'dub')
const diarize = envMetrics.service.find((row) => row.id === 'diarize')
check('environment-dub-card-model-installable', !!dub && /OmniVoice/i.test(dub.model) && /audio\.dub/i.test(dub.model) && /Connector/i.test(dub.runner) && dub.steps >= 3 && dub.primary && (dub.connect || dub.chat) && (dub.rescan || dub.chat), dub ? dub.text : 'missing')
check('environment-diarize-card-model-installable', !!diarize && /Sortformer/i.test(diarize.model) && /media\.diarize/i.test(diarize.model) && /Connector/i.test(diarize.runner) && diarize.steps >= 3 && diarize.primary && (diarize.connect || diarize.chat) && (diarize.rescan || diarize.chat), diarize ? diarize.text : 'missing')
check('environment-service-connection-wording', /Connection steps/i.test(`${dub?.text ?? ''} ${diarize?.text ?? ''}`), `${dub?.text ?? ''} ${diarize?.text ?? ''}`.slice(0, 240))
check('environment-service-setup-collapsed', !!dub && !!diarize && !dub.open && !diarize.open, `dubOpen=${dub?.open} diarizeOpen=${diarize?.open}`)
check(
  'environment-service-copy-no-gpu-host',
  !/GPU host|remote GPU box|tunnel/i.test(`${dub?.text ?? ''} ${diarize?.text ?? ''}`),
  `${dub?.text ?? ''} ${diarize?.text ?? ''}`.slice(0, 240),
)
check('environment-stt-canary-visible', envMetrics.stt.some((option) => /canary/i.test(option.text) && /MMS_FA/i.test(option.text)), JSON.stringify(envMetrics.stt))
check(
  'environment-default-export-folder-visible',
  !!envMetrics.exportFolder?.present
    && !!envMetrics.exportFolder?.pick
    && /Default save folder/i.test(envMetrics.exportFolder?.text || '')
    && /Exports and recordings/i.test(envMetrics.exportFolder?.text || '')
    && /Save As can override one file/i.test(envMetrics.exportFolder?.text || ''),
  JSON.stringify(envMetrics.exportFolder),
)
check('environment-visible-no-major-overflow', envMetrics.clipped.length === 0, JSON.stringify(envMetrics.clipped.slice(0, 3)))

await ev(`document.querySelector('[data-cut-environment-close]')?.click()`)
await sleep(350)
const recordModeClicked = await mouse('[data-cut-mode="record"]')
await waitSel('[data-cut-panel="record"]', 8000)
const recordDefaultClicked = await mouse('[data-cut-action="record-output-default-folder"]')
const recordDefaultOpenedSettings = await waitFor(
  async () => await ev(`!!document.querySelector('[data-cut-environment] [data-cut-export-default-folder]')`),
  12000,
)
check('record-default-folder-opens-settings', recordModeClicked && recordDefaultClicked && recordDefaultOpenedSettings === true, `mode=${recordModeClicked} clicked=${recordDefaultClicked} opened=${recordDefaultOpenedSettings}`)
await ev(`document.querySelector('[data-cut-environment-close]')?.click()`)
await sleep(350)
await mouse('[data-cut-mode="edit"]')
await sleep(350)
await openPanel('environment')
await waitSel('[data-cut-environment]', 4000)

if (dub?.connect) {
  await mouse('[data-cut-env-service-connect="dub"]')
  await sleep(250)
  const dubOpen = await ev(`!!document.querySelector('[data-cut-env-card="dub"] [data-cut-env-service-setup="dub"][open]')`)
  check('environment-dub-connect-opens-setup', dubOpen === true, `open=${dubOpen}`)
}
if (dub?.chat) {
  await mouse('[data-cut-env-service-chat="dub"]')
  await sleep(500)
  const dubPrompt = await ev(`document.querySelector('[data-cut-agent-chat-input]')?.value || document.querySelector('[data-cut-chat-input]')?.value || document.querySelector('textarea')?.value || ''`)
  check(
    'environment-dub-chat-prefills-setup',
    /Help me connect OmniVoice TTS|Dub the timeline audio into Latvian/i.test(String(dubPrompt)),
    String(dubPrompt).slice(0, 120),
  )
}
await openPanel('environment')
await waitSel('[data-cut-environment]', 4000)
if (diarize?.connect) {
  await mouse('[data-cut-env-service-connect="diarize"]')
  await sleep(250)
  const diarizeOpen = await ev(`!!document.querySelector('[data-cut-env-card="diarize"] [data-cut-env-service-setup="diarize"][open]')`)
  check('environment-diarize-connect-opens-setup', diarizeOpen === true, `open=${diarizeOpen}`)
}
if (diarize?.chat) {
  await mouse('[data-cut-env-service-chat="diarize"]')
  await sleep(500)
  const diarizePrompt = await ev(`document.querySelector('[data-cut-agent-chat-input]')?.value || document.querySelector('[data-cut-chat-input]')?.value || document.querySelector('textarea')?.value || ''`)
  check(
    'environment-diarize-chat-prefills-setup',
    (/Help me connect Sortformer v2/i.test(String(diarizePrompt)) || /Label the speakers/i.test(String(diarizePrompt))) && /diarize|speaker/i.test(String(diarizePrompt)),
    String(diarizePrompt).slice(0, 160),
  )
}
await mouse('[data-cut-environment-close]')
await waitFor(async () => (await qs('[data-cut-environment]')) === 0, 4000)

// Generate workspace
await openPanel('generate')
await waitSel('[data-cut-panel="generate-templates"]', 8000)
await waitSel('[data-cut-generate-template-card]', 10000)
evidence.screenshots.generateTemplates = await screenshot('generate-templates')
const genState = await uiState()
check('generate-ui-state-templates', genState?.panels?.includes('left:generate') && genState?.panels?.includes('generate:templates'), JSON.stringify(genState?.panels ?? []))
const generateTabs = await ev(`[...document.querySelectorAll('[data-cut-generate-tab]')].map(b=>({tab:b.getAttribute('data-cut-generate-tab'),text:b.textContent.trim(),selected:b.getAttribute('aria-selected')}))`)
check('generate-tabs-visible', ['templates', 'prompt', 'storyboard', 'media'].every((tab) => generateTabs.some((row) => row.tab === tab)), JSON.stringify(generateTabs))
check('generate-tabs-user-facing-labels', generateTabs.some((row) => row.tab === 'prompt' && /Native prompt/i.test(row.text)) && generateTabs.some((row) => row.tab === 'media' && /AI media/i.test(row.text)), JSON.stringify(generateTabs))
await openPanel('generate-prompt')
await waitSel('[data-cut-generate-prompt-panel]', 5000)
const promptOpenState = await uiState()
check('generate-ui-open-prompt-route', promptOpenState?.panels?.includes('left:generate') && promptOpenState?.panels?.includes('generate:prompt'), JSON.stringify(promptOpenState?.panels ?? []))
await openPanel('generate')
await waitSel('[data-cut-generate-template-card]', 8000)
await mouse('[data-cut-generate-template-id="builtin.lower-third.clean"]')
await sleep(500)
await setValue('[data-cut-generate-param="name"]', UIUX_TITLE)
await setValue('[data-cut-generate-param-text="accent"]', '#33CC99')
await mouse('[data-cut-generate-template-preview]')
const previewOk = await waitFor(async () => {
  return ev(`(()=>{const img=document.querySelector('[data-cut-generate-template-preview-img]');return !!(img && img.complete && img.naturalWidth>0 && img.naturalHeight>0)})()`)
}, 12000)
const previewImage = await ev(`(()=>{const img=document.querySelector('[data-cut-generate-template-preview-img]');return img?{src:img.src,complete:img.complete,naturalWidth:img.naturalWidth,naturalHeight:img.naturalHeight}:null})()`)
evidence.metrics.generatePreviewImage = previewImage
check('generate-template-preview-image', !!previewOk, previewImage ? `${previewImage.naturalWidth}x${previewImage.naturalHeight} ${previewImage.src}` : 'missing')
evidence.screenshots.generatePreview = await screenshot('generate-preview')
await mouse('[data-cut-generate-template-insert]')
const insertedState = await waitFor(async () => {
  const s = await state()
  const clips = (s?.tracks ?? []).flatMap((track) => track.clips ?? [])
  return clips.some((clip) => clip.title_text === UIUX_TITLE) ? s : null
}, 12000)
check('generate-template-insert-mutates-timeline', !!insertedState, insertedState ? 'title clip inserted' : 'no title clip found')
await mouse('[data-cut-generate-tab="storyboard"]')
await waitSel('[data-cut-generate-storyboard]', 5000)
const storyState = await uiState()
check('generate-ui-state-storyboard', storyState?.panels?.includes('generate:storyboard'), JSON.stringify(storyState?.panels ?? []))
await openPanel('generate-media')
await waitSel('[data-cut-generate-media-panel]', 5000)
const mediaOpenState = await uiState()
check('generate-ui-open-media-route', mediaOpenState?.panels?.includes('left:generate') && mediaOpenState?.panels?.includes('generate:media'), JSON.stringify(mediaOpenState?.panels ?? []))
const mediaIntro = await ev(`(document.querySelector('[data-cut-generate-media-intro]')?.textContent||'').replace(/\\s+/g,' ').trim()`)
check('generate-media-intro-visible', /AI media/i.test(String(mediaIntro)) && /assets/i.test(String(mediaIntro)), String(mediaIntro).slice(0, 160))
evidence.screenshots.generateMedia = await screenshot('generate-media')
const assetsBefore = assetCount(await state())
await setValue('[data-cut-generate-prompt]', 'simple blue square for UI guard test')
await mouse('[data-cut-generate-run]')
await sleep(500)
const mediaGuard = await ev(`(()=>{const b=document.querySelector('[data-cut-generate-run]');return {armed:b?.getAttribute('data-cut-generate-armed')||'',text:(b?.textContent||'').replace(/\\s+/g,' ').trim(),assets:${assetsBefore}}})()`)
const assetsAfterArm = assetCount(await state())
check('generate-media-paid-guard-arms-without-mutation', mediaGuard.armed === 'true' && assetsAfterArm === assetsBefore, `armed=${mediaGuard.armed} assets ${assetsBefore}->${assetsAfterArm} text=${mediaGuard.text}`)
await mouse('[data-cut-generate-cancel]')

// Library surface and mutations.
const libTag = `uiux0650-${Date.now()}`
const libAdd = await post('library.add', { path: MEDIA, tags: [libTag], source: 'user', copy: false })
const libId = libAdd?.result?.item?.id
check('library.seed-add', libAdd?.ok === true && !!libId, libId || JSON.stringify(libAdd?.error ?? libAdd).slice(0, 180))
const libRelinkContract = await post('library.relink', { id: libId, path: MEDIA })
check('library-relink-same-content-contract', libRelinkContract?.ok === true && libRelinkContract?.result?.item?.id === libId, JSON.stringify(libRelinkContract?.error ?? libRelinkContract?.result ?? {}).slice(0, 180))
await openPanel('library')
await waitSel(`[data-cut-library-card="${libId}"]`, 10000)
evidence.screenshots.library = await screenshot('library')
const libraryPaging = await ev(`(()=>{
  const previous=document.querySelector('[data-cut-library-page-prev]');
  const next=document.querySelector('[data-cut-library-page-next]');
  const status=document.querySelector('[data-cut-library-page-status]');
  return {previous:!!previous,next:!!next,status:(status?.textContent||'').trim()};
})()`)
check('library-visible-pagination', libraryPaging?.previous && libraryPaging?.next && /of \\d+/.test(libraryPaging?.status || ''), JSON.stringify(libraryPaging))
await mouse(`[data-cut-library-fav="${libId}"]`)
await sleep(500)
const favList = await post('library.list', { ids: [libId], limit: 1 })
const favItem = favList?.result?.items?.find((item) => item.id === libId)
check('library-favorite-ui-mutates', favItem?.favorite === true, `favorite=${favItem?.favorite}`)
const folder = `UIUX ${Date.now()}`
await setValue('[data-cut-library-newfolder]', folder)
await key('[data-cut-library-newfolder]', 'Enter')
await waitFor(async () => {
  const r = await post('library.list', {})
  return r?.result?.folders?.includes(folder)
}, 6000)
const folderOptionReady = await waitFor(async () => ev(`!!document.querySelector('[data-cut-library-move="${libId}"] option[value="${folder}"]')`), 6000)
const selectedFolder = folderOptionReady ? await setSelect(`[data-cut-library-move="${libId}"]`, folder) : ''
const moved = await waitFor(async () => {
  const movedList = await post('library.list', { ids: [libId], limit: 1 })
  const item = movedList?.result?.items?.find((row) => row.id === libId)
  return item?.folder === folder ? item : null
}, 8000)
check('library-folder-ui-mutates', moved?.folder === folder, `folder=${moved?.folder} selected=${selectedFolder} optionReady=${!!folderOptionReady}`)
await mouse(`[data-cut-library-tagbtn="${libId}"]`)
const tagInputReady = await waitSel('[data-cut-library-taginput]', 6000)
const tagValueSet = tagInputReady ? await setValue('[data-cut-library-taginput]', `${libTag}, verified`) : false
await sleep(120)
const tagEnterSent = tagValueSet ? await key('[data-cut-library-taginput]', 'Enter') : false
const tagged = await waitFor(async () => {
  const taggedList = await post('library.list', { ids: [libId], limit: 1 })
  return taggedList?.result?.items?.find((item) => item.id === libId && item.tags?.includes('verified')) ?? null
}, 8000)
const taggedListAfter = await post('library.list', { ids: [libId], limit: 1 })
const taggedAfter = taggedListAfter?.result?.items?.find((item) => item.id === libId)
check(
  'library-tag-ui-mutates',
  !!tagInputReady && tagValueSet === true && tagEnterSent === true && !!tagged,
  `input=${!!tagInputReady} set=${tagValueSet} enter=${tagEnterSent} tags=${JSON.stringify(taggedAfter?.tags ?? [])}`,
)
const assetsBeforeLibraryAdd = assetCount(await state())
await mouse(`[data-cut-library-toproject="${libId}"]`)
const libProjectState = await waitFor(async () => {
  const s = await state()
  const count = assetCount(s)
  return count > assetsBeforeLibraryAdd ? s : null
}, 12000)
check('library-add-to-project-mutates-project', !!libProjectState, `assets ${assetsBeforeLibraryAdd}->${assetCount(libProjectState)}`)

await openPanel('library')
await waitSel(`[data-cut-library-card="${libId}"]`, 10000)
const beforeInsertState = await state()
const beforeInsertClips = (beforeInsertState?.tracks ?? []).flatMap((track) => track.clips ?? []).length
const insertClicked = await mouse(`[data-cut-library-insert="${libId}"]`)
const insertResult = await waitFor(async () => {
  const s = await state()
  const clips = (s?.tracks ?? []).flatMap((track) => track.clips ?? []).length
  return clips > beforeInsertClips ? { clips } : null
}, 75000, 1000)
check(
  'library-insert-at-playhead-mutates-timeline',
  insertClicked === true && !!insertResult,
  `clicked=${insertClicked} clips ${beforeInsertClips}->${insertResult?.clips ?? beforeInsertClips}`,
)

await openPanel('library')
await waitSel(`[data-cut-library-card="${libId}"]`, 10000)
const removeClicked = await mouse(`[data-cut-library-remove="${libId}"]`)
const removed = await waitFor(async () => {
  const removedList = await post('library.list', { ids: [libId], limit: 1 })
  return removedList?.result?.items?.some((item) => item.id === libId) === false
}, 8000)
const removedList = await post('library.list', { ids: [libId], limit: 1 })
const stillThere = removedList?.result?.items?.some((item) => item.id === libId)
check('library-remove-ui-mutates', removeClicked === true && removed === true && stillThere === false, `clicked=${removeClicked} stillThere=${stillThere}`)

const cleanup = {}
const folderRemove = await post('library.folder_remove', { name: folder })
cleanup.folderRemove = folderRemove?.result ?? folderRemove?.error ?? folderRemove
check('test-cleanup-library-folder', folderRemove?.ok === true && folderRemove?.result?.removed === true, `removed=${folderRemove?.result?.removed}`)
const projectClose = await post('project.close', {})
cleanup.projectClose = projectClose?.result ?? projectClose?.error ?? projectClose
const projectDelete = await post('project.delete', { path: projectDir })
cleanup.projectDelete = projectDelete?.result ?? projectDelete?.error ?? projectDelete
await sleep(700)
const projectDirWsl = winToWslPath(projectDir)
const projectForget = await post('project.forget', { path: projectDir })
cleanup.projectForget = projectForget?.result ?? projectForget?.error ?? projectForget
let goneStreak = 0
for (let attempt = 0; attempt < 45; attempt += 1) {
  const wslExists = existsSync(projectDirWsl)
  const winExists = windowsPathExists(projectDir).ok
  if (wslExists || winExists) {
    try {
      if (wslExists) {
        rmSync(projectDirWsl, { recursive: true, force: true })
        cleanup.projectDirRm = 'wsl'
      }
    } catch (error) {
      cleanup.projectDirWslRmError = String(error?.message || error).slice(0, 500)
    }
    const winRm = removeWindowsDir(projectDir)
    cleanup.projectDirWinRm = winRm
    goneStreak = existsSync(projectDirWsl) || windowsPathExists(projectDir).ok ? 0 : 1
  } else {
    goneStreak += 1
    if (goneStreak >= 10) break
  }
  await sleep(1000)
}
cleanup.projectDirGone = !existsSync(projectDirWsl) && !windowsPathExists(projectDir).ok
const projectListAfterCleanup = await post('project.list', { q: projectName })
cleanup.projectIndexGone = !projectListAfterCleanup?.result?.projects?.some((project) => project.path === projectDir || project.name === projectName)
const deleteOrForgetOk = projectDelete?.ok === true || projectForget?.result?.forgotten === true || cleanup.projectIndexGone === true
check(
  'test-cleanup-project-dir',
  projectClose?.ok === true && deleteOrForgetOk && cleanup.projectDirGone === true && cleanup.projectIndexGone === true,
  `deleteOk=${!!projectDelete?.ok} forget=${projectForget?.result?.forgotten} residualGone=${cleanup.projectDirGone} indexGone=${cleanup.projectIndexGone}`,
)
evidence.metrics.cleanup = cleanup

const receipt = withBooleanReceiptSummary({ ...evidence, results }, { version: EXPECTED_VERSION })
const receiptPath = join(RECEIPT_DIR, 'receipt.json')
writeFileSync(receiptPath, JSON.stringify(receipt, null, 2))
console.log(`RECEIPT ${receiptPath}`)
console.log(`SUMMARY pass=${receipt.summary.pass} fail=${receipt.summary.fail}`)
ws.close()
process.exit(receipt.summary.fail > 0 ? 1 : 0)
