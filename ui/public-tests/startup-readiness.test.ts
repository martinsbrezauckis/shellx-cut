import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { createCutAppRootMountReporter } from '../src/app/startupReadiness'
import { LAYOUT_DEFAULTS } from '../src/layout/useLayout'

let notices = 0
const reportMounted = createCutAppRootMountReporter(() => { notices += 1 })
const nonRoot = { hasAttribute: () => false } as unknown as HTMLDivElement
const cutRoot = {
  hasAttribute: (name: string) => name === 'data-cut-app-root',
} as unknown as HTMLDivElement

reportMounted(null)
reportMounted(nonRoot)
assert.equal(notices, 0, 'no DOM root must not emit the UI-mount handoff')
reportMounted(cutRoot)
reportMounted(cutRoot)
assert.equal(notices, 1, 'React remount/ref repeats must emit one UI-mount handoff')

const appSource = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')
const mainSource = readFileSync(new URL('../src/main.tsx', import.meta.url), 'utf8')
const reviewSource = readFileSync(new URL('../src/panels/Review/index.tsx', import.meta.url), 'utf8')
assert.match(
  appSource,
  /ref=\{embeddedManualFrontend \? undefined : appRootMountReporter\.current\}/,
  'App must wire the reporter only to the non-embedded desktop root',
)
assert.match(appSource, /data-cut-app-root/, 'the reported element must be Cut\'s real app-root selector')
assert.match(mainSource, /if \(isMockRuntime\(\)\) void loadMockRuntime\(\)\.then\(mount\)/, 'mock startup must install its adapter before mount')
assert.doesNotMatch(reviewSource, /import ['"]\.\/mock['"]/, 'opening Review must not own or delay mock API installation')

const rightRailSource = readFileSync(new URL('../src/app/AppRightRail.tsx', import.meta.url), 'utf8')
assert.equal(LAYOUT_DEFAULTS.railCollapsed, true, 'a fresh Cut layout keeps the Review rail collapsed')
assert.equal(LAYOUT_DEFAULTS.railPinned, false, 'a fresh Cut layout does not mount the Review rail')
assert.match(
  rightRailSource,
  /const Review = lazy\(\(\) => import\('\.\.\/panels\/Review'\)\)/,
  'the default-collapsed Review rail must stay outside the entry import graph',
)
assert.doesNotMatch(
  rightRailSource,
  /^import Review from ['"]\.\.\/panels\/Review['"]$/m,
  'the inactive Review rail must not be a static startup import',
)
assert.match(
  rightRailSource,
  /\{railPinned && \(\s*<Suspense fallback=\{<SurfaceLoading label="Loading review" \/>\}>\s*<Review/s,
  'Review must load only with the user-pinned rail and retain a bounded loading surface',
)
