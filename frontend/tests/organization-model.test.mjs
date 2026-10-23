import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { renderToStaticMarkup } from 'react-dom/server'
import React from 'react'
import ts from 'typescript'
import { BusinessError, StoresClient } from '../src/client.ts'
import { StoresController } from '../src/controller.ts'
import { context, grantContext, platformOrganization, organizationPage, store, report, member, page, response, deferred,
  organization, child, outside, user, memberId, storeId } from './domain.fixture.mjs'

const create = { resource: 'store', action: 'create' }
const childOrganization = { ...platformOrganization, id: child, parentId: organization, name: '华东直营店', code: 'EAST-STORE', path: '/总部/华东直营店' }
const childStore = { ...store, organizationId: child, name: childOrganization.name, organization: childOrganization }
const subtreeContext = () => {
  const props = context({ routePath: '/micro/stores/store-create' })
  return { ...props, dataGrants: props.dataGrants.map(grant => ({ ...grant, scope: 'ORG_SUBTREE', organizationIds: [organization, child] })) }
}
const tick = async () => { await new Promise(resolve => setImmediate(resolve)); await new Promise(resolve => setImmediate(resolve)) }
function client(fetchImpl, props = subtreeContext()) { const value = new StoresClient({ fetchImpl }); value.setContext(props); return value }

// Render the real React views without a second bundler or a DOM implementation.
let source = ts.transpileModule(readFileSync(new URL('../src/app.tsx', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX },
}).outputText.replace(/^import ['"]\.\/styles\.css['"];?\s*$/m, '')
source = source.replace(/(from\s*['"])([^'"]+)(['"])/g, (all, start, specifier, end) =>
  start + (specifier.startsWith('.') ? new URL('../src/' + specifier.slice(2), import.meta.url).href : import.meta.resolve(specifier)) + end)
const { StoresApp } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'))
const render = controller => renderToStaticMarkup(React.createElement(StoresApp, { controller }))

test('STORE directory includes platform organizations with no business profile and uses its explicit purpose', async () => {
  let request
  const api = client(async (url, init) => { request = { url, init }; return response(organizationPage(childOrganization, 1, 20)) })
  try {
    const result = await api.listOrganizations(create, { page: 1, size: 20, q: '华东' })
    assert.equal(result.records[0].id, child)
    assert.equal(result.records[0].organizationType, 'STORE')
    assert.match(request.url, /\/organizations\?resource=store&action=create&page=1&size=20&q=/)
    assert.equal(request.init.credentials, 'omit')
    assert.equal(request.init.headers.get('Authorization'), 'Bearer test-app-bearer')
  } finally { api.dispose() }
})

test('directory rejects foreign, non-STORE, malformed and mismatched projections', async t => {
  for (const [name, projection] of [
    ['foreign tenant', { ...childOrganization, tenantId: outside }],
    ['outside ceiling', { ...childOrganization, id: outside }],
    ['company', { ...childOrganization, organizationType: 'COMPANY' }],
    ['inactive', { ...childOrganization, status: 'DISABLED' }],
    ['nullable address', { ...childOrganization, address: null }],
    ['missing identity', { ...childOrganization, parentId: undefined }],
    ['invalid revision', { ...childOrganization, revision: 'not-a-revision' }],
    ['context mismatch', { ...childOrganization, contextVersion: 'c'.repeat(64) }],
  ]) await t.test(name, async () => {
    const envelope = organizationPage(projection, 1, 20); envelope.contextVersion = platformOrganization.contextVersion
    const api = client(async () => response(envelope))
    try { await assert.rejects(api.listOrganizations(create), error => error instanceof BusinessError && error.kind === 'PROTOCOL') }
    finally { api.dispose() }
  })
})

test('directory and single organization proofs must match current authorization and remain unexpired on receipt', async t => {
  const expired = '2000-01-01T00:00:00.000Z'
  for (const [name, body, single] of [
    ['uniform old directory context', organizationPage({ ...childOrganization, contextVersion: 'c'.repeat(64) }), false],
    ['expired directory envelope', { ...organizationPage(childOrganization), expiresAt: expired }, false],
    ['expired directory record', organizationPage({ ...childOrganization, expiresAt: expired }), false],
    ['old single context', { ...childOrganization, contextVersion: 'c'.repeat(64) }, true],
    ['expired single proof', { ...childOrganization, expiresAt: expired }, true],
  ]) await t.test(name, async () => {
    const api = client(async () => response(body))
    try { await assert.rejects(single ? api.getOrganization(child, create) : api.listOrganizations(create), error => error.kind === 'PROTOCOL') }
    finally { api.dispose() }
  })
})

test('HQ subtree creates the selected STORE while SELF cannot target a child; identity fields never reach writes', async () => {
  const calls = [], api = client(async (url, init) => { calls.push({ url, body: JSON.parse(init.body) }); return response(childStore, 201) })
  const body = { organizationId: child, businessStatus: 'OPEN', businessContact: '营业电话', managerUserId: null }
  try {
    assert.equal((await api.createStore(body)).organizationId, child)
    assert.deepEqual(calls[0].body, body)
    for (const field of ['name', 'code', 'address', 'organization']) {
      assert.throws(() => api.createStore({ ...body, [field]: 'not-owned-here' }), error => error.code === 'VALIDATION_ERROR')
      assert.throws(() => api.updateStore(storeId, { version: 1, businessContact: '', [field]: 'not-owned-here' }), error => error.code === 'VALIDATION_ERROR')
    }
    assert.throws(() => api.updateStore(storeId, { version: 1, organizationId: child, businessStatus: 'OPEN' }), error => error.code === 'VALIDATION_ERROR')
    assert.equal(calls.length, 1)
  } finally { api.dispose() }
  const self = client(async () => { throw new Error('must not dispatch') }, grantContext(create, 'SELF'))
  try { assert.throws(() => self.createStore(body), error => error.status === 403) }
  finally { self.dispose() }
})

test('store identity must match its live platform projection and SELF still checks the business owner', async () => {
  for (const changed of [{ ...store, name: 'local alias' }, { ...store, organization: { ...platformOrganization, id: child } }]) {
    const api = client(async () => response(changed))
    try { await assert.rejects(api.getStore(storeId), error => error.kind === 'PROTOCOL') } finally { api.dispose() }
  }
  const api = client(async () => response({ ...store, ownerUserId: outside }), grantContext({ resource: 'store', action: 'read' }, 'SELF'))
  try { await assert.rejects(api.getStore(storeId), error => error.kind === 'PROTOCOL') } finally { api.dispose() }
})

test('staff is a platform Member page with no local mutation methods or relationship metadata', async () => {
  const calls = [], api = client(async (url, init) => { calls.push({ url, method: init.method }); return response(page(member)) })
  try {
    assert.deepEqual((await api.listStaff(storeId, { q: '员工' })).records, [member])
    assert.equal(api.assignStaff, undefined); assert.equal(api.unassignStaff, undefined)
    assert.match(calls[0].url, /\/staff\?q=/); assert.equal(calls[0].method, 'GET')
  } finally { api.dispose() }
})

test('SELF member directory includes other members in the current STORE and rejects child or outside organizations', async () => {
  const purpose = { resource: 'store-staff', action: 'read' }, props = grantContext(purpose, 'SELF')
  const api = client(async () => response(page(member)), props)
  try {
    assert.notEqual(member.id, props.user.id)
    assert.deepEqual((await api.listStaff(storeId)).records, [member])
  } finally { api.dispose() }
  for (const organizationId of [child, outside]) {
    const restricted = client(async () => response(page({ ...member, organizationId })), props)
    try { await assert.rejects(restricted.listStaff(storeId), error => error.kind === 'PROTOCOL') }
    finally { restricted.dispose() }
  }
})

test('real React create view selects a platform STORE before showing business fields and never edits its identity', async () => {
  const writes = [], controller = new StoresController({ fetchImpl: async (url, init) => {
    if (String(url).includes('/organizations?')) return response(organizationPage(childOrganization))
    writes.push(JSON.parse(init.body)); return response(childStore, 201)
  } })
  try {
    controller.setContext(subtreeContext()); await tick()
    const form = controller.form('store-create')
    let html = render(controller)
    assert.match(html, /选择平台门店/); assert.match(html, /华东直营店/)
    assert.doesNotMatch(html, /name="businessStatus"/)
    await controller.submit(form.id); assert.equal(writes.length, 0)
    controller.selectOrganization(form.id, outside); assert.equal(controller.state.selectedOrganization, null)
    controller.selectOrganization(form.id, child)
    html = render(controller)
    assert.match(html, /平台门店 · 只读/); assert.match(html, /name="businessStatus"/)
    assert.doesNotMatch(html, /name="(?:name|code|address|organizationId)"/)
    controller.patch(form.id, 'name', '伪造组织名称')
    controller.patch(form.id, 'businessContact', '营业联络信息')
    await controller.submit(form.id)
    assert.deepEqual(writes, [{ organizationId: child, businessStatus: 'OPEN', businessContact: '营业联络信息', managerUserId: null }])
  } finally { controller.dispose() }
})

test('selected organization binds contact lookup; changing it clears a pending old contact search', async () => {
  const pending = deferred(), calls = [], other = { ...childOrganization, id: organization, name: '本店' }
  const directory = { ...organizationPage(childOrganization), records: [childOrganization, other], total: 2 }
  const controller = new StoresController({ fetchImpl: async (url) => {
    if (String(url).includes('/organizations?')) return response(directory)
    calls.push(String(url)); return pending.promise
  } })
  try {
    controller.setContext(subtreeContext()); await tick()
    const form = controller.form('store-create'); controller.selectOrganization(form.id, child)
    const search = controller.searchMembers(form.id, '员工')
    assert.match(calls[0], new RegExp('organizationId=' + child))
    controller.patch(form.id, 'managerUserId', memberId)
    controller.selectOrganization(form.id, organization)
    assert.equal(form.fields.managerUserId, '')
    pending.resolve(response(page({ ...member, organizationId: child }, 1, 10))); await search
    assert.equal(controller.state.directory, null)
  } finally { controller.dispose() }
})

test('unknown create reads back the original selected STORE, never HQ, without retrying', async () => {
  const calls = [], controller = new StoresController({ apiBase: 'http://127.0.0.1:3031/stores-api', fetchImpl: async (url, init) => {
    if (String(url).includes('/organizations?')) return response(organizationPage(childOrganization))
    calls.push({ url: String(url), method: init.method })
    if (init.method === 'POST') throw new TypeError('transport lost after possible commit')
    return response(page(childStore, 1, 50))
  } })
  try {
    controller.setContext(subtreeContext()); await tick()
    const form = controller.form('store-create'); controller.selectOrganization(form.id, child)
    controller.patch(form.id, 'businessContact', '原始联系信息'); await controller.submit(form.id)
    assert.equal(form.sent.outcome, 'unknown')
    controller.selectOrganization(form.id, organization); await controller.submit(form.id)
    assert.equal(calls.length, 1)
    await controller.readback(form.id)
    assert.match(calls[1].url, new RegExp('organizationId=' + child))
    assert.equal(form.sent.checked, true); assert.equal(form.sent.fields.businessContact, '原始联系信息')
    assert.equal(calls.filter(row => row.method === 'POST').length, 1)
  } finally { controller.dispose() }
})

test('SELF owned-child edit preserves an unproven business contact until the user explicitly clears it', async () => {
  const writes = [], existing = { ...childStore, managerUserId: memberId, managerDisplayName: '门店联系人' }
  const controller = new StoresController({ fetchImpl: async (_url, init) => {
    if (init.method === 'GET') return response(existing)
    const body = JSON.parse(init.body); writes.push(body)
    return writes.length === 1 ? response({ code: 'MEMBER_NOT_FOUND' }, 404) : response({ ...childStore, version: 2 })
  } })
  try {
    const props = context({ routePath: '/micro/stores/stores/' + storeId })
    controller.setContext({ ...props, dataGrants: props.dataGrants.map(grant => ({ ...grant, scope: 'SELF', organizationIds: [organization, child] })) })
    await tick(); const form = controller.form('store-edit')
    controller.patch(form.id, 'businessStatus', 'CLOSED'); await controller.submit(form.id)
    assert.equal(form.sent, null); assert.equal(form.fields.managerUserId, memberId)
    assert.equal(controller.allowed({ resource: 'store', action: 'update' }, existing), true)
    assert.match(form.error, /清空联系人后保存.*切换/)
    assert.equal(writes[0].managerUserId, memberId)
    controller.patch(form.id, 'managerUserId', ''); await controller.submit(form.id)
    assert.equal(writes[1].managerUserId, null)
    assert.equal(form.version, 2)
  } finally { controller.dispose() }
})

test('organization responses cannot restore revoked context; retired deep links render guidance with zero member writes', async () => {
  const pending = deferred(), controller = new StoresController({ fetchImpl: () => pending.promise })
  try {
    const props = subtreeContext(); controller.setContext(props)
    controller.setContext({ ...props, token: 'revoked', permissions: [], dataGrants: [], menus: [], contextEpoch: 2 })
    pending.resolve(response(organizationPage(childOrganization))); await tick()
    assert.equal(controller.state.organizations, null); assert.equal(controller.state.selectedOrganization, null)
  } finally { controller.dispose() }
  let requests = 0, destination
  const legacy = new StoresController({ fetchImpl: async () => { requests++; throw new Error('no data access expected') } })
  try {
    legacy.setContext(context({ routePath: '/micro/stores/staff-assign', menus: [], permissions: [], dataGrants: [], onNavigate(path) { destination = path } }))
    const html = render(legacy)
    assert.match(html, /员工关联与解除操作已迁移/); assert.doesNotMatch(html, /解除关联<\/button>|关联员工<\/button>/)
    legacy.openPlatformOrganizations(); assert.equal(destination, '/system/organizations')
    assert.equal(legacy.state.forms.length, 0); assert.equal(requests, 0)
  } finally { legacy.dispose() }
})

test('authorization renewal revalidates the selected STORE by ID and keeps visible business drafts while metadata is pending', async () => {
  const refreshed = deferred(), oldSignal = new AbortController(), requests = []
  let renewal = false
  const controller = new StoresController({ fetchImpl: async (url, init) => {
    requests.push({ url: String(url), bearer: init.headers.get('Authorization') })
    if (String(url).includes('/organizations/')) return refreshed.promise
    return response(renewal ? { ...organizationPage(null), contextVersion: 'c'.repeat(64) } : organizationPage(childOrganization))
  } })
  try {
    const props = { ...subtreeContext(), signal: oldSignal.signal }; controller.setContext(props); await tick()
    let form = controller.form('store-create'); controller.selectOrganization(form.id, child)
    controller.patch(form.id, 'businessContact', '续期保留的营业草稿'); controller.patch(form.id, 'managerUserId', memberId)
    renewal = true; oldSignal.abort()
    assert.equal(controller.state.selectedOrganization, null)
    controller.setContext({ ...props, token: 'renewed-bearer', contextEpoch: 2, contextVersion: 'c'.repeat(64), signal: new AbortController().signal })
    form = controller.form('store-create')
    assert.equal(form.fields.organizationId, child); assert.equal(form.fields.businessContact, '续期保留的营业草稿')
    let html = render(controller)
    assert.match(html, /续期保留的营业草稿/); assert.match(html, /正在重新验证/)
    assert.doesNotMatch(html, /平台门店 · 只读/)
    refreshed.resolve(response({ ...childOrganization, name: '平台更新后的门店名', contextVersion: 'c'.repeat(64), revision: 'd'.repeat(64) })); await tick()
    assert.equal(controller.state.selectedOrganization.id, child)
    assert.equal(controller.state.selectedOrganization.name, '平台更新后的门店名')
    assert.equal(controller.state.organizations.records.length, 0, 'selection restoration must not depend on the first result page')
    html = render(controller); assert.match(html, /平台更新后的门店名/); assert.match(html, /续期保留的营业草稿/)
    assert.equal(form.fields.managerUserId, memberId)
    assert.equal(requests.find(row => row.url.includes('/organizations/')).bearer, 'Bearer renewed-bearer')
  } finally { controller.dispose() }
})

test('renewal replays submitted report, summary and staff queries with new credentials while preserving unsent filter drafts', async () => {
  const requests = [], controller = new StoresController({ fetchImpl: async (url, init) => {
    requests.push({ url: String(url), bearer: init.headers.get('Authorization') })
    if (String(url).includes('/summary?')) return response({ count: 1, totalAmount: '1280.50', dateFrom: '2026-10-18', dateTo: '2026-10-18', storeId })
    if (String(url).includes('/staff?')) return response(page(member))
    return response(page({ ...report, amount: '1280.50' }))
  } })
  try {
    const host = new AbortController(), props = context({ routePath: '/micro/stores/reports', signal: host.signal })
    controller.setContext(props)
    for (const kind of ['report-query', 'summary']) {
      const form = controller.form(kind)
      for (const [field, value] of Object.entries({ storeId, dateFrom: '2026-10-18', dateTo: '2026-10-18' })) controller.patch(form.id, field, value)
      await controller.submit(form.id)
    }
    assert.equal(controller.state.reports.records[0].amount, '1280.50'); assert.equal(controller.state.summary.totalAmount, '1280.50')
    controller.patch(controller.form('report-query').id, 'dateFrom', '2026-10-17')
    host.abort(); assert.equal(controller.state.reports, null); assert.equal(controller.state.summary, null)
    const refreshed = { ...props, token: 'report-renewal', contextEpoch: 2, signal: new AbortController().signal }
    controller.setContext(refreshed)
    assert.equal(controller.state.reports, null); assert.equal(controller.state.summary, null)
    await tick()
    assert.equal(controller.form('report-query').fields.dateFrom, '2026-10-17')
    assert.equal(controller.state.reports.records[0].amount, '1280.50'); assert.equal(controller.state.summary.totalAmount, '1280.50')
    const replayed = requests.filter(row => row.bearer === 'Bearer report-renewal')
    assert.equal(replayed.length, 2)
    for (const request of replayed) assert.equal(new URL(request.url, 'http://localhost').searchParams.get('dateFrom'), '2026-10-18')

    const staffProps = { ...refreshed, routePath: '/micro/stores/staff' }; controller.setContext(staffProps)
    const staffForm = controller.form('staff-query')
    controller.patch(staffForm.id, 'storeId', storeId); controller.patch(staffForm.id, 'q', '已提交员工'); await controller.submit(staffForm.id)
    controller.patch(staffForm.id, 'q', '未提交草稿')
    controller.setContext({ ...staffProps, token: 'staff-renewal', contextEpoch: 3, signal: new AbortController().signal }); await tick()
    assert.deepEqual(controller.state.staff.records, [member]); assert.equal(controller.form('staff-query').fields.q, '未提交草稿')
    const staffReplay = requests.find(row => row.bearer === 'Bearer staff-renewal')
    assert.equal(new URL(staffReplay.url, 'http://localhost').searchParams.get('q'), '已提交员工')
  } finally { controller.dispose() }
})

test('store renewal restores the submitted search and preserves later unsent search input', async () => {
  const requests = [], controller = new StoresController({ fetchImpl: async (url, init) => {
    requests.push({ url: String(url), bearer: init.headers.get('Authorization') }); return response(page(store))
  } })
  try {
    const props = context(); controller.setContext(props); await tick()
    let form = controller.form('store-query'); controller.patch(form.id, 'q', '已执行检索'); await controller.submit(form.id)
    controller.patch(form.id, 'q', '尚未执行的草稿')
    controller.setContext({ ...props, token: 'store-renewal', contextEpoch: 2, signal: new AbortController().signal }); await tick()
    form = controller.form('store-query'); assert.equal(form.fields.q, '尚未执行的草稿'); assert.equal(controller.state.stores.records[0].id, storeId)
    const replay = requests.find(row => row.bearer === 'Bearer store-renewal')
    assert.equal(new URL(replay.url, 'http://localhost').searchParams.get('q'), '已执行检索')
  } finally { controller.dispose() }
})

test('revocation and identity changes cannot replay an old query or restore a late selected projection', async () => {
  const pending = deferred(), requests = []
  const controller = new StoresController({ fetchImpl: async (url, init) => {
    requests.push({ url: String(url), signal: init.signal })
    if (String(url).includes('/organizations/')) return pending.promise
    if (String(url).includes('/organizations?')) return response(organizationPage(childOrganization))
    return response(page({ ...report, amount: '1280.50' }))
  } })
  try {
    const props = context({ routePath: '/micro/stores/reports' }); controller.setContext(props)
    const query = controller.form('report-query')
    for (const [field, value] of Object.entries({ storeId, dateFrom: '2026-10-18', dateTo: '2026-10-18' })) controller.patch(query.id, field, value)
    await controller.submit(query.id)
    const count = requests.length
    const revoked = { ...props, token: 'revoked', contextEpoch: 2, permissions: [], dataGrants: [], menus: [], signal: new AbortController().signal }
    controller.setContext(revoked); await tick()
    assert.equal(controller.state.reports, null); assert.equal(requests.length, count)
    controller.setContext({ ...props, token: 'other-user', contextEpoch: 3, user: { ...props.user, id: outside }, dataGrants: props.dataGrants.map(grant => ({ ...grant, subjectUserId: outside })), signal: new AbortController().signal }); await tick()
    assert.equal(controller.form('report-query').fields.storeId, ''); assert.equal(controller.form('report-query').lastQueryFields, null)
    assert.equal(requests.length, count)

    const selectionProps = subtreeContext(); controller.setContext(selectionProps); await tick()
    const createForm = controller.form('store-create'); controller.selectOrganization(createForm.id, child); controller.patch(createForm.id, 'businessContact', '保留草稿')
    controller.setContext({ ...selectionProps, token: 'pending-renewal', contextEpoch: 2, signal: new AbortController().signal }); await tick()
    const restore = requests.find(row => row.url.includes('/organizations/')); assert.ok(restore)
    controller.setContext({ ...selectionProps, token: 'scope-narrowed', contextEpoch: 3, organizationScopeIds: [organization], dataGrants: selectionProps.dataGrants.map(grant => ({ ...grant, scope: 'ORG', organizationIds: [organization] })), signal: new AbortController().signal })
    assert.equal(restore.signal.aborted, true); assert.equal(controller.state.selectedOrganization, null)
    pending.resolve(response(childOrganization)); await tick()
    assert.equal(controller.state.selectedOrganization, null); assert.equal(controller.form('store-create').fields.businessContact, '保留草稿')
    assert.equal(requests.filter(row => row.url.includes('/organizations/')).length, 1)
  } finally { controller.dispose() }
})

test('renewal keeps business drafts visible but cannot restore old-context or expired selected metadata', async t => {
  for (const [name, invalid] of [
    ['old context', childOrganization],
    ['expired proof', { ...childOrganization, contextVersion: 'c'.repeat(64), expiresAt: '2000-01-01T00:00:00.000Z' }],
  ]) await t.test(name, async () => {
    let renewed = false
    const controller = new StoresController({ fetchImpl: async url => {
      if (String(url).includes('/organizations/')) return response(invalid)
      return response(organizationPage(renewed ? { ...childOrganization, contextVersion: 'c'.repeat(64) } : childOrganization))
    } })
    try {
      const props = subtreeContext(); controller.setContext(props); await tick()
      const old = controller.form('store-create'); controller.selectOrganization(old.id, child); controller.patch(old.id, 'businessContact', '新租约仍保留输入')
      renewed = true; controller.setContext({ ...props, token: 'fresh-context', contextVersion: 'c'.repeat(64), contextEpoch: 2, signal: new AbortController().signal }); await tick()
      assert.equal(controller.state.selectedOrganization, null)
      assert.equal(controller.form('store-create').fields.businessContact, '新租约仍保留输入')
      const html = render(controller); assert.match(html, /新租约仍保留输入/); assert.doesNotMatch(html, /平台门店 · 只读/)
    } finally { controller.dispose() }
  })
})

test('known-success create stays blocked through renewal and tracks each selected organization separately', async () => {
  const writes = [], directory = { ...organizationPage(childOrganization), records: [childOrganization, platformOrganization], total: 2 }
  const controller = new StoresController({ fetchImpl: async (url, init) => {
    if (String(url).includes('/organizations/')) return response(childOrganization)
    if (String(url).includes('/organizations?')) return response(directory)
    const body = JSON.parse(init.body); writes.push(body); return response(body.organizationId === child ? childStore : { ...store, id: memberId }, 201)
  } })
  try {
    const props = subtreeContext(); controller.setContext(props); await tick()
    let form = controller.form('store-create'); controller.selectOrganization(form.id, child); await controller.submit(form.id)
    assert.equal(form.completedId, storeId); await controller.submit(form.id); assert.equal(writes.length, 1)
    controller.setContext({ ...props, token: 'create-renewal', contextEpoch: 2, signal: new AbortController().signal }); await tick()
    form = controller.form('store-create'); assert.equal(form.completedId, storeId); await controller.submit(form.id); assert.equal(writes.length, 1)
    assert.match(render(controller), /已登记营业资料/)
    controller.selectOrganization(form.id, organization); assert.equal(form.completedId, null); await controller.submit(form.id)
    assert.equal(writes.length, 2); assert.equal(form.completedId, memberId)
    controller.selectOrganization(form.id, child); assert.equal(form.completedId, storeId); await controller.submit(form.id); assert.equal(writes.length, 2)
  } finally { controller.dispose() }
})
