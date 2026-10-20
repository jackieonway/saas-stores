import type { MicroLifecycle, MicroProps } from './sdk.ts'

type BusinessModule = { createLifecycle: (bridge: unknown) => MicroLifecycle }

/** The SDK listener is ready synchronously; a cancelled import cannot mount stale UI. */
export function createLazyLifecycle(loadModule: () => BusinessModule | Promise<BusinessModule>, getBridge: () => unknown): MicroLifecycle {
  let pending: Promise<MicroLifecycle | null> | undefined
  let application: MicroLifecycle | undefined
  let target: HTMLElement | undefined
  let stopped = false
  let mounted = false
  let unmounted = false
  let newest = 0
  const load = () => {
    if (!pending) pending = Promise.resolve().then(loadModule).then(module => {
      if (stopped) return null
      application = module.createLifecycle(getBridge())
      return application
    })
    return pending
  }
  async function apply(props: MicroProps) {
    const request = ++newest
    if (stopped || props.signal.aborted || !target) return
    const lifecycle = await load()
    if (!lifecycle || stopped || props.signal.aborted || request !== newest) return
    if (!mounted) {
      mounted = true
      await lifecycle.mount(target, props)
    } else {
      await lifecycle.update?.(props)
    }
  }
  return {
    name: 'stores',
    mount(container, props) {
      if (stopped) return
      target = container
      return apply(props)
    },
    update: apply,
    async unmount() {
      stopped = true
      newest++
      if (!application || unmounted) return
      unmounted = true
      await application.unmount()
    },
  }
}
