const JUDGE_SOURCE_ROOT = '../../perception/py/judge/'
const JUDGE_TARGET_ROOT = 'perception/judge/'

export const JUDGE_RESOURCE_FILES = Object.freeze([
  'judge.py',
  'adapters/antigravity_judge.py',
  'adapters/cli_judge.py',
  'adapters/codex_judge.py',
  'adapters/diagnostics.py',
  'adapters/grok_judge.py',
  'adapters/grok_tool_policy.py',
  'adapters/ladder_judge.py',
  'adapters/restricted_claude.py',
  'adapters/restricted_claude_windows.py',
])

export const JUDGE_RESOURCES = Object.freeze(Object.fromEntries(
  JUDGE_RESOURCE_FILES.map((relative) => [
    `${JUDGE_SOURCE_ROOT}${relative}`,
    `${JUDGE_TARGET_ROOT}${relative}`,
  ]),
))

export function judgeResourcesFromBundle(resources = {}) {
  return Object.fromEntries(
    Object.entries(resources).filter(([source]) =>
      source === JUDGE_SOURCE_ROOT.slice(0, -1) || source.startsWith(JUDGE_SOURCE_ROOT),
    ),
  )
}

export function hasGeneratedJudgeBytecode(resources) {
  return Object.keys(resources).some((source) => source.includes('__pycache__') || source.endsWith('.pyc'))
}
