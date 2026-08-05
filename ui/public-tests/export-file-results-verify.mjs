// export-file-results-verify.mjs - runtime proof for export destination results.
//
// Verifies the behavior users complained about:
//   - a configured default save folder is used for file-writing exports
//   - default names auto-suffix when the destination already exists
//   - explicit Save As paths stay exact and can overwrite the chosen file
//   - clearing the folder returns exports to the project exports directory
//
// RUN:
//   SWEEP_CUTD=http://127.0.0.1:6161 node ui/public-tests/export-file-results-verify.mjs

import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, join, resolve, win32 } from 'node:path'
import { resolveDriverPath } from '../../scripts/lib/cross-host-media.mjs'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6161'
const RECEIPT = process.env.CUT_RECEIPT || ''
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 45000)

const results = []
const evidence = {
  cutd: CUTD,
  tmp: '',
  projectDir: '',
  outputDir: '',
  paths: {},
}

function check(name, ok, detail = '') {
  const item = { name, ok: !!ok, detail }
  results.push(item)
  console.log(`${item.ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

async function verb(name, args = {}, timeoutMs = VERB_TIMEOUT_MS) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-cut-actor': 'human:ui:export-file-results-verify',
    },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(timeoutMs),
  })
  return response.json()
}

function pathString(value) {
  return String(value || '')
}

function stripExtendedPath(value) {
  const raw = String(value || '')
  if (raw.startsWith('\\\\?\\UNC\\')) return `\\\\${raw.slice(8)}`
  if (raw.startsWith('\\\\?\\')) return raw.slice(4)
  return raw
}

function driverPath(value) {
  return resolveDriverPath(String(value || ''))
}

function looksWindowsPath(value) {
  const raw = stripExtendedPath(value)
  return /^[a-z]:[\\/]/i.test(raw) || raw.startsWith('\\\\')
}

function pathBase(value) {
  const raw = driverPath(value)
  return looksWindowsPath(raw) ? win32.basename(raw) : basename(raw)
}

function samePath(left, right) {
  const leftDriver = driverPath(left)
  const rightDriver = driverPath(right)
  if (looksWindowsPath(leftDriver) || looksWindowsPath(rightDriver)) {
    return win32.resolve(stripExtendedPath(leftDriver)).toLowerCase() === win32.resolve(stripExtendedPath(rightDriver)).toLowerCase()
  }
  return resolve(leftDriver) === resolve(rightDriver)
}

function isJpeg(path) {
  const local = driverPath(path)
  if (!existsSync(local)) return false
  const bytes = readFileSync(local)
  return bytes.length > 8 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff
}

function under(child, parent) {
  const childDriver = driverPath(child)
  const parentDriver = driverPath(parent)
  if (looksWindowsPath(childDriver) || looksWindowsPath(parentDriver)) {
    const c = win32.resolve(stripExtendedPath(childDriver)).toLowerCase()
    const p = win32.resolve(stripExtendedPath(parentDriver)).toLowerCase()
    return c === p || c.startsWith(`${p}\\`)
  }
  const c = resolve(childDriver)
  const p = resolve(parentDriver)
  return c === p || c.startsWith(`${p}/`)
}

async function main() {
  const tmp = mkdtempSync(join(tmpdir(), 'shellx-cut-export-files-'))
  const name = `export_files_${Math.random().toString(36).slice(2, 8)}`
  const projectDir = join(tmp, `${name}.cutproj`)
  const outputDir = join(tmp, 'chosen-output')
  mkdirSync(outputDir, { recursive: true })
  evidence.tmp = tmp
  evidence.projectDir = projectDir
  evidence.outputDir = outputDir

  try {
    const created = await verb('project.create', {
      name,
      dir: projectDir,
      settings: { width: 1280, height: 720, fps: 30 },
    })
    check('project.create', created?.ok === true, created?.ok ? projectDir : JSON.stringify(created?.error ?? created).slice(0, 240))
    if (created?.ok !== true) throw new Error('cannot continue without project')

    const title = await verb('title.add', {
      text: 'Export path proof',
      range_ms: [0, 2000],
      rationale: 'export path verifier seed',
    })
    check('title.add-seed', title?.ok === true, title?.ok ? String(title.result?.clip_id ?? '') : JSON.stringify(title?.error ?? title).slice(0, 240))

    const setDir = await verb('project.set_output_dir', { dir: outputDir })
    check('project.set_output_dir', setDir?.ok === true && under(pathString(setDir.result?.dir), outputDir), JSON.stringify(setDir?.result ?? setDir?.error ?? setDir).slice(0, 240))

    const first = await verb('export.frame', { at_ms: 0, to_asset: false })
    const firstPath = pathString(first?.result?.path)
    evidence.paths.firstDefault = firstPath
    check('default-folder-export-created', first?.ok === true && under(firstPath, outputDir) && pathBase(firstPath) === 'frame_0.jpg' && isJpeg(firstPath), firstPath)

    const second = await verb('export.frame', { at_ms: 0, to_asset: false })
    const secondPath = pathString(second?.result?.path)
    evidence.paths.secondDefault = secondPath
    check('default-output-dedup-suffix', second?.ok === true && under(secondPath, outputDir) && pathBase(secondPath) === 'frame_0-2.jpg' && isJpeg(secondPath), `${firstPath} -> ${secondPath}`)

    const saveAsPath = join(outputDir, 'chosen-save-as.jpg')
    writeFileSync(saveAsPath, Buffer.from('old'))
    const oldSize = statSync(saveAsPath).size
    const explicit = await verb('export.frame', { at_ms: 0, to_asset: false, path: saveAsPath })
    const explicitPath = pathString(explicit?.result?.path)
    evidence.paths.explicit = explicitPath
    const newSize = existsSync(saveAsPath) ? statSync(saveAsPath).size : 0
    check(
      'save-as-overwrites-explicit-path',
      explicit?.ok === true && samePath(explicitPath, saveAsPath) && oldSize === 3 && newSize > oldSize && isJpeg(saveAsPath),
      `${explicitPath} size ${oldSize}->${newSize}`,
    )

    const cleared = await verb('project.set_output_dir', {})
    check('project.set_output_dir-clear', cleared?.ok === true && cleared.result?.cleared === true, JSON.stringify(cleared?.result ?? cleared?.error ?? cleared).slice(0, 240))

    const projectDefault = await verb('export.frame', { at_ms: 0, to_asset: false })
    const projectDefaultPath = pathString(projectDefault?.result?.path)
    evidence.paths.projectDefault = projectDefaultPath
    check(
      'cleared-folder-returns-to-project-exports',
      projectDefault?.ok === true && under(projectDefaultPath, join(projectDir, 'exports')) && isJpeg(projectDefaultPath),
      projectDefaultPath,
    )
  } finally {
    await verb('project.close', {}).catch(() => {})
    await verb('project.delete', { path: projectDir }).catch(() => {})
    await verb('project.forget', { path: projectDir }).catch(() => {})
    rmSync(tmp, { recursive: true, force: true })
  }

  const fail = results.filter((r) => !r.ok).length
  const pass = results.length - fail
  const receipt = { pass, fail, results, evidence }
  if (RECEIPT) writeFileSync(RECEIPT, `${JSON.stringify(receipt, null, 2)}\n`)
  console.log(`SUMMARY pass=${pass} fail=${fail}`)
  if (fail) process.exit(1)
}

main().catch((error) => {
  console.error(error?.stack || String(error))
  if (RECEIPT) writeFileSync(RECEIPT, `${JSON.stringify({ pass: 0, fail: 1, error: String(error?.stack || error), results, evidence }, null, 2)}\n`)
  process.exit(1)
})
