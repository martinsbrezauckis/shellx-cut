import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { strict as assert } from 'node:assert'

const root = resolve(import.meta.dirname, '..')
const preference = readFileSync(resolve(root, 'src/lib/themePref.ts'), 'utf8')
const toggle = readFileSync(resolve(root, 'src/components/ThemeToggle.tsx'), 'utf8')

assert.match(preference, /THEME_CHANGE_EVENT/)
assert.match(preference, /dispatchEvent\(new CustomEvent<ThemeName>/)
assert.match(toggle, /addEventListener\(THEME_CHANGE_EVENT, sync\)/)
assert.match(toggle, /removeEventListener\(THEME_CHANGE_EVENT, sync\)/)

console.log('PASS theme toggle cross-instance synchronization contract')
