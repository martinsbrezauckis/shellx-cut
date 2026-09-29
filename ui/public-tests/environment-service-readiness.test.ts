import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { DoctorCard } from '../src/lib/doctor'
import { ServiceRuntimeActions, ServiceRuntimeDetail } from '../src/panels/Environment/ServiceRuntime'

function visibleService(status: DoctorCard['status'], reachable: boolean, runnerAvailable: boolean) {
  const card: DoctorCard = {
    id: 'dub',
    kind: 'service',
    status,
    details: { reachable, runner_available: runnerAvailable },
  }
  return renderToStaticMarkup(createElement('section', null,
    createElement(ServiceRuntimeActions, { card, busy: false, onOpenSetup: () => {} }),
    createElement(ServiceRuntimeDetail, { card, open: true, onOpenChange: () => {}, onChanged: () => {} }),
  ))
}

const noConnector = visibleService('unknown', true, false)
assert.match(noConnector, /Connect service/)
assert.doesNotMatch(noConnector, /Open Transcript tools/)
assert.match(noConnector, /Connector missing/)
assert.match(noConnector, /External service ready/)
assert.match(noConnector, /Install or repair the ShellX Cut perception connector/)
assert.doesNotMatch(noConnector, /Ready for re-voicing/)

const noService = visibleService('unknown', false, true)
assert.match(noService, /Connect service/)
assert.match(noService, /Connector ready/)
assert.match(noService, /External service not connected/)
assert.doesNotMatch(noService, /Open Transcript tools/)

const ready = visibleService('ok', true, true)
assert.match(ready, /Open Transcript tools/)
assert.match(ready, /Connector ready/)
assert.match(ready, /External service ready/)
assert.match(ready, /Ready for re-voicing/)
assert.doesNotMatch(ready, /Connect service/)

console.log('Environment service readiness checks passed')
