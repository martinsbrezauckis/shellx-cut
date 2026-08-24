import { createHash } from 'node:crypto'
import { existsSync, lstatSync, readFileSync, realpathSync } from 'node:fs'
import { isAbsolute, resolve } from 'node:path'

export const SEALED_PLAYWRIGHT_CHROMIUM_PATH = 'SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_PATH'
export const SEALED_PLAYWRIGHT_CHROMIUM_SHA256 = 'SHELLX_CUT_SEALED_PLAYWRIGHT_CHROMIUM_SHA256'
export const REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM = 'SHELLX_CUT_REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM'

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function sealedValue(env, name) {
  const value = env[name]
  return typeof value === 'string' && value.length > 0 ? value : ''
}

// Normal developer runs retain Playwright's standard browser selection. The
// runtime-sealed gate supplies all three variables and therefore does not read
// an ambient cache from its per-flow HOME.
export function sealedPlaywrightChromiumLaunchOptions(env = process.env) {
  const required = sealedValue(env, REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM)
  const requested = sealedValue(env, SEALED_PLAYWRIGHT_CHROMIUM_PATH)
  const expectedSha256 = sealedValue(env, SEALED_PLAYWRIGHT_CHROMIUM_SHA256)
  const sealed = Boolean(required || requested || expectedSha256)
  if (!sealed) return {}
  if (required !== '1') throw new Error(`${REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM} must be exactly 1`)
  if (!requested) throw new Error(`${REQUIRE_SEALED_PLAYWRIGHT_CHROMIUM}=1 requires ${SEALED_PLAYWRIGHT_CHROMIUM_PATH}`)
  if (!/^[a-f0-9]{64}$/.test(expectedSha256)) throw new Error(`${SEALED_PLAYWRIGHT_CHROMIUM_SHA256} must be an exact SHA-256 digest`)
  if (sealedValue(env, 'PLAYWRIGHT_BROWSERS_PATH')) throw new Error('sealed Playwright launch forbids PLAYWRIGHT_BROWSERS_PATH')
  if (!isAbsolute(requested)) throw new Error(`${SEALED_PLAYWRIGHT_CHROMIUM_PATH} must be absolute`)
  if (!existsSync(requested)) throw new Error(`sealed Playwright Chromium does not exist: ${requested}`)
  const path = resolve(realpathSync.native(requested))
  if (resolve(requested) !== path) throw new Error(`sealed Playwright Chromium must not traverse a symbolic link: ${requested}`)
  const entry = lstatSync(path)
  if (!entry.isFile() || entry.size <= 0) throw new Error(`sealed Playwright Chromium must be a non-empty regular file: ${path}`)
  if (process.platform !== 'win32' && (entry.mode & 0o111) === 0) throw new Error(`sealed Playwright Chromium must be executable: ${path}`)
  if (sha256(path) !== expectedSha256) throw new Error('sealed Playwright Chromium SHA-256 drifted before launch')
  return { executablePath: path }
}
