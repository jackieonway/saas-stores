import { installIframeBridge } from './sdk.ts'
import { createLazyLifecycle } from './lazy-lifecycle.ts'

declare global {
  interface Window {
    __STORES_CONFIG__?: Readonly<{ version: string; allowedHostOrigins: string[]; apiBase: string }>
  }
}

const container = document.querySelector<HTMLElement>('#app')
const config = window.__STORES_CONFIG__
if (container) {
  if (!config?.allowedHostOrigins?.length) {
    container.textContent = '门店应用尚未配置宿主连接，请联系平台管理员。'
  } else {
    // Register the one-time MessagePort listener before loading React or business UI.
    installIframeBridge(window)
    let bridge: unknown
    const lifecycle = createLazyLifecycle(() => import('./app.tsx'), () => bridge)
    bridge = window.MicroIframe.connect({ appCode: 'stores', allowedHostOrigins: config.allowedHostOrigins, container, lifecycle })
  }
}
