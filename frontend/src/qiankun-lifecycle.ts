import type { MicroProps } from './sdk.ts'

export type RenderLifecycle = {
  mount(container: HTMLElement, props: MicroProps): void | Promise<void>
  update(props: MicroProps): void | Promise<void>
  unmount(): void | Promise<void>
}

export type QiankunProps = { container: Element; platform: MicroProps; services?: unknown }

/** Only the nested platform object is forwarded to the registered SDK. */
export function createQiankunLifecycle(createApplication: () => RenderLifecycle, options: { allowedHostOrigins?: readonly string[] } = {}) {
  let application: RenderLifecycle | undefined
  let queue: Promise<unknown> = Promise.resolve()
  const enqueue = (action: () => Promise<void>) => {
    const result = queue.then(action)
    queue = result.catch(() => {})
    return result
  }
  const platform = (props: QiankunProps) => {
    if (!props?.platform?.signal) throw new TypeError('qiankun requires props.platform with the platform context')
    return props.platform
  }
  return {
    async bootstrap() {},
    mount(props: QiankunProps) {
      return enqueue(async () => {
        const context = platform(props)
        if (context.signal.aborted) return
        if (application) throw new Error('微应用已挂载')
        if (options.allowedHostOrigins && !options.allowedHostOrigins.includes(window.location.origin)) throw new Error('当前宿主来源未获准')
        const target = props.container?.querySelector<HTMLElement>('[data-stores-mount]')
        if (!target) throw new Error('缺少门店微应用挂载容器')
        application = createApplication()
        try { await application.mount(target, context) }
        catch (error) { await application.unmount(); application = undefined; throw error }
      })
    },
    update(props: QiankunProps) {
      return enqueue(async () => {
        const context = platform(props)
        if (!application || context.signal.aborted) return
        await application.update?.(context)
      })
    },
    unmount() {
      return enqueue(async () => {
        const previous = application
        application = undefined
        await previous?.unmount()
      })
    },
  }
}
