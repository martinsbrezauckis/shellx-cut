#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'
import { coverageAuditPlan } from '../lib/coverage-audit-policy.mjs'

// Git for Windows ships bash only through PATH; POSIX hosts keep the fixed absolute path.
const BASH = process.platform === 'win32' ? 'bash' : '/bin/bash'
const root = resolve(import.meta.dirname, '../..')
const source = readFileSync(resolve(root, 'scripts/coverage-audit.sh'), 'utf8')
const auditScript = resolve(root, 'scripts/coverage-audit.sh')
const policyScript = resolve(root, 'scripts/lib/coverage-audit-policy.mjs')
const requestHelper = resolve(root, 'scripts/lib/coverage-audit-request.sh')
const runtimeHelper = resolve(root, 'scripts/lib/coverage-audit-runtime.sh')
const schemaPath = resolve(root, 'schema/verbs.json')
const contractsPath = resolve(root, 'app/core/src/verb_contract.rs')
const schema = JSON.parse(readFileSync(schemaPath, 'utf8'))
const contracts = readFileSync(contractsPath, 'utf8')

test('Cargo artifact selection and MCP probe use the same freshly reported executable', () => {
  const directory = mkdtempSync(join(tmpdir(), 'coverage-audit-cargo-artifact-'))
  try {
    const executable = join(directory, 'different-target-cutd')
    const messages = join(directory, 'cargo.jsonl')
    writeFileSync(executable, '#!/usr/bin/env node\nprocess.stdin.setEncoding("utf8");let data="";process.stdin.on("data",chunk=>{data+=chunk;if(data.split("\\n").length>=3)console.log(JSON.stringify({jsonrpc:"2.0",id:2,result:{tools:[{name:"fresh_binary"}]}}))})\n')
    chmodSync(executable, 0o755)
    const artifact = { reason: 'compiler-artifact', target: { name: 'cutd', kind: ['bin'] }, executable }
    writeFileSync(messages, `${JSON.stringify({ reason: 'compiler-message' })}\n${JSON.stringify(artifact)}\n`)
    const choose = () => spawnSync(BASH, ['-c', 'source "$1"; coverage_audit_cargo_executable "$2"', 'artifact-test', runtimeHelper, messages], { encoding: 'utf8' })
    const selected = choose()
    assert.equal(selected.status, 0, selected.stderr)
    assert.equal(selected.stdout.trim(), executable)
    // The fixture cutd is a shebang script; Windows cannot execute one directly (real cutd is cutd.exe).
    if (process.platform !== 'win32') {
      const probe = spawnSync(process.execPath, [resolve(root, 'scripts/mcp-probe.mjs'), '--cutd', selected.stdout.trim()], { encoding: 'utf8', timeout: 5000 })
      assert.equal(probe.status, 0, probe.stderr)
      assert.deepEqual(JSON.parse(probe.stdout).tools, ['fresh_binary'])

      const budgetPass = spawnSync(process.execPath, [resolve(root, 'scripts/mcp-probe.mjs'), '--cutd', selected.stdout.trim(), '--budget'], { encoding: 'utf8', timeout: 5000 })
      assert.equal(budgetPass.status, 0, budgetPass.stderr)
      const parsed = JSON.parse(budgetPass.stdout)
      assert.equal(parsed.ok, true)
      assert.equal(parsed.maxPercent, 80)
      assert.match(budgetPass.stderr, /tools\/list is \d+\/393216 bytes \(\d+\.\d{2}%/)

      const overBudgetExe = join(directory, 'over-budget-cutd')
      writeFileSync(overBudgetExe, `#!/usr/bin/env node\nprocess.stdin.setEncoding("utf8");let data="";process.stdin.on("data",chunk=>{data+=chunk;if(data.split("\\n").length>=3)console.log(JSON.stringify({jsonrpc:"2.0",id:2,result:{tools:[{name:"big",description:"${'x'.repeat(320_000)}"}]}}))})\n`)
      chmodSync(overBudgetExe, 0o755)
      const budgetFail = spawnSync(process.execPath, [resolve(root, 'scripts/mcp-probe.mjs'), '--cutd', overBudgetExe, '--budget'], { encoding: 'utf8', timeout: 5000 })
      assert.notEqual(budgetFail.status, 0)
      assert.equal(JSON.parse(budgetFail.stdout).ok, false)
    }
    writeFileSync(messages, `${JSON.stringify(artifact)}\n${JSON.stringify(artifact)}\n`)
    assert.notEqual(choose().status, 0, 'ambiguous Cargo artifacts fail closed')
    writeFileSync(messages, `${JSON.stringify({ reason: 'build-finished' })}\n`)
    assert.notEqual(choose().status, 0, 'missing Cargo artifact fails closed')
    assert.match(source, /setsid "\$CUTD_BIN" serve/)
    assert.match(source, /mcp-probe\.mjs" --cutd "\$CUTD_BIN"/)
  } finally { rmSync(directory, { recursive: true, force: true }) }
})

assert.match(source, /AUDIT_REST_TIMEOUT_SECONDS:-30/, 'ordinary structural routes keep a bounded 30-second default')
assert.match(source, /coverage_audit_request "\$ADDR" "\$v" "\$payload" "\$timeout_seconds"/,
  'the selected bounded timeout and policy payload flow through the real request helper')
assert.doesNotMatch(source, /declare -A|\[\[.*-v /, 'coverage audit remains usable by macOS system Bash 3')
assert.match(source, /accepted\)[\s\S]*handler accepted an intentionally invalid structural payload[\s\S]*rest_failed/,
  'an accepted response must fail the audit instead of counting as route coverage')
const runtimeSource = readFileSync(runtimeHelper, 'utf8')
assert.match(runtimeSource, /node --input-type=module -/, 'the stdin holder scanner is explicitly parsed as ESM')
assert.match(runtimeSource, /process\.exit\(10\)/, 'only the completed no-holder scanner result may permit cleanup')

function runPolicyCli() {
  const result = spawnSync(process.execPath, [policyScript, '--schema', schemaPath, '--contracts', contractsPath], { encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr)
  const plan = new Map()
  for (const line of result.stdout.trim().split('\n')) {
    const [name, payload, extra] = line.split('\t')
    assert.ok(name && (payload === '{}' || payload === '[]') && extra === undefined, `invalid policy row ${JSON.stringify(line)}`)
    assert.ok(!plan.has(name), `duplicate policy row for ${name}`)
    plan.set(name, payload)
  }
  return plan
}

function mockDispatch(argsSchema, payload) {
  const value = JSON.parse(payload)
  if (argsSchema.type === 'object' && (value === null || Array.isArray(value) || typeof value !== 'object')) {
    return { ok: false, error: { code: 'invalid_args' } }
  }
  if ((argsSchema.required || []).some((key) => !(key in value))) {
    return { ok: false, error: { code: 'invalid_args' } }
  }
  if (argsSchema.oneOf?.length && !argsSchema.oneOf.some((branch) =>
    (branch.required || []).every((key) => key in value))) {
    return { ok: false, error: { code: 'invalid_args' } }
  }
  return { ok: true, handlerWouldRun: true }
}

test('prelaunch policy covers every schema verb and rejects each structural request before dispatch', () => {
  const directPlan = coverageAuditPlan(schema, contracts)
  const cliPlan = runPolicyCli()
  assert.equal(cliPlan.size, schema.verbs.length, 'the policy must produce exactly one entry per schema verb')

  const protectedVerbs = []
  for (const verb of schema.verbs) {
    const entry = directPlan.find((candidate) => candidate.name === verb.name)
    assert.ok(entry, `${verb.name} must have a direct policy entry`)
    assert.equal(cliPlan.get(verb.name), entry.payload, `${verb.name} CLI policy must equal the parsed schema/contract policy`)
    assert.deepEqual(mockDispatch(verb.args, entry.payload), { ok: false, error: { code: 'invalid_args' } },
      `${verb.name} must be rejected before dispatch`)
    if ((verb.args.required || []).length === 0) protectedVerbs.push(verb.name)
  }
  assert.ok(protectedVerbs.length > 0, 'the current zero-required object routes must receive invalid non-objects without a hand-maintained allowlist')
  for (const name of ['project.set_output_dir', 'system.set_stt_model', 'system.set_ffmpeg', 'system.setup_perception', 'system.setup_matte']) {
    const verb = schema.verbs.find((entry) => entry.name === name)
    assert.ok(verb, `${name} must remain registered`)
    assert.equal(cliPlan.get(name), '[]', `${name} must not receive a valid default object`)
    assert.deepEqual(mockDispatch(verb.args, '{}'), { ok: true, handlerWouldRun: true }, `${name} would have dispatched with the old default`)
  }
  for (const name of ['system.doctor', 'system.mcp_test', 'screen_record.system_audio_probe']) {
    assert.equal(cliPlan.get(name), '[]', `${name} must not invoke its zero-argument handler during structural coverage`)
  }
})

test('policy fails closed for a changed object schema or an unclassified verb before launch', () => {
  const changedShape = structuredClone(schema)
  changedShape.verbs.find((verb) => verb.name === 'system.setup_perception').args.type = 'array'
  assert.throws(() => coverageAuditPlan(changedShape, contracts), /system\.setup_perception must retain an object args schema/)

  const unclassified = structuredClone(schema)
  unclassified.verbs.push({ name: 'test.unclassified_mutator', args: { type: 'object' } })
  assert.throws(() => coverageAuditPlan(unclassified, contracts), /missing verb contract for test\.unclassified_mutator/)
})

test('real request helper transmits the policy invalid payload to the server boundary', () => {
  const directory = mkdtempSync(join(tmpdir(), 'coverage-audit-request-'))
  const capture = join(directory, 'curl-argv.nul')
  try {
    const payload = runPolicyCli().get('system.setup_perception')
    const result = spawnSync(BASH, [
      '-c',
      'set -euo pipefail; source "$1"; curl() { printf "%s\\0" "$@" > "$CAPTURE"; printf \'{"ok":false,"error":{"code":"invalid_args"}}\\n200\\n\'; }; coverage_audit_request 127.0.0.1:6167 system.setup_perception "$PAYLOAD" 30 >/dev/null',
      'coverage-audit-request-test',
      requestHelper,
    ], {
      env: { ...process.env, CAPTURE: capture, PAYLOAD: payload },
      encoding: 'utf8',
    })
    assert.equal(result.status, 0, result.stderr)
    const argv = readFileSync(capture).toString('utf8').split('\0').filter(Boolean)
    assert.deepEqual(argv, [
      '-sS', '--max-time', '30', '-H', 'content-type: application/json', '-X', 'POST',
      'http://127.0.0.1:6167/api/verb/system.setup_perception', '-d', '[]', '-w', '\n%{http_code}',
    ])
    const validation = spawnSync(BASH, ['-c', 'source "$1"; coverage_audit_response_verdict "$2"', 'coverage-audit-response-test', requestHelper, '{"ok":false,"error":{"code":"invalid_args"}}'], { encoding: 'utf8' })
    assert.equal(validation.status, 0, validation.stderr)
    assert.equal(validation.stdout, 'validation\n')
    const accepted = spawnSync(BASH, ['-c', 'source "$1"; coverage_audit_response_verdict "$2"', 'coverage-audit-response-test', requestHelper, '{"ok":true}'], { encoding: 'utf8' })
    assert.equal(accepted.status, 0, accepted.stderr)
    assert.equal(accepted.stdout, 'accepted\n', 'an unexpected handler acceptance must be a visible audit failure')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('coverage audit refuses existing servers and validates the plan before cargo can run', () => {
  assert.match(source, /CUTD_ADDR is unsupported: coverage audit always owns an isolated cutd/)
  assert.match(source, /SHELLX_CUT_HOME="\$AUDIT_RUNTIME_ROOT\/state"/)
  assert.match(source, /SHELLX_CUT_PROJECTS_DIR="\$AUDIT_RUNTIME_ROOT\/projects"/)
  assert.match(source, /XDG_DATA_HOME="\$AUDIT_RUNTIME_ROOT\/data"/)
  assert.match(source, /XDG_CACHE_HOME="\$AUDIT_RUNTIME_ROOT\/cache"/)
  assert.match(source, /TMPDIR="\$AUDIT_RUNTIME_ROOT\/tmp"/)
  assert.match(source, /HF_HOME="\$AUDIT_RUNTIME_ROOT\/model-cache"/)
  assert.match(source, /unset SHELLX_CUT_PYTHON[\s\S]*MATTE_RUNNER_SCRIPT/)
  assert.match(source, /coverage_audit_cleanup_runtime "\$ROOT"/)
  assert.doesNotMatch(source, /export HOME=/, 'coverage audit must not repoint a provider-owned home')

  const existingServer = spawnSync(BASH, [auditScript], {
    cwd: root,
    env: { ...process.env, CUTD_ADDR: '127.0.0.1:6161' },
    encoding: 'utf8',
  })
  assert.notEqual(existingServer.status, 0)
  assert.match(existingServer.stderr, /CUTD_ADDR is unsupported/)

  const directory = mkdtempSync(join(tmpdir(), 'coverage-audit-prelaunch-'))
  try {
    const bin = join(directory, 'bin')
    const cargoMarker = join(directory, 'cargo-invoked')
    const nodeStub = join(bin, 'node')
    const cargoStub = join(bin, 'cargo')
    requireMkdir(bin)
    writeFileSync(nodeStub, '#!/usr/bin/env bash\nprintf \'system.doctor\\t{}\\n\'\n')
    writeFileSync(cargoStub, `#!/usr/bin/env bash\nprintf invoked > ${shellSingleQuote(cargoMarker)}\nexit 99\n`)
    chmodSync(nodeStub, 0o755)
    chmodSync(cargoStub, 0o755)
    const incompletePlan = spawnSync(BASH, [auditScript], {
      cwd: root,
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
      encoding: 'utf8',
    })
    assert.notEqual(incompletePlan.status, 0)
    assert.match(incompletePlan.stderr, /coverage payload policy entry 0 must be project\.create/)
    assert.throws(() => readFileSync(cargoMarker), { code: 'ENOENT' }, 'cargo must remain uninvoked when policy is incomplete')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('cleanup retains evidence when an owned group fails to stop or a process still holds the runtime', () => {
  const directory = mkdtempSync(join(tmpdir(), 'coverage-audit-cleanup-'))
  const projectRoot = join(directory, 'project')
  const scratch = join(projectRoot, '.scratch')
  const failureRoot = join(scratch, 'coverage-audit-state.stop-failed')
  const holderRoot = join(scratch, 'coverage-audit-state.holder-live')
  const holderChild = join(directory, 'holder-child.pid')
  try {
    requireMkdir(failureRoot)
    requireMkdir(holderRoot)
    const script = [
      'set -euo pipefail',
      'source "$1"',
      'safe_stop_owned_process_group() { return 1; }',
      '/bin/sh -c "sleep 0.05" & failed_pid=$!',
      'AUDIT_RUNTIME_ROOT="$2" CUTD_PID="$failed_pid" coverage_audit_cleanup_runtime "$4"',
      'test -d "$2"',
      '/bin/sh -c "cd \"$3\" && exec sleep 20" >/dev/null 2>&1 & holder_pid=$!',
      'AUDIT_RUNTIME_ROOT="$3" CUTD_PID="" coverage_audit_cleanup_runtime "$4"',
      'test -d "$3"',
      'printf "%s\\n" "$holder_pid" > "$5"',
    ].join('\n')
    const result = spawnSync(BASH, [
      '-c', script, 'coverage-audit-cleanup-test', runtimeHelper, failureRoot, holderRoot, projectRoot, holderChild,
    ], { encoding: 'utf8' })
    assert.equal(result.status, 0, result.stderr)
    const holderPid = Number.parseInt(readFileSync(holderChild, 'utf8'), 10)
    assert.ok(Number.isInteger(holderPid) && holderPid > 0)
    terminateProcess(holderPid, 'SIGTERM')
  } finally {
    try {
      const holderPid = Number.parseInt(readFileSync(holderChild, 'utf8'), 10)
      if (Number.isInteger(holderPid) && holderPid > 0) terminateProcess(holderPid, 'SIGKILL')
    } catch (error) {
      if (error?.code !== 'ENOENT' && error?.code !== 'ESRCH') throw error
    }
    rmSync(directory, { recursive: true, force: true })
  }
})

test('holder scanners reserve no-holder status and retain evidence on lsof or Node uncertainty', () => {
  const directory = mkdtempSync(join(tmpdir(), 'coverage-audit-scanner-'))
  const projectRoot = join(directory, 'project')
  const runtimeRoot = join(projectRoot, '.scratch', 'coverage-audit-state.scanner')
  const bin = join(directory, 'bin')
  const lsofStub = join(bin, 'lsof')
  const nodeStub = join(bin, 'node')
  try {
    requireMkdir(runtimeRoot)
    requireMkdir(bin)
    writeExecutable(lsofStub, '#!/usr/bin/env bash\nexit 1\n')
    let result = spawnSync(BASH, [
      '-c', 'source "$1"; PATH="$2:$PATH"; coverage_audit_lsof_has_holders "$3"',
      'coverage-audit-lsof-clean', runtimeHelper, bin, runtimeRoot,
    ], { encoding: 'utf8' })
    assert.equal(result.status, 1, result.stderr)

    writeExecutable(lsofStub, '#!/usr/bin/env bash\nprintf unavailable >&2\nexit 1\n')
    result = spawnSync(BASH, [
      '-c', 'source "$1"; PATH="$2:$PATH"; coverage_audit_lsof_has_holders "$3"',
      'coverage-audit-lsof-uncertain', runtimeHelper, bin, runtimeRoot,
    ], { encoding: 'utf8' })
    assert.equal(result.status, 0, result.stderr)

    if (existsSync('/proc')) {
      writeExecutable(nodeStub, '#!/usr/bin/env bash\nexit 10\n')
      result = spawnSync(BASH, [
        '-c', 'source "$1"; PATH="$2:$PATH"; coverage_audit_runtime_has_holders "$3"',
        'coverage-audit-node-clear', runtimeHelper, bin, runtimeRoot,
      ], { encoding: 'utf8' })
      assert.equal(result.status, 1, result.stderr)

      writeExecutable(nodeStub, '#!/usr/bin/env bash\nexit 1\n')
      result = spawnSync(BASH, [
        '-c', 'source "$1"; PATH="$2:$PATH"; AUDIT_RUNTIME_ROOT="$3" CUTD_PID="" coverage_audit_cleanup_runtime "$4"; test -d "$3"',
        'coverage-audit-node-startup-failure', runtimeHelper, bin, runtimeRoot, projectRoot,
      ], { encoding: 'utf8' })
      assert.equal(result.status, 0, result.stderr)
    }
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

function requireMkdir(path) {
  mkdirSync(path, { recursive: true })
}

function shellSingleQuote(value) {
  return `'${value.replaceAll("'", "'\\\"'\\\"'")}'`
}

function writeExecutable(path, source) {
  writeFileSync(path, source)
  chmodSync(path, 0o755)
}

function terminateProcess(pid, signal) {
  try {
    process.kill(pid, signal)
  } catch (error) {
    if (error?.code !== 'ESRCH') throw error
  }
}

console.log('coverage audit timeout contract tests passed')
