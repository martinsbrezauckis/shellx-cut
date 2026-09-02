import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const root = resolve(new URL('../..', import.meta.url).pathname)
const owners = [
  'app/server/src/jobs/process/tree.rs',
  'app/media/src/ffmpeg/process/tree.rs',
  'app/recorder/record-render/src/ffmpeg/process/process_tree.rs',
]
const shellOwners = [
  'scripts/live-test-exports.sh',
  'scripts/coverage-audit.sh',
  'scripts/e2e.sh',
]

for (const relative of owners) {
  const source = readFileSync(resolve(root, relative), 'utf8')
  const validation = source.indexOf('validate_process_group(pgid)?;')
  const signal = source.indexOf('libc::kill(-pgid, signal)')
  assert.ok(signal >= 0, `${relative} owns a Unix process-group signal`)
  assert.ok(
    validation >= 0 && validation < signal,
    `${relative} rejects unsafe process groups before negating the id`,
  )
  assert.match(source, /if pgid <= 1 \{/, `${relative} rejects pgid <= 1`)
  assert.doesNotMatch(source, /pgid\s*:\s*1\b/, `${relative} never uses process group 1 as a fixture`)
}

const shellGuard = readFileSync(resolve(root, 'scripts/lib/safe-process-group.sh'), 'utf8')
assert.match(shellGuard, /\(\( owned_pid <= 1 \)\)/, 'shell process-group cleanup rejects pid <= 1')
for (const relative of shellOwners) {
  const source = readFileSync(resolve(root, relative), 'utf8')
  assert.match(source, /safe_stop_owned_process_group/, `${relative} uses the shared safe group guard`)
  assert.doesNotMatch(source, /kill\s+--\s+-"\$/, `${relative} has no unguarded negative group kill`)
}

const serverOwner = readFileSync(resolve(root, owners[0]), 'utf8')
assert.match(
  serverOwner,
  /pub\(super\) fn establish\(child:\s*&Child\)[\s\S]*?validate_process_group\(pgid\)\?;[\s\S]*?Ok\(Self \{[\s\S]*?armed:\s*true,/,
  'the server process-tree owner establishes itself armed after validating its process group',
)
assert.match(
  serverOwner,
  /failing_for_test\(child:\s*&Child\)[\s\S]*?validate_process_group\(pgid\)\?;[\s\S]*?armed:\s*true,[\s\S]*?force_stop_failure:\s*true/,
  'the forced-failure fixture derives, validates, and arms the actual child process group',
)
assert.match(
  serverOwner,
  /pub\(super\) fn hard_stop\(&mut self\) -> io::Result<\(\)> \{[\s\S]*?signal_group\(self\.pgid, libc::SIGKILL\)\?;[\s\S]*?self\.armed = false;[\s\S]*?Ok\(\(\)\)/,
  'a successful or ESRCH hard-stop disarms the server process-tree owner before return',
)
const softStop = serverOwner.slice(
  serverOwner.indexOf('pub(super) fn soft_stop'),
  serverOwner.indexOf('pub(super) fn hard_stop'),
)
assert.doesNotMatch(softStop, /armed\s*=\s*false/, 'a soft-stop keeps the drop fallback armed')
assert.match(
  serverOwner,
  /impl Drop for ProcessTree \{[\s\S]*?if self\.armed \{[\s\S]*?signal_group\(self\.pgid, libc::SIGKILL\)/,
  'the server process-tree Drop fallback signals only while armed',
)

console.log(`PASS process-tree-safety (${owners.length} Rust and ${shellOwners.length} shell owners fail closed)`)
