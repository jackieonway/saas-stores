// The standalone application consumes the registered SDK release, including its generated types.
export type { MicroProps, MicroTheme, MicroDataScope } from '../vendor/micro-sdk/index.mjs'
export {
  SDK_VERSION, MicroSdkError, requestWithContext, authorizationContextSignature,
  assertSupportedSdkVersion,
} from '../vendor/micro-sdk/index.mjs'
