// Connection status is a singleton fact. Project-session remounts must receive
// the latest state without waiting for another socket transition.

import { strict as assert } from 'node:assert'

import { EventsClient } from '../src/lib/events'

class FakeWebSocket {
  static readonly OPEN = 1
  static instances: FakeWebSocket[] = []

  readyState = 0
  onopen: ((event: Event) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null

  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this)
  }

  send(): void {}

  open(): void {
    this.readyState = FakeWebSocket.OPEN
    this.onopen?.({} as Event)
  }

  closeFromServer(): void {
    this.readyState = 3
    this.onclose?.({} as CloseEvent)
  }

  close(): void {}
}

const priorWebSocket = Object.getOwnPropertyDescriptor(globalThis, 'WebSocket')
const priorLocation = Object.getOwnPropertyDescriptor(globalThis, 'location')
Object.defineProperty(globalThis, 'WebSocket', { configurable: true, value: FakeWebSocket })
Object.defineProperty(globalThis, 'location', { configurable: true, value: { protocol: 'http:', host: '127.0.0.1:6161' } })

try {
  const client = new EventsClient()
  const firstSubscriber: string[] = []
  client.onStatus((status) => firstSubscriber.push(status))
  client.connect()
  const socket = FakeWebSocket.instances.at(-1)
  assert.ok(socket, 'client constructed its WebSocket')
  socket.open()

  const remountedSubscriber: string[] = []
  const unsubscribe = client.onStatus((status) => remountedSubscriber.push(status))
  assert.equal(typeof unsubscribe, 'function', 'late subscriber receives the usual unsubscribe callback')
  assert.deepEqual(remountedSubscriber, ['open'], 'post-open remount immediately receives the retained connection state')

  unsubscribe()
  socket.closeFromServer()
  assert.deepEqual(firstSubscriber, ['connecting', 'connecting', 'open', 'closed'], 'existing listener receives initial replay and later state transitions')
  assert.deepEqual(remountedSubscriber, ['open'], 'unsubscribed remount listener receives no later transition')
  let replayFailures = 0
  assert.throws(() => client.onStatus(() => { replayFailures += 1; throw new Error('listener failed') }), /listener failed/, 'a throwing replay listener propagates without becoming retained')
  assert.doesNotThrow(() => client.connect(), 'a throwing replay listener is not retained for later transitions')
  assert.equal(replayFailures, 1, 'throwing replay listener ran only during its failed registration')
  client.close()
} finally {
  if (priorWebSocket) Object.defineProperty(globalThis, 'WebSocket', priorWebSocket)
  else delete (globalThis as { WebSocket?: unknown }).WebSocket
  if (priorLocation) Object.defineProperty(globalThis, 'location', priorLocation)
  else delete (globalThis as { location?: unknown }).location
}

console.log('PASS events status replay')
