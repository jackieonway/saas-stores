import type { MicroProps } from './sdk.ts'

export const APP_CODE = 'stores'
export type Props = MicroProps
export type UUID = string
export type BusinessDate = string
export type DecimalAmount = string
export type BusinessStatus = 'OPEN' | 'CLOSED'
export type Operation =
  | { resource: 'store'; action: 'read' | 'create' | 'update' }
  | { resource: 'store-staff'; action: 'read' | 'assign' | 'unassign' }
  | { resource: 'store-daily-report'; action: 'read' | 'create' | 'update' }
export type MemberPurpose =
  | { resource: 'store'; action: 'create' | 'update' }
  | { resource: 'store-staff'; action: 'assign' }
export interface ScopedRow { tenantId: UUID; organizationId: UUID; ownerUserId: UUID }
export interface VersionedRow extends ScopedRow { id: UUID; version: number; createdAt: string; updatedAt: string }
export interface Store extends VersionedRow {
  name: string; businessStatus: BusinessStatus; businessContact: string
  managerUserId: UUID | null; managerDisplayName: string | null
}
export interface StaffLink extends VersionedRow {
  storeId: UUID; userId: UUID; displayName: string; createdByUserId: UUID; removedAt: null
}
export interface DailyReport extends VersionedRow { storeId: UUID; businessDate: BusinessDate; amount: DecimalAmount; note: string }
export interface Member { id: UUID; displayName: string; organizationId: UUID }
export interface Page<T> { records: T[]; total: number; page: number; size: number }
export interface Summary { count: number; totalAmount: DecimalAmount; dateFrom: BusinessDate; dateTo: BusinessDate; storeId: UUID | null }

export const validUUID = (value: unknown): value is UUID => typeof value === 'string' &&
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value) &&
  value !== '00000000-0000-0000-0000-000000000000'
export function validBusinessDate(value: unknown): value is BusinessDate {
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return false
  const [year, month, day] = value.split('-').map(Number)
  if (year < 1 || month < 1 || month > 12 || day < 1) return false
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0)
  return day <= [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1]!
}
export const validAmount = (value: unknown): value is DecimalAmount => typeof value === 'string' && /^\d{1,16}(?:\.\d{1,2})?$/.test(value)
export function normalizeAmount(value: unknown): DecimalAmount | null {
  if (!validAmount(value)) return null
  const [whole, fraction = ''] = value.split('.')
  return whole!.replace(/^0+(?=\d)/, '') + '.' + fraction.padEnd(2, '0')
}
export function shanghaiToday(now = Date.now()): BusinessDate {
  const date = new Date(now + 8 * 60 * 60 * 1000)
  if (!Number.isFinite(date.getTime())) throw new TypeError('营业日期无效')
  return date.toISOString().slice(0, 10)
}

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value)
const ids = (value: unknown): value is string[] => Array.isArray(value) && value.length <= 10000 && value.every(validUUID)
const equal = (left: string, right: string) => left.toLowerCase() === right.toLowerCase()
const includes = (values: string[], id: string) => values.some(value => equal(value, id))
export function validContext(props: unknown, now = Date.now()): props is Props {
  if (!object(props)) return false
  const signal = props.signal as AbortSignal | undefined
  return props.contractVersion === '2.0' && (props.sdkVersion === undefined || props.sdkVersion === '1.0.0') &&
    props.appCode === APP_CODE && props.basePath === '/micro/stores' &&
    Number.isSafeInteger(props.contextEpoch) && (props.contextEpoch as number) > 0 &&
    typeof props.contextVersion === 'string' && props.contextVersion.length > 0 &&
    typeof props.token === 'string' && props.token.length > 0 && !/[\r\n\0]/.test(props.token) &&
    !!signal && typeof signal.addEventListener === 'function' && typeof signal.removeEventListener === 'function' && signal.aborted === false &&
    typeof props.expiresAt === 'string' && Date.parse(props.expiresAt) > now &&
    typeof props.tokenExpiresAt === 'string' && Date.parse(props.tokenExpiresAt) > now &&
    object(props.user) && validUUID(props.user.id) && object(props.tenant) && validUUID(props.tenant.id) &&
    object(props.organization) && validUUID(props.organization.id) &&
    Array.isArray(props.permissions) && props.permissions.every(item => typeof item === 'string') &&
    Array.isArray(props.dataGrants) && Array.isArray(props.menus) &&
    (props.organizationScopeIds == null || (ids(props.organizationScopeIds) && includes(props.organizationScopeIds, props.organization.id)))
}
/** Draft partition only: credentials and authorization signatures never enter this key. */
export function identityKey(props: Props): string {
  return JSON.stringify([props.appCode, props.tenant.id.toLowerCase(), props.organization.id.toLowerCase(), props.user.id.toLowerCase()])
}
function realOperation(operation: Operation): boolean {
  return !!operation && (operation.resource === 'store' || operation.resource === 'store-daily-report'
    ? ['read', 'create', 'update'].includes(operation.action)
    : operation.resource === 'store-staff' && ['read', 'assign', 'unassign'].includes(operation.action))
}
function grantFor(props: Props | null, operation: Operation) {
  if (!validContext(props) || !realOperation(operation) || !props.permissions.includes(operation.resource + ':' + operation.action)) return null
  const matching = props.dataGrants.filter(grant => grant?.resource === operation.resource && grant?.action === operation.action)
  if (matching.length !== 1) return null
  const grant = matching[0]!
  if (!validUUID(grant.subjectUserId) || !equal(grant.subjectUserId, props.user.id) ||
      !validUUID(grant.tenantId) || !equal(grant.tenantId, props.tenant.id) || !ids(grant.organizationIds)) return null
  const unique = [...new Set(grant.organizationIds.map(id => id.toLowerCase()))]
  const current = props.organization.id.toLowerCase()
  if (grant.scope === 'SELF' && (unique.length === 0 || unique.includes(current)) ||
      grant.scope === 'ORG' && unique.length === 1 && unique[0] === current ||
      grant.scope === 'ORG_SUBTREE' && unique.includes(current) || grant.scope === 'TENANT' && unique.length === 0) return grant
  return null
}
function withinCeiling(props: Props, organizationId: UUID): boolean {
  return props.organizationScopeIds === null || (props.organizationScopeIds === undefined
    ? equal(organizationId, props.organization.id) : includes(props.organizationScopeIds, organizationId))
}
export function can(props: Props | null, operation: Operation, row?: ScopedRow): boolean {
  const grant = grantFor(props, operation)
  if (!grant || !props) return false
  if (!row) return true
  if (!validUUID(row.tenantId) || !validUUID(row.organizationId) || !validUUID(row.ownerUserId) ||
      !equal(row.tenantId, props.tenant.id) || !withinCeiling(props, row.organizationId)) return false
  if (grant.scope === 'SELF') return equal(row.ownerUserId, props.user.id) &&
    (grant.organizationIds.length === 0 || includes(grant.organizationIds, row.organizationId))
  if (grant.scope === 'ORG') return equal(row.organizationId, props.organization.id)
  if (grant.scope === 'ORG_SUBTREE') return includes(grant.organizationIds, row.organizationId)
  return grant.scope === 'TENANT'
}
export function canMemberLookup(props: Props | null, purpose: MemberPurpose, organizationId: UUID): boolean {
  if (!purpose || !(purpose.resource === 'store' && ['create', 'update'].includes(purpose.action) ||
      purpose.resource === 'store-staff' && purpose.action === 'assign')) return false
  const grant = grantFor(props, purpose)
  if (!grant || !props || !validUUID(organizationId) || !withinCeiling(props, organizationId)) return false
  if (grant.scope === 'SELF' || grant.scope === 'ORG') return equal(organizationId, props.organization.id)
  return grant.scope === 'TENANT' || includes(grant.organizationIds, organizationId)
}

export type StoresRoute =
  | { kind: 'stores' | 'store-create' | 'staff' | 'staff-assign' | 'reports' | 'report-create' | 'not-found' }
  | { kind: 'store-detail' | 'report-detail'; id: UUID }
const routePaths: Record<string, { kind: StoresRoute['kind']; permission: string }> = {
  '/micro/stores': { kind: 'stores', permission: '' },
  '/micro/stores/store-create': { kind: 'store-create', permission: 'store:create' },
  '/micro/stores/stores/:id': { kind: 'store-detail', permission: 'store:read' },
  '/micro/stores/staff': { kind: 'staff', permission: 'store-staff:read' },
  '/micro/stores/staff-assign': { kind: 'staff-assign', permission: 'store-staff:assign' },
  '/micro/stores/reports': { kind: 'reports', permission: 'store-daily-report:read' },
  '/micro/stores/daily-report-create': { kind: 'report-create', permission: 'store-daily-report:create' },
  '/micro/stores/daily-reports/:id': { kind: 'report-detail', permission: 'store-daily-report:read' },
}
function routeMatch(routePath: string): { template: string; route: StoresRoute } | null {
  if (typeof routePath !== 'string') return null
  const path = routePath.endsWith('/') ? routePath.slice(0, -1) : routePath
  const exact = routePaths[path]
  if (exact && !path.includes(':')) return { template: path, route: { kind: exact.kind } as StoresRoute }
  const match = /^\/micro\/stores\/(stores|daily-reports)\/([^/]+)$/.exec(path)
  if (!match || !validUUID(match[2])) return null
  return { template: '/micro/stores/' + match[1] + '/:id',
    route: { kind: match[1] === 'stores' ? 'store-detail' : 'report-detail', id: match[2].toLowerCase() } }
}
export function routeView(routePath: string): StoresRoute { return routeMatch(routePath)?.route ?? { kind: 'not-found' } }
export function canRoute(props: Props | null, routePath: string): boolean {
  const matched = routeMatch(routePath)
  if (!validContext(props) || !matched) return false
  const expected = routePaths[matched.template]!
  const pending: unknown[] = [...props.menus]
  const visited = new Set<unknown>()
  while (pending.length && visited.size < 10000) {
    const menu = pending.pop()
    if (!object(menu) || visited.has(menu)) continue
    visited.add(menu)
    if (menu.type === 'MENU' && menu.path === matched.template &&
        (expected.permission === '' || menu.permission === expected.permission && props.permissions.includes(expected.permission))) return true
    if (Array.isArray(menu.children)) pending.push(...menu.children)
  }
  return false
}
