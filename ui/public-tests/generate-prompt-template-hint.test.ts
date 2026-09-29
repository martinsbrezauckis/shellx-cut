import assert from 'node:assert/strict'
import { promptTemplateHint } from '../src/panels/GenerateTemplates/model'

assert.equal(promptTemplateHint('builtin.lower-third.clean', null), null, 'automatic catalog default must leave prompt template choice open')
assert.equal(promptTemplateHint('builtin.title-card.episode', 'builtin.title-card.episode'), 'builtin.title-card.episode', 'explicit catalog choice constrains prompt')
assert.equal(promptTemplateHint('builtin.lower-third.clean', 'builtin.title-card.episode'), null, 'a changed catalog selection must not carry an old constraint')
assert.equal(promptTemplateHint(null, 'builtin.title-card.episode'), null, 'no selected template leaves prompt choice open')

console.log('Generate prompt template hint checks passed')
