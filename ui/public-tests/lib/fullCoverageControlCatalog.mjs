// Declarative control plane for the native full-coverage matrix.  This module
// owns section ordering and profile membership; full-coverage-verify.mjs owns
// the section implementations themselves.

export const FULL_COVERAGE_CONTROL_CATALOG_SCHEMA = 'shellx-cut/full-coverage-control-catalog@1'
export const FULL_COVERAGE_RESULT_SCHEMA = 'shellx-cut/full-coverage-results@1'
export const FULL_COVERAGE_SECTION_CHECKPOINT_SCHEMA = 'shellx-cut/full-coverage-section-checkpoint@1'
export const UI_SOURCE_ACTION_MANIFEST_SCHEMA = 'shellx-cut/ui-source-action-manifest@1'

const EXPECTED_COUNTS = Object.freeze({
  modules: 63,
  profiles: 2,
  ordinaryModules: 37,
  heavyModules: 26,
  sourceActionIds: 729,
})

export const FULL_COVERAGE_ACTION_REGISTRY = Object.freeze({
  id: 'ui-source-actions',
  schema: UI_SOURCE_ACTION_MANIFEST_SCHEMA,
  path: 'ui/public-tests/full-ui-action-manifest.json',
  expectedCount: EXPECTED_COUNTS.sourceActionIds,
  scope: 'catalog-wide',
})

export const FULL_COVERAGE_RUNNER_ID = 'full-coverage-verify'

const HOST_POLICY = Object.freeze({
  hosts: Object.freeze([
    Object.freeze({
      surface: 'windows-installed',
      runnerId: 'windows-installed-full-coverage',
      mode: 'installed',
      profileSelection: true,
      resume: 'signed-final identity-matched sections or profile',
    }),
    Object.freeze({
      surface: 'linux-control',
      runnerId: 'linux-wdio-full-coverage',
      mode: 'candidate-or-installed-package',
      profileSelection: false,
      resume: 'explicit FCV_SECTION checkpoints',
    }),
    Object.freeze({
      surface: 'macos-installed',
      runnerId: 'macos-wdio-full-coverage',
      mode: 'instrumented-candidate',
      profileSelection: false,
      resume: 'explicit FCV_SECTION checkpoints',
      qualification: 'candidate-only',
    }),
  ]),
  strictQualificationHost: 'windows-installed',
  notes: Object.freeze([
    'Windows installed runner supports ordinary/heavy profile selection and signed-final resume.',
    'Linux runner accepts explicit FCV_SECTION selection for native candidate and installed-package runs.',
    'macOS runner is an instrumented native candidate; its receipt is not installed-app qualification.',
  ]),
})

const SHARED_PREREQUISITES = Object.freeze([
  'native-ui-attachment',
  'isolated-projects-directory',
  'committed-ui-source-action-manifest',
  'section-checkpoint-directory',
])

const STRICT_PREREQUISITES = Object.freeze([
  'FCV_REQUIRE_FULL=1',
  'FCV_FINAL_ALL_ACTIONS=1',
  'installed-native-app',
  'real-release-media',
])

const RESUME_IDENTITY = Object.freeze([
  'source.gitCommit',
  'source.contentManifestSha256',
  'runtime.sourceContentManifestSha256',
  'gate.surface',
  'gate.full',
  'gate.strictAllActions',
  'sourceActionIds',
  'expectedSourceActionIds',
])

const SECTION_DEFINITIONS = Object.freeze([
  ['settings', 'ordinary'],
  ['user-action-feedback', 'ordinary'],
  ['app-chrome-actions', 'ordinary'],
  ['statusbar-actions', 'ordinary'],
  ['search-actions', 'ordinary'],
  ['clips-actions', 'ordinary'],
  ['autopilot-actions', 'ordinary'],
  ['inspector-conditional-actions', 'ordinary'],
  ['project', 'ordinary'],
  ['timeline-toolbar-actions', 'ordinary'],
  ['timeline-dialog-actions', 'ordinary'],
  ['layer-actions', 'ordinary'],
  ['mask-actions', 'ordinary'],
  ['matte-actions', 'heavy'],
  ['preview-actions', 'ordinary'],
  ['record-actions', 'heavy'],
  ['environment-actions', 'ordinary'],
  ['grade-actions', 'ordinary'],
  ['shape-actions', 'ordinary'],
  ['title-actions', 'ordinary'],
  ['sequence-index-actions', 'ordinary'],
  ['sequence-switcher-actions', 'ordinary'],
  ['native-otio-actions', 'ordinary'],
  ['topbar-actions', 'ordinary'],
  ['topbar-dialog-actions', 'ordinary'],
  ['review-actions', 'ordinary'],
  ['recipe-actions', 'heavy'],
  ['render-queue-actions', 'heavy'],
  ['scopes-actions', 'heavy'],
  ['chat-actions', 'ordinary'],
  ['director-actions', 'heavy'],
  ['transcript-actions', 'heavy'],
  ['video', 'heavy'],
  ['audio', 'heavy'],
  ['blend', 'heavy'],
  ['typed', 'ordinary'],
  ['multi', 'ordinary'],
  ['ctxmenu', 'heavy'],
  ['editverbs', 'heavy'],
  ['range', 'ordinary'],
  ['export', 'heavy'],
  ['renderqueue', 'heavy'],
  ['menus', 'ordinary'],
  ['drawers', 'ordinary'],
  ['agent', 'ordinary'],
  ['aiservices', 'heavy'],
  ['record', 'heavy'],
  ['generate', 'heavy'],
  ['director', 'heavy'],
  ['assemble', 'heavy'],
  ['transcript', 'heavy'],
  ['reviewqc', 'heavy'],
  ['kinetic', 'heavy'],
  ['matte', 'heavy'],
  ['recipe', 'heavy'],
  ['library', 'ordinary'],
  ['projects', 'ordinary'],
  ['assets', 'ordinary'],
  ['comments', 'ordinary'],
  ['mixer', 'heavy'],
  ['autopilot', 'heavy'],
  ['residual', 'ordinary'],
  ['timeline-source-audit', 'ordinary'],
])

function sectionModule([section, weight], index) {
  return Object.freeze({
    id: `fcv.section.${section}`,
    module: section,
    surface: section,
    weight,
    prerequisites: Object.freeze({
      required: SHARED_PREREQUISITES,
      strict: STRICT_PREREQUISITES,
      profile: weight === 'heavy'
        ? Object.freeze(['perception-cv'])
        : Object.freeze([]),
    }),
    command: Object.freeze({
      id: `fcv.section.${section}`,
      runnerId: FULL_COVERAGE_RUNNER_ID,
      selector: Object.freeze({ environment: 'FCV_SECTION', value: section }),
    }),
    actionRegistry: FULL_COVERAGE_ACTION_REGISTRY.id,
    receipt: Object.freeze({
      schema: FULL_COVERAGE_RESULT_SCHEMA,
      checkpointSchema: FULL_COVERAGE_SECTION_CHECKPOINT_SCHEMA,
      section: Object.freeze({ index, total: SECTION_DEFINITIONS.length, key: section }),
    }),
    resume: Object.freeze({
      strategy: 'identity-matched-section-checkpoints',
      identity: RESUME_IDENTITY,
    }),
    hostPolicy: HOST_POLICY,
  })
}

const MODULES = Object.freeze(SECTION_DEFINITIONS.map(sectionModule))
const ORDINARY_MODULES = Object.freeze(MODULES.filter((entry) => entry.weight === 'ordinary'))
const HEAVY_MODULES = Object.freeze(MODULES.filter((entry) => entry.weight === 'heavy'))

function profile(id, modules, perceptionCv) {
  return Object.freeze({
    id,
    command: Object.freeze({
      id: `fcv.profile.${id}`,
      runnerId: FULL_COVERAGE_RUNNER_ID,
      selector: Object.freeze({ environment: 'FCV_MATRIX_PROFILE', value: id }),
    }),
    modules: Object.freeze(modules.map((entry) => entry.module)),
    prerequisites: Object.freeze({ perceptionCv }),
    receipt: Object.freeze({
      schema: FULL_COVERAGE_RESULT_SCHEMA,
      matrixProfile: id,
      totalSections: MODULES.length,
    }),
    resume: Object.freeze({ strategy: 'identity-matched-profile-checkpoints' }),
    hostPolicy: HOST_POLICY,
  })
}

const PROFILES = Object.freeze([
  profile('ordinary', ORDINARY_MODULES, false),
  profile('heavy', HEAVY_MODULES, true),
])

export const FULL_COVERAGE_CONTROL_CATALOG = Object.freeze({
  schema: FULL_COVERAGE_CONTROL_CATALOG_SCHEMA,
  expectedCounts: EXPECTED_COUNTS,
  actionRegistry: FULL_COVERAGE_ACTION_REGISTRY,
  commands: Object.freeze([
    Object.freeze({
      id: FULL_COVERAGE_RUNNER_ID,
      command: 'node ui/public-tests/full-coverage-verify.mjs',
      selectors: Object.freeze(['FCV_SECTION', 'FCV_MATRIX_PROFILE']),
    }),
    Object.freeze({
      id: 'windows-installed-full-coverage',
      command: 'node scripts/windows-installed-full-coverage.mjs',
      selectors: Object.freeze(['--matrix-profile', '--diagnostic-section', '--resume-section', '--resume-profile']),
    }),
    Object.freeze({
      id: 'linux-wdio-full-coverage',
      command: 'node scripts/linux-wdio-full-coverage.mjs',
      selectors: Object.freeze(['--section', '--installed-final']),
    }),
    Object.freeze({
      id: 'macos-wdio-full-coverage',
      command: 'node scripts/macos-wdio-track-controls.mjs --suite full-coverage',
      selectors: Object.freeze(['--section', '--strict-candidate-actions']),
    }),
  ]),
  modules: MODULES,
  profiles: PROFILES,
})

export const FULL_MATRIX_SECTION_KEYS = Object.freeze(MODULES.map((entry) => entry.module))
export const ORDINARY_MATRIX_SECTION_KEYS = Object.freeze(ORDINARY_MODULES.map((entry) => entry.module))
export const HEAVY_MATRIX_SECTION_KEYS = Object.freeze(HEAVY_MODULES.map((entry) => entry.module))
