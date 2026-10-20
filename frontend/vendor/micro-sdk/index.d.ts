import type { MicroProps, MicroLifecycle, NotificationPublishBody, NotificationPublication } from './types.js'
import { SDK_VERSION, IFRAME_PROTOCOL_VERSION } from './protocol.mjs'
export * from './protocol.mjs'
export type * from './types.js'
export { MicroSdkError, DEFAULT_MAX_RESPONSE_BYTES, MAX_RESPONSE_BYTES, requestWithContext } from './http.mjs'
export type { MicroSdkErrorKind, ContextRequestOptions } from './http.mjs'
export { detectBrowserCapabilities, assertBrowserCapabilities } from './capabilities.mjs'
/** Memory-only key contains credentials; unknown context fields refuse reuse. Never log the result. */
export function authorizationContextSignature(props: unknown): string | null
export function sameAuthorizationContext(previous: unknown, next: unknown): boolean

export interface ContextClient {
  readonly sdkVersion: typeof SDK_VERSION
  current(): MicroProps | null
  update(props: MicroProps): void
  dispose(): void
  requestJson<T = unknown>(url: string | URL, options?: RequestInit): Promise<T>
}
export function createContextClient(props: MicroProps, fetchImpl?: typeof fetch): ContextClient
export interface NotificationPublisher {
  readonly sdkVersion: typeof SDK_VERSION
  publish<T = NotificationPublication>(body: NotificationPublishBody): Promise<T>
}
export function createNotificationPublisher(client: Pick<ContextClient, 'current' | 'requestJson'> & { readonly sdkVersion?: string }): NotificationPublisher

export interface DownloadCsvOptions {
  resource: string
  filename: string
  content: string
}
export interface IframeConnection {
  readonly sdkVersion: typeof SDK_VERSION
  current(): MicroProps | null
  dispose(): void
  requestJson<T = unknown>(url: string | URL, options?: RequestInit): Promise<T>
  downloadCsv(file: DownloadCsvOptions): Promise<void>
}
export interface IframeConnectOptions {
  appCode: string
  allowedHostOrigins: string[]
  lifecycle: MicroLifecycle
  container?: HTMLElement
  sdkVersion?: typeof SDK_VERSION
}
export interface MicroIframeApi {
  readonly sdkVersion: typeof SDK_VERSION
  readonly contractVersion: typeof IFRAME_PROTOCOL_VERSION
  connect(options: IframeConnectOptions): IframeConnection
}
/** Installs the same bridge as the classic bundle; call before connecting in a bundled application. */
export function installIframeBridge(environment: Window): void
declare global {
  interface Window {
    MicroIframe: MicroIframeApi
    MicroSdk: typeof import('./index.mjs') & { readonly sdkVersion: typeof SDK_VERSION }
  }
}
