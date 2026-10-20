import test from 'node:test'
import assert from 'node:assert/strict'
import { BusinessError, StoresClient } from '../src/client.ts'
import { context, grantContext, baseRow, store, staff, report, member, page, response, deferred,
  tenant, organization, outside, user, storeId, staffId, reportId, memberId, id } from './domain.fixture.mjs'

const clientWith = (fetchImpl, props = context(), options = {}) => {
  const client = new StoresClient({ fetchImpl, ...options }); client.setContext(props); return client
}
const failure = (status, code) => response({ code, message: 'SECRET upstream-token=never-display' }, status)
const rejection = promise => Promise.resolve(promise).then(() => { throw new Error('Expected a rejection') }, error => error)

test('all actual HTTP methods use current Bearer, omit cookies and preserve wire decimal/date values', async t => {
  const cases = [
    ['listStores', [ { q: '门店%_\\', size: 20 } ], page(store), 200, 'GET', '/stores?q=%E9%97%A8%E5%BA%97%25_%5C&size=20'],
    ['getStore', [storeId], store, 200, 'GET', '/stores/' + storeId],
    ['createStore', [{ name: '城北门店', businessStatus: 'OPEN', businessContact: '', managerUserId: null }], store, 201, 'POST', '/stores'],
    ['updateStore', [storeId, { version: 1, businessStatus: 'CLOSED', managerUserId: null }], { ...store, version: 2, businessStatus: 'CLOSED' }, 200, 'PATCH', '/stores/' + storeId],
    ['listStaff', [storeId], page(staff), 200, 'GET', '/stores/' + storeId + '/staff'],
    ['assignStaff', [storeId, { userId: memberId }], staff, 201, 'POST', '/stores/' + storeId + '/staff'],
    ['unassignStaff', [storeId, staffId, { version: 1 }], { ok: true }, 200, 'DELETE', '/stores/' + storeId + '/staff/' + staffId],
    ['listDailyReports', [storeId, { dateFrom: report.businessDate, dateTo: report.businessDate }], page(report), 200, 'GET', '/stores/' + storeId + '/daily-reports?dateFrom=2026-10-18&dateTo=2026-10-18'],
    ['getDailyReport', [reportId], report, 200, 'GET', '/daily-reports/' + reportId],
    ['createDailyReport', [storeId, { businessDate: report.businessDate, amount: report.amount, note: report.note }], report, 201, 'POST', '/stores/' + storeId + '/daily-reports'],
    ['updateDailyReport', [reportId, { version: 1, amount: '9007199254740993.01' }], { ...report, version: 2, amount: '9007199254740993.01' }, 200, 'PATCH', '/daily-reports/' + reportId],
    ['getSummary', [{ dateFrom: '2026-10-01', dateTo: '2026-10-31' }], { count: 2, totalAmount: '19999999999999999.98', dateFrom: '2026-10-01', dateTo: '2026-10-31', storeId: null }, 200, 'GET', '/daily-reports/summary?dateFrom=2026-10-01&dateTo=2026-10-31'],
    ['listMembers', [{ resource: 'store-staff', action: 'assign' }, { q: '员工' }], page(member), 200, 'GET', '/members?resource=store-staff&action=assign&q=%E5%91%98%E5%B7%A5&organizationId=' + organization],
  ]
  for (const [method, args, result, status, verb, suffix] of cases) await t.test(method, async () => {
    const calls = [], props = context()
    const client = clientWith(async (url, init) => { calls.push({ url, init }); return response(result, status) }, props)
    try {
      assert.deepEqual(await client[method](...args), result)
      assert.equal(calls.length, 1)
      const { url, init } = calls[0]
      assert.equal(url, '/stores-api' + suffix); assert.equal(init.method, verb)
      assert.equal(init.credentials, 'omit'); assert.equal(init.redirect, 'error'); assert.equal(init.cache, 'no-store')
      assert.equal(init.headers.get('Authorization'), 'Bearer ' + props.token)
      assert.match(init.headers.get('X-Request-Id'), /^[a-f0-9-]{36}$/i)
      assert.equal(init.headers.has('Cookie'), false); assert.equal(init.headers.has('Idempotency-Key'), false)
      if (verb !== 'GET') {
        const input = args.findLast(value => typeof value === 'object')
        assert.deepEqual(JSON.parse(init.body), input)
        assert.equal(init.headers.get('Content-Type'), 'application/json')
      }
    } finally { client.dispose() }
  })
})

test('staff, reports and member writes operate without an implicit store read', async () => {
  for (const [purpose, method, args, result, status] of [
    [{ resource: 'store-staff', action: 'read' }, 'listStaff', [storeId], page(staff), 200],
    [{ resource: 'store-staff', action: 'assign' }, 'assignStaff', [storeId, { userId: memberId }], staff, 201],
    [{ resource: 'store-daily-report', action: 'create' }, 'createDailyReport', [storeId, { businessDate: report.businessDate, amount: '1.00' }], { ...report, amount: '1.00' }, 201],
  ]) {
    const paths = [], client = clientWith(async url => { paths.push(url); return response(result, status) }, grantContext(purpose))
    try { await client[method](...args); assert.equal(paths.length, 1); assert.notEqual(paths[0], '/stores-api/stores/' + storeId) }
    finally { client.dispose() }
  }
})

test('strict runtime DTO validation rejects incorrect paging, identities, owner semantics and numeric money', async t => {
  const badCases = [
    ['page mismatch', 'listStores', [], { ...page(store), page: 2 }],
    ['size mismatch', 'listStores', [], { ...page(store), size: 50 }],
    ['unsafe total', 'listStores', [], { ...page(store), total: Number.MAX_SAFE_INTEGER + 1 }],
    ['duplicate rows', 'listStores', [], { records: [store, store], total: 2, page: 1, size: 20 }],
    ['foreign tenant', 'getStore', [storeId], { ...store, tenantId: outside }],
    ['foreign organization', 'getStore', [storeId], { ...store, organizationId: outside }],
    ['wrong detail identity', 'getStore', [storeId], { ...store, id: id(88) }],
    ['extra field', 'getStore', [storeId], { ...store, clientSecret: 'SECRET' }],
    ['unpaired manager', 'getStore', [storeId], { ...store, managerUserId: memberId }],
    ['invalid timestamp day', 'getStore', [storeId], { ...store, updatedAt: '2026-02-30T01:00:00.000Z' }],
    ['staff creator is not owner', 'listStaff', [storeId], page({ ...staff, ownerUserId: user })],
    ['removed staff', 'listStaff', [storeId], page({ ...staff, removedAt: '2026-10-18T01:00:00.000Z' })],
    ['wrong staff store', 'listStaff', [storeId], page({ ...staff, storeId: id(89) })],
    ['number amount', 'getDailyReport', [reportId], { ...report, amount: 1 }],
    ['unfixed amount', 'getDailyReport', [reportId], { ...report, amount: '1.2' }],
    ['impossible date', 'getDailyReport', [reportId], { ...report, businessDate: '2026-02-29' }],
    ['wrong member DTO', 'listMembers', [{ resource: 'store', action: 'create' }], page({ userId: memberId, displayName: '成员', organizationId: organization })],
    ['wrong member org', 'listMembers', [{ resource: 'store', action: 'create' }], page({ ...member, organizationId: outside })],
    ['summary date mismatch', 'getSummary', [{ dateFrom: '2026-10-01', dateTo: '2026-10-31' }], { count: 0, totalAmount: '0.00', dateFrom: '2026-10-02', dateTo: '2026-10-31', storeId: null }],
  ]
  for (const [name, method, args, result] of badCases) await t.test(name, async () => {
    const client = clientWith(async () => response(result))
    try { await assert.rejects(async () => client[method](...args), error => error instanceof BusinessError && error.kind === 'PROTOCOL' && error.unknown === false && !JSON.stringify(error).includes('SECRET')) }
    finally { client.dispose() }
  })
})

test('invalid input is rejected before transport, including implicit report date and undeclared member purpose', async () => {
  let calls = 0
  const client = clientWith(async () => { calls++; return response(store) })
  const invalid = [
    () => client.getStore('not-a-uuid'), () => client.listStores({ size: 51 }), () => client.listStores({ page: 0 }),
    () => client.listStores({ unexpected: true }), () => client.createStore({ name: ' ', businessStatus: 'OPEN', businessContact: '' }),
    () => client.updateStore(storeId, { version: 1 }), () => client.updateStore(storeId, { version: '1', name: 'a' }),
    () => client.createDailyReport(storeId, { amount: '1' }), () => client.createDailyReport(storeId, { businessDate: '2026-02-29', amount: '1' }),
    () => client.createDailyReport(storeId, { businessDate: '2026-10-18', amount: 1 }),
    () => client.createDailyReport(storeId, { businessDate: '2026-10-18', amount: '1.001' }),
    () => client.updateDailyReport(reportId, { version: 1, businessDate: '2026-10-18' }),
    () => client.unassignStaff(storeId, staffId, { version: 2147483647 }),
    () => client.getSummary({ dateFrom: '2026-11-01', dateTo: '2026-10-31' }),
    () => client.getSummary({ dateFrom: '2026-10-01', dateTo: '2026-10-31', size: 10 }),
    () => client.listMembers({ resource: 'store', action: 'read' }),
    () => client.listMembers({ resource: 'store-staff', action: 'assign' }, { q: ' '.repeat(101) }),
  ]
  try { for (const action of invalid) await assert.rejects(async () => action(), error => error instanceof BusinessError && [400, 403].includes(error.status)); assert.equal(calls, 0) }
  finally { client.dispose() }
})

test('same authorization preserves request and snapshot across theme, route and callback updates', async () => {
  const pending = deferred(), props = context(), client = clientWith(() => pending.promise, props)
  try {
    const snapshot = client.capture(), result = client.listStores()
    const next = { ...props, theme: 'dark', routePath: '/micro/stores/reports', onNotify() {}, onNavigate() {} }
    assert.equal(client.setContext(next), false); assert.equal(client.current(), next); assert.equal(client.isCurrent(snapshot), true)
    pending.resolve(response(page(store)))
    assert.deepEqual(await result, page(store))
  } finally { client.dispose() }
})

test('every real authorization change synchronously fences old reads, including same-version credentials', async t => {
  const changes = [
    ['token', props => ({ ...props, token: 'new-token' })],
    ['expiry', props => ({ ...props, expiresAt: new Date(Date.parse(props.expiresAt) + 1000).toISOString() })],
    ['token expiry', props => ({ ...props, tokenExpiresAt: new Date(Date.parse(props.tokenExpiresAt) + 1000).toISOString() })],
    ['epoch', props => ({ ...props, contextEpoch: 2 })], ['version', props => ({ ...props, contextVersion: 'b'.repeat(64) })],
    ['permissions', props => ({ ...props, permissions: props.permissions.filter(value => value !== 'store:update') })],
    ['grant', props => ({ ...props, dataGrants: props.dataGrants.map(grant => ({ ...grant, scope: 'TENANT', organizationIds: [] })) })],
    ['menus', props => ({ ...props, menus: props.menus.slice(1) })], ['host signal', props => ({ ...props, signal: new AbortController().signal })],
  ]
  for (const [name, change] of changes) await t.test(name, async () => {
    const pending = deferred(), props = context(), client = clientWith(() => pending.promise, props)
    try {
      const snapshot = client.capture(), result = rejection(client.listStores())
      assert.equal(client.setContext(change(props)), true); assert.equal(snapshot.signal.aborted, true); assert.equal(client.isCurrent(snapshot), false)
      pending.resolve(response(page(store)))
      assert.equal((await result).name, 'AbortError')
    } finally { client.dispose() }
  })
})

test('late old 401 cannot invalidate replacement context; current 401 synchronously hides it', async () => {
  const old = deferred(), props = context(); let count = 0, invalidations = 0
  const client = clientWith(() => ++count === 1 ? old.promise : Promise.resolve(failure(401, 'INVALID_APP_TOKEN')), props, { onInvalidate() { invalidations++ } })
  try {
    const pending = rejection(client.listStores())
    const next = { ...props, token: 'replacement-token' }; client.setContext(next)
    const countAfterReplacement = invalidations
    old.resolve(failure(401, 'INVALID_APP_TOKEN')); assert.equal((await pending).name, 'AbortError')
    await Promise.resolve(); assert.equal(client.current(), next); assert.equal(invalidations, countAfterReplacement)
    await assert.rejects(client.listStores(), error => error instanceof BusinessError && error.status === 401)
    assert.equal(client.current(), null); assert.equal(invalidations, countAfterReplacement + 1)
  } finally { client.dispose() }
})

test('403 freezes just its exact pair through theme update while preserving unrelated context', async () => {
  const props = context(); let calls = 0, invalidations = 0
  const client = clientWith(async url => { calls++; return calls === 1 ? failure(403, 'DATA_DENIED') : response(url.endsWith('/staff') ? page(staff) : page(store)) }, props, { onInvalidate() { invalidations++ } })
  try {
    await assert.rejects(client.listStores(), error => error.status === 403 && error.code === 'OPERATION_DENIED')
    assert.equal(invalidations, 0); assert.equal(client.current(), props)
    assert.equal(client.setContext({ ...props, theme: 'dark' }), false)
    await assert.rejects(client.listStores(), error => error.status === 403); assert.equal(calls, 1)
    await client.listStaff(storeId); assert.equal(calls, 2)
    client.setContext({ ...props, token: 'renewed-token' }); await client.listStores(); assert.equal(calls, 3)
  } finally { client.dispose() }
})

test('host abort, expiry and dispose clear views immediately; expiry uses a controlled clock', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  let invalidations = 0
  const host = new AbortController(), props = context({ signal: host.signal })
  const client = clientWith(async () => response(page(store)), props, { onInvalidate() { invalidations++ } })
  host.abort(); assert.equal(client.current(), null); assert.equal(invalidations, 1)
  const next = context({ expiresAt: new Date(Date.now() + 100).toISOString() })
  client.setContext(next); const snapshot = client.capture()
  t.mock.timers.tick(101)
  assert.equal(client.current(), null); assert.equal(snapshot.signal.aborted, true); assert.equal(invalidations, 2)
  client.setContext(context()); client.dispose(); assert.equal(client.current(), null); assert.equal(invalidations, 3)
  client.dispose(); assert.equal(invalidations, 3)
})

test('network, malformed-success, 5xx and sent-write cancellation are unknown without any retry', async t => {
  const original = { businessDate: '2026-10-18', amount: '9007199254740993.01', note: '原始记录' }
  for (const [name, fetchImpl] of [
    ['network', async () => { throw new Error('SECRET connection-string') }],
    ['malformed committed success', async () => response({ ...report, amount: 1 }, 201)],
    ['explicit unknown', async () => failure(503, 'CREATE_RESULT_UNKNOWN')],
    ['database failure', async () => failure(503, 'DATABASE_UNAVAILABLE')],
  ]) await t.test(name, async () => {
    let calls = 0
    const client = clientWith((...args) => { calls++; return fetchImpl(...args) })
    try {
      await assert.rejects(client.createDailyReport(storeId, original), error => error instanceof BusinessError && error.unknown && !JSON.stringify(error).includes('SECRET'))
      assert.equal(calls, 1); assert.deepEqual(original, { businessDate: '2026-10-18', amount: '9007199254740993.01', note: '原始记录' })
    } finally { client.dispose() }
  })
  await t.test('cancel after dispatch', async () => {
    const pending = deferred(), route = new AbortController(); let calls = 0
    const client = clientWith(() => { calls++; return pending.promise })
    try {
      const result = rejection(client.createDailyReport(storeId, original, { signal: route.signal })); route.abort()
      const error = await result
      assert.equal(error.unknown, true); assert.equal(error.kind, 'CANCELLED'); assert.equal(calls, 1)
      pending.resolve(response({ ...report, amount: original.amount }, 201)); await Promise.resolve()
      assert.equal(calls, 1)
    } finally { client.dispose() }
  })
})

test('pre-dispatch cancellation sends nothing and independent writes survive a departing read cancellation', async () => {
  const read = deferred(), write = deferred(), route = new AbortController(), stopped = new AbortController(); stopped.abort()
  let calls = 0
  const client = clientWith((_url, init) => { calls++; return init.method === 'GET' ? read.promise : write.promise })
  try {
    await assert.rejects(client.createStore({ name: '城北门店', businessStatus: 'OPEN', businessContact: '' }, { signal: stopped.signal }), error => error.name === 'AbortError')
    assert.equal(calls, 0)
    const pendingRead = rejection(client.listStores({}, { signal: route.signal })), pendingWrite = client.updateStore(storeId, { version: 1, businessStatus: 'CLOSED' })
    route.abort(); assert.equal((await pendingRead).name, 'AbortError')
    write.resolve(response({ ...store, version: 2, businessStatus: 'CLOSED' })); assert.equal((await pendingWrite).version, 2)
    read.resolve(response(page(store))); assert.equal(calls, 2)
  } finally { client.dispose() }
})

test('bounded safe business codes distinguish unique conflict from version conflict without exposing remote text', async () => {
  for (const code of ['STORE_ALREADY_EXISTS', 'STAFF_ALREADY_ASSIGNED', 'DAILY_REPORT_EXISTS', 'VERSION_CONFLICT']) {
    const client = clientWith(async () => failure(409, code))
    try {
      await assert.rejects(client.createStore({ name: '门店', businessStatus: 'OPEN', businessContact: '' }), error => {
        assert.equal(error.code, code); assert.equal(error.status, 409); assert.equal(error.unknown, false)
        assert.equal(error.requestId, id(999)); assert.equal(JSON.stringify(error).includes('SECRET'), false); return true
      })
    } finally { client.dispose() }
  }
  const huge = clientWith(async () => response({ code: 'VERSION_CONFLICT', message: 'SECRET'.repeat(3000) }, 409))
  try { await assert.rejects(huge.listStores(), error => error.code === 'CONFLICT' && !JSON.stringify(error).includes('SECRET')) }
  finally { huge.dispose() }
})

test('success streaming bodies are bounded and wrong success status marks writes unknown', async () => {
  const client = clientWith(async () => response(store, 200, { 'Content-Length': String(4 * 1024 * 1024 + 1) }))
  try { await assert.rejects(client.getStore(storeId), error => error.status === 413 && error.code === 'RESPONSE_TOO_LARGE') }
  finally { client.dispose() }
  const wrongStatus = clientWith(async () => response(store, 200))
  try { await assert.rejects(wrongStatus.createStore({ name: '门店', businessStatus: 'OPEN', businessContact: '' }), error => error.kind === 'PROTOCOL' && error.unknown) }
  finally { wrongStatus.dispose() }
})

test('body-stream overflow, malformed JSON and read cancellation remain bounded and never become success', async () => {
  const streamClient = clientWith(async () => new Response(new ReadableStream({
    start(controller) { controller.enqueue(new Uint8Array(4 * 1024 * 1024 + 1)); controller.close() },
  })))
  try { await assert.rejects(streamClient.listStores(), error => error.code === 'RESPONSE_TOO_LARGE' && !error.unknown) }
  finally { streamClient.dispose() }
  const malformed = clientWith(async () => new Response('{invalid', { status: 200 }))
  try { await assert.rejects(malformed.listStores(), error => error.kind === 'PROTOCOL' && error.code === 'INVALID_JSON') }
  finally { malformed.dispose() }
  const wait = new AbortController()
  const waiting = clientWith(async () => new Response(new ReadableStream({ start() {} })))
  try {
    const result = rejection(waiting.listStores({}, { signal: wait.signal }))
    await Promise.resolve(); await Promise.resolve(); wait.abort()
    assert.equal((await result).name, 'AbortError')
  } finally { waiting.dispose() }
})

test('a sent report keeps its confirmed original business date and body if the caller edits its object', async () => {
  const pending = deferred(), original = { businessDate: '2026-10-18', amount: '9007199254740993.01', note: '原始输入' }
  let sent
  const client = clientWith((_url, init) => { sent = JSON.parse(init.body); return pending.promise })
  try {
    const result = client.createDailyReport(storeId, original)
    original.businessDate = '2026-10-19'; original.amount = '1.00'; original.note = '次日新输入'
    pending.resolve(response({ ...report, amount: '9007199254740993.01', note: '原始输入' }, 201))
    assert.equal((await result).businessDate, '2026-10-18')
    assert.deepEqual(sent, { businessDate: '2026-10-18', amount: '9007199254740993.01', note: '原始输入' })
  } finally { client.dispose() }
})

test('wrong-status recovery codes are discarded rather than changing conflict semantics', async () => {
  const client = clientWith(async () => failure(409, 'CREATE_RESULT_UNKNOWN'))
  try { await assert.rejects(client.createStore({ name: '门店', businessStatus: 'OPEN', businessContact: '' }), error => error.code === 'CONFLICT' && error.unknown === false) }
  finally { client.dispose() }
})

test('API base rejects credentialed, relative escape, insecure non-loopback and query targets', () => {
  for (const apiBase of ['https://user:secret@example.test/stores-api', '//example.test/stores-api', '/api/../stores-api', '/stores-api?secret=x', 'http://example.test/stores-api', 'javascript:alert(1)', '/%2e%2e/stores-api']) assert.throws(() => new StoresClient({ apiBase }), /门店服务地址无效/)
  for (const apiBase of ['/stores-api', 'https://example.test/stores-api', 'http://127.0.0.1:3031/stores-api']) { const client = new StoresClient({ apiBase }); client.dispose() }
})

const drainMicrotasks = async () => { for (let index = 0; index < 12; index++) await Promise.resolve() }

test('the default 15-second deadline ends a hanging fetch while preserving context and other operations', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  const pending = deferred(); let calls = 0, invalidations = 0
  const client = clientWith(() => ++calls === 1 ? pending.promise : Promise.resolve(response(page(staff))), context(), { onInvalidate() { invalidations++ } })
  try {
    const snapshot = client.capture(); let settled = false
    const result = rejection(client.listStores()).then(error => { settled = true; return error })
    t.mock.timers.tick(14999); await drainMicrotasks(); assert.equal(settled, false)
    t.mock.timers.tick(1)
    const error = await result
    assert.equal(error.status, 408); assert.equal(error.kind, 'TIMEOUT'); assert.equal(error.code, 'REQUEST_TIMEOUT')
    assert.equal(error.message, '请求超时，请稍后重新查询'); assert.equal(error.unknown, false)
    assert.equal(invalidations, 0); assert.equal(client.isCurrent(snapshot), true)
    assert.deepEqual(await client.listStaff(storeId), page(staff)); assert.equal(calls, 2)
    // A late unauthorized response after this request's deadline cannot invalidate its still-current context.
    pending.resolve(failure(401, 'INVALID_APP_TOKEN')); await drainMicrotasks()
    assert.equal(client.isCurrent(snapshot), true); assert.equal(invalidations, 0)
  } finally { client.dispose() }
})

test('continuous body progress does not reset the absolute deadline', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  let body, pulls = 0, cancelledBody = false
  const stream = new ReadableStream({
    start(controller) { body = controller },
    pull(controller) { if (++pulls === 1) controller.enqueue(new TextEncoder().encode('{"records":[')) },
    cancel() { cancelledBody = true },
  }, { highWaterMark: 0 })
  const client = clientWith(async () => new Response(stream), context(), { requestTimeoutMs: 100 })
  try {
    const snapshot = client.capture(), result = rejection(client.listStores())
    await drainMicrotasks()
    for (let index = 0; index < 2; index++) {
      t.mock.timers.tick(40); body.enqueue(new TextEncoder().encode(' ')); await drainMicrotasks()
    }
    assert.ok(pulls >= 3, 'multiple body reads progressed before the deadline')
    t.mock.timers.tick(20)
    assert.equal((await result).kind, 'TIMEOUT'); assert.equal(cancelledBody, true); assert.equal(client.isCurrent(snapshot), true)
  } finally { client.dispose() }
})

test('the total deadline includes a hanging business-error body, without exposing that body', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  let calls = 0, cancelledBody = false
  const client = clientWith(async () => {
    calls++
    return new Response(new ReadableStream({
      start(controller) { controller.enqueue(new TextEncoder().encode('{"message":"SECRET upstream credential')) },
      cancel() { cancelledBody = true },
    }), { status: 503 })
  }, context(), { requestTimeoutMs: 100 })
  try {
    const result = rejection(client.listStores()); await drainMicrotasks(); t.mock.timers.tick(100)
    const error = await result
    assert.equal(error.kind, 'TIMEOUT'); assert.equal(error.unknown, false); assert.equal(cancelledBody, true)
    assert.equal(JSON.stringify(error).includes('SECRET'), false); assert.equal(calls, 1)
  } finally { client.dispose() }
})

test('a dispatched write timeout is unknown, retains the original body and never retries', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  let calls = 0, sentBody
  const pending = deferred(), original = { businessDate: '2026-10-18', amount: '9007199254740993.01', note: '原输入' }
  const client = clientWith((_url, init) => { calls++; sentBody = JSON.parse(init.body); return pending.promise }, context(), { requestTimeoutMs: 100 })
  try {
    const snapshot = client.capture(), result = rejection(client.createDailyReport(storeId, original))
    t.mock.timers.tick(100)
    const error = await result
    assert.equal(error.kind, 'TIMEOUT'); assert.equal(error.code, 'REQUEST_TIMEOUT'); assert.equal(error.unknown, true)
    assert.match(error.message, /请求超时.*结果未知.*读回核对.*勿重复提交/)
    assert.deepEqual(sentBody, original); assert.equal(client.isCurrent(snapshot), true)
    t.mock.timers.tick(1000); await drainMicrotasks(); assert.equal(calls, 1)
    pending.resolve(response(report, 201)); await drainMicrotasks(); assert.equal(calls, 1)
  } finally { client.dispose() }
})

test('a completed request removes its deadline timer and cannot later abort its transport or a new context', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  let requestSignal, invalidations = 0
  const props = context(), client = clientWith(async (_url, init) => { requestSignal = init.signal; return response(page(store)) }, props,
    { requestTimeoutMs: 100, onInvalidate() { invalidations++ } })
  try {
    await client.listStores(); assert.equal(requestSignal.aborted, false)
    t.mock.timers.tick(1000); assert.equal(requestSignal.aborted, false); assert.equal(invalidations, 0)
    const next = { ...props, token: 'new-authoritative-token' }; client.setContext(next)
    t.mock.timers.tick(1000); assert.equal(client.current(), next); assert.equal(invalidations, 1)
  } finally { client.dispose() }
})

test('host abort wins over an HTTP deadline and the timeout budget remains bounded', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  const host = new AbortController(), pending = deferred()
  const client = clientWith(() => pending.promise, context({ signal: host.signal }), { requestTimeoutMs: 100 })
  try {
    const result = rejection(client.listStores()); host.abort(); t.mock.timers.tick(100)
    assert.equal((await result).name, 'AbortError'); assert.equal(client.current(), null)
  } finally { client.dispose() }
  for (const requestTimeoutMs of [0, -1, 15001, Infinity, 1.5, '100']) assert.throws(() => new StoresClient({ requestTimeoutMs }), /请求超时预算无效/)
})

test('403 headers freeze the pair immediately even when the untrusted error body never closes', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  let calls = 0, bodyCancelled = false, invalidations = 0
  const client = clientWith(async () => {
    calls++
    return new Response(new ReadableStream({
      start(controller) { controller.enqueue(new TextEncoder().encode('{"code":"DATA_DENIED","message":"SECRET')) },
      cancel() { bodyCancelled = true },
    }), { status: 403 })
  }, context(), { requestTimeoutMs: 100, onInvalidate() { invalidations++ } })
  try {
    let settled = false
    const first = rejection(client.listStores()).then(error => { settled = true; return error })
    await drainMicrotasks()
    assert.equal(settled, true, 'status 403 is authoritative before an error body or deadline')
    const error = await first
    assert.equal(error.status, 403); assert.equal(error.kind, 'FORBIDDEN'); assert.equal(error.unknown, false)
    assert.equal(JSON.stringify(error).includes('SECRET'), false); assert.equal(bodyCancelled, true)
    await assert.rejects(client.listStores(), error => error.status === 403)
    assert.equal(calls, 1); assert.equal(invalidations, 0); assert.ok(client.current())
  } finally { client.dispose() }
})

test('a same-pair parallel 200 cannot return data after another request receives 403', async () => {
  const summary = deferred(), deniedResponse = deferred(); let calls = 0
  const client = clientWith(async url => {
    calls++
    if (url.includes('/summary')) return summary.promise
    if (url.endsWith('/staff')) return response(page(staff))
    return deniedResponse.promise
  })
  try {
    const pendingSummary = rejection(client.getSummary({ dateFrom: '2026-10-18', dateTo: '2026-10-18' }))
    const pendingList = rejection(client.listDailyReports(storeId))
    deniedResponse.resolve(failure(403, 'DATA_DENIED'))
    assert.equal((await pendingList).status, 403)
    summary.resolve(response({ count: 1, totalAmount: '42.00', dateFrom: '2026-10-18', dateTo: '2026-10-18', storeId: null }))
    const result = await pendingSummary
    assert.ok(result instanceof BusinessError, 'the parallel result must be fenced rather than exposed as data')
    assert.equal(result.status, 403); assert.equal(result.kind, 'FORBIDDEN')
    assert.deepEqual(await client.listStaff(storeId), page(staff)); assert.equal(calls, 3)
  } finally { client.dispose() }
})

test('a 403 response is a definite rejection while a concurrent dispatched write remains unknown', async () => {
  const committed = deferred(); let calls = 0
  const client = clientWith(async () => ++calls === 1 ? committed.promise : failure(403, 'DATA_DENIED'))
  const body = { name: '门店', businessStatus: 'OPEN', businessContact: '' }
  try {
    const pendingCommit = rejection(client.createStore(body))
    const rejected = await rejection(client.createStore(body))
    assert.equal(rejected.status, 403); assert.equal(rejected.unknown, false)
    const uncertain = await pendingCommit
    assert.equal(uncertain.unknown, true); assert.equal(uncertain.kind, 'CANCELLED')
    assert.match(uncertain.message, /结果未知.*查询核对.*勿重复提交/)
    committed.resolve(response(store, 201)); await drainMicrotasks()
    assert.equal(calls, 2); assert.ok(client.current())
  } finally { client.dispose() }
})

test('401 headers invalidate immediately without waiting for an untrusted body or deadline', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: Date.parse('2026-10-18T01:00:00.000Z') })
  let bodyCancelled = false, invalidations = 0
  const client = clientWith(async () => new Response(new ReadableStream({
    start(controller) { controller.enqueue(new TextEncoder().encode('{"message":"SECRET')) },
    cancel() { bodyCancelled = true },
  }), { status: 401 }), context(), { requestTimeoutMs: 100, onInvalidate() { invalidations++ } })
  try {
    const result = rejection(client.listStores()); await drainMicrotasks()
    const error = await result
    assert.equal(error.status, 401); assert.equal(error.kind, 'UNAUTHORIZED'); assert.equal(error.unknown, false)
    assert.equal(client.current(), null); assert.equal(invalidations, 1); assert.equal(bodyCancelled, true)
    assert.equal(JSON.stringify(error).includes('SECRET'), false)
  } finally { client.dispose() }
})

test('a late 403 from an old context cannot freeze that pair under replacement credentials', async () => {
  const old = deferred(); let calls = 0
  const props = context(), client = clientWith(async () => ++calls === 1 ? old.promise : response(page(store)), props)
  try {
    const oldRead = rejection(client.listStores())
    const next = { ...props, token: 'new-authorized-token' }; client.setContext(next)
    old.resolve(failure(403, 'DATA_DENIED')); assert.equal((await oldRead).name, 'AbortError')
    await drainMicrotasks()
    assert.deepEqual(await client.listStores(), page(store)); assert.equal(client.current(), next); assert.equal(calls, 2)
  } finally { client.dispose() }
})
