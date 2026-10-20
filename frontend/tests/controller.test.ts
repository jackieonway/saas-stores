import test from 'node:test'
import assert from 'node:assert/strict'
import { StoresController } from '../src/controller.ts'
import { BusinessError } from '../src/client.ts'
import { context, grantContext, id, organization, child, storeId, reportId, memberId, store, staff, report, member, page, response, deferred } from './domain.fixture.mjs'

const base = '/micro/stores'
const tick = async () => { await new Promise(resolve => setImmediate(resolve)); await new Promise(resolve => setImmediate(resolve)) }
const fixture = (fetcher: (url: string, init: RequestInit) => Promise<Response> | Response, now?: () => number) => new StoresController({ apiBase: 'http://127.0.0.1:3000/stores-api', fetchImpl: ((url: string | URL | Request, init: RequestInit) => fetcher(String(url), init)) as typeof fetch, now })
const fillStore = (controller: StoresController, name = '原始门店') => { const form = controller.form('store-create')!; controller.patch(form.id, 'name', name); controller.patch(form.id, 'businessContact', '保留联系方式'); return form }
const withRoute = (route: string, overrides = {}) => context({ routePath: base + route, ...overrides })

test('theme-only updates retain exact form identity, typed draft, and ongoing request', async () => {
  const waiting = deferred(), calls: RequestInit[] = []
  const controller = fixture((_url, init) => { calls.push(init); return waiting.promise })
  const props = withRoute('/store-create'); controller.setContext(props)
  const form = fillStore(controller), writing = controller.submit(form.id)
  controller.setContext({ ...props, theme: 'dark' })
  assert.equal(controller.form('store-create')!.id, form.id)
  assert.equal(controller.form('store-create')!.fields.name, '原始门店')
  assert.equal(calls.length, 1)
  waiting.resolve(response({ ...store, name: '原始门店', businessContact: '保留联系方式' }, 201)); await writing
  assert.equal(controller.form('store-create')!.sent, null)
  controller.dispose()
})

test('same-identity renewal rotates form IDs, preserves legal inputs, and rejects old handlers', async () => {
  const calls: RequestInit[] = [], controller = fixture((_url, init) => { calls.push(init); return response({ ...store, name: '原始门店', businessContact: '保留联系方式' }, 201) })
  const props = withRoute('/store-create'); controller.setContext(props); const old = fillStore(controller)
  controller.setContext({ ...props, token: 'renewed-bearer', contextEpoch: 2, contextVersion: 'b'.repeat(64), signal: new AbortController().signal })
  const fresh = controller.form('store-create')!
  assert.notEqual(fresh.id, old.id); assert.equal(fresh.fields.name, '原始门店')
  controller.patch(old.id, 'name', '旧闭包'); await controller.submit(old.id)
  assert.equal(calls.length, 0); assert.equal(fresh.fields.name, '原始门店')
  await controller.submit(fresh.id)
  assert.equal(calls.length, 1); assert.equal((calls[0]!.headers as Headers).get('Authorization'), 'Bearer renewed-bearer')
  controller.dispose()
})

test('organization switch hides old drafts and rejects old-form writes before transport', async () => {
  let calls = 0; const controller = fixture(() => { calls++; return response(store, 201) })
  const props = withRoute('/store-create'); controller.setContext(props); const old = fillStore(controller, '甲组织私有草稿')
  const next = withRoute('/store-create', { token: 'org-b-bearer', contextEpoch: 2, contextVersion: 'b'.repeat(64), organization: { id: child, name: '乙组织', path: '/' + child }, dataGrants: props.dataGrants.map(grant => ({ ...grant, organizationIds: [child] })) })
  controller.setContext(next)
  assert.equal(controller.form('store-create')!.fields.name, '')
  controller.patch(old.id, 'name', '污染'); await controller.submit(old.id); assert.equal(calls, 0)
  controller.setContext({ ...props, token: 'back-to-a', signal: new AbortController().signal })
  assert.equal(controller.form('store-create')!.fields.name, '甲组织私有草稿')
  controller.dispose()
})

test('late list and member responses cannot revive departed route or organization data', async () => {
  const waiting = deferred(), calls: string[] = [], controller = fixture((url) => { calls.push(url); return waiting.promise })
  const props = context(); controller.setContext(props)
  controller.setContext({ ...props, routePath: base + '/store-create' })
  waiting.resolve(response(page(store))); await tick()
  assert.equal(controller.state.stores, null); assert.equal(controller.state.route.kind, 'store-create')
  const memberWait = deferred(), directory = fixture(() => memberWait.promise)
  directory.setContext(withRoute('/store-create')); const form = fillStore(directory)
  const pending = directory.searchMembers(form.id, '成员')
  directory.setContext(withRoute('/store-create', { token: 'new-org', organization: { id: child, name: '乙', path: '/' + child } }))
  memberWait.resolve(response(page(member, 1, 10))); await pending
  assert.equal(directory.state.directory, null)
  controller.dispose(); directory.dispose()
})

test('report writes freeze Shanghai business date and decimal text across midnight', async () => {
  let now = Date.parse('2026-10-18T15:59:59Z'), body: any, calls = 0
  const controller = fixture((_url, init) => { calls++; body = JSON.parse(String(init.body)); return response({ ...report, amount: '9999999999999999.99', note: '精确金额' }, 201) }, () => now)
  controller.setContext(withRoute('/daily-report-create', { expiresAt: '2027-01-01T00:00:00Z', tokenExpiresAt: '2027-01-01T00:00:00Z' })); const form = controller.form('report-create')!
  assert.equal(form.fields.businessDate, '2026-10-18')
  controller.patch(form.id, 'storeId', storeId); controller.patch(form.id, 'amount', '9999999999999999.99'); controller.patch(form.id, 'note', '精确金额')
  now = Date.parse('2026-10-18T16:00:01Z'); await controller.submit(form.id)
  assert.equal(calls, 1); assert.deepEqual(body, { businessDate: '2026-10-18', amount: '9999999999999999.99', note: '精确金额' }); assert.equal(typeof body.amount, 'string')
  controller.dispose()
})

test('unknown write holds original payload, performs explicit original-target readback, and never auto-retries', async () => {
  const requests: { url: string; method: string; body: any }[] = []
  const controller = fixture((url, init) => { const method = init.method || 'GET'; requests.push({ url, method, body: init.body && JSON.parse(String(init.body)) }); return method === 'POST' ? Promise.reject(new TypeError('network lost')) : response(page({ ...report, amount: '12.30', note: '原备注' }, 1, 50)) })
  const props = withRoute('/daily-report-create'); controller.setContext(props); const form = controller.form('report-create')!
  controller.patch(form.id, 'storeId', storeId); controller.patch(form.id, 'businessDate', '2026-10-18'); controller.patch(form.id, 'amount', '12.3'); controller.patch(form.id, 'note', '原备注')
  await controller.submit(form.id)
  assert.equal(form.sent!.outcome, 'unknown'); controller.patch(form.id, 'storeId', id(99)); assert.equal(form.fields.storeId, storeId)
  controller.setContext({ ...props, theme: 'dark' }); await controller.submit(form.id)
  assert.equal(requests.length, 1)
  await controller.readback(form.id)
  assert.match(requests[1]!.url, new RegExp(`/stores/${storeId}/daily-reports\\?`)); assert.match(requests[1]!.url, /dateFrom=2026-10-18/)
  assert.equal(form.sent!.checked, true); assert.equal(requests.filter(item => item.method === 'POST').length, 1)
  controller.acknowledgeReadback(form.id); assert.equal(form.sent, null); assert.equal(form.fields.amount, '12.3'); assert.equal(requests.length, 2)
  controller.dispose()
})

test('write-only grant can write normally; unknown result explains absent read permission without probing', async () => {
  let calls = 0, fail = false
  const controller = fixture(() => { calls++; return fail ? Promise.reject(new TypeError('lost')) : response(store, 201) })
  const props = grantContext({ resource: 'store', action: 'create' }, 'ORG', { routePath: base + '/store-create' })
  controller.setContext(props); let form = fillStore(controller)
  await controller.submit(form.id); assert.equal(calls, 1); assert.equal(form.sent, null)
  fail = true; await controller.submit(form.id); assert.equal(calls, 2)
  assert.equal(controller.readbackPermission(form), false); await controller.readback(form.id)
  assert.match(form.error, /没有对应读取权限/); assert.equal(calls, 2); assert.equal(form.fields.name, '原始门店')
  controller.dispose()
})

test('version conflict preserves input and requires explicit latest-version adoption', async () => {
  let reads = 0; const writes: any[] = []
  const controller = fixture((_url, init) => {
    if (init.method === 'PATCH') { writes.push(JSON.parse(String(init.body))); return writes.length === 1 ? response({ code: 'VERSION_CONFLICT', message: 'remote text' }, 409) : response({ ...store, version: 3, name: '我的草稿', businessStatus: 'CLOSED' }) }
    reads++; return response({ ...store, version: reads === 1 ? 1 : 2, name: reads === 1 ? '旧值' : '他人最新值' })
  })
  controller.setContext(withRoute('/stores/' + storeId)); await tick(); const form = controller.form('store-edit')!
  controller.patch(form.id, 'name', '我的草稿'); controller.patch(form.id, 'businessStatus', 'CLOSED'); await controller.submit(form.id)
  assert.equal(form.conflict, true); assert.equal(form.fields.name, '我的草稿'); assert.equal(form.version, 1)
  await controller.readLatest(form.id); assert.equal(form.latest!.version, 2); assert.equal(form.version, 1); assert.equal(form.fields.name, '我的草稿')
  controller.useLatest(form.id); assert.equal(form.version, 2); assert.equal(form.fields.name, '我的草稿')
  await controller.submit(form.id); assert.deepEqual(writes.map(body => body.version), [1, 2]); assert.equal(writes[1]!.businessStatus, 'CLOSED')
  controller.dispose()
})

test('edit renewal clears server version and preserves typed values until a fresh read is explicitly adopted', async () => {
  const controller = fixture(() => response(store)), props = withRoute('/stores/' + storeId)
  controller.setContext(props); await tick(); const old = controller.form('store-edit')!; controller.patch(old.id, 'name', '续期保留')
  controller.setContext({ ...props, token: 'renewal', contextEpoch: 2, signal: new AbortController().signal }); await tick()
  const fresh = controller.form('store-edit')!
  assert.notEqual(fresh.id, old.id); assert.equal(fresh.fields.name, '续期保留'); assert.equal(fresh.version, null); assert.equal(fresh.awaitingRead, true); assert.equal(fresh.latest!.version, 1)
  controller.useLatest(fresh.id); assert.equal(fresh.version, 1); assert.equal(fresh.fields.name, '续期保留')
  controller.dispose()
})

test('403 suspends the exact action across theme updates and clears related server views', async () => {
  let calls = 0; const controller = fixture(() => { calls++; return response({ code: 'OPERATION_DENIED', message: 'safe' }, 403) })
  const props = withRoute('/staff'); controller.setContext(props); const form = controller.form('staff-query')!
  controller.patch(form.id, 'storeId', storeId); await controller.submit(form.id)
  assert.equal(controller.state.staff, null); assert.equal(controller.state.denied.has('store-staff:read'), true)
  controller.setContext({ ...props, theme: 'dark' }); await controller.submit(controller.form('staff-query')!.id); assert.equal(calls, 1)
  controller.setContext({ ...props, token: 'fresh-auth', contextEpoch: 2, signal: new AbortController().signal }); assert.equal(controller.state.denied.size, 0)
  await controller.submit(controller.form('staff-query')!.id); assert.equal(calls, 2)
  controller.dispose()
})

test('read denial immediately publishes the cleared server view to UI subscribers', async () => {
  const controller = fixture(url => url.includes('/summary?')
    ? response({ count: 1, totalAmount: '12.30', dateFrom: '2026-10-18', dateTo: '2026-10-18', storeId: null })
    : response({ code: 'DATA_DENIED', message: 'denied' }, 403))
  controller.setContext(withRoute('/reports'))
  const summary = controller.form('summary')!, query = controller.form('report-query')!
  controller.patch(summary.id, 'dateFrom', '2026-10-18'); controller.patch(summary.id, 'dateTo', '2026-10-18')
  await controller.submit(summary.id)
  assert.ok(controller.state.summary)
  const rendered: { summary: unknown; denied: boolean; loading: boolean; error: string }[] = []
  const unsubscribe = controller.subscribe(() => rendered.push({ summary: controller.state.summary,
    denied: controller.state.denied.has('store-daily-report:read'), loading: controller.state.loading.has('reports'), error: query.error }))
  controller.patch(query.id, 'storeId', storeId)
  await controller.submit(query.id)
  assert.deepEqual(rendered.at(-1), { summary: null, denied: true, loading: false, error: query.error })
  assert.ok(query.error)
  unsubscribe(); controller.dispose()
})

test('staff and reports work with independent grants and direct store ID without implicit store reads', async () => {
  const requests: string[] = [], controller = fixture((url) => { requests.push(url); return response(page(null)) })
  const props = grantContext({ resource: 'store-staff', action: 'read' }, 'ORG', { routePath: base + '/staff' })
  controller.setContext(props); const form = controller.form('staff-query')!; controller.patch(form.id, 'storeId', storeId); await controller.submit(form.id)
  assert.equal(requests.length, 1); assert.match(requests[0]!, /\/staff\?/); assert.equal(controller.allowed({ resource: 'store', action: 'read' }), false)
  const reports = grantContext({ resource: 'store-daily-report', action: 'read' }, 'ORG', { routePath: base + '/reports' })
  controller.setContext(reports); const query = controller.form('report-query')!; controller.patch(query.id, 'storeId', storeId); await controller.submit(query.id)
  assert.equal(requests.length, 2); assert.match(requests[1]!, /\/daily-reports\?/)
  controller.dispose()
})

test('late write settlement cannot overwrite the current route, and transport abort hides server data immediately', async () => {
  const waiting = deferred(), controller = fixture(() => waiting.promise)
  const host = new AbortController(), props = withRoute('/store-create', { signal: host.signal }); controller.setContext(props)
  const form = fillStore(controller), writing = controller.submit(form.id)
  controller.setContext({ ...props, routePath: base + '/daily-report-create' })
  waiting.resolve(response(store, 201)); await writing
  assert.equal(controller.state.detail, null); assert.equal(controller.state.notice, ''); assert.equal(controller.state.route.kind, 'report-create')
  host.abort(); assert.equal(controller.state.active, false); assert.equal(controller.state.forms.length, 0); assert.equal(controller.state.directory, null)
  controller.dispose()
})

test('summary displays only exact server decimal text, including totals beyond a single report limit', async () => {
  const controller = fixture(() => response({ count: 3, totalAmount: '29999999999999999.97', dateFrom: '2026-10-18', dateTo: '2026-10-18', storeId: null }))
  controller.setContext(withRoute('/reports')); const form = controller.form('summary')!
  controller.patch(form.id, 'dateFrom', '2026-10-18'); controller.patch(form.id, 'dateTo', '2026-10-18'); await controller.submit(form.id)
  assert.equal(controller.state.summary!.totalAmount, '29999999999999999.97'); assert.equal(typeof controller.state.summary!.totalAmount, 'string')
  assert.equal(controller.state.reports, null); controller.dispose()
})

test('unknown staff unlink remains available for readback after an explicit list refresh shows no effective link', async () => {
  let removed = false, deletes = 0
  const controller = fixture((_url, init) => {
    if (init.method === 'DELETE') { deletes++; removed = true; return Promise.reject(new TypeError('response lost after commit')) }
    return response(page(removed ? null : staff))
  })
  controller.setContext(withRoute('/staff')); const query = controller.form('staff-query')!
  controller.patch(query.id, 'storeId', storeId); await controller.submit(query.id)
  const unlink = controller.form('staff-unassign')!; await controller.submit(unlink.id)
  assert.equal(unlink.sent!.outcome, 'unknown'); await controller.submit(query.id)
  assert.equal(controller.state.staff!.records.length, 0); assert.equal(controller.form('staff-unassign')!.id, unlink.id)
  // The original staff/store/version remain frozen even though the effective row disappeared.
  assert.equal(unlink.sent!.target!.id, staff.id); assert.equal(unlink.sent!.fields.storeId, storeId); assert.equal(unlink.sent!.version, 1)
  await controller.submit(unlink.id); assert.equal(deletes, 1)
  controller.dispose()
})

test('revoked menu/resource context immediately removes rows and disables a previously valid form ID', async () => {
  const requests: string[] = [], controller = fixture((url) => { requests.push(url); return response(page(store)) })
  const props = context(); controller.setContext(props); await tick()
  const old = controller.form('store-query')!; assert.equal(controller.state.stores!.records.length, 1)
  controller.setContext({ ...props, contextEpoch: 2, token: 'revoked-context', permissions: [], dataGrants: [], menus: [], signal: new AbortController().signal })
  assert.equal(controller.state.stores, null); assert.equal(controller.routeAllowed(), false)
  await controller.submit(old.id); assert.equal(requests.length, 1)
  controller.dispose()
})

test('renewal during a dispatched write keeps an unknown original event and never attaches its old handler to new credentials', async () => {
  const waiting = deferred(), requests: RequestInit[] = [], controller = fixture((_url, init) => { requests.push(init); return waiting.promise })
  const props = withRoute('/store-create'); controller.setContext(props); const old = fillStore(controller, '原发送内容')
  const pending = controller.submit(old.id)
  controller.setContext({ ...props, token: 'new-generation', contextEpoch: 2, signal: new AbortController().signal })
  const fresh = controller.form('store-create')!
  assert.equal(fresh.sent!.outcome, 'pending'); assert.equal(fresh.sent!.inFlight, true); assert.equal(fresh.pending, true); assert.equal(fresh.sent!.fields.name, '原发送内容'); assert.notEqual(fresh.id, old.id)
  controller.patch(old.id, 'name', '新组织误配'); await controller.submit(old.id); await controller.submit(fresh.id)
  assert.equal(requests.length, 1)
  waiting.resolve(response(store, 201)); await pending
  assert.equal(fresh.sent!.outcome, 'unknown'); assert.equal(fresh.sent!.inFlight, false); assert.equal(fresh.pending, false); assert.equal(controller.state.notice, '')
  controller.dispose()
})

test('a version conflict cannot be sent again before an explicit latest read and version adoption', async () => {
  let writes = 0
  const controller = fixture((_url, init) => {
    if (init.method === 'PATCH') { writes++; return response({ code: 'VERSION_CONFLICT', message: 'conflict' }, 409) }
    return response({ ...report, version: 2 })
  })
  controller.setContext(withRoute('/daily-reports/' + reportId)); await tick(); const form = controller.form('report-edit')!
  controller.patch(form.id, 'amount', '88.01'); await controller.submit(form.id); assert.equal(writes, 1)
  await controller.submit(form.id); assert.equal(writes, 1); assert.equal(form.awaitingRead, true)
  await controller.readLatest(form.id); await controller.submit(form.id); assert.equal(writes, 1)
  controller.useLatest(form.id); await controller.submit(form.id); assert.equal(writes, 2)
  controller.dispose()
})

test('unknown staff unlink restores its original recovery event on route return and renewal even with an empty server list', async () => {
  let removed = false, deletes = 0
  const controller = fixture((url, init) => {
    if (init.method === 'DELETE') { deletes++; removed = true; return Promise.reject(new TypeError('lost commit response')) }
    const size = Number(new URL(url).searchParams.get('size') || '20')
    return response(page(removed ? null : staff, 1, size))
  })
  const props = withRoute('/staff'); controller.setContext(props)
  let query = controller.form('staff-query')!; controller.patch(query.id, 'storeId', storeId); await controller.submit(query.id)
  const original = controller.form('staff-unassign')!; await controller.submit(original.id)
  controller.setContext({ ...props, routePath: base + '/staff-assign' }); controller.setContext(props)
  let recovered = controller.form('staff-unassign')!
  assert.ok(recovered); assert.notEqual(recovered.id, original.id); assert.equal(recovered.sent!.target!.id, staff.id); assert.equal(recovered.sent!.version, 1); assert.equal(recovered.sent!.fields.storeId, storeId)
  query = controller.form('staff-query')!; await controller.submit(query.id); assert.equal(controller.state.staff!.records.length, 0)
  assert.equal(controller.form('staff-unassign')!.id, recovered.id)
  const beforeRenewId = recovered.id
  controller.setContext({ ...props, token: 'renew-after-unknown', contextEpoch: 2, signal: new AbortController().signal })
  recovered = controller.form('staff-unassign')!; assert.ok(recovered); assert.notEqual(recovered.id, beforeRenewId); assert.equal(recovered.sent!.target!.id, staff.id)
  await controller.readback(recovered.id); assert.deepEqual(recovered.readback, []); assert.equal(recovered.sent!.checked, true); assert.equal(deletes, 1)
  controller.dispose()
})

test('route return cannot acknowledge or resend a still-pending write before its original transport settles', async () => {
  const waiting = deferred(), requests: { method: string; url: string }[] = []
  const controller = fixture((url, init) => {
    requests.push({ method: init.method || 'GET', url })
    return init.method === 'POST' ? waiting.promise : response(page(null, 1, 50))
  })
  const props = withRoute('/store-create'); controller.setContext(props); const original = fillStore(controller, '仍在提交的原内容')
  const pending = controller.submit(original.id)
  controller.setContext({ ...props, routePath: base + '/daily-report-create' }); controller.setContext(props)
  const restored = controller.form('store-create')!
  assert.equal(restored.pending, true); assert.equal(restored.sent!.inFlight, true); assert.equal(restored.sent!.outcome, 'pending')
  await controller.readback(restored.id); controller.acknowledgeReadback(restored.id); await controller.submit(restored.id)
  assert.equal(requests.length, 1); assert.equal(restored.sent!.inFlight, true); assert.equal(restored.fields.name, '仍在提交的原内容')
  waiting.reject(new TypeError('eventually lost response')); await pending
  assert.equal(restored.pending, false); assert.equal(restored.sent!.outcome, 'unknown'); assert.equal(restored.sent!.inFlight, false)
  await controller.readback(restored.id); assert.equal(requests.length, 2); assert.equal(restored.sent!.checked, true)
  controller.acknowledgeReadback(restored.id); assert.equal(restored.sent, null); assert.equal(requests.filter(item => item.method === 'POST').length, 1)
  controller.dispose()
})

test('pending success settles every restored draft copy without reviving a departed route', async () => {
  const waiting = deferred(), controller = fixture(() => waiting.promise)
  const props = withRoute('/store-create'); controller.setContext(props); const original = fillStore(controller)
  const pending = controller.submit(original.id)
  controller.setContext({ ...props, routePath: base + '/staff-assign' }); controller.setContext(props)
  const restored = controller.form('store-create')!; assert.equal(restored.pending, true)
  controller.setContext({ ...props, routePath: base + '/daily-report-create' })
  waiting.resolve(response(store, 201)); await pending
  assert.equal(controller.state.notice, ''); assert.equal(controller.state.route.kind, 'report-create')
  controller.setContext(props); const settled = controller.form('store-create')!
  assert.equal(settled.pending, false); assert.equal(settled.sent, null); assert.equal(settled.fields.name, '原始门店')
  controller.dispose()
})

test('403 in one parallel report read prevents a late summary from restoring denied server data', async () => {
  const listWait = deferred(), summaryWait = deferred()
  const controller = fixture(url => url.includes('/summary?') ? summaryWait.promise : listWait.promise)
  const props = withRoute('/reports'); controller.setContext(props)
  const query = controller.form('report-query')!, summary = controller.form('summary')!
  controller.patch(query.id, 'storeId', storeId); controller.patch(query.id, 'dateFrom', '2026-10-18'); controller.patch(query.id, 'dateTo', '2026-10-18')
  controller.patch(summary.id, 'dateFrom', '2026-10-18'); controller.patch(summary.id, 'dateTo', '2026-10-18')
  const listing = controller.submit(query.id), summarizing = controller.submit(summary.id)
  listWait.resolve(response({ code: 'DATA_DENIED', message: 'denied' }, 403)); await listing
  assert.equal(controller.state.denied.has('store-daily-report:read'), true)
  summaryWait.resolve(response({ count: 1, totalAmount: '12.30', dateFrom: '2026-10-18', dateTo: '2026-10-18', storeId: null })); await summarizing
  assert.equal(controller.state.summary, null); assert.equal(controller.state.reports, null); assert.equal(controller.state.loading.has('summary'), false)
  controller.setContext({ ...props, theme: 'dark' }); assert.equal(controller.state.summary, null); assert.equal(controller.state.denied.has('store-daily-report:read'), true)
  controller.dispose()
})
