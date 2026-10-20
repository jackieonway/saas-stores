import test from 'node:test'
import assert from 'node:assert/strict'
import { can, canMemberLookup, canRoute, routeView, identityKey, validContext,
  validUUID, validBusinessDate, validAmount, normalizeAmount, shanghaiToday } from '../src/domain.ts'
import { context, grantContext, operations, baseRow, id, user, tenant, organization, child, outside, staff } from './domain.fixture.mjs'

test('all nine exact operation pairs require their own permission and a unique identity-bound grant', () => {
  for (const operation of operations) {
    const props = grantContext(operation)
    assert.equal(can(props, operation, baseRow), true)
    assert.equal(can({ ...props, permissions: [] }, operation), false)
    assert.equal(can({ ...props, dataGrants: [] }, operation), false)
    assert.equal(can({ ...props, dataGrants: [...props.dataGrants, props.dataGrants[0]] }, operation), false)
    for (const change of [{ tenantId: outside }, { subjectUserId: outside }, { scope: 'STORE' }, { organizationIds: [outside] }]) {
      assert.equal(can({ ...props, dataGrants: [{ ...props.dataGrants[0], ...change }] }, operation), false)
    }
    for (const other of operations.filter(item => item.resource !== operation.resource || item.action !== operation.action)) assert.equal(can(props, other), false)
  }
  for (const bad of [{ resource: 'store', action: 'delete' }, { resource: 'store-daily-report', action: 'summary' }, { resource: 'store-staff', action: 'update' }]) assert.equal(can(context(), bad), false)
})

test('the organization ceiling intersects every scope; missing ceiling means current organization only', () => {
  const operation = { resource: 'store', action: 'read' }
  for (const scope of ['SELF', 'ORG', 'ORG_SUBTREE', 'TENANT']) {
    const props = grantContext(operation, scope)
    assert.equal(can(props, operation, { ...baseRow, organizationId: outside }), false, scope)
    assert.equal(can(props, operation, { ...baseRow, tenantId: outside }), false, scope)
  }
  const broad = grantContext(operation, 'TENANT')
  assert.equal(can(broad, operation, { ...baseRow, organizationId: child }), true)
  assert.equal(can({ ...broad, organizationScopeIds: undefined }, operation, { ...baseRow, organizationId: child }), false)
  assert.equal(can({ ...broad, organizationScopeIds: null }, operation, { ...baseRow, organizationId: outside }), true)
  const subtree = grantContext(operation, 'ORG_SUBTREE')
  assert.equal(can(subtree, operation, { ...baseRow, organizationId: child }), true)
  assert.equal(can({ ...subtree, organizationScopeIds: [organization] }, operation, { ...baseRow, organizationId: child }), false)
})

test('SELF follows each actual resource owner, not manager or staff creator', () => {
  for (const operation of [{ resource: 'store', action: 'update' }, { resource: 'store-daily-report', action: 'read' }]) {
    const props = grantContext(operation, 'SELF')
    assert.equal(can(props, operation, baseRow), true)
    assert.equal(can(props, operation, { ...baseRow, ownerUserId: outside, managerUserId: user }), false)
  }
  const operation = { resource: 'store-staff', action: 'unassign' }, props = grantContext(operation, 'SELF')
  assert.equal(staff.createdByUserId, user)
  assert.equal(can(props, operation, staff), false)
  assert.equal(can(props, operation, { ...staff, ownerUserId: user, userId: user, createdByUserId: outside }), true)
  const restricted = { ...props, dataGrants: [{ ...props.dataGrants[0], organizationIds: [organization] }] }
  assert.equal(can(restricted, operation, { ...baseRow, organizationId: child }), false)
})

test('member lookup has exactly three purposes and independently limits target organization', () => {
  const purposes = [{ resource: 'store', action: 'create' }, { resource: 'store', action: 'update' }, { resource: 'store-staff', action: 'assign' }]
  for (const purpose of purposes) {
    for (const scope of ['SELF', 'ORG', 'ORG_SUBTREE', 'TENANT']) {
      const props = grantContext(purpose, scope)
      assert.equal(canMemberLookup(props, purpose, organization), true)
      assert.equal(canMemberLookup(props, purpose, child), scope === 'ORG_SUBTREE' || scope === 'TENANT')
      assert.equal(canMemberLookup(props, purpose, outside), false)
    }
  }
  for (const operation of operations.filter(operation => !purposes.some(purpose => purpose.resource === operation.resource && purpose.action === operation.action))) assert.equal(canMemberLookup(context(), operation, organization), false)
})

test('frozen MENU routes authorize independently; BUTTON, missing MENU and unknown paths cannot stand in', () => {
  const props = context()
  const paths = [
    ['/micro/stores', 'stores'], ['/micro/stores/store-create', 'store-create'],
    ['/micro/stores/stores/' + id(10), 'store-detail'], ['/micro/stores/staff', 'staff'],
    ['/micro/stores/staff-assign', 'staff-assign'], ['/micro/stores/reports', 'reports'],
    ['/micro/stores/daily-report-create', 'report-create'], ['/micro/stores/daily-reports/' + id(12), 'report-detail'],
  ]
  for (const [path, kind] of paths) {
    assert.equal(routeView(path).kind, kind)
    assert.equal(canRoute(props, path), true)
    assert.equal(canRoute({ ...props, menus: [] }, path), false)
    assert.equal(canRoute({ ...props, menus: props.menus.map(menu => ({ ...menu, type: 'BUTTON' })) }, path), false)
  }
  assert.equal(canRoute({ ...props, permissions: [], dataGrants: [] }, '/micro/stores'), true)
  assert.equal(can({ ...props, permissions: [], dataGrants: [] }, { resource: 'store', action: 'read' }), false)
  assert.equal(canRoute({ ...props, permissions: [] }, '/micro/stores/store-create'), false)
  assert.equal(canRoute({ ...props, menus: [{ ...props.menus[0], type: 'DIR', children: [props.menus[2]] }] }, '/micro/stores/store-create'), true)
  for (const path of ['/elsewhere', '/micro/stores/missing', '/micro/stores/stores/store-create', '/micro/stores/stores/:id', '/micro/stores/daily-report-create/extra', '/micro/stores/daily-reports/summary']) {
    assert.deepEqual(routeView(path), { kind: 'not-found' }); assert.equal(canRoute(props, path), false)
  }
})

test('dates are real civil dates and today changes at Shanghai midnight without local timezone assumptions', () => {
  for (const date of ['0001-01-01', '9999-12-31', '2024-02-29', '2000-02-29']) assert.equal(validBusinessDate(date), true)
  for (const date of ['0000-01-01', '2026-02-29', '1900-02-29', '2026-2-01', '2026-04-31', '2026-00-01', '2026-10-18T00:00:00+08:00', null]) assert.equal(validBusinessDate(date), false)
  assert.equal(shanghaiToday(Date.parse('2026-10-18T15:59:59.999Z')), '2026-10-18')
  assert.equal(shanghaiToday(Date.parse('2026-10-18T16:00:00.000Z')), '2026-10-19')
})

test('amounts retain exact cents above Number precision and never round or coerce', () => {
  for (const [amount, expected] of [['0', '0.00'], ['00012.3', '12.30'], ['9007199254740993.01', '9007199254740993.01'], ['9999999999999999.99', '9999999999999999.99']]) {
    assert.equal(validAmount(amount), true); assert.equal(normalizeAmount(amount), expected)
  }
  for (const amount of [0, '-0.01', '+1', ' 1', '1 ', '1e2', '1,000', '.01', '1.', '1.001', '10000000000000000', '１２', null]) {
    assert.equal(validAmount(amount), false); assert.equal(normalizeAmount(amount), null)
  }
})

test('context validation and draft identity reject invalid credentials without placing them into draft keys', () => {
  const props = context()
  assert.equal(validContext(props), true)
  for (const change of [{ appCode: 'work-orders' }, { token: '' }, { token: 'a\nb' }, { contextEpoch: 0 },
    { expiresAt: '2000-01-01T00:00:00Z' }, { tokenExpiresAt: '2000-01-01T00:00:00Z' },
    { organizationScopeIds: [child] }, { user: { id: 'bad' } }, { signal: {} }]) assert.equal(validContext({ ...props, ...change }), false)
  const controller = new AbortController(); controller.abort()
  assert.equal(validContext({ ...props, signal: controller.signal }), false)
  const key = identityKey(props)
  assert.deepEqual(JSON.parse(key), ['stores', tenant, organization, user])
  assert.equal(key.includes(props.token), false)
  assert.equal(identityKey({ ...props, token: 'renewed' }), key)
  assert.notEqual(identityKey({ ...props, organization: { ...props.organization, id: outside } }), key)
  assert.equal(validUUID(id(1)), true); assert.equal(validUUID('00000000-0000-0000-0000-000000000000'), false)
})
