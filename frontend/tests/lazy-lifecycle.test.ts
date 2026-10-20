import assert from 'node:assert/strict'
import test from 'node:test'
import { createLazyLifecycle } from '../src/lazy-lifecycle.ts'
import type { MicroLifecycle, MicroProps } from '../src/sdk.ts'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(done => { resolve = done })
  return { promise, resolve }
}
const props = (signal: AbortSignal) => ({ signal }) as MicroProps
function fixture(loader?: () => { createLifecycle: (bridge: unknown) => MicroLifecycle } | Promise<{ createLifecycle: (bridge: unknown) => MicroLifecycle }>) {
  const calls: unknown[][] = [], bridge = {}, application: MicroLifecycle = {
    name: 'stores-test',
    mount: (container, current) => { calls.push(['mount', container, current]) },
    update: current => { calls.push(['update', current]) },
    unmount: () => { calls.push(['unmount']) },
  }
  const module = { createLifecycle(value: unknown) { assert.equal(value, bridge); calls.push(['create']); return application } }
  return { calls, module, lifecycle: createLazyLifecycle(loader || (() => module), () => bridge) }
}

test('a lifecycle is available synchronously and React business loading waits for mount', async () => {
  let loads = 0
  const f = fixture(() => { loads++; return f.module })
  assert.equal(typeof f.lifecycle.mount, 'function')
  assert.equal(loads, 0)
  const current = props(new AbortController().signal), container = {} as HTMLElement
  await f.lifecycle.mount(container, current)
  await f.lifecycle.update?.(current)
  assert.equal(loads, 1)
  assert.deepEqual(f.calls.map(call => call[0]), ['create', 'mount', 'update'])
  assert.equal(f.calls[1][1], container)
  assert.equal(f.calls[1][2], current)
})

test('a first import cancelled by the host can mount with a later fresh SDK update', async () => {
  const imported = deferred<ReturnType<typeof fixture>['module']>(), f = fixture(() => imported.promise)
  const cancelled = new AbortController(), fresh = props(new AbortController().signal), container = {} as HTMLElement
  const first = f.lifecycle.mount(container, props(cancelled.signal))
  cancelled.abort()
  imported.resolve(f.module)
  await first
  assert.deepEqual(f.calls.map(call => call[0]), ['create'])
  await f.lifecycle.update?.(fresh)
  assert.deepEqual(f.calls.map(call => call[0]), ['create', 'mount'])
  assert.equal(f.calls[1][1], container)
  assert.equal(f.calls[1][2], fresh)
})

test('only the newest update waiting on the same import can mount visible business UI', async () => {
  const imported = deferred<ReturnType<typeof fixture>['module']>(), f = fixture(() => imported.promise)
  const stale = props(new AbortController().signal), fresh = props(new AbortController().signal), container = {} as HTMLElement
  const first = f.lifecycle.mount(container, stale)
  const latest = f.lifecycle.update?.(fresh)
  imported.resolve(f.module)
  await Promise.all([first, latest])
  assert.deepEqual(f.calls.map(call => call[0]), ['create', 'mount'])
  assert.equal(f.calls[1][2], fresh)
})

test('disposal before import completion prevents factory creation and all later mounts', async () => {
  const imported = deferred<ReturnType<typeof fixture>['module']>(), f = fixture(() => imported.promise)
  const current = props(new AbortController().signal), mounting = f.lifecycle.mount({} as HTMLElement, current)
  await f.lifecycle.unmount()
  imported.resolve(f.module)
  await mounting
  await f.lifecycle.mount({} as HTMLElement, current)
  await f.lifecycle.update?.(current)
  await f.lifecycle.unmount()
  assert.deepEqual(f.calls, [])
})

test('loaded business UI is unmounted exactly once and cannot update after disposal', async () => {
  const f = fixture(), current = props(new AbortController().signal)
  await f.lifecycle.mount({} as HTMLElement, current)
  await f.lifecycle.unmount()
  await f.lifecycle.unmount()
  await f.lifecycle.update?.(current)
  assert.deepEqual(f.calls.map(call => call[0]), ['create', 'mount', 'unmount'])
})

test('an already aborted context does not start importing business code', async () => {
  let loads = 0
  const f = fixture(() => { loads++; return f.module }), cancelled = new AbortController()
  cancelled.abort()
  await f.lifecycle.mount({} as HTMLElement, props(cancelled.signal))
  assert.equal(loads, 0)
  assert.deepEqual(f.calls, [])
})
