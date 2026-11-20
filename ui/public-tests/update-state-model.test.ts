// UI update-state model — topbar offer and Settings > About presentation.
// This is deliberately standalone so the current product contract remains a
// normal `npm run test:lib` discovery target rather than growing the legacy
// lib.test.ts aggregate.

import { strict as assert } from 'node:assert'
import {
  describeUpdateStatus,
  formatCheckedAgo,
  releaseNotesUrl,
  shouldShowUpdateButton,
  updateButtonLabel,
  validShellUpdateState,
} from '../src/lib/updateState'

const snapshot = (over: Record<string, unknown> = {}) => ({
  schema: 'shellx-cut/update-state/1' as const,
  status: 'none' as const,
  version: null,
  current: '0.6.105',
  checked_at: 1_000,
  error: null,
  checking: false,
  installing: false,
  supported: true,
  ...over,
})

assert.equal(validShellUpdateState(snapshot()), true, 'a well-formed shell snapshot validates')
assert.equal(validShellUpdateState(null), false, 'null payload is rejected')
assert.equal(validShellUpdateState({ ...snapshot(), schema: 'shellx-cut/update-state/2' }), false, 'a future schema is rejected, not misread')
assert.equal(validShellUpdateState({ ...snapshot(), status: 'sideways' }), false, 'an unknown status is rejected')
assert.equal(validShellUpdateState({ ...snapshot(), checking: 'yes' }), false, 'non-boolean flags are rejected')

const available = snapshot({ status: 'available', version: '0.7.0' }) as never
assert.equal(shouldShowUpdateButton(available), true, 'available shows the topbar button')
assert.equal(shouldShowUpdateButton(snapshot() as never), false, 'up-to-date hides the topbar button')
assert.equal(shouldShowUpdateButton(snapshot({ status: 'idle' }) as never), false, 'idle hides the topbar button')
assert.equal(shouldShowUpdateButton(snapshot({ status: 'error', error: 'offline' }) as never), false, 'a check error alone hides the button')
assert.equal(shouldShowUpdateButton(snapshot({ status: 'unsupported', supported: false }) as never), false, 'Linux package builds hide the updater button')
assert.equal(shouldShowUpdateButton(snapshot({ status: 'available', version: '' }) as never), false, 'an offer without a version stays hidden')
assert.equal(shouldShowUpdateButton(null), false, 'a browser build without a snapshot hides the button')

assert.equal(updateButtonLabel(available), 'Update to v0.7.0', 'the button labels the offered version')
assert.equal(
  updateButtonLabel(snapshot({ status: 'available', version: '0.7.0', installing: true }) as never),
  'Installing update…',
  'an in-flight install relabels the button',
)

assert.equal(describeUpdateStatus(snapshot() as never).text, "You're on the latest version.", 'none reads as latest')
assert.equal(describeUpdateStatus(available).text, 'ShellX Cut 0.7.0 is available.', 'available names the version')
assert.deepEqual(
  describeUpdateStatus(snapshot({ status: 'error', error: 'update check failed: dns' }) as never),
  { tone: 'error', text: 'Update check failed: update check failed: dns' },
  'a failed check surfaces the exact failure text',
)
assert.equal(
  describeUpdateStatus(snapshot({ status: 'unsupported', supported: false }) as never).text,
  'Linux builds update through deb/rpm package downloads — the in-app updater is not used.',
  'Linux explains package delivery instead of a dead surface',
)
assert.equal(describeUpdateStatus(snapshot({ checking: true }) as never).text, 'Checking for updates…', 'in-flight check reads as checking')
assert.equal(describeUpdateStatus(snapshot({ status: 'idle', checked_at: null }) as never).tone, 'muted', 'idle stays muted')

assert.equal(formatCheckedAgo(null, 100_000), null, 'no completed check has no fake timestamp')
assert.equal(formatCheckedAgo(90_000, 100_000), 'Checked just now', 'under 45 seconds reads as just now')
assert.equal(formatCheckedAgo(100_000, 160_000), 'Checked a minute ago', 'one-minute band is correct')
assert.equal(formatCheckedAgo(0.5 * 3_600_000, 1.0 * 3_600_000), 'Checked 30 minutes ago', 'minute band is exact')
assert.equal(formatCheckedAgo(0, 3_600_000), null, 'epoch zero means never checked, not 1970')
assert.equal(formatCheckedAgo(1_000, 90 * 60_000 + 1_000), 'Checked an hour ago', 'one-hour band is correct')
assert.equal(formatCheckedAgo(1_000, 5 * 3_600_000 + 1_000), 'Checked 5 hours ago', 'hour band is correct')
assert.equal(formatCheckedAgo(1_000, 26 * 3_600_000 + 1_000), 'Checked 1 day ago', 'day band singular is correct')
assert.equal(formatCheckedAgo(1_000, 72 * 3_600_000 + 1_000), 'Checked 3 days ago', 'day band plural is correct')

assert.equal(
  releaseNotesUrl(available),
  'https://github.com/martinsbrezauckis/shellx-cut/releases/tag/v0.7.0',
  'release notes link the exact offered release',
)
assert.equal(
  releaseNotesUrl(snapshot() as never),
  'https://github.com/martinsbrezauckis/shellx-cut/releases/latest',
  'without an offer, release notes link latest',
)
assert.equal(
  releaseNotesUrl(null),
  'https://github.com/martinsbrezauckis/shellx-cut/releases/latest',
  'a browser build links latest release notes',
)

console.log('PASS update-state model')
