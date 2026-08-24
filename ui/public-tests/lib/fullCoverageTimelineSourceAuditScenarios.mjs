// Declarative aliases for the v0.6.110 timeline/source audit scenarios.
// Execution and receipt creation remain in the canonical full-coverage runner.

export const TIMELINE_SOURCE_AUDIT_SCENARIOS = Object.freeze([
  Object.freeze({
    id: 'e2e-trim-tools-01',
    runner: 'timeline-source-audit',
    surface: 'browser-ui',
    receiptSchema: 'shellx-cut/full-coverage-results@1',
    command: Object.freeze([
      'FCV_SECTION=timeline-source-audit',
      'FCV_ONLY=e2e-trim-tools-01',
      'node public-tests/full-coverage-verify.mjs',
    ]),
    receiptRoot: 'full-coverage',
  }),
  Object.freeze({
    id: 'e2e-source-insert-01',
    runner: 'timeline-source-audit',
    surface: 'browser-ui',
    receiptSchema: 'shellx-cut/full-coverage-results@1',
    command: Object.freeze([
      'FCV_SECTION=timeline-source-audit',
      'FCV_ONLY=e2e-source-insert-01',
      'node public-tests/full-coverage-verify.mjs',
    ]),
    receiptRoot: 'full-coverage',
  }),
  Object.freeze({
    id: 'e2e-selection-sync-01',
    runner: 'timeline-source-audit',
    surface: 'browser-ui',
    receiptSchema: 'shellx-cut/full-coverage-results@1',
    command: Object.freeze([
      'FCV_SECTION=timeline-source-audit',
      'FCV_ONLY=e2e-selection-sync-01',
      'node public-tests/full-coverage-verify.mjs',
    ]),
    receiptRoot: 'full-coverage',
  }),
  Object.freeze({
    id: 'e2e-keyboard-focus-01',
    runner: 'timeline-source-audit',
    surface: 'browser-ui',
    receiptSchema: 'shellx-cut/full-coverage-results@1',
    command: Object.freeze([
      'FCV_SECTION=timeline-source-audit',
      'FCV_ONLY=e2e-keyboard-focus-01',
      'node public-tests/full-coverage-verify.mjs',
    ]),
    receiptRoot: 'full-coverage',
  }),
  Object.freeze({
    id: 'e2e-interaction-fuzz-01',
    runner: 'timeline-source-audit',
    surface: 'browser-ui',
    receiptSchema: 'shellx-cut/full-coverage-results@1',
    command: Object.freeze([
      'FCV_SECTION=timeline-source-audit',
      'FCV_ONLY=e2e-interaction-fuzz-01',
      'FCV_INTERACTION_FUZZ_SEED=<required-seed>',
      'FCV_SOURCE_GIT_COMMIT=<frozen-candidate-commit>',
      'FCV_SOURCE_CONTENT_MANIFEST_SHA256=<frozen-candidate-manifest>',
      'FCV_RESULT_RECEIPT=<candidate-bound-receipt>',
      'node public-tests/full-coverage-verify.mjs',
    ]),
    receiptRoot: 'FCV_RESULT_RECEIPT',
  }),
])
