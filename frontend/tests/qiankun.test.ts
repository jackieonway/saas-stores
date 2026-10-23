import assert from 'node:assert/strict'
import test from 'node:test'
import { createQiankunLifecycle, type QiankunProps } from '../src/qiankun-lifecycle.ts'
import type { MicroProps } from '../src/sdk.ts'

const context = (routePath = '/micro/stores') => ({ signal: new AbortController().signal, routePath }) as MicroProps
function fixture() {
  const calls: unknown[][] = [], target = {} as HTMLElement
  const container = { querySelector: (selector: string) => { assert.equal(selector, '[data-stores-mount]'); return target } } as unknown as Element
  const lifecycle = createQiankunLifecycle(() => ({
    mount: (...args) => { calls.push(['mount', ...args]) },
    update: props => { calls.push(['update', props]) },
    unmount: () => { calls.push(['unmount']) },
  }))
  return { lifecycle, calls, container, target }
}

test('qiankun metadata and services do not enter MicroProps and deep route updates remain available', async () => {
  const f = fixture(), first = context(), next = context('/micro/stores/reports')
  await f.lifecycle.bootstrap()
  await f.lifecycle.mount({ container: f.container, platform: first, services: { injected: true } })
  await f.lifecycle.update({ container: f.container, platform: next })
  assert.deepEqual(f.calls, [['mount', f.target, first], ['update', next]])
  await f.lifecycle.unmount(); await f.lifecycle.unmount()
  await f.lifecycle.update({ container: f.container, platform: next })
  await f.lifecycle.mount({ container: f.container, platform: next })
  assert.deepEqual(f.calls.map(call => call[0]), ['mount', 'update', 'unmount', 'mount'])
})

test('missing and cancelled platform contexts cannot mount', async () => {
  const f = fixture()
  await assert.rejects(f.lifecycle.mount({ container: f.container } as QiankunProps), /props.platform/)
  await f.lifecycle.mount({ container: f.container, platform: { ...context(), signal: AbortSignal.abort() } })
  assert.deepEqual(f.calls, [])
})
