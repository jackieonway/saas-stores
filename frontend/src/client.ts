import {
  SDK_VERSION, MicroSdkError, requestWithContext, authorizationContextSignature, assertSupportedSdkVersion,
} from './sdk.ts'
import {
  can, canMemberLookup, validContext, validUUID, validBusinessDate, validAmount, normalizeAmount,
} from './domain.ts'
import type {
  Props, UUID, BusinessDate, DecimalAmount, BusinessStatus, Operation, MemberPurpose,
  ScopedRow, Store, StaffLink, DailyReport, Member, Page, Summary,
} from './domain.ts'

export interface RequestOptions { signal?: AbortSignal }
export interface PageQuery { page?: number; size?: number }
export interface StoreQuery extends PageQuery { q?: string; businessStatus?: BusinessStatus; organizationId?: UUID }
export interface ReportQuery extends PageQuery { dateFrom?: BusinessDate; dateTo?: BusinessDate }
export interface SummaryQuery { dateFrom: BusinessDate; dateTo: BusinessDate; storeId?: UUID }
export interface MemberQuery extends PageQuery { organizationId?: UUID; q?: string }
export interface CreateStore { name: string; businessStatus: BusinessStatus; businessContact: string; managerUserId?: UUID | null }
export interface UpdateStore { version: number; name?: string; businessStatus?: BusinessStatus; businessContact?: string; managerUserId?: UUID | null }
export interface CreateDailyReport { businessDate: BusinessDate; amount: DecimalAmount; note?: string }
export interface UpdateDailyReport { version: number; amount?: DecimalAmount; note?: string }
export interface ClientSnapshot { readonly props: Props; readonly generation: number; readonly signal: AbortSignal }
export type BusinessErrorKind = 'UNAUTHORIZED' | 'FORBIDDEN' | 'CONFLICT' | 'NETWORK' | 'PROTOCOL' | 'HTTP' | 'CANCELLED' | 'TIMEOUT'

const messages: Record<string, string> = {
  INVALID_CONTEXT: '应用认证已失效，请等待平台刷新授权',
  INVALID_APP_TOKEN: '应用认证已失效，请等待平台刷新授权',
  OPERATION_DENIED: '当前操作未获授权，请等待平台更新上下文',
  DATA_DENIED: '当前数据范围未获授权，请等待平台更新上下文',
  STORE_ALREADY_EXISTS: '当前组织已有门店，请查询并核对现有门店',
  STAFF_ALREADY_ASSIGNED: '该员工已关联，请查询并核对现有关联',
  DAILY_REPORT_EXISTS: '该门店当日已有营业记录，请查询并核对现有记录',
  VERSION_CONFLICT: '记录已被修改，请保留输入，重新读取并核对版本',
  RESOURCE_NOT_FOUND: '记录不存在或不在当前授权范围',
  MEMBER_NOT_FOUND: '该成员已失效或不在目标组织，请重新查询',
  VALIDATION_ERROR: '输入格式或范围无效，请检查表单',
  RESPONSE_TOO_LARGE: '服务返回的数据过大，请缩小查询范围',
  CREATE_RESULT_UNKNOWN: '创建结果未知，请保留原输入并先查询核对，勿重复提交',
  WRITE_RESULT_UNKNOWN: '写入结果未知，请保留原输入并先查询核对，勿重复提交',
  MEMBER_VALIDATION_EXPIRED: '成员校验已过期，请保留输入并重新查询核对',
  REQUEST_TIMEOUT: '请求超时，请稍后重新查询',
}
const kindMessages: Record<BusinessErrorKind, string> = {
  UNAUTHORIZED: messages.INVALID_CONTEXT!, FORBIDDEN: messages.OPERATION_DENIED!,
  CONFLICT: '业务状态有冲突，请保留输入并查询核对',
  NETWORK: '业务服务暂时无法连接，请保留输入并稍后查询',
  PROTOCOL: '业务服务返回的数据无法识别', HTTP: '操作未完成，请保留输入并稍后查询',
  CANCELLED: '当前请求已取消',
  TIMEOUT: messages.REQUEST_TIMEOUT!,
}
/** Only fixed messages and bounded public metadata are retained; no remote text, URL, body or cause. */
export class BusinessError extends Error {
  readonly status: number
  readonly code: string
  readonly kind: BusinessErrorKind
  readonly requestId?: string
  readonly unknown: boolean
  constructor(status: number, code = 'HTTP', kind: BusinessErrorKind = 'HTTP', requestId?: string, unknown = false) {
    const safeKind = Object.hasOwn(kindMessages, kind) ? kind : 'HTTP'
    const safeCode = Object.hasOwn(messages, code) || /^[A-Z][A-Z0-9_]{0,63}$/.test(code) ? code : safeKind
    super(unknown ? safeKind === 'TIMEOUT' ? '请求超时，写入结果未知，请保留原输入并先读回核对，勿重复提交'
      : messages[code === 'CREATE_RESULT_UNKNOWN' ? code : 'WRITE_RESULT_UNKNOWN'] : messages[safeCode] ?? kindMessages[safeKind])
    this.name = 'BusinessError'
    this.status = Number.isInteger(status) && status >= 0 && status <= 599 ? status : 0
    this.code = safeCode; this.kind = safeKind; this.unknown = unknown
    if (validUUID(requestId)) this.requestId = requestId
  }
  toJSON() { return { status: this.status, code: this.code, kind: this.kind, message: this.message,
    unknown: this.unknown, ...(this.requestId ? { requestId: this.requestId } : {}) } }
}
const cancelled = () => new DOMException('门店应用上下文已变更', 'AbortError')
const validation = () => new BusinessError(400, 'VALIDATION_ERROR')
const denied = () => new BusinessError(403, 'OPERATION_DENIED', 'FORBIDDEN')
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value)
const text = (value: unknown, maximum: number): value is string => typeof value === 'string' && !value.includes('\0') && Array.from(value).length <= maximum
const equal = (left: string, right: string) => left.toLowerCase() === right.toLowerCase()
function shape(condition: unknown): asserts condition { if (!condition) throw new TypeError('业务数据格式无效') }
function keys(value: unknown, allowed: readonly string[], required: readonly string[] = allowed): asserts value is Record<string, unknown> {
  shape(object(value) && Object.keys(value).every(key => allowed.includes(key)) && required.every(key => Object.hasOwn(value, key)))
}
function validVersion(value: unknown, write = false): value is number {
  return Number.isInteger(value) && (value as number) >= 1 && (value as number) <= (write ? 2147483646 : 2147483647)
}
const timestamp = (value: unknown): value is string => typeof value === 'string' &&
  /^\d{4}-\d{2}-\d{2}T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d{1,9})?Z$/.test(value) &&
  validBusinessDate(value.slice(0, 10)) && Number.isFinite(Date.parse(value))
const baseFields = ['id', 'tenantId', 'organizationId', 'ownerUserId', 'version', 'createdAt', 'updatedAt']
function base(value: Record<string, unknown>) {
  shape(['id', 'tenantId', 'organizationId', 'ownerUserId'].every(key => validUUID(value[key])) &&
    validVersion(value.version) && timestamp(value.createdAt) && timestamp(value.updatedAt) && value.updatedAt >= value.createdAt)
}
function storeResponse(value: unknown): Store {
  keys(value, [...baseFields, 'name', 'businessStatus', 'businessContact', 'managerUserId', 'managerDisplayName'])
  base(value)
  shape(text(value.name, 200) && !!value.name.trim() && text(value.businessContact, 200) &&
    (value.businessStatus === 'OPEN' || value.businessStatus === 'CLOSED') &&
    (value.managerUserId === null && value.managerDisplayName === null ||
      validUUID(value.managerUserId) && text(value.managerDisplayName, 100) && !!value.managerDisplayName.trim()))
  return value as unknown as Store
}
function staffResponse(value: unknown): StaffLink {
  keys(value, [...baseFields, 'storeId', 'userId', 'displayName', 'createdByUserId', 'removedAt'])
  base(value)
  shape(validUUID(value.storeId) && validUUID(value.userId) && validUUID(value.createdByUserId) &&
    equal(value.userId, value.ownerUserId as string) && text(value.displayName, 100) && !!value.displayName.trim() && value.removedAt === null)
  return value as unknown as StaffLink
}
function reportResponse(value: unknown): DailyReport {
  keys(value, [...baseFields, 'storeId', 'businessDate', 'amount', 'note'])
  base(value)
  shape(validUUID(value.storeId) && validBusinessDate(value.businessDate) && validAmount(value.amount) &&
    normalizeAmount(value.amount) === value.amount && text(value.note, 8192))
  return value as unknown as DailyReport
}
function memberResponse(value: unknown): Member {
  keys(value, ['id', 'displayName', 'organizationId'])
  shape(validUUID(value.id) && validUUID(value.organizationId) && text(value.displayName, 100) && !!value.displayName.trim())
  return value as unknown as Member
}
function pageResponse<T extends { id: string }>(value: unknown, decode: (value: unknown) => T, query: PageQuery): Page<T> {
  keys(value, ['records', 'total', 'page', 'size'])
  shape(Array.isArray(value.records) && Number.isSafeInteger(value.total) && (value.total as number) >= 0 &&
    value.page === (query.page ?? 1) && value.size === (query.size ?? 20) &&
    value.records.length <= (value.size as number) &&
    value.records.length <= Math.max(0, (value.total as number) - ((value.page as number) - 1) * (value.size as number)))
  const records = value.records.map(decode)
  shape(new Set(records.map(row => row.id.toLowerCase())).size === records.length)
  return { records, total: value.total as number, page: value.page as number, size: value.size as number }
}
function pageQuery(query: PageQuery) {
  shape(query.page === undefined || Number.isInteger(query.page) && query.page >= 1 && query.page <= 1000000)
  shape(query.size === undefined || Number.isInteger(query.size) && query.size >= 1 && query.size <= 50)
}
function dates(query: { dateFrom?: string; dateTo?: string }, required = false) {
  shape((query.dateFrom === undefined && !required || validBusinessDate(query.dateFrom)) &&
    (query.dateTo === undefined && !required || validBusinessDate(query.dateTo)) &&
    (!query.dateFrom || !query.dateTo || query.dateFrom <= query.dateTo))
}
function input(check: () => void) { try { check() } catch { throw validation() } }
function queryString(query: object): string {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) if (value !== undefined) params.set(key, String(value))
  shape(new TextEncoder().encode(params.toString()).length <= 8192)
  return params.size ? '?' + params : ''
}
function storeInput(value: unknown, update: boolean) {
  const fields = ['name', 'businessStatus', 'businessContact', 'managerUserId']
  keys(value, update ? ['version', ...fields] : fields, update ? ['version'] : ['name', 'businessStatus', 'businessContact'])
  shape(!update || validVersion(value.version, true) && fields.some(key => Object.hasOwn(value, key)))
  if (Object.hasOwn(value, 'name')) shape(text(value.name, 200) && !!value.name.trim())
  if (Object.hasOwn(value, 'businessStatus')) shape(value.businessStatus === 'OPEN' || value.businessStatus === 'CLOSED')
  if (Object.hasOwn(value, 'businessContact')) shape(text(value.businessContact, 200))
  if (Object.hasOwn(value, 'managerUserId')) shape(value.managerUserId === null || validUUID(value.managerUserId))
}
function reportInput(value: unknown, update: boolean) {
  keys(value, update ? ['version', 'amount', 'note'] : ['businessDate', 'amount', 'note'], update ? ['version'] : ['businessDate', 'amount'])
  shape(!update || validVersion(value.version, true) && (Object.hasOwn(value, 'amount') || Object.hasOwn(value, 'note')))
  if (!update) shape(validBusinessDate(value.businessDate))
  if (Object.hasOwn(value, 'amount')) shape(validAmount(value.amount))
  if (Object.hasOwn(value, 'note')) shape(text(value.note, 8192))
}

const statusCodes: Readonly<Record<number, readonly string[]>> = {
  400: ['VALIDATION_ERROR'], 403: ['OPERATION_DENIED', 'DATA_DENIED', 'ORIGIN_DENIED'],
  404: ['RESOURCE_NOT_FOUND', 'MEMBER_NOT_FOUND', 'ROUTE_NOT_FOUND'],
  405: ['METHOD_NOT_ALLOWED'], 409: ['STORE_ALREADY_EXISTS', 'STAFF_ALREADY_ASSIGNED', 'DAILY_REPORT_EXISTS', 'VERSION_CONFLICT'],
  413: ['BODY_TOO_LARGE'], 415: ['JSON_REQUIRED'],
  503: ['PLATFORM_UNAVAILABLE', 'AUTHORIZATION_UNAVAILABLE', 'DATABASE_UNAVAILABLE', 'MEMBER_VALIDATION_EXPIRED', 'CREATE_RESULT_UNKNOWN', 'WRITE_RESULT_UNKNOWN'],
}
/** Inspect only a bounded allowlisted business code; upstream messages are never retained. */
async function businessCode(response: Response, signal: AbortSignal): Promise<string | undefined> {
  if (response.status === 401 || response.ok || !response.body?.getReader) return undefined
  const maximum = 8192
  const declared = response.headers.get('Content-Length')
  if (declared && /^\d+$/.test(declared) && Number(declared) > maximum) { void response.body.cancel().catch(() => {}); return undefined }
  const reader = response.body.getReader(), decoder = new TextDecoder('utf-8', { fatal: true })
  let length = 0, content = '', rejectAbort: (reason: unknown) => void = () => {}
  const aborted = new Promise<never>((_, reject) => { rejectAbort = reject })
  const abort = () => { rejectAbort(cancelled()); void reader.cancel().catch(() => {}) }
  signal.addEventListener('abort', abort, { once: true })
  if (signal.aborted) abort()
  try {
    for (;;) {
      const { value, done } = await Promise.race([reader.read(), aborted])
      if (done) break
      length += value.byteLength
      if (length > maximum) return undefined
      content += decoder.decode(value, { stream: true })
    }
    const code: unknown = JSON.parse(content + decoder.decode())?.code
    return typeof code === 'string' && statusCodes[response.status]?.includes(code) ? code : undefined
  } catch { return undefined }
  finally { signal.removeEventListener('abort', abort); void reader.cancel().catch(() => {}); try { reader.releaseLock() } catch {} }
}
function apiLocation(value: string): string {
  if (typeof value !== 'string' || !value || /[\s\0\\?#]/.test(value)) throw new TypeError('门店服务地址无效')
  if (value.startsWith('/') && !value.startsWith('//') && !value.split('/').some(part => part === '.' || part === '..' || /%/i.test(part))) return value.replace(/\/$/, '')
  let url: URL
  try { url = new URL(value) } catch { throw new TypeError('门店服务地址无效') }
  const dev = (import.meta as unknown as { env?: { DEV?: boolean } }).env?.DEV === true || typeof location === 'undefined'
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)
  if (url.username || url.password || url.search || url.hash ||
      !(url.protocol === 'https:' || dev && loopback && url.protocol === 'http:') || /%/.test(url.pathname)) throw new TypeError('门店服务地址无效')
  return url.href.replace(/\/$/, '')
}

export class StoresClient {
  readonly sdkVersion = SDK_VERSION
  private readonly apiBase: string
  private readonly fetchImpl: typeof fetch
  private readonly onInvalidate: () => void
  private readonly requestTimeoutMs: number
  private props: Props | null = null
  private generation = 0
  private lifetime: AbortController | null = null
  private unbind: (() => void) | null = null
  private timer: ReturnType<typeof setTimeout> | null = null
  private authorizationKey: string | null = null
  private hostSignal: AbortSignal | null = null
  private readonly denials = new Set<string>()
  private readonly requests = new Set<{ operation: string; generation: number; controller: AbortController }>()
  private disposed = false
  constructor({ apiBase = '/stores-api', fetchImpl = globalThis.fetch, onInvalidate = () => {}, requestTimeoutMs = 15000 }:
    { apiBase?: string; fetchImpl?: typeof fetch; onInvalidate?: () => void; requestTimeoutMs?: number } = {}) {
    // The injected budget can shorten tests, but cannot extend the production ceiling.
    if (!Number.isSafeInteger(requestTimeoutMs) || requestTimeoutMs < 1 || requestTimeoutMs > 15000) throw new TypeError('请求超时预算无效')
    this.apiBase = apiLocation(apiBase); this.fetchImpl = fetchImpl; this.onInvalidate = onInvalidate
    this.requestTimeoutMs = requestTimeoutMs
  }
  current(): Props | null {
    if (this.props && (!validContext(this.props) || this.lifetime?.signal.aborted)) this.invalidate()
    return this.props
  }
  setContext(props: Props): boolean {
    try { if (props?.sdkVersion !== undefined) assertSupportedSdkVersion(props.sdkVersion) }
    catch { this.invalidate(); throw cancelled() }
    if (this.disposed || !validContext(props)) { this.invalidate(); throw cancelled() }
    const signature = authorizationContextSignature(props)
    if (this.current() && signature !== null && signature === this.authorizationKey && this.hostSignal === props.signal) {
      this.props = props
      return false
    }
    this.invalidate()
    this.denials.clear()
    this.props = props; this.authorizationKey = signature; this.hostSignal = props.signal
    const lifetime = new AbortController()
    this.lifetime = lifetime
    const abort = () => { if (this.lifetime === lifetime) this.invalidate() }
    props.signal.addEventListener('abort', abort, { once: true })
    this.unbind = () => props.signal.removeEventListener('abort', abort)
    const expire = () => {
      if (this.lifetime !== lifetime) return
      const remaining = Math.min(Date.parse(props.expiresAt), Date.parse(props.tokenExpiresAt)) - Date.now()
      if (remaining <= 0) this.invalidate()
      else {
        this.timer = setTimeout(expire, Math.min(remaining, 2147483647))
        ;(this.timer as unknown as { unref?: () => void }).unref?.()
      }
    }
    expire()
    if (props.signal.aborted) abort()
    return true
  }
  private invalidate() {
    const hadContext = !!this.props || !!this.lifetime
    this.generation++
    const lifetime = this.lifetime
    this.props = null; this.lifetime = null; this.authorizationKey = null; this.hostSignal = null
    this.unbind?.(); this.unbind = null
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = null
    lifetime?.abort()
    if (hadContext) this.onInvalidate()
  }
  capture(): ClientSnapshot {
    const props = this.current()
    if (!props || !this.lifetime) throw cancelled()
    return { props, generation: this.generation, signal: this.lifetime.signal }
  }
  isCurrent(snapshot: ClientSnapshot): boolean {
    return !!this.current() && snapshot.generation === this.generation && snapshot.signal === this.lifetime?.signal && !snapshot.signal.aborted
  }
  dispose() { this.disposed = true; this.invalidate(); this.denials.clear() }
  private async request<T>(path: string, operation: Operation, decode: (value: unknown, props: Props) => T,
    options: RequestOptions = {}, method = 'GET', body?: object, expectedStatus = 200): Promise<T> {
    const snapshot = this.capture()
    const operationKey = operation.resource + ':' + operation.action
    if (!can(snapshot.props, operation) || this.denials.has(operationKey)) throw denied()
    const controller = new AbortController()
    const abort = () => controller.abort()
    snapshot.signal.addEventListener('abort', abort, { once: true })
    options.signal?.addEventListener('abort', abort, { once: true })
    if (snapshot.signal.aborted || options.signal?.aborted) abort()
    let receivedForbidden = false, receivedUnauthorized = false
    const check = () => {
      if (!this.isCurrent(snapshot) || controller.signal.aborted || this.denials.has(operationKey) && !receivedForbidden) throw cancelled()
    }
    let sent = false, observedCode: string | undefined, observedStatus = 0, requestId: string | undefined
    const write = method !== 'GET'
    const unknownCode = method === 'POST' ? 'CREATE_RESULT_UNKNOWN' : 'WRITE_RESULT_UNKNOWN'
    let timedOut = false
    // One deadline covers connection, headers and every body chunk, including error-code inspection.
    const timeout = setTimeout(() => { timedOut = true; controller.abort() }, this.requestTimeoutMs)
    const active = { operation: operationKey, generation: snapshot.generation, controller }
    this.requests.add(active)
    try {
      check()
      const serialized = body === undefined ? undefined : JSON.stringify(body)
      if (serialized !== undefined && new TextEncoder().encode(serialized).length > 65536) throw validation()
      const fetchRequest = this.fetchImpl
      const transport: typeof fetch = async (url, init) => {
        check(); sent = true
        requestId = new Headers(init?.headers).get('X-Request-Id') ?? undefined
        const response = await fetchRequest(url, { ...init, redirect: 'error', cache: 'no-store' })
        check()
        observedStatus = response?.status
        const echoed = response?.headers?.get('X-Request-Id')
        if (validUUID(echoed)) requestId = echoed
        if (observedStatus === 403) {
          // The status is authoritative now. An untrusted body cannot delay revocation,
          // and a parallel success for this exact capability must never refill its views.
          receivedForbidden = true
          this.denials.add(operationKey)
          for (const request of this.requests) {
            if (request !== active && request.generation === snapshot.generation && request.operation === operationKey) request.controller.abort()
          }
          void response.body?.cancel().catch(() => {})
          return response
        }
        if (observedStatus === 401) {
          receivedUnauthorized = true
          this.invalidate()
          void response.body?.cancel().catch(() => {})
          return response
        }
        observedCode = await businessCode(response, controller.signal)
        return response
      }
      const result = await requestWithContext({ props: snapshot.props, url: this.apiBase + path,
        options: { method, headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
          signal: controller.signal, body: serialized },
        fetchImpl: transport, check, maxBytes: 4 * 1024 * 1024 })
      check()
      try { shape(observedStatus === expectedStatus); return decode(result, snapshot.props) }
      catch { throw new MicroSdkError('PROTOCOL', { status: observedStatus, requestId, code: 'INVALID_BUSINESS_RESPONSE' }) }
    } catch (error) {
      if (receivedForbidden) throw new BusinessError(403, 'OPERATION_DENIED', 'FORBIDDEN', requestId)
      if (receivedUnauthorized) throw new BusinessError(401, 'INVALID_CONTEXT', 'UNAUTHORIZED', requestId)
      if (this.isCurrent(snapshot) && this.denials.has(operationKey) && !options.signal?.aborted) {
        // An already-dispatched concurrent write may have committed before its sibling
        // was denied. Fence its result while retaining the explicit readback requirement.
        throw write && sent ? new BusinessError(0, unknownCode, 'CANCELLED', requestId, true) : denied()
      }
      if (timedOut && this.isCurrent(snapshot) && !options.signal?.aborted) {
        throw new BusinessError(408, 'REQUEST_TIMEOUT', 'TIMEOUT', requestId, write && sent)
      }
      if (error instanceof BusinessError) throw error
      if (error instanceof MicroSdkError && this.isCurrent(snapshot) && !controller.signal.aborted) {
        if (error.status === 401) this.invalidate()
        // A forbidden pair does not revoke unrelated operations. The controller clears
        // that operation's views when it catches this error; full invalidation is separate.
        if (error.status === 403) this.denials.add(operationKey)
        const unknown = write && sent && (error.kind === 'NETWORK' || error.kind === 'PROTOCOL' || error.status >= 500)
        const status = error.kind === 'NETWORK' ? 503 : error.kind === 'PROTOCOL' && error.status !== 413 ? 502 : error.status
        const code = observedCode ?? (unknown ? unknownCode : error.status === 401 ? 'INVALID_CONTEXT' : error.status === 403 ? 'OPERATION_DENIED' : error.code)
        throw new BusinessError(status, code, error.kind, requestId ?? error.requestId, unknown)
      }
      if (write && sent) throw new BusinessError(0, unknownCode, 'CANCELLED', requestId, true)
      throw cancelled()
    } finally {
      clearTimeout(timeout)
      this.requests.delete(active)
      snapshot.signal.removeEventListener('abort', abort)
      options.signal?.removeEventListener('abort', abort)
    }
  }
  private row<T extends ScopedRow>(value: T, props: Props, operation: Operation): T { shape(can(props, operation, value)); return value }
  listStores(query: StoreQuery = {}, options?: RequestOptions): Promise<Page<Store>> {
    input(() => { keys(query, ['page', 'size', 'q', 'businessStatus', 'organizationId'], []); pageQuery(query)
      shape(query.q === undefined || text(query.q, 200)); shape(query.businessStatus === undefined || query.businessStatus === 'OPEN' || query.businessStatus === 'CLOSED')
      shape(query.organizationId === undefined || validUUID(query.organizationId)) })
    return this.request('/stores' + queryString(query), { resource: 'store', action: 'read' }, (value, props) =>
      pageResponse(value, raw => { const row = this.row(storeResponse(raw), props, { resource: 'store', action: 'read' })
        shape(!query.organizationId || equal(query.organizationId, row.organizationId)); shape(!query.businessStatus || query.businessStatus === row.businessStatus); return row }, query), options)
  }
  getStore(id: UUID, options?: RequestOptions): Promise<Store> {
    input(() => shape(validUUID(id)))
    return this.request('/stores/' + id, { resource: 'store', action: 'read' }, (value, props) => {
      const row = this.row(storeResponse(value), props, { resource: 'store', action: 'read' }); shape(equal(row.id, id)); return row
    }, options)
  }
  createStore(body: CreateStore, options?: RequestOptions): Promise<Store> {
    input(() => storeInput(body, false))
    body = { ...body }
    return this.request('/stores', { resource: 'store', action: 'create' }, (value, props) => {
      const row = this.row(storeResponse(value), props, { resource: 'store', action: 'create' })
      shape(row.version === 1 && equal(row.organizationId, props.organization.id) && equal(row.ownerUserId, props.user.id)); return row
    }, options, 'POST', body, 201)
  }
  updateStore(id: UUID, body: UpdateStore, options?: RequestOptions): Promise<Store> {
    input(() => { shape(validUUID(id)); storeInput(body, true) })
    body = { ...body }
    return this.request('/stores/' + id, { resource: 'store', action: 'update' }, (value, props) => {
      const row = this.row(storeResponse(value), props, { resource: 'store', action: 'update' }); shape(equal(row.id, id) && row.version === body.version + 1); return row
    }, options, 'PATCH', body)
  }
  listStaff(storeId: UUID, query: PageQuery = {}, options?: RequestOptions): Promise<Page<StaffLink>> {
    input(() => { shape(validUUID(storeId)); keys(query, ['page', 'size'], []); pageQuery(query) })
    return this.request('/stores/' + storeId + '/staff' + queryString(query), { resource: 'store-staff', action: 'read' }, (value, props) =>
      pageResponse(value, raw => { const row = this.row(staffResponse(raw), props, { resource: 'store-staff', action: 'read' }); shape(equal(row.storeId, storeId)); return row }, query), options)
  }
  assignStaff(storeId: UUID, body: { userId: UUID }, options?: RequestOptions): Promise<StaffLink> {
    input(() => { shape(validUUID(storeId)); keys(body, ['userId']); shape(validUUID(body.userId)) })
    body = { ...body }
    return this.request('/stores/' + storeId + '/staff', { resource: 'store-staff', action: 'assign' }, (value, props) => {
      const row = this.row(staffResponse(value), props, { resource: 'store-staff', action: 'assign' })
      shape(equal(row.storeId, storeId) && equal(row.userId, body.userId) && equal(row.createdByUserId, props.user.id) && row.version === 1); return row
    }, options, 'POST', body, 201)
  }
  unassignStaff(storeId: UUID, linkId: UUID, body: { version: number }, options?: RequestOptions): Promise<{ ok: true }> {
    input(() => { shape(validUUID(storeId) && validUUID(linkId)); keys(body, ['version']); shape(validVersion(body.version, true)) })
    body = { ...body }
    return this.request('/stores/' + storeId + '/staff/' + linkId, { resource: 'store-staff', action: 'unassign' }, value => {
      keys(value, ['ok']); shape(value.ok === true); return { ok: true }
    }, options, 'DELETE', body)
  }
  listDailyReports(storeId: UUID, query: ReportQuery = {}, options?: RequestOptions): Promise<Page<DailyReport>> {
    input(() => { shape(validUUID(storeId)); keys(query, ['page', 'size', 'dateFrom', 'dateTo'], []); pageQuery(query); dates(query) })
    return this.request('/stores/' + storeId + '/daily-reports' + queryString(query), { resource: 'store-daily-report', action: 'read' }, (value, props) =>
      pageResponse(value, raw => { const row = this.row(reportResponse(raw), props, { resource: 'store-daily-report', action: 'read' })
        shape(equal(row.storeId, storeId) && (!query.dateFrom || row.businessDate >= query.dateFrom) && (!query.dateTo || row.businessDate <= query.dateTo)); return row }, query), options)
  }
  getDailyReport(id: UUID, options?: RequestOptions): Promise<DailyReport> {
    input(() => shape(validUUID(id)))
    return this.request('/daily-reports/' + id, { resource: 'store-daily-report', action: 'read' }, (value, props) => {
      const row = this.row(reportResponse(value), props, { resource: 'store-daily-report', action: 'read' }); shape(equal(row.id, id)); return row
    }, options)
  }
  createDailyReport(storeId: UUID, body: CreateDailyReport, options?: RequestOptions): Promise<DailyReport> {
    input(() => { shape(validUUID(storeId)); reportInput(body, false) })
    body = { ...body }
    return this.request('/stores/' + storeId + '/daily-reports', { resource: 'store-daily-report', action: 'create' }, (value, props) => {
      const row = this.row(reportResponse(value), props, { resource: 'store-daily-report', action: 'create' })
      shape(equal(row.storeId, storeId) && row.businessDate === body.businessDate && row.version === 1 && equal(row.ownerUserId, props.user.id)); return row
    }, options, 'POST', body, 201)
  }
  updateDailyReport(id: UUID, body: UpdateDailyReport, options?: RequestOptions): Promise<DailyReport> {
    input(() => { shape(validUUID(id)); reportInput(body, true) })
    body = { ...body }
    return this.request('/daily-reports/' + id, { resource: 'store-daily-report', action: 'update' }, (value, props) => {
      const row = this.row(reportResponse(value), props, { resource: 'store-daily-report', action: 'update' }); shape(equal(row.id, id) && row.version === body.version + 1); return row
    }, options, 'PATCH', body)
  }
  getSummary(query: SummaryQuery, options?: RequestOptions): Promise<Summary> {
    input(() => { keys(query, ['dateFrom', 'dateTo', 'storeId'], ['dateFrom', 'dateTo']); dates(query, true); shape(query.storeId === undefined || validUUID(query.storeId)) })
    return this.request('/daily-reports/summary' + queryString(query), { resource: 'store-daily-report', action: 'read' }, value => {
      keys(value, ['count', 'totalAmount', 'dateFrom', 'dateTo', 'storeId'])
      shape(Number.isSafeInteger(value.count) && (value.count as number) >= 0 && typeof value.totalAmount === 'string' &&
        /^(0|[1-9]\d*)\.\d{2}$/.test(value.totalAmount) && value.dateFrom === query.dateFrom && value.dateTo === query.dateTo &&
        (query.storeId ? validUUID(value.storeId) && equal(value.storeId, query.storeId) : value.storeId === null) &&
        (value.count !== 0 || value.totalAmount === '0.00'))
      return value as unknown as Summary
    }, options)
  }
  listMembers(purpose: MemberPurpose, query: MemberQuery = {}, options?: RequestOptions): Promise<Page<Member>> {
    input(() => { keys(purpose, ['resource', 'action']); keys(query, ['page', 'size', 'organizationId', 'q'], []); pageQuery(query)
      shape(query.organizationId === undefined || validUUID(query.organizationId)); shape(query.q === undefined || text(query.q, 100)) })
    const props = this.current(), organizationId = query.organizationId ?? props?.organization.id
    if (!props || !organizationId || !canMemberLookup(props, purpose, organizationId)) throw denied()
    return this.request('/members' + queryString({ resource: purpose.resource, action: purpose.action, ...query, organizationId }), purpose,
      value => pageResponse(value, raw => { const member = memberResponse(raw); shape(equal(member.organizationId, organizationId)); return member }, query), options)
  }
}
