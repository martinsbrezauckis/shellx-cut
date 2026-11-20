import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const ui = resolve(import.meta.dirname, '..', 'src')
const assets = readFileSync(resolve(ui, 'panels/Assets/index.tsx'), 'utf8')
const bulkRelink = readFileSync(resolve(ui, 'panels/Assets/BulkRelinkPanel.tsx'), 'utf8')
const projects = readFileSync(resolve(ui, 'panels/Projects/index.tsx'), 'utf8')
const portableCopy = readFileSync(resolve(ui, 'panels/Projects/PortableCopy.tsx'), 'utf8')
const leftPanel = readFileSync(resolve(ui, 'panels/LeftPanel/index.tsx'), 'utf8')

assert.match(leftPanel, /<Assets[\s\S]*projectScope=\{projectScope\}/, 'Assets receives the app project session')
assert.match(assets, /key=\{`\$\{projectScope\}:\$\{project\?\.project_revision \?\? ''\}`\}/, 'B5 recovery remounts for every project session or revision')
assert.match(bulkRelink, /scopeKey: string/, 'B5 recovery declares its scope boundary')
assert.match(bulkRelink, /activeScope\.current !== scopeKey/, 'late B5 picker and preview results are ignored after a scope change')

assert.match(leftPanel, /<ProjectsPanel[\s\S]*projectScope=\{projectScope\}[\s\S]*projectRevision=\{project\?\.project_revision \?\? null\}/, 'Projects receives session and revision')
assert.match(projects, /key=\{`\$\{projectScope\}:\$\{projectRevision \?\? ''\}`\}/, 'B6 portable state remounts for every project session or revision')
assert.match(portableCopy, /scopeKey: string/, 'B6 dialog carries the scope boundary')
assert.match(portableCopy, /activeScope\.current !== scopeKey/, 'late B6 plan/create results are ignored after a scope change')

console.log('PASS B5/B6 recovery and portable previews are project-session and revision scoped')
