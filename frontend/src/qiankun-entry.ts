import { createLifecycle } from './app.tsx'
import { createQiankunLifecycle } from './qiankun-lifecycle.ts'
import { SDK_VERSION, assertSupportedSdkVersion } from './sdk.ts'

declare const __QIANKUN_PUBLIC_CONFIG__: Readonly<{ version: string; allowedHostOrigins: string[]; apiBase: string }>
const lifecycle = createQiankunLifecycle(() => createLifecycle(__QIANKUN_PUBLIC_CONFIG__), __QIANKUN_PUBLIC_CONFIG__)
const sdkContract = Object.freeze({ version: SDK_VERSION, assertSupportedSdkVersion })
// Keep the four qiankun exports while exposing the actual bundled SDK's version
// negotiation for delivery checks. This contains no identity or credentials.
export const bootstrap = Object.defineProperty(lifecycle.bootstrap, 'sdkContract', {
  value: sdkContract,
  writable: false, configurable: false, enumerable: false,
}) as typeof lifecycle.bootstrap & { readonly sdkContract: typeof sdkContract }
export const { mount, update, unmount } = lifecycle
