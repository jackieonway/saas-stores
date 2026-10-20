import React, { useState, useSyncExternalStore } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { StoresController, type Form, type FormKind } from './controller.ts'
import { canRoute, canMemberLookup } from './domain.ts'
import type { Operation, Store, DailyReport, MemberPurpose } from './domain.ts'
import type { MicroLifecycle } from './sdk.ts'
import './styles.css'

const base = '/micro/stores'
const navigation = [
  ['', '门店台账', '01'], ['/store-create', '新建门店', '+'], ['/staff', '员工查询', '02'],
  ['/staff-assign', '关联员工', '+'], ['/reports', '营业日报', '03'], ['/daily-report-create', '录入日报', '+'],
] as const
const operation = (resource: Operation['resource'], action: Operation['action']) => ({ resource, action }) as Operation
const storeRead = operation('store', 'read'), reportRead = operation('store-daily-report', 'read')
const statuses = { OPEN: '营业中', CLOSED: '已停业' }
const titles: Record<string, [string, string]> = {
  stores: ['门店台账', '查阅门店资料，维护每一处营业现场。'],
  'store-create': ['新建门店', '一店一组织。负责人从当前授权的成员目录中选择。'],
  'store-detail': ['门店资料', '维护营业状态、业务联系信息与负责人。'],
  staff: ['员工查询', '按门店查询当前有效的员工关联。'],
  'staff-assign': ['关联员工', '选择门店与有效成员，建立新的员工关联。'],
  reports: ['营业日报', '按营业日查阅记录，由服务端精确汇总。'],
  'report-create': ['录入日报', '请核对门店、营业日期与金额后提交。'],
  'report-detail': ['日报详情', '营业日期与门店固定；修改金额或备注时校验版本。'],
  'not-found': ['页面不存在', '此地址没有对应的门店业务页面。'],
}

function Field({ form, controller, name, label, type = 'text', hint, multiline = false, children, readOnly = false }: {
  form: Form; controller: StoresController; name: string; label: string; type?: string; hint?: string;
  multiline?: boolean; children?: React.ReactNode; readOnly?: boolean;
}) {
  const id = `${form.id}-${name}`, locked = form.pending || !!form.sent || readOnly
  const shared = { id, name, value: form.fields[name] ?? '', disabled: locked, onChange: (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) => controller.patch(form.id, name, event.target.value), 'aria-describedby': hint ? `${id}-hint` : undefined }
  return <label className={`field field-${name}`} htmlFor={id}>
    <span>{label}</span>
    {children ? <select {...shared}>{children}</select> : multiline ? <textarea {...shared} rows={4} /> : <input {...shared} type={type} inputMode={name === 'amount' ? 'decimal' : undefined} autoComplete="off" spellCheck={false} />}
    {hint && <small id={`${id}-hint`}>{hint}</small>}
  </label>
}
function Notice({ children, tone = 'info' }: { children: React.ReactNode; tone?: 'info' | 'error' | 'success' }) {
  return <div className={`notice notice-${tone}`} role={tone === 'error' ? 'alert' : 'status'}>{children}</div>
}
function Permission({ children = '当前授权没有此项读取权限。可用操作请从左侧菜单进入。' }: { children?: React.ReactNode }) {
  return <div className="empty permission"><span className="empty-mark">—</span><h3>暂不可访问</h3><p>{children}</p></div>
}
function Empty({ children = '暂无符合条件的记录。' }: { children?: React.ReactNode }) { return <div className="empty"><span className="empty-mark">∅</span><p>{children}</p></div> }
function Loading() { return <div className="loading" role="status"><i />正在读取服务端记录…</div> }
function Pager({ page, total, size, previous, next, busy }: { page: number; total: number; size: number; previous: () => void; next: () => void; busy?: boolean }) {
  return <div className="pagination"><span>共 <b>{total}</b> 条 · 第 {page} 页</span><div><button type="button" className="quiet" disabled={busy || page <= 1} onClick={previous}>上一页</button><button type="button" className="quiet" disabled={busy || page * size >= total} onClick={next}>下一页</button></div></div>
}
function RowMeta({ row }: { row: Store | DailyReport }) {
  return <dl className="record-meta"><div><dt>记录 ID</dt><dd className="uuid">{row.id}</dd></div><div><dt>所属组织</dt><dd className="uuid">{row.organizationId}</dd></div><div><dt>录入人 ID</dt><dd className="uuid">{row.ownerUserId}</dd></div><div><dt>服务端版本</dt><dd className="numeric">v{row.version}</dd></div></dl>
}
function Recovery({ form, controller }: { form: Form; controller: StoresController }) {
  const sent = form.sent
  return <>
    {sent?.inFlight && <Notice>原写入请求仍在进行中。请等待结果；当前不能读回确认或再次提交。</Notice>}
    {form.error && <Notice tone="error">{form.error}</Notice>}
    {form.conflict && !sent && <Notice tone="error">{form.kind.endsWith('-edit') || form.kind === 'staff-unassign' ? '版本已变化。原输入保留，请读取最新版本、核对后使用该版本。' : '已有相同门店、员工关联或营业日期的记录。请到对应查询页核对，勿重复提交。'}</Notice>}
    {(form.awaitingRead || form.conflict) && (form.kind.endsWith('-edit') || form.kind === 'staff-unassign') && !sent && <div className="actions recovery-actions"><button type="button" className="secondary" onClick={() => void controller.readLatest(form.id)}>读取最新版本</button></div>}
    {form.latest && !sent && <div className="recovery-panel"><h3>服务端最新版本 <span className="numeric">v{form.latest.version}</span></h3><p>原输入仍在表单中。请核对服务端当前值，再决定是否继续保存。</p><dl className="compare-values">{'name' in form.latest ? <><dt>门店名称</dt><dd>{form.latest.name}</dd><dt>营业状态</dt><dd>{statuses[form.latest.businessStatus]}</dd><dt>业务联系信息</dt><dd>{form.latest.businessContact || '—'}</dd><dt>负责人</dt><dd>{form.latest.managerDisplayName || '未设置'}</dd></> : 'amount' in form.latest ? <><dt>营业金额</dt><dd className="money">{form.latest.amount}</dd><dt>备注</dt><dd>{form.latest.note || '—'}</dd></> : <><dt>员工</dt><dd>{form.latest.displayName}</dd></>}</dl><button type="button" className="secondary" onClick={() => controller.useLatest(form.id)}>使用最新版本</button></div>}
    {sent?.outcome === 'unknown' && !sent.inFlight && <div className="recovery-panel" data-testid="unknown-write"><h3>写入结果未知</h3><p>已保留原组织、目标与发送内容。请保持当前页面开启；刷新、关闭或卸载页面可能丢失本地核对信息。</p><dl className="compare-values"><dt>原门店</dt><dd className="uuid">{sent.fields.storeId || sent.target?.id || '当前组织门店'}</dd>{sent.fields.businessDate && <><dt>原营业日期</dt><dd>{sent.fields.businessDate}</dd></>}{sent.fields.userId && <><dt>原员工 ID</dt><dd className="uuid">{sent.fields.userId}</dd></>}{sent.fields.amount && <><dt>原营业金额</dt><dd className="money">{sent.fields.amount}</dd></>}{sent.version && <><dt>发送时版本</dt><dd>v{sent.version}</dd></>}</dl>
      {controller.readbackPermission(form) ? <button type="button" className="secondary" onClick={() => void controller.readback(form.id)}>读回核对</button> : <Notice>当前没有对应读取权限，无法核对本次写入。原输入已保留；请取得读取授权后核对，勿直接重复提交。</Notice>}
      {form.readback && <><h4>本次读回的服务端记录</h4>{form.readback.length ? <div className="readback-records">{form.readback.map(row => <dl key={row.id}><dt className="uuid">{row.id} · v{row.version}</dt><dd>{'name' in row ? `${row.name} / ${statuses[row.businessStatus]} / ${row.businessContact} / ${row.managerDisplayName || '无负责人'}` : 'amount' in row ? `${row.businessDate} / ${row.amount} / ${row.note}` : `${row.displayName} / ${row.userId}`}</dd><dd className="uuid">组织 {row.organizationId} · 录入人 {row.ownerUserId}</dd></dl>)}</div> : <p>当前授权范围内未查到对应记录。请结合原目标与服务端状态判断。</p>}<button type="button" className="secondary" onClick={() => controller.acknowledgeReadback(form.id)}>已核对，允许新操作</button></>}
    </div>}
  </>
}
function Submit({ form, controller, children }: { form: Form; controller: StoresController; children: React.ReactNode }) {
  return <div className="actions"><button type="submit" className="primary" disabled={form.pending || !!form.sent || form.awaitingRead}>{form.pending ? '正在提交…' : children}</button>{form.version && <span className="version">当前提交基于 v{form.version}</span>}</div>
}
function BusinessForm({ form, controller, children, className = '' }: { form: Form; controller: StoresController; children: React.ReactNode; className?: string }) {
  return <form className={`business-form ${className}`} data-form-kind={form.kind} data-form-id={form.id} onSubmit={event => { event.preventDefault(); void controller.submit(form.id) }}>{children}<Recovery form={form} controller={controller} /></form>
}
function MemberPicker({ form, controller }: { form: Form; controller: StoresController }) {
  const state = controller.state, props = state.props
  const [query, setQuery] = useState('')
  const purpose = operation(form.kind === 'staff-assign' ? 'store-staff' : 'store', form.kind === 'store-create' ? 'create' : form.kind === 'staff-assign' ? 'assign' : 'update') as MemberPurpose
  const organizationId = form.target?.organizationId ?? props?.organization.id
  if (!props || !organizationId || !canMemberLookup(props, purpose, organizationId)) return null
  const directory = state.directoryFormId === form.id ? state.directory : null
  return <section className="directory" aria-label="受控成员目录"><div className="section-heading"><h3>{form.kind === 'staff-assign' ? '选择关联员工' : '选择门店负责人'}</h3><span>受控成员目录</span></div><div className="inline-search"><label htmlFor={`${form.id}-member-search`}>成员搜索<input id={`${form.id}-member-search`} value={query} disabled={form.pending || !!form.sent} onChange={event => setQuery(event.target.value)} placeholder="按显示名称搜索" /></label><button type="button" className="secondary" disabled={form.pending || !!form.sent} onClick={() => void controller.searchMembers(form.id, query)}>搜索成员</button></div>
    {state.loading.has('directory') && state.directoryFormId === form.id && <Loading />}
    {directory && (directory.records.length ? <><ul className="member-list">{directory.records.map(member => <li key={member.id}><div><strong>{member.displayName}</strong><small className="uuid">{member.id}</small></div><button type="button" className="quiet" disabled={form.pending || !!form.sent} onClick={() => controller.selectMember(form.id, member.id)}>选择 {member.displayName}</button></li>)}</ul><Pager {...directory} previous={() => void controller.searchMembers(form.id, state.directoryQuery, directory.page - 1)} next={() => void controller.searchMembers(form.id, state.directoryQuery, directory.page + 1)} /></> : <Empty>当前用途与组织内没有符合条件的有效成员。</Empty>)}
  </section>
}
function StoreTarget({ form, controller, optional = false }: { form: Form; controller: StoresController; optional?: boolean }) {
  const [search, setSearch] = useState(''), state = controller.state
  return <><Field form={form} controller={controller} name="storeId" label="门店ID" hint={optional ? '留空汇总当前授权范围内全部门店；填写 UUID 可收窄范围。' : '可直接输入门店 UUID；此操作只校验自己的业务权限。'} />
    {controller.allowed(storeRead) && <details className="store-selector"><summary>从可读门店选择</summary><div className="inline-search"><label htmlFor={`${form.id}-store-search`}>门店搜索<input id={`${form.id}-store-search`} value={search} onChange={event => setSearch(event.target.value)} /></label><button type="button" className="secondary" disabled={form.pending || !!form.sent} onClick={() => void controller.loadStoreChoices(form.id, search)}>搜索门店</button></div>{state.loading.has('choices') && <Loading />}{state.storeChoices && <><ul className="member-list">{state.storeChoices.records.map(store => <li key={store.id}><div><strong>{store.name}</strong><small className="uuid">{store.id}</small></div><button type="button" className="quiet" disabled={form.pending || !!form.sent} onClick={() => controller.selectStore(form.id, store.id)}>选择 {store.name}</button></li>)}</ul>{state.storeChoices.records.length === 0 && <Empty />}<Pager {...state.storeChoices} previous={() => void controller.loadStoreChoices(form.id, search, state.storeChoices!.page - 1)} next={() => void controller.loadStoreChoices(form.id, search, state.storeChoices!.page + 1)} /></>}</details>}
  </>
}
function StoreFields({ form, controller }: { form: Form; controller: StoresController }) {
  return <><div className="form-grid"><Field form={form} controller={controller} name="name" label="门店名称" /><Field form={form} controller={controller} name="businessStatus" label="营业状态"><option value="OPEN">营业中</option><option value="CLOSED">已停业</option></Field><Field form={form} controller={controller} name="businessContact" label="业务联系信息" hint="选填，最多 200 个字符。" /><Field form={form} controller={controller} name="managerUserId" label="负责人用户ID" hint="从下方目录选择或输入有效 UUID；留空表示无负责人。" /></div><MemberPicker key={form.id} form={form} controller={controller} /></>
}
function MoneyFields({ form, controller }: { form: Form; controller: StoresController }) {
  return <><Field form={form} controller={controller} name="amount" label="营业金额" hint="元。非负十进制文本，最多 16 位整数、2 位小数。" /><Field form={form} controller={controller} name="note" label="备注" multiline hint="选填，最多 8192 个字符。" /></>
}
function Dates({ form, controller }: { form: Form; controller: StoresController }) {
  return <div className="form-grid"><Field form={form} controller={controller} name="dateFrom" label="开始日期" type="date" /><Field form={form} controller={controller} name="dateTo" label="结束日期" type="date" /></div>
}
function StoresPage({ controller }: { controller: StoresController }) {
  const state = controller.state, form = controller.form('store-query')!, result = state.stores
  if (!controller.allowed(storeRead)) return <Permission />
  return <><BusinessForm form={form} controller={controller} className="filter-form"><div className="filter-fields"><Field form={form} controller={controller} name="q" label="搜索门店" hint="按名称或业务联系信息检索。" /><Field form={form} controller={controller} name="businessStatus" label="营业状态"><option value="">全部状态</option><option value="OPEN">营业中</option><option value="CLOSED">已停业</option></Field><button className="secondary" type="submit">查询门店</button></div></BusinessForm>
    {state.loading.has('stores') ? <Loading /> : result && (result.records.length ? <><div className="table-wrap"><table><thead><tr><th>门店</th><th>营业状态</th><th>负责人</th><th>业务联系信息</th><th className="align-right">操作</th></tr></thead><tbody>{result.records.map(store => <tr key={store.id} data-row-id={store.id}><td><strong>{store.name}</strong><small className="uuid">{store.id}</small></td><td><span className={`status status-${store.businessStatus}`}>{statuses[store.businessStatus]}</span></td><td>{store.managerDisplayName || '未设置'}</td><td>{store.businessContact || '—'}</td><td className="align-right"><button type="button" className="text-button" disabled={!state.props || !canRoute(state.props, `${base}/stores/${store.id}`)} onClick={() => controller.navigate(`${base}/stores/${store.id}`)}>查看资料 ↗</button></td></tr>)}</tbody></table></div><Pager {...result} previous={() => void controller.paginate(form.id, -1)} next={() => void controller.paginate(form.id, 1)} /></> : <Empty>暂无符合条件的门店；可通过“新建门店”登记当前组织门店。</Empty>)}
  </>
}
function StoreEditor({ controller, create = false }: { controller: StoresController; create?: boolean }) {
  const state = controller.state, form = controller.form(create ? 'store-create' : 'store-edit')!, detail = state.detail as Store | null
  const writable = controller.allowed(operation('store', create ? 'create' : 'update'), form.target)
  if (!create && state.loading.has('detail')) return <Loading />
  if (!create && !controller.allowed(storeRead)) return <Permission />
  return <>{!create && detail && <><RowMeta row={detail} /><div className="record-overview"><h2>{detail.name}</h2><span className={`status status-${detail.businessStatus}`}>{statuses[detail.businessStatus]}</span><p>负责人：{detail.managerDisplayName || '未设置'}</p></div></>}
    {writable ? <BusinessForm form={form} controller={controller}><StoreFields form={form} controller={controller} /><Submit form={form} controller={controller}>{create ? '创建门店' : '保存门店'}</Submit></BusinessForm> : !create && detail ? <dl className="compare-values"><dt>业务联系信息</dt><dd>{detail.businessContact || '—'}</dd><dt>负责人 ID</dt><dd className="uuid">{detail.managerUserId || '—'}</dd></dl> : <Permission>当前授权不允许创建或修改门店。</Permission>}
    {!create && !detail && !state.loading.has('detail') && !writable && <Notice>记录不存在、暂不可读，或已超出当前授权范围。</Notice>}
  </>
}
function StaffPage({ controller, assign = false }: { controller: StoresController; assign?: boolean }) {
  const state = controller.state, form = controller.form(assign ? 'staff-assign' : 'staff-query')!, result = state.staff
  if (!controller.allowed(operation('store-staff', assign ? 'assign' : 'read'))) return <Permission />
  return <><BusinessForm form={form} controller={controller}><StoreTarget key={form.id} form={form} controller={controller} />{assign && <><Field form={form} controller={controller} name="userId" label="员工用户ID" hint="选择下方有效成员或输入用户 UUID。" /><MemberPicker key={form.id} form={form} controller={controller} /></>}<Submit form={form} controller={controller}>{assign ? '关联员工' : '查询员工'}</Submit></BusinessForm>
    {!assign && (state.loading.has('staff') ? <Loading /> : result ? result.records.length ? <><div className="table-wrap"><table><thead><tr><th>员工</th><th>关联 ID</th><th>版本</th><th className="align-right">操作</th></tr></thead><tbody>{result.records.map(link => { const unlink = state.forms.find(candidate => candidate.kind === 'staff-unassign' && candidate.key.endsWith(`|${link.id}`)); return <tr key={link.id} data-row-id={link.id}><td><strong>{link.displayName}</strong><small className="uuid">{link.userId}</small></td><td className="uuid">{link.id}</td><td className="numeric">v{link.version}</td><td className="align-right">{unlink && controller.allowed(operation('store-staff', 'unassign'), link) ? <><button type="button" className="text-button danger" disabled={unlink.pending || !!unlink.sent} onClick={() => void controller.submit(unlink.id)}>{unlink.pending ? '正在解除…' : '解除关联'}</button>{!unlink.sent && <Recovery form={unlink} controller={controller} />}</> : <span className="muted">无解除权限</span>}</td></tr> })}</tbody></table></div><Pager {...result} previous={() => void controller.paginate(form.id, -1)} next={() => void controller.paginate(form.id, 1)} /></> : <Empty>此门店当前授权内没有有效员工关联。</Empty> : <Empty>输入门店 ID，查询有效员工关联。</Empty>)}
    {!assign && state.forms.filter(candidate => candidate.kind === 'staff-unassign' && candidate.sent).map(candidate => <Recovery key={candidate.id} form={candidate} controller={controller} />)}
  </>
}
function ReportsPage({ controller }: { controller: StoresController }) {
  const state = controller.state, form = controller.form('report-query')!, summaryForm = controller.form('summary')!, result = state.reports
  if (!controller.allowed(reportRead)) return <Permission />
  return <><section className="ledger-section"><div className="section-heading"><h2>每日记录</h2><span>每店每日一份</span></div><BusinessForm form={form} controller={controller}><StoreTarget key={form.id} form={form} controller={controller} /><Dates form={form} controller={controller} /><Submit form={form} controller={controller}>查询日报</Submit></BusinessForm>
    {state.loading.has('reports') ? <Loading /> : result ? result.records.length ? <><div className="table-wrap"><table><thead><tr><th>营业日期</th><th className="align-right">营业金额（元）</th><th>备注</th><th>版本</th><th className="align-right">操作</th></tr></thead><tbody>{result.records.map(report => <tr key={report.id} data-row-id={report.id}><td className="numeric">{report.businessDate}</td><td className="money align-right">{report.amount}</td><td className="note-cell">{report.note || '—'}</td><td className="numeric">v{report.version}</td><td className="align-right"><button type="button" className="text-button" disabled={!state.props || !canRoute(state.props, `${base}/daily-reports/${report.id}`)} onClick={() => controller.navigate(`${base}/daily-reports/${report.id}`)}>查看日报 ↗</button></td></tr>)}</tbody></table></div><Pager {...result} previous={() => void controller.paginate(form.id, -1)} next={() => void controller.paginate(form.id, 1)} /></> : <Empty>当前门店与日期范围内暂无可读日报。</Empty> : <Empty>输入门店与日期范围，查询每日记录。</Empty>}</section>
    <section className="ledger-section summary-section"><div className="section-heading"><h2>营业汇总</h2><span>服务端精确汇总 · 全部符合范围的记录</span></div><BusinessForm form={summaryForm} controller={controller}><StoreTarget key={summaryForm.id} form={summaryForm} controller={controller} optional /><Dates form={summaryForm} controller={controller} /><Submit form={summaryForm} controller={controller}>查询汇总</Submit></BusinessForm>{state.loading.has('summary') && <Loading />}{state.summary && <div className="summary-result" data-testid="server-summary"><div><small>服务端营业总额（元）</small><strong className="money">{state.summary.totalAmount}</strong></div><dl><div><dt>日报记录</dt><dd>{state.summary.count} 份</dd></div><div><dt>营业日期</dt><dd>{state.summary.dateFrom} 至 {state.summary.dateTo}</dd></div><div><dt>门店范围</dt><dd className={state.summary.storeId ? 'uuid' : ''}>{state.summary.storeId || '当前授权内全部门店'}</dd></div></dl></div>}</section>
  </>
}
function ReportEditor({ controller, create = false }: { controller: StoresController; create?: boolean }) {
  const state = controller.state, form = controller.form(create ? 'report-create' : 'report-edit')!, detail = state.detail as DailyReport | null
  if (!create && state.loading.has('detail')) return <Loading />
  if (!create && !controller.allowed(reportRead)) return <Permission />
  const writable = controller.allowed(operation('store-daily-report', create ? 'create' : 'update'), form.target)
  return <>{!create && detail && <><RowMeta row={detail} /><div className="record-overview"><h2>{detail.businessDate}</h2><strong className="money">{detail.amount} <small>元</small></strong><p className="uuid">门店 {detail.storeId}</p></div></>}{writable ? <BusinessForm form={form} controller={controller}>{create && <><StoreTarget key={form.id} form={form} controller={controller} /><Field form={form} controller={controller} name="businessDate" label="营业日期" type="date" hint="默认取 Asia/Shanghai 当天；提交时发送当前确认日期，日期不会因跨午夜自动变化。" /></>}<MoneyFields form={form} controller={controller} /><Submit form={form} controller={controller}>{create ? '录入日报' : '保存日报'}</Submit></BusinessForm> : detail ? <dl className="compare-values"><dt>备注</dt><dd>{detail.note || '—'}</dd></dl> : <Permission>当前授权不允许录入或修改日报。</Permission>}</>
}
function Page({ controller }: { controller: StoresController }) {
  switch (controller.state.route.kind) {
    case 'stores': return <StoresPage controller={controller} />
    case 'store-create': return <StoreEditor controller={controller} create />
    case 'store-detail': return <StoreEditor controller={controller} />
    case 'staff': return <StaffPage controller={controller} />
    case 'staff-assign': return <StaffPage controller={controller} assign />
    case 'reports': return <ReportsPage controller={controller} />
    case 'report-create': return <ReportEditor controller={controller} create />
    case 'report-detail': return <ReportEditor controller={controller} />
    default: return <Empty>请通过已授权菜单进入门店业务。</Empty>
  }
}
function App({ controller }: { controller: StoresController }) {
  useSyncExternalStore(controller.subscribe, controller.snapshot, controller.snapshot)
  const state = controller.state, props = state.props, title = titles[state.route.kind]
  return <div className="stores-app" data-testid="stores-app" data-theme={props?.theme ?? 'light'}>
    <aside className="app-sidebar"><div className="brand"><span className="brand-seal">店</span><div><strong>营业台账</strong><small>STORES / OPERATIONS</small></div></div><div className="sidebar-rule" /><nav aria-label="门店业务导航">{navigation.map(([path, label, number]) => props && canRoute(props, base + path) ? <button type="button" key={path} aria-current={state.routeKey === base + path ? 'page' : undefined} onClick={() => controller.navigate(base + path)}><span>{number}</span>{label}</button> : null)}</nav><div className="sidebar-context"><small>当前营业组织</small><strong>{props?.organization.name || '等待有效授权'}</strong><span>{props?.tenant.name || '由平台提供身份与授权'}</span></div></aside>
    <main className="app-main"><div className="topline"><span>门店运营 / BUSINESS LEDGER</span><span>{props?.user.displayName || '身份未就绪'}</span></div><header className="page-header"><div><p className="eyebrow">{state.route.kind.startsWith('report') ? 'DAILY RECORDS' : state.route.kind.startsWith('staff') ? 'STAFF DIRECTORY' : 'STORE REGISTER'}</p><h1>{title[0]}</h1><p>{title[1]}</p></div><div className="edition"><span>营业资料簿</span><strong>STORE<span> / </span>01</strong></div></header>
      {!state.active ? <Permission>当前上下文已失效或正在更新。请等待平台重新提供有效授权。</Permission> : state.route.kind === 'not-found' ? <Empty>此地址没有对应的门店业务页面，请通过已授权菜单进入。</Empty> : !controller.routeAllowed() ? <Permission>当前页面需要平台登记的菜单授权。资源权限与菜单权限分别校验。</Permission> : <div key={state.routeKey} className="page-body">{state.error && <Notice tone="error">{state.error}</Notice>}{state.notice && <Notice tone="success">{state.notice}</Notice>}<Page controller={controller} /></div>}
      <footer className="app-footer"><span>数据范围由当前应用授权决定</span><span>金额单位：元 · 营业日：Asia/Shanghai</span></footer>
    </main>
  </div>
}

export function createLifecycle(_bridge: unknown): MicroLifecycle {
  let root: Root | null = null, controller: StoresController | null = null
  return {
    name: 'stores',
    mount(container, props) {
      if (root) return
      const config = (window as unknown as { __STORES_CONFIG__?: { apiBase?: string } }).__STORES_CONFIG__
      controller = new StoresController({ apiBase: config?.apiBase })
      controller.setContext(props); root = createRoot(container); root.render(<App controller={controller} />)
    },
    update(props) { controller?.setContext(props) },
    unmount() { controller?.dispose(); controller = null; root?.unmount(); root = null },
  }
}
