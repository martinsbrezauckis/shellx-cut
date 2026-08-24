import { lstatSync } from 'node:fs'
import { isAbsolute, join, resolve, win32 } from 'node:path'
import { DEFAULT_WINDOWS_QUALIFICATION_ROOT, windowsQualificationLayout } from './windows-qualification-layout.mjs'
import { prepareWindowsEvidenceOutput } from './windows-test-control-completion.mjs'
import { removeWindowsOwnedDirectory } from './windows-owned-directory.mjs'
const SHA256 = /^[a-f0-9]{64}$/
const LOCAL_DRIVE = /^[A-Za-z]:\\/
function quotedPowerShell(value) {
  return `'${String(value).replaceAll("'", "''")}'`
}
function sameWindowsPath(left, right) {
  return win32.normalize(String(left || '')).toLowerCase() === win32.normalize(String(right || '')).toLowerCase()
}
function parsePowerShellJson(raw, label) {
  try { return JSON.parse(String(raw || '')) } catch { throw new Error(`${label} did not return JSON`) }
}
export function assertWindowsInstalledLocalDrivePath(value, label) {
  const raw = String(value || '').trim()
  const normalized = win32.normalize(raw).replace(/[\\/]+$/, '')
  if (!LOCAL_DRIVE.test(normalized) || normalized.startsWith('\\\\')) {
    throw new Error(`${label} must resolve to a Windows local drive path, never a WSL UNC path: ${raw || '(empty)'}`)
  }
  return normalized
}
export function assertWindowsInstalledLocalRoots(roots) {
  return Object.fromEntries(Object.entries(roots).map(([label, path]) => [
    label,
    assertWindowsInstalledLocalDrivePath(path, `Windows installed ${label} root`),
  ]))
}
export function assertWindowsInstalledRuntimeRoots({ qualificationEnv, runtimeHomeWin, projectsWin, exportWin, verifierTempWin, mediaWin }) {
  assertWindowsInstalledLocalRoots({
    runtime: qualificationEnv.SHELLX_CUT_HOME,
    projects: qualificationEnv.SHELLX_CUT_PROJECTS_DIR,
    export: exportWin,
    verifier: verifierTempWin,
    media: mediaWin,
  })
  if (qualificationEnv.SHELLX_CUT_HOME !== runtimeHomeWin || qualificationEnv.SHELLX_CUT_PROJECTS_DIR !== projectsWin) {
    throw new Error('Windows installed runtime roots differ from the provisioned exact local-drive workspace')
  }
}
// --workspace-test-root remains an explicit WSL input for operators, but only
// when wslpath resolves it onto a writable Windows drive. The old .scratch
// fallback resolves to \\wsl.localhost and is deliberately not a valid runtime.
export function resolveWindowsInstalledWorkspaceTestRoot({ workspaceTestRoot, windowsPath }) {
  const requested = String(workspaceTestRoot || '').trim()
  if (!requested || !isAbsolute(requested)) {
    throw new Error('--workspace-test-root requires an absolute WSL/Linux path on a Windows-mounted drive')
  }
  if (typeof windowsPath !== 'function') throw new Error('workspace test root requires a WSL-to-Windows path converter')
  const linuxRoot = resolve(requested)
  const windowsRoot = assertWindowsInstalledLocalDrivePath(
    windowsPath(linuxRoot),
    '--workspace-test-root',
  )
  return { linuxRoot, windowsRoot }
}
// Provision only the exact per-run children. The WSL mount may be read-only,
// so direct Node mkdir/copy calls through /mnt/c are intentionally forbidden.
export function provisionWindowsInstalledRunDirectories({
  stageWin,
  artifactsWin,
  directories,
  captureSync,
}) {
  if (typeof captureSync !== 'function') throw new Error('Windows workspace provisioning requires PowerShell capture')
  const stage = assertWindowsInstalledLocalDrivePath(stageWin, 'Windows installed run directory')
  const artifacts = assertWindowsInstalledLocalDrivePath(artifactsWin, 'Windows installed artifact directory')
  const stageParent = assertWindowsInstalledLocalDrivePath(win32.dirname(stage), 'Windows installed run parent')
  const artifactsParent = assertWindowsInstalledLocalDrivePath(win32.dirname(artifacts), 'Windows installed artifact parent')
  if (win32.dirname(stage).toLowerCase() !== stageParent.toLowerCase() ||
      win32.dirname(artifacts).toLowerCase() !== artifactsParent.toLowerCase()) {
    throw new Error('Windows installed workspace roots must remain direct children of their owned parents')
  }
  const ownedDirectories = Object.fromEntries(Object.entries(directories || {}).map(([label, path]) => {
    const local = assertWindowsInstalledLocalDrivePath(path, `Windows installed ${label} directory`)
    if (!local.toLowerCase().startsWith(`${stage.toLowerCase()}\\`)) {
      throw new Error(`Windows installed ${label} directory must stay beneath the exact run directory`)
    }
    return [label, local]
  }))
  const childValues = Object.values(ownedDirectories)
  const script = [
    '$ErrorActionPreference="Stop"',
    `$stage=${quotedPowerShell(stage)}`,
    `$artifacts=${quotedPowerShell(artifacts)}`,
    `$stageParent=${quotedPowerShell(stageParent)}`,
    `$artifactsParent=${quotedPowerShell(artifactsParent)}`,
    `$children=@(${childValues.map(quotedPowerShell).join(',')})`,
    'function Assert-DirectOwnedChild([string]$parent,[string]$target){',
    '  $parentFull=[System.IO.Path]::GetFullPath($parent).TrimEnd("\\")',
    '  $targetFull=[System.IO.Path]::GetFullPath($target)',
    '  if(-not $targetFull.StartsWith($parentFull+"\\",[System.StringComparison]::OrdinalIgnoreCase)){throw "owned path escaped its direct parent: $targetFull"}',
    '}',
    'function Assert-NoReparseAncestors([string]$path){',
    '  $full=[System.IO.Path]::GetFullPath($path)',
    '  $root=[System.IO.Path]::GetPathRoot($full)',
    '  $cursor=$root',
    '  foreach($part in $full.Substring($root.Length).Split(@("\\"),[System.StringSplitOptions]::RemoveEmptyEntries)){',
    '    $cursor=[System.IO.Path]::Combine($cursor,$part)',
    '    $item=Get-Item -LiteralPath $cursor -Force -ErrorAction SilentlyContinue',
    '    if($null -eq $item){break}',
    '    if(-not $item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned path contains a non-directory or reparse ancestor: $cursor"}',
    '  }',
    '}',
    'function Assert-RealDirectory([string]$path){',
    '  $item=Get-Item -LiteralPath $path -Force',
    '  if(-not $item.PSIsContainer -or ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned path is not a real directory: $path"}',
    '}',
    'Assert-DirectOwnedChild $stageParent $stage',
    'Assert-DirectOwnedChild $artifactsParent $artifacts',
    'foreach($pair in @(@($stageParent,$stage),@($artifactsParent,$artifacts))){',
    '  Assert-NoReparseAncestors $pair[0]',
    '  [System.IO.Directory]::CreateDirectory($pair[0])|Out-Null',
    '  Assert-NoReparseAncestors $pair[0]',
    '  Assert-RealDirectory $pair[0]',
    '  $existing=Get-Item -LiteralPath $pair[1] -Force -ErrorAction SilentlyContinue',
    '  if($null -ne $existing){throw "owned Windows run directory already exists: $($pair[1])"}',
    '  Assert-NoReparseAncestors $pair[1]',
    '  [System.IO.Directory]::CreateDirectory($pair[1])|Out-Null',
    '  Assert-NoReparseAncestors $pair[1]',
    '  Assert-RealDirectory $pair[1]',
    '}',
    'foreach($child in $children){',
    '  Assert-DirectOwnedChild $stage $child',
    '  Assert-NoReparseAncestors $child',
    '  [System.IO.Directory]::CreateDirectory($child)|Out-Null',
    '  Assert-NoReparseAncestors $child',
    '  Assert-RealDirectory $child',
    '}',
    '[pscustomobject]@{stage=[System.IO.Path]::GetFullPath($stage);artifacts=[System.IO.Path]::GetFullPath($artifacts);directories=@($children|ForEach-Object{[System.IO.Path]::GetFullPath($_)})}|ConvertTo-Json -Compress',
  ].join(';')
  const receipt = parsePowerShellJson(captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', script,
  ]), 'Windows workspace provisioning')
  if (!sameWindowsPath(receipt?.stage, stage) || !sameWindowsPath(receipt?.artifacts, artifacts) ||
      !Array.isArray(receipt?.directories) || receipt.directories.length !== childValues.length ||
      receipt.directories.some((path, index) => !sameWindowsPath(path, childValues[index]))) {
    throw new Error('Windows workspace provisioning readback differs from the exact requested local-drive directories')
  }
  return { stage, artifacts, directories: ownedDirectories }
}
// Source files may remain in the WSL checkout and therefore arrive as UNC
// paths. PowerShell performs the read from that source and the write directly
// to the owned C: destination, then hashes both sides before returning.
export function copyWindowsOwnedFile({ source, destinationWin, ownedRootWin, windowsPath, captureSync, hash }) {
  if (typeof windowsPath !== 'function' || typeof captureSync !== 'function' || typeof hash !== 'function') {
    throw new Error('Windows owned-file staging requires path conversion, PowerShell, and SHA-256 helpers')
  }
  const sourceLinux = resolve(String(source || ''))
  let sourceMetadata
  try { sourceMetadata = lstatSync(sourceLinux) } catch { throw new Error(`Windows owned-file staging requires an exact readable source: ${sourceLinux}`) }
  if (!sourceMetadata.isFile() || sourceMetadata.isSymbolicLink()) {
    throw new Error(`Windows owned-file source must be a regular non-symlink file: ${sourceLinux}`)
  }
  const sourceWin = String(windowsPath(sourceLinux) || '').trim()
  const destination = assertWindowsInstalledLocalDrivePath(destinationWin, 'Windows owned-file destination')
  const ownedRoot = assertWindowsInstalledLocalDrivePath(ownedRootWin, 'Windows owned-file root')
  const destinationRelative = win32.relative(ownedRoot, destination)
  if (!destinationRelative || destinationRelative.startsWith('..\\') || destinationRelative === '..' || win32.isAbsolute(destinationRelative)) {
    throw new Error(`Windows owned-file destination must stay beneath the exact owned root: ${destination}`)
  }
  const expectedSha256 = String(hash(sourceLinux) || '').toLowerCase()
  if (!sourceWin || !SHA256.test(expectedSha256)) throw new Error(`Windows owned-file staging requires an exact readable source: ${sourceLinux}`)
  const script = [
    '$ErrorActionPreference="Stop"',
    `$source=${quotedPowerShell(sourceWin)}`,
    `$destination=${quotedPowerShell(destination)}`,
    `$ownedRoot=${quotedPowerShell(ownedRoot)}`,
    `$expected=${quotedPowerShell(expectedSha256)}`,
    'function Assert-NoReparseAncestors([string]$path){',
    '  $full=[System.IO.Path]::GetFullPath($path)',
    '  $root=[System.IO.Path]::GetPathRoot($full)',
    '  $cursor=$root',
    '  foreach($part in $full.Substring($root.Length).Split(@("\\"),[System.StringSplitOptions]::RemoveEmptyEntries)){',
    '    $cursor=[System.IO.Path]::Combine($cursor,$part)',
    '    $item=Get-Item -LiteralPath $cursor -Force -ErrorAction SilentlyContinue',
    '    if($null -eq $item){break}',
    '    if(($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned file path contains a reparse ancestor: $cursor"}',
    '  }',
    '}',
    'if(-not (Test-Path -LiteralPath $source -PathType Leaf)){throw "owned source file is missing: $source"}',
    '$sourceItem=Get-Item -LiteralPath $source -Force',
    'if($sourceItem.PSIsContainer -or ($sourceItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned source is not a regular non-reparse file: $source"}',
    '$rootFull=[System.IO.Path]::GetFullPath($ownedRoot).TrimEnd("\\")',
    '$destinationFull=[System.IO.Path]::GetFullPath($destination)',
    'if(-not $destinationFull.StartsWith($rootFull+"\\",[System.StringComparison]::OrdinalIgnoreCase)){throw "owned destination escaped its exact root: $destinationFull"}',
    '$parent=[System.IO.Path]::GetDirectoryName($destination)',
    'if(-not [System.IO.Directory]::Exists($parent)){throw "owned destination parent is missing: $parent"}',
    'Assert-NoReparseAncestors $ownedRoot',
    'Assert-NoReparseAncestors $parent',
    '$parentItem=Get-Item -LiteralPath $parent -Force',
    'if(-not $parentItem.PSIsContainer -or ($parentItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned destination parent is not a real directory: $parent"}',
    '$destinationExisting=Get-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue',
    'if($null -ne $destinationExisting){throw "owned destination already exists: $destination"}',
    'Copy-Item -LiteralPath $source -Destination $destination',
    '$destinationItem=Get-Item -LiteralPath $destination -Force',
    'if($destinationItem.PSIsContainer -or ($destinationItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned destination is not a regular non-reparse file: $destination"}',
    '$sourceItemAfter=Get-Item -LiteralPath $source -Force',
    'if($sourceItemAfter.PSIsContainer -or ($sourceItemAfter.Attributes -band [System.IO.FileAttributes]::ReparsePoint)){throw "owned source changed into a non-file or reparse point: $source"}',
    '$sourceHash=(Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant()',
    '$destinationHash=(Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash.ToLowerInvariant()',
    'if($sourceHash -ne $expected -or $destinationHash -ne $expected){throw "owned Windows file hash readback differs from source"}',
    '[pscustomobject]@{path=[System.IO.Path]::GetFullPath($destination);sha256=$destinationHash}|ConvertTo-Json -Compress',
  ].join(';')
  const receipt = parsePowerShellJson(captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', script,
  ]), 'Windows owned-file staging')
  if (!sameWindowsPath(receipt?.path, destination) || String(receipt?.sha256 || '').toLowerCase() !== expectedSha256) {
    throw new Error(`Windows owned-file staging readback differs from ${destination}`)
  }
  return { path: destination, sha256: expectedSha256 }
}
// --clean-after may delete only the exact direct run child that this helper
// created. It refuses reparse points and never accepts a broad root.
export function removeWindowsInstalledRunDirectory({ runsRootWin, stageWin, captureSync }) {
  removeWindowsOwnedDirectory({
    parentWin: assertWindowsInstalledLocalDrivePath(runsRootWin, 'Windows installed cleanup root'),
    targetWin: assertWindowsInstalledLocalDrivePath(stageWin, 'Windows installed cleanup directory'),
    captureSync,
    label: 'Windows installed run directory',
  })
}
// Materialize the per-run Windows workspace before build/install work starts.
// Windows runtime state is C:-local; only evidence remains in the WSL/control
// output tree, which may legitimately map to a UNC path.
export function prepareWindowsInstalledRunWorkspace({
  root,
  workspaceTestRoot,
  windowsTestRootProvided,
  requestedWindowsTestRoot,
  inheritedWindowsTestRoot,
  runId,
  outArgument,
  testControlBindingPath,
  testControlAuthority,
  finalResumeRequest,
  assertFinalResumeOutput,
  roleArgs,
  windowsPath,
  linuxPath,
  captureSync,
  hash,
  provisionWindowsRun = provisionWindowsInstalledRunDirectories,
  copyWindowsFile = copyWindowsOwnedFile,
  stamp,
}) {
  if (workspaceTestRoot && windowsTestRootProvided) {
    throw new Error('--workspace-test-root and --windows-test-root are mutually exclusive')
  }
  const workspaceRoot = workspaceTestRoot
    ? resolveWindowsInstalledWorkspaceTestRoot({ workspaceTestRoot: resolve(workspaceTestRoot), windowsPath })
    : null
  const layout = windowsQualificationLayout({
    root: workspaceRoot?.windowsRoot || requestedWindowsTestRoot || inheritedWindowsTestRoot || DEFAULT_WINDOWS_QUALIFICATION_ROOT,
    runId,
  })
  const stageWin = assertWindowsInstalledLocalDrivePath(layout.stage, 'Windows installed stage')
  const runsRootWin = assertWindowsInstalledLocalDrivePath(win32.join(layout.root, 'runs'), 'Windows installed runs root')
  const artifactsWin = assertWindowsInstalledLocalDrivePath(layout.artifacts, 'Windows installed artifacts')
  const verifierTempWin = win32.join(stageWin, 'verifier-temp')
  const mediaWin = win32.join(stageWin, 'media')
  const fixtureWin = win32.join(stageWin, 'agent-fixtures')
  const runtimeHomeWin = win32.join(stageWin, 'app-home')
  const projectsWin = win32.join(stageWin, 'projects')
  const exportWin = win32.join(projectsWin, 'exports')
  const installerWin = win32.join(stageWin, 'installer')
  const localRoots = assertWindowsInstalledLocalRoots({
    runtime: runtimeHomeWin,
    projects: projectsWin,
    export: exportWin,
    verifier: verifierTempWin,
    media: mediaWin,
    fixtures: fixtureWin,
    installer: installerWin,
  })
  // The controlled receipt output intentionally remains outside the C: runtime
  // tree. Do not coerce --out or the registered control receipt root.
  const defaultEvidenceOut = join(root, '.scratch', 'windows-installed-evidence', runId)
  const out = resolve(outArgument || defaultEvidenceOut)
  const testControlBinding = prepareWindowsEvidenceOutput({
    bindingPath: testControlBindingPath,
    authority: testControlAuthority,
    out,
    finalResumeRequest,
    assertFinalResumeOutput,
  })
  const provisioned = provisionWindowsRun({
    stageWin,
    artifactsWin,
    directories: localRoots,
    captureSync,
  })
  const artifacts = linuxPath(artifactsWin)
  const stage = linuxPath(stageWin)
  const media = linuxPath(mediaWin)
  const fixtureDir = linuxPath(fixtureWin)
  const windowsBasePath = captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    "[Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [Environment]::GetEnvironmentVariable('PATH','User')",
  ])
  const localAppDataWin = captureSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', '$env:LOCALAPPDATA',
  ])
  const runtimeLocalAppDataWin = runtimeHomeWin
  const webviewDataToken = `ShellXCutFinalAction-${stamp()}`
  const webviewDataWin = win32.join(runtimeLocalAppDataWin, 'ShellX Cut WebView Tests', webviewDataToken)
  assertWindowsInstalledLocalRoots({ webview: webviewDataWin, nativeCompile: win32.join(runtimeLocalAppDataWin, 'native-fixture-compile') })
  const adapterPythonWin = win32.join(localAppDataWin, 'ShellX Cut', 'perception', '.venv', 'Scripts', 'python.exe')
  const nativeCompileWin = win32.join(runtimeLocalAppDataWin, 'native-fixture-compile')
  const copyOwnedFile = (source, destination) => copyWindowsFile({
    source,
    destinationWin: assertWindowsInstalledLocalDrivePath(windowsPath(destination), 'Windows owned-file destination'),
    ownedRootWin: stageWin,
    windowsPath,
    captureSync,
    hash,
  })
  const copyOwnedArtifactFile = (source, destination) => copyWindowsFile({
    source,
    destinationWin: assertWindowsInstalledLocalDrivePath(windowsPath(destination), 'Windows owned-artifact destination'),
    ownedRootWin: artifactsWin,
    windowsPath,
    captureSync,
    hash,
  })
  const staged = {}
  for (const [role, source] of Object.entries(roleArgs)) {
    const name = `${role}${source.slice(source.lastIndexOf('.')) || '.mp4'}`
    const destinationWin = win32.join(mediaWin, name)
    copyWindowsFile({ source, destinationWin, ownedRootWin: stageWin, windowsPath, captureSync, hash })
    staged[role] = destinationWin
  }
  const mediaIdentity = Object.fromEntries(Object.entries(roleArgs).map(([role, path]) => [role, { sha256: hash(path) }]))
  const fullCoverageReceipt = join(out, 'full-coverage-receipt.json')
  const coverageRunReceipt = finalResumeRequest ? join(out, 'resume-section-receipt.json') : fullCoverageReceipt
  return {
    workspaceRoot, layout, out, testControlBinding, artifacts, artifactsWin, runsRootWin, stageWin, stage,
    verifierTempWin, mediaWin, media, fixtureDir, fixtureWin, windowsBasePath,
    localAppDataWin, runtimeLocalAppDataWin, runtimeHomeWin, projectsWin, exportWin,
    webviewDataToken, webviewDataWin, adapterPythonWin, nativeCompileWin, staged,
    mediaIdentity, fullCoverageReceipt, coverageRunReceipt, copyOwnedFile, copyOwnedArtifactFile, provisioned,
  }
}
