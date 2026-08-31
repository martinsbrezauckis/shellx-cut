import { existsSync, lstatSync } from 'node:fs'
import { join, win32 } from 'node:path'

import { assertWindowsOwnedDirectChild, windowsOwnedPathPowerShell } from './windows-owned-directory.mjs'

function windowsEnvironment(values) {
  const additions = Object.keys(values)
  const inherited = String(process.env.WSLENV || '').split(':').filter(Boolean)
  return { ...process.env, ...values, WSLENV: [...new Set([...inherited, ...additions])].join(':') }
}

function commandEnvironment(values) {
  return windowsEnvironment(Object.fromEntries(Object.entries(values).map(([key, value]) => [key, String(value)])))
}

export function compileWindowsAntigravityFixture({
  fixtureDir,
  fixtureWin,
  nativeCompileParentWin,
  nativeCompileWin,
  captureCommand,
}) {
  if (typeof captureCommand !== 'function') throw new Error('Windows Antigravity fixture compilation requires a command capture helper')
  const sourceName = 'agy-windows-launcher.rs'
  const source = join(fixtureDir, sourceName)
  const output = join(fixtureDir, 'agy.exe')
  let sourceMetadata
  try { sourceMetadata = lstatSync(source) } catch { throw new Error(`Windows qualification requires the native Antigravity launcher source ${sourceName}`) }
  if (!sourceMetadata.isFile() || sourceMetadata.isSymbolicLink()) {
    throw new Error(`Windows qualification requires ${sourceName} to be a regular non-symlink file`)
  }
  const compile = assertWindowsOwnedDirectChild({
    parentWin: nativeCompileParentWin,
    targetWin: nativeCompileWin,
    label: 'Windows native fixture compile root',
  })
  const sourceWin = win32.join(fixtureWin, sourceName)
  const destinationWin = win32.join(fixtureWin, 'agy.exe')
  const localSourceWin = win32.join(compile.target, sourceName)
  const localOutputWin = win32.join(compile.target, 'agy.exe')
  const rustcWin = captureCommand('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    '(Get-Command rustc.exe -ErrorAction Stop).Source',
  ], fixtureDir)
  const rustc = captureCommand('wslpath', ['-u', rustcWin], fixtureDir)
  const helpers = windowsOwnedPathPowerShell()
  let compileError = null
  let cleanupError = null
  let created = false
  try {
    const setupScript = [
      '$ErrorActionPreference="Stop"',
      '$parent=$env:SHELLX_CUT_NATIVE_COMPILE_PARENT',
      '$root=$env:SHELLX_CUT_NATIVE_COMPILE_ROOT',
      '$fixtureRoot=$env:SHELLX_CUT_NATIVE_FIXTURE_ROOT',
      '$source=$env:SHELLX_CUT_NATIVE_COMPILE_SOURCE',
      '$localSource=Join-Path $root "agy-windows-launcher.rs"',
      helpers,
      'try {',
      '  Assert-DirectOwnedChild $parent $root',
      '  Assert-DirectOwnedChild $fixtureRoot $source',
      '  Assert-NoReparseAncestors $parent',
      '  Assert-RealDirectory $parent',
      '  Assert-NoReparseAncestors $fixtureRoot',
      '  Assert-RealDirectory $fixtureRoot',
      '  Assert-NoReparseAncestors $source',
      '  Assert-RealFile $source',
      '  $existing=Get-Item -LiteralPath $root -Force -ErrorAction SilentlyContinue',
      '  if($null -ne $existing){throw "native compile root already exists"}',
      '  Assert-NoReparseAncestors $root',
      '  [System.IO.Directory]::CreateDirectory($root)|Out-Null',
      '  Assert-NoReparseAncestors $root',
      '  Assert-RealDirectory $root',
      '  if($null -ne (Get-Item -LiteralPath $localSource -Force -ErrorAction SilentlyContinue)){throw "native compile source destination already exists"}',
      '  Assert-NoReparseAncestors $localSource',
      '  Copy-Item -LiteralPath $source -Destination $localSource',
      '  Assert-NoReparseAncestors $localSource',
      '  Assert-RealFile $localSource',
      '  Assert-RealFile $source',
      '  $sourceHash=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash',
      '  $copyHash=(Get-FileHash -LiteralPath $localSource -Algorithm SHA256).Hash',
      '  if(-not [string]::Equals($sourceHash,$copyHash,[System.StringComparison]::OrdinalIgnoreCase)){throw "native compile source copy hash mismatch"}',
      '} catch {',
      '  $primary=$_.Exception',
      '  if($null -ne (Get-Item -LiteralPath $root -Force -ErrorAction SilentlyContinue)){',
      '    try { Remove-ExactOwnedDirectory $parent $root } catch { throw "native compile setup failed: $($primary.Message); cleanup failed: $($_.Exception.Message)" }',
      '  }',
      '  throw $primary',
      '}',
    ].join(';')
    captureCommand('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', setupScript], fixtureDir, commandEnvironment({
      SHELLX_CUT_NATIVE_COMPILE_PARENT: compile.parent,
      SHELLX_CUT_NATIVE_COMPILE_ROOT: compile.target,
      SHELLX_CUT_NATIVE_FIXTURE_ROOT: fixtureWin,
      SHELLX_CUT_NATIVE_COMPILE_SOURCE: sourceWin,
    }))
    created = true
    captureCommand(rustc, [
      '--edition=2021', '--crate-name=shellx_cut_agy_fixture', '-C', 'opt-level=1',
      '-o', localOutputWin, localSourceWin,
    ], fixtureDir)
    const copyScript = [
      '$ErrorActionPreference="Stop"',
      '$compileRoot=$env:SHELLX_CUT_NATIVE_COMPILE_ROOT',
      '$artifact=$env:SHELLX_CUT_NATIVE_COMPILE_ARTIFACT',
      '$fixtureRoot=$env:SHELLX_CUT_NATIVE_FIXTURE_ROOT',
      '$destination=$env:SHELLX_CUT_NATIVE_COMPILE_DESTINATION',
      helpers,
      'Assert-DirectOwnedChild $compileRoot $artifact',
      'Assert-DirectOwnedChild $fixtureRoot $destination',
      'Assert-NoReparseAncestors $compileRoot',
      'Assert-RealDirectory $compileRoot',
      'Assert-NoReparseAncestors $artifact',
      'Assert-RealFile $artifact',
      'Assert-NoReparseAncestors $fixtureRoot',
      'Assert-RealDirectory $fixtureRoot',
      'Assert-NoReparseAncestors $destination',
      'if($null -ne (Get-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue)){throw "native fixture destination already exists"}',
      'Copy-Item -LiteralPath $artifact -Destination $destination',
      'Assert-NoReparseAncestors $destination',
      'Assert-RealFile $destination',
      'Assert-RealFile $artifact',
      '$artifactHash=(Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash',
      '$destinationHash=(Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash',
      'if(-not [string]::Equals($artifactHash,$destinationHash,[System.StringComparison]::OrdinalIgnoreCase)){throw "native fixture artifact copy hash mismatch"}',
    ].join(';')
    captureCommand('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', copyScript], fixtureDir, commandEnvironment({
      SHELLX_CUT_NATIVE_COMPILE_ROOT: compile.target,
      SHELLX_CUT_NATIVE_COMPILE_ARTIFACT: localOutputWin,
      SHELLX_CUT_NATIVE_FIXTURE_ROOT: fixtureWin,
      SHELLX_CUT_NATIVE_COMPILE_DESTINATION: destinationWin,
    }))
  } catch (error) {
    compileError = error
  } finally {
    if (created) {
      try {
        const cleanupScript = [
          '$ErrorActionPreference="Stop"',
          '$parent=$env:SHELLX_CUT_NATIVE_COMPILE_PARENT',
          '$root=$env:SHELLX_CUT_NATIVE_COMPILE_ROOT',
          helpers,
          'Remove-ExactOwnedDirectory $parent $root',
        ].join(';')
        captureCommand('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', cleanupScript], fixtureDir, commandEnvironment({
          SHELLX_CUT_NATIVE_COMPILE_PARENT: compile.parent,
          SHELLX_CUT_NATIVE_COMPILE_ROOT: compile.target,
        }))
      } catch (error) {
        cleanupError = error
      }
    }
  }
  if (compileError && cleanupError) throw new AggregateError([compileError, cleanupError], 'native fixture compile and cleanup failed')
  if (compileError) throw compileError
  if (cleanupError) throw cleanupError
  if (!existsSync(output)) throw new Error('native Antigravity fixture compilation did not produce agy.exe')
  const outputMetadata = lstatSync(output)
  if (!outputMetadata.isFile() || outputMetadata.isSymbolicLink()) {
    throw new Error('native Antigravity fixture output is not a regular non-symlink file')
  }
}
