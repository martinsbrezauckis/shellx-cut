#!/usr/bin/env node
// Dependency gate for the shipped Linux packages (DEB and RPM).
//
// A Linux program that links a shared library the machine does not have never
// starts: the loader prints "error while loading shared libraries: X: cannot
// open shared object file" before any of our code runs, so the app cannot
// explain what is missing. The Tauri bundler writes the package dependency
// list from the configuration only; it does not read the binaries. This gate
// reads the binaries and compares them with what the finished package declares.
//
// For every ELF file in an unpacked package it checks, and fails the build when
// one of these is not true:
//   1. Each DT_NEEDED library is a base-system library (BASE_SYSTEM), part of
//      the closure of the webview packages the bundler always declares
//      (WEBVIEW_CLOSURE), or listed in LIBRARY_PACKAGES with a package that the
//      package metadata really declares.
//   2. Each library name that appears as a text literal in the file (what
//      dlopen and libloading use, which DT_NEEDED does not show) is covered the
//      same way, or is listed in OPTIONAL_AT_RUN_TIME with a reason.
//   3. The highest GLIBC_x.y symbol version the file needs is declared as a
//      minimum libc version (otherwise an older distribution installs the
//      package and then fails with "version GLIBC_2.39 not found").
//
// libstdc++ is deliberately not in BASE_SYSTEM: Rust programs do not link it,
// and a minimal image does not promise it. If a binary starts to need it, the
// package must declare it. libgcc_s is in BASE_SYSTEM (priority-required on
// Debian and part of every Fedora/openSUSE base).
//
// Usage:
//   node scripts/lib/linux-elf-needed.mjs --format deb --depends-file F <dir>...
//   node scripts/lib/linux-elf-needed.mjs --format rpm --requires-file F <dir>...
// F holds the package's declared dependencies: the `Depends` field of the DEB
// (dpkg-deb -f PKG Depends) or the output of `rpm -qp --requires PKG`.
// Exit status 0 only when at least one ELF was read and everything is covered.

import { closeSync, lstatSync, openSync, readdirSync, readFileSync, readSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import { checkExecutedDeclared } from './linux-executed-programs.mjs'

// Library stems (name without .so and version) that every supported
// distribution has on a minimal install. glibc itself and the compiler runtime.
export const BASE_SYSTEM = new Set([
  'ld-linux-x86-64',
  'libc',
  'libdl',
  'libgcc_s',
  'libm',
  'libpthread',
  'libresolv',
  'librt',
  'libutil',
  'linux-vdso',
])

// Libraries that arrive with the dependencies the Tauri bundler always writes:
// libwebkit2gtk-4.1 and libgtk-3 (DEB: libwebkit2gtk-4.1-0, libgtk-3-0; RPM:
// the matching soname capabilities). Each is a direct dependency of those
// packages or of one of their hard dependencies. Installing the packages on
// clean minimal images is what proves the closure; this list only says what
// the build trusts.
export const WEBVIEW_CLOSURE = new Set([
  'libcairo',
  'libdbus-1',
  'libgdk-3',
  'libgdk_pixbuf-2.0',
  'libgio-2.0',
  'libglib-2.0',
  'libgobject-2.0',
  'libgtk-3',
  'libjavascriptcoregtk-4.1',
  'libsoup-3.0',
  'libwebkit2gtk-4.1',
])

// Libraries the package must declare itself. `deb` lists acceptable package
// names (any one is enough), `rpm` lists acceptable package names or
// capabilities; the soname capability `<soname>()(64bit)` is always accepted.
export const LIBRARY_PACKAGES = {
  libasound: { deb: ['libasound2t64', 'libasound2'], rpm: ['alsa-lib'] },
  'libgstreamer-1.0': { deb: ['libgstreamer1.0-0'], rpm: ['gstreamer1'] },
  'libpipewire-0.3': { deb: ['libpipewire-0.3-0t64', 'libpipewire-0.3-0'], rpm: ['pipewire-libs'] },
  libX11: { deb: ['libx11-6'], rpm: ['libX11'] },
  libXi: { deb: ['libxi6'], rpm: ['libXi'] },
  libXtst: { deb: ['libxtst6'], rpm: ['libXtst'] },
}

// Library names that appear as literals but are optional extras: the program
// works without them. Each entry says what is lost when it is absent.
export const OPTIONAL_AT_RUN_TIME = {
  libunity: 'Unity launcher progress badge; absent on every current desktop',
}

const GLIBC_PACKAGE = { deb: 'libc6', rpmCapability: (version) => `libc.so.6(GLIBC_${version})(64bit)` }

export function stemOf(name) {
  return name.replace(/\.so(\.[0-9]+)*$/, '')
}

const PT_LOAD = 1
const PT_DYNAMIC = 2
const DT_NULL = 0
const DT_NEEDED = 1
const DT_STRTAB = 5
const DT_VERNEED = 0x6ffffffe
const DT_VERNEEDNUM = 0x6fffffff

function fail(file, message) {
  throw new Error(`${file}: ${message}`)
}

export function isElf(buffer) {
  return buffer.length >= 4 && buffer[0] === 0x7f && buffer[1] === 0x45 && buffer[2] === 0x4c && buffer[3] === 0x46
}

// Returns { needed: [soname], glibc: [version strings], literals: [library names] } for a
// 64-bit little-endian ELF image.
export function readElfNeeded(buffer, file = '<buffer>') {
  if (!isElf(buffer)) fail(file, 'not an ELF file')
  if (buffer[4] !== 2 || buffer[5] !== 1) fail(file, 'only 64-bit little-endian ELF is supported')
  const u16 = (offset) => {
    if (offset < 0 || offset + 2 > buffer.length) fail(file, 'truncated ELF file')
    return buffer.readUInt16LE(offset)
  }
  const u32 = (offset) => {
    if (offset < 0 || offset + 4 > buffer.length) fail(file, 'truncated ELF file')
    return buffer.readUInt32LE(offset)
  }
  const u64 = (offset) => {
    if (offset < 0 || offset + 8 > buffer.length) fail(file, 'truncated ELF file')
    return Number(buffer.readBigUInt64LE(offset))
  }
  const phoff = u64(0x20)
  const phentsize = u16(0x36)
  const phnum = u16(0x38)
  const loads = []
  let dynamic = null
  for (let index = 0; index < phnum; index += 1) {
    const header = phoff + index * phentsize
    const type = u32(header)
    const entry = { offset: u64(header + 8), vaddr: u64(header + 16), filesz: u64(header + 32) }
    if (type === PT_LOAD) loads.push(entry)
    else if (type === PT_DYNAMIC) dynamic = entry
  }
  const needed = []
  const glibc = []
  if (dynamic) {
    const toOffset = (vaddr) => {
      for (const load of loads) {
        if (vaddr >= load.vaddr && vaddr < load.vaddr + load.filesz) return load.offset + (vaddr - load.vaddr)
      }
      return fail(file, `address 0x${vaddr.toString(16)} is in no loaded segment`)
    }
    const neededOffsets = []
    let strtab = 0
    let verneed = 0
    let verneedNum = 0
    for (let at = dynamic.offset; at + 16 <= dynamic.offset + dynamic.filesz; at += 16) {
      const tag = u64(at)
      const value = u64(at + 8)
      if (tag === DT_NULL) break
      if (tag === DT_NEEDED) neededOffsets.push(value)
      else if (tag === DT_STRTAB) strtab = value
      else if (tag === DT_VERNEED) verneed = value
      else if (tag === DT_VERNEEDNUM) verneedNum = value
    }
    const cString = (offset) => {
      const end = buffer.indexOf(0, offset)
      if (end < 0 || end - offset > 512) fail(file, 'unterminated string in the dynamic section')
      return buffer.toString('latin1', offset, end)
    }
    if (neededOffsets.length > 0) {
      if (strtab === 0) fail(file, 'DT_NEEDED without a string table')
      const table = toOffset(strtab)
      for (const offset of neededOffsets) needed.push(cString(table + offset))
    }
    if (verneed !== 0 && verneedNum > 0) {
      const table = toOffset(strtab)
      let entry = toOffset(verneed)
      for (let index = 0; index < verneedNum; index += 1) {
        const count = u16(entry + 2)
        const next = u32(entry + 12)
        let aux = entry + u32(entry + 8)
        for (let auxIndex = 0; auxIndex < count; auxIndex += 1) {
          const name = cString(table + u32(aux + 8))
          if (name.startsWith('GLIBC_') && /^GLIBC_[0-9]+(\.[0-9]+)*$/.test(name)) glibc.push(name.slice(6))
          const auxNext = u32(aux + 12)
          if (auxNext === 0) break
          aux += auxNext
        }
        if (next === 0) break
        entry += next
      }
    }
  }
  const literals = new Set()
  for (const match of buffer.toString('latin1').matchAll(/lib[A-Za-z0-9_+-]+\.so(?:\.[0-9]+)*/g)) literals.add(match[0])
  return { needed, glibc, literals: [...literals].sort() }
}

export function compareVersions(a, b) {
  const left = a.split('.').map(Number)
  const right = b.split('.').map(Number)
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0)
    if (difference !== 0) return difference
  }
  return 0
}

// Parses a DEB Depends field. Returns { names: Set of package names (every
// alternative), libc: highest "libc6 (>= X)" version or null }.
export function parseDebDepends(text) {
  const names = new Set()
  let libc = null
  for (const clause of text.replace(/\n\s+/g, ' ').split(',')) {
    for (const alternative of clause.split('|')) {
      const match = alternative.trim().match(/^([a-z0-9][a-z0-9+.-]*)(?::[a-z0-9-]+)?\s*(?:\(\s*(>=|>>|=)\s*([^)\s]+)\s*\))?/)
      if (!match) continue
      names.add(match[1])
      if (match[1] === 'libc6' && match[2] && (libc === null || compareVersions(match[3].replace(/-.*$/, ''), libc) > 0)) {
        libc = match[3].replace(/-.*$/, '')
      }
    }
  }
  return { names, libc }
}

// Parses `rpm -qp --requires` output. Returns { names: Set of requirement
// strings, libc: highest GLIBC version required through libc.so.6 or null }.
export function parseRpmRequires(text) {
  const names = new Set()
  let libc = null
  for (const raw of text.split('\n')) {
    const line = raw.trim().replace(/\s*(>=|=|<=|>|<)\s.*$/, '')
    if (!line || line.startsWith('rpmlib(')) continue
    names.add(line)
    const match = line.match(/^libc\.so\.6\(GLIBC_([0-9.]+)\)/)
    if (match && (libc === null || compareVersions(match[1], libc) > 0)) libc = match[1]
  }
  return { names, libc }
}

function declaredCovers(format, declared, stem, soname) {
  const entry = LIBRARY_PACKAGES[stem]
  if (!entry) return false
  if (format === 'deb') return entry.deb.some((name) => declared.names.has(name))
  // A literal such as libX11.so carries no version, so accept any declared
  // soname capability of the same library (libX11.so.6()(64bit)).
  const capabilityPrefix = `${stem}.so.`
  return entry.rpm.some((name) => declared.names.has(name))
    || [...declared.names].some((name) => name.startsWith(capabilityPrefix) && name.endsWith('()(64bit)'))
}

function webviewDeclared(format, declared) {
  return format === 'deb'
    ? declared.names.has('libwebkit2gtk-4.1-0')
    : declared.names.has('libwebkit2gtk-4.1.so.0()(64bit)') || declared.names.has('webkit2gtk4.1')
}

// Checks one ELF description against the declared dependencies. Returns the
// list of problem strings (empty when covered).
export function checkElf(info, format, declared, file = '<elf>') {
  const problems = []
  const needsStems = new Map(info.needed.map((soname) => [stemOf(soname), soname]))
  const classify = (stem, soname, kind) => {
    if (BASE_SYSTEM.has(stem)) return
    if (WEBVIEW_CLOSURE.has(stem)) {
      if (!webviewDeclared(format, declared)) {
        problems.push(`${file} ${kind} ${soname}: comes with the webview packages, but they are not declared`)
      }
      return
    }
    if (LIBRARY_PACKAGES[stem]) {
      if (!declaredCovers(format, declared, stem, soname)) {
        const wanted = format === 'deb'
          ? LIBRARY_PACKAGES[stem].deb.join(' or ')
          : `${stemOf(soname)}.so.<N>()(64bit)`.replace('<N>', soname.match(/\.so\.(\d+)/)?.[1] ?? '<N>')
        problems.push(`${file} ${kind} ${soname}: no declared dependency covers it (declare ${wanted})`)
      }
      return
    }
    if (kind === 'literal' && OPTIONAL_AT_RUN_TIME[stem]) return
    problems.push(`${file} ${kind} ${soname}: not a base-system library and not covered by any declared dependency`)
  }
  for (const soname of info.needed) classify(stemOf(soname), soname, 'needs')
  const seenLiterals = new Set()
  for (const literal of info.literals) {
    const stem = stemOf(literal)
    // A literal that is also linked normally was checked above; report each
    // library once even when it appears with and without a version.
    if (needsStems.has(stem) || seenLiterals.has(stem)) continue
    seenLiterals.add(stem)
    // Literals of base-system libraries (libc.so, libm.so ...) need no check.
    if (BASE_SYSTEM.has(stem)) continue
    classify(stem, literal, 'literal')
  }
  if (info.glibc.length > 0) {
    const highest = info.glibc.reduce((a, b) => (compareVersions(a, b) >= 0 ? a : b))
    if (declared.libc === null || compareVersions(declared.libc, highest) < 0) {
      const wanted = format === 'deb' ? `${GLIBC_PACKAGE.deb} (>= ${highest})` : GLIBC_PACKAGE.rpmCapability(highest)
      problems.push(`${file} needs GLIBC_${highest}: the package must declare ${wanted} (declared: ${declared.libc ?? 'no minimum'})`)
    }
  }
  return problems
}

function collect(path, out) {
  const stats = lstatSync(path)
  if (stats.isSymbolicLink()) return
  if (stats.isDirectory()) {
    for (const name of readdirSync(path).sort()) collect(join(path, name), out)
  } else if (stats.isFile() && stats.size >= 64) {
    const fd = openSync(path, 'r')
    try {
      const head = Buffer.alloc(4)
      readSync(fd, head, 0, 4, 0)
      if (isElf(head)) out.push(path)
    } finally {
      closeSync(fd)
    }
  }
}

// Checks every ELF file under the given paths. Returns { files, problems }.
export function checkLinuxPackage(paths, format, declaredText) {
  const declared = format === 'deb' ? parseDebDepends(declaredText) : parseRpmRequires(declaredText)
  const files = []
  for (const path of paths) collect(resolve(path), files)
  const results = []
  const problems = []
  for (const file of files) {
    const info = readElfNeeded(readFileSync(file), file)
    results.push({ file, ...info })
    problems.push(...checkElf(info, format, declared, file))
  }
  // Programs the app starts (tar, xz, gst-launch-1.0 ...): see linux-executed-programs.mjs.
  if (files.length > 0) problems.push(...checkExecutedDeclared(format, declared))
  return { files: results, problems }
}

function main(argv) {
  let format = null
  let declaredFile = null
  const paths = []
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--format') format = argv[++index]
    else if (arg === '--depends-file' || arg === '--requires-file') declaredFile = argv[++index]
    else paths.push(arg)
  }
  if (!['deb', 'rpm'].includes(format) || !declaredFile || paths.length === 0) {
    console.error('usage: linux-elf-needed.mjs --format deb|rpm --depends-file|--requires-file <file> <dir>...')
    return 2
  }
  let report
  try {
    report = checkLinuxPackage(paths, format, readFileSync(declaredFile, 'utf8'))
  } catch (error) {
    console.error(`FAIL: Linux dependency gate could not read the package: ${error.message}`)
    return 1
  }
  if (report.files.length === 0) {
    console.error(`FAIL: Linux dependency gate found no ELF file under: ${paths.join(' ')}`)
    return 1
  }
  for (const { file, needed, glibc, literals } of report.files) {
    const highest = glibc.length ? glibc.reduce((a, b) => (compareVersions(a, b) >= 0 ? a : b)) : 'none'
    console.log(`[dep-gate:${format}] ${file}: needs ${needed.join(' ')} | glibc ${highest} | literals ${literals.join(' ') || 'none'}`)
  }
  if (report.problems.length > 0) {
    for (const problem of report.problems) console.error(`FAIL: ${problem}`)
    console.error(`FAIL: a shipped Linux binary needs a library the ${format.toUpperCase()} does not declare; a clean machine would not start it`)
    return 1
  }
  console.log(`[dep-gate:${format}] OK: ${report.files.length} ELF files, every library is base-system, in the webview closure, or declared`)
  return 0
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = main(process.argv.slice(2))
}
