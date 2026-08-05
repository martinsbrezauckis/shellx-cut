#!/usr/bin/env node
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import {
  buildUiSourceActionManifest,
  serializeUiSourceActionManifest,
  UI_ACTION_MANIFEST_PATH,
} from '../update-ui-action-manifest.mjs'

const expected = serializeUiSourceActionManifest(buildUiSourceActionManifest())
const committed = readFileSync(UI_ACTION_MANIFEST_PATH, 'utf8')
assert.equal(committed, expected, 'committed UI source-action manifest must match the strict source/native inventory')
const parsed = JSON.parse(committed)
assert.equal(parsed.actionCount, parsed.actions.length)
assert.equal(new Set(parsed.actions).size, parsed.actions.length)

console.log(`PASS UI source-action manifest (${parsed.actionCount} actions)`)
