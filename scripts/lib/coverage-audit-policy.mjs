#!/usr/bin/env node
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const MUTATION_CLASSES = new Set([
  'Read',
  'ProjectMetadata',
  'AssetMetadata',
  'Timeline',
  'Navigation',
  'ExternalSideEffect',
])

function fail(message) {
  throw new Error(`coverage audit policy: ${message}`)
}

export function parseVerbContracts(source) {
  const contracts = new Map()
  for (const match of source.matchAll(/^\s*"([^"]+)" => Some\(VerbContract \{ (.*) \}\),$/gm)) {
    const [, name, contract] = match
    if (contracts.has(name)) fail(`duplicate verb contract for ${name}`)
    const mutation = /mutation_class: MutationClass::([A-Za-z]+)/.exec(contract)?.[1]
    if (!mutation || !MUTATION_CLASSES.has(mutation)) fail(`unsupported mutation class for ${name}`)
    contracts.set(name, mutation)
  }
  return contracts
}

export function coverageAuditPlan(schema, contractSource) {
  if (!Array.isArray(schema?.verbs) || schema.verbs.length === 0) fail('schema has no verbs')
  const contracts = parseVerbContracts(contractSource)
  const names = new Set()
  const plan = []
  for (const verb of schema.verbs) {
    const name = verb?.name
    const args = verb?.args
    if (typeof name !== 'string' || !name || names.has(name)) fail('schema has a missing or duplicate verb name')
    names.add(name)
    const mutation = contracts.get(name)
    if (!mutation) fail(`missing verb contract for ${name}`)
    if (!args || args.type !== 'object') fail(`${name} must retain an object args schema`)
    const required = args.required ?? []
    if (!Array.isArray(required) || required.some((key) => typeof key !== 'string' || !key) || new Set(required).size !== required.length) {
      fail(`${name} has malformed required args`)
    }
    plan.push({
      name,
      mutation,
      // Every schema verb accepts an object. A non-object is the only shared
      // structural payload that cannot reach any zero-argument handler,
      // including Read-class routes which may still probe a process or device.
      payload: required.length === 0 ? '[]' : '{}',
    })
  }
  for (const name of contracts.keys()) {
    if (!names.has(name)) fail(`verb contract ${name} is absent from the schema`)
  }
  return plan
}

function cliArgs(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    if (!['--schema', '--contracts'].includes(flag) || values[flag]) fail(`unsupported or duplicate argument ${flag}`)
    const value = argv[index + 1]
    if (!value || value.startsWith('--')) fail(`missing value for ${flag}`)
    values[flag] = value
    index += 1
  }
  if (!values['--schema'] || !values['--contracts']) fail('--schema and --contracts are required')
  return values
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args = cliArgs(process.argv.slice(2))
    const schema = JSON.parse(readFileSync(resolve(args['--schema']), 'utf8'))
    const contracts = readFileSync(resolve(args['--contracts']), 'utf8')
    process.stdout.write(`${coverageAuditPlan(schema, contracts).map(({ name, payload }) => `${name}\t${payload}`).join('\n')}\n`)
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
}
