export const id = number => 'a0000000-0000-4000-8000-' + number.toString(16).padStart(12, '0')
export const tenant = id(1), organization = id(2), user = id(3), child = id(4), outside = id(5)
export const storeId = id(10), staffId = id(11), reportId = id(12), memberId = id(13)
export const operations = [
  ...['read', 'create', 'update'].map(action => ({ resource: 'store', action })),
  ...['read', 'assign', 'unassign'].map(action => ({ resource: 'store-staff', action })),
  ...['read', 'create', 'update'].map(action => ({ resource: 'store-daily-report', action })),
]
export const menuDefinitions = [
  ['/micro/stores', ''], ['/micro/stores/stores/:id', 'store:read'],
  ['/micro/stores/store-create', 'store:create'], ['/micro/stores/staff', 'store-staff:read'],
  ['/micro/stores/staff-assign', 'store-staff:assign'], ['/micro/stores/reports', 'store-daily-report:read'],
  ['/micro/stores/daily-reports/:id', 'store-daily-report:read'],
  ['/micro/stores/daily-report-create', 'store-daily-report:create'],
]
export function context(overrides = {}) {
  return {
    sdkVersion: '1.0.0', contractVersion: '2.0', appCode: 'stores', basePath: '/micro/stores',
    contextVersion: 'a'.repeat(64), contextEpoch: 1, routePath: '/micro/stores',
    token: 'test-app-bearer', expiresAt: new Date(Date.now() + 60000).toISOString(),
    tokenExpiresAt: new Date(Date.now() + 120000).toISOString(),
    user: { id: user, username: 'ordinary', displayName: '业务人员' },
    tenant: { id: tenant, code: 'tenant', name: '测试租户' },
    organization: { id: organization, name: '组织甲', path: '/' + organization },
    organizationScopeIds: [organization, child], roles: [], theme: 'light',
    permissions: operations.map(item => item.resource + ':' + item.action),
    dataGrants: operations.map(item => ({ ...item, scope: 'ORG', subjectUserId: user, tenantId: tenant, organizationIds: [organization] })),
    menus: menuDefinitions.map(([path, permission], index) => ({ id: id(100 + index), parentId: null,
      type: 'MENU', title: path, path, icon: null, sortOrder: index, visible: true, permission })),
    signal: new AbortController().signal, onNavigate() {}, onNotify() {}, ...overrides,
  }
}
export function grantContext(operation, scope = 'ORG', overrides = {}) {
  const props = context(overrides)
  return { ...props, permissions: [operation.resource + ':' + operation.action], dataGrants: [{
    ...operation, scope, subjectUserId: props.user.id, tenantId: props.tenant.id,
    organizationIds: scope === 'TENANT' || scope === 'SELF' ? [] : scope === 'ORG_SUBTREE' ? [organization, child] : [organization],
  }] }
}
export const baseRow = { tenantId: tenant, organizationId: organization, ownerUserId: user,
  version: 1, createdAt: '2026-10-18T01:00:00.000Z', updatedAt: '2026-10-18T01:00:00.000Z' }
export const store = { ...baseRow, id: storeId, name: '城北门店', businessStatus: 'OPEN', businessContact: '', managerUserId: null, managerDisplayName: null }
export const staff = { ...baseRow, id: staffId, storeId, userId: memberId, ownerUserId: memberId,
  createdByUserId: user, displayName: '门店员工', removedAt: null }
export const report = { ...baseRow, id: reportId, storeId, businessDate: '2026-10-18', amount: '9999999999999999.99', note: '营业录入' }
export const member = { id: memberId, organizationId: organization, displayName: '门店员工' }
export const page = (row, pageNumber = 1, size = 20) => ({ records: row ? [row] : [], total: row ? 1 : 0, page: pageNumber, size })
export const response = (body, status = 200, headers = {}) => new Response(JSON.stringify(body), { status,
  headers: { 'Content-Type': 'application/json', 'X-Request-Id': id(999), ...headers } })
export const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no }); return { promise, resolve, reject } }
