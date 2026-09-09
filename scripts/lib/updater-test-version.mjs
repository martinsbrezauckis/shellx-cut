export const UPDATER_BASELINE_QUALIFIER = 'updater-test.0'

const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/

function fail(message) { throw new Error(`same-frozen-source updater baseline: ${message}`) }

export function updaterBaselineVersion(candidateVersion) {
  if (!STABLE_VERSION.test(candidateVersion || '')) fail('candidate version must use major.minor.patch')
  return `${candidateVersion}-${UPDATER_BASELINE_QUALIFIER}`
}

export function isUpdaterBaselineVersion(value, candidateVersion = '') {
  const parsed = /^((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))-(updater-test[.]0)$/.exec(value || '')
  return Boolean(parsed && parsed[2] === UPDATER_BASELINE_QUALIFIER && (!candidateVersion || parsed[1] === candidateVersion))
}

/** Compares the stable candidate with a stable or updater-baseline installed version. */
export function isCandidateHigherThanInstalled(candidateVersion, installedVersion) {
  if (!STABLE_VERSION.test(candidateVersion || '')) fail('candidate version must use major.minor.patch')
  if (!STABLE_VERSION.test(installedVersion || '') && !isUpdaterBaselineVersion(installedVersion)) {
    fail('installed version must use major.minor.patch or the maintained updater-test prerelease')
  }
  const candidate = candidateVersion.split('.').map(Number)
  const installed = installedVersion.split('-')[0].split('.').map(Number)
  for (let index = 0; index < candidate.length; index += 1) {
    if (candidate[index] !== installed[index]) return candidate[index] > installed[index]
  }
  return Boolean(installedVersion.includes('-'))
}
