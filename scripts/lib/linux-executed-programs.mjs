// The external programs the installed app starts on Linux, and how each one is
// covered when a clean machine does not have it. The ELF gate
// (linux-elf-needed.mjs) reads libraries; this list covers programs, which the
// dynamic section does not show. The test that belongs to this list
// scans the Rust sources for every `Command::new("<name>")` literal and fails
// when a program is missing here, so a new one must be classified on purpose.
//
// kind:
//   declared  the package must declare it (deb names / rpm names or file
//             capabilities; checked against the finished package by the gate)
//   base      POSIX base of every supported system (Essential on Debian and
//             Ubuntu, in the base of Fedora); not declared
//   named     optional; the app names the program and how to get it at the
//             moment it is needed (`where` says where)
//   optional  a best-effort probe or an optimisation with a fallback; its
//             absence is not a failure
//   other-os  never started on Linux

export const EXECUTED_PROGRAMS = {
  tar: {
    kind: 'declared', deb: ['tar'], rpm: ['/usr/bin/tar'],
    why: 'unpacks the FFmpeg and uv downloads (fetch/pinned.rs, fetch.rs)',
  },
  xz: {
    kind: 'declared', deb: ['xz-utils'], rpm: ['/usr/bin/xz'],
    why: 'started by tar -J for the FFmpeg .tar.xz; also checked before the download (fetch/unpack_tools.rs)',
  },
  gzip: {
    kind: 'declared', deb: ['gzip'], rpm: ['/usr/bin/gzip'],
    why: 'started by tar -z for the uv .tar.gz (Install captions)',
  },
  'gst-launch-1.0': {
    kind: 'declared', deb: ['gstreamer1.0-tools'], rpm: ['/usr/bin/gst-launch-1.0'],
    why: 'the Linux screen recorder (record-capture linux_gst_capture.rs)',
  },
  'gst-inspect-1.0': {
    kind: 'declared', deb: ['gstreamer1.0-tools'], rpm: ['/usr/bin/gst-launch-1.0'],
    why: 'Doctor probe of the recorder elements; same package as gst-launch-1.0',
  },
  sh: { kind: 'base', why: 'process-tree helpers (ffmpeg process, jobs)' },
  kill: { kind: 'base', why: 'stops a recorder child' },
  nice: { kind: 'base', why: 'lowers the priority of ffmpeg when systemd-run is unavailable' },
  sleep: { kind: 'base', why: 'shell engine-stop helper' },
  true: { kind: 'base', why: 'process probes' },
  'systemd-run': { kind: 'optional', why: 'memory-capped ffmpeg scope; falls back to nice' },
  loginctl: { kind: 'optional', why: 'Doctor probe: is the desktop locked; silent probe, "not locked" when absent' },
  gdbus: { kind: 'optional', why: 'Doctor probe: GNOME screen saver; silent probe' },
  'xdg-open': {
    kind: 'named', where: 'source_reveal.rs: the refusal names xdg-open and the xdg-utils package',
    why: 'Reveal in folder',
  },
  ffmpeg: {
    kind: 'named', where: 'system.doctor tool_missing and the Preview setup notice (download or distribution package)',
    why: 'every media verb; not bundled',
  },
  ffprobe: { kind: 'named', where: 'as ffmpeg', why: 'media probing; downloaded with ffmpeg' },
  python3: {
    kind: 'named', where: 'Install captions (uv provisions its own Python; no PATH Python on an installed app)',
    why: 'captions and perception sidecar',
  },
  uv: { kind: 'named', where: 'Install captions downloads it (needs tar and gzip, declared)', why: 'captions runtime' },
  cutd: { kind: 'declared', deb: [], rpm: [], shipped: true, why: 'the engine itself, shipped in the package' },
  // Not started on Linux.
  '/usr/bin/codesign': { kind: 'other-os' },
  caffeinate: { kind: 'other-os' },
  cmd: { kind: 'other-os' },
  'cmd.exe': { kind: 'other-os' },
  'compact.exe': { kind: 'other-os' },
  'explorer.exe': { kind: 'other-os' },
  icacls: { kind: 'other-os', why: 'Windows-only test of the file-owner check (atomic_output/windows_acl.rs)' },
  open: { kind: 'other-os' },
  powershell: { kind: 'other-os' },
  ps: { kind: 'other-os' },
  sysctl: { kind: 'other-os' },
  // Test-only start sites.
  'cutd-test': { kind: 'other-os', why: 'macOS controller-owner test fixture' },
}

// Problems for the programs a finished package must declare. `declared` is the
// parsed Depends (deb) or Requires (rpm) of linux-elf-needed.mjs.
export function checkExecutedDeclared(format, declared) {
  const problems = []
  for (const [program, entry] of Object.entries(EXECUTED_PROGRAMS)) {
    if (entry.kind !== 'declared' || entry.shipped) continue
    const wanted = format === 'deb' ? entry.deb : entry.rpm
    if (!wanted.some((name) => declared.names.has(name))) {
      problems.push(`the app starts '${program}' (${entry.why}); declare ${wanted.join(' or ')}`)
    }
  }
  return problems
}
