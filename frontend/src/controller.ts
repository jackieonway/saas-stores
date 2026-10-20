import { StoresClient, BusinessError } from './client.ts'
import { can, canRoute, canMemberLookup, identityKey, routeView, validContext, validUUID, validBusinessDate, validAmount, shanghaiToday } from './domain.ts'
import type { Props, Store, StaffLink, DailyReport, Member, Page, Summary, Operation, MemberPurpose } from './domain.ts'

type Row = Store | StaffLink | DailyReport
export type FormKind = 'store-query' | 'store-create' | 'store-edit' | 'staff-query' | 'staff-assign' | 'staff-unassign' | 'report-query' | 'report-create' | 'report-edit' | 'summary'
export type Fields = Record<string, string>
export type SentWrite = { operation: Operation; fields: Fields; target: Row | null; version: number | null; identity: string; outcome: 'pending' | 'unknown'; inFlight: boolean; checked: boolean }
export type Form = {
  id: string; key: string; kind: FormKind; identity: string; route: string; epoch: number; fields: Fields;
  target: Row | null; version: number | null; latest: Row | null; dirty: boolean; awaitingRead: boolean;
  pending: boolean; error: string; conflict: boolean; sent: SentWrite | null; readback: Row[] | null;
}
export type ControllerState = {
  revision: number; active: boolean; props: Props | null; route: ReturnType<typeof routeView>; routeKey: string;
  forms: Form[]; stores: Page<Store> | null; storeChoices: Page<Store> | null; staff: Page<StaffLink> | null;
  reports: Page<DailyReport> | null; detail: Store | DailyReport | null; summary: Summary | null;
  directory: Page<Member> | null; directoryFormId: string | null; directoryQuery: string; directoryPage: number;
  loading: Set<string>; denied: Set<string>; error: string; notice: string;
}
type Client = Pick<StoresClient, 'setContext' | 'current' | 'capture' | 'isCurrent' | 'dispose' | 'listStores' | 'getStore' | 'createStore' | 'updateStore' | 'listStaff' | 'assignStaff' | 'unassignStaff' | 'listDailyReports' | 'getDailyReport' | 'createDailyReport' | 'updateDailyReport' | 'getSummary' | 'listMembers'>
type Options = { client?: Client; apiBase?: string; fetchImpl?: typeof fetch; now?: () => number }
const op = (resource: Operation['resource'], action: Operation['action']) => ({ resource, action }) as Operation
const STORE_READ = op('store', 'read'), REPORT_READ = op('store-daily-report', 'read'), STAFF_READ = op('store-staff', 'read')
const operationKey = (operation: Operation) => `${operation.resource}:${operation.action}`
const message = (error: unknown) => error instanceof BusinessError ? error.message : '操作暂未完成，请稍后再试。'

/** An application-local state machine. No tokens, host storage, or browser globals enter draft records. */
export class StoresController {
  readonly state: ControllerState = { revision: 0, active: false, props: null, route: { kind: 'not-found' }, routeKey: '', forms: [], stores: null, storeChoices: null, staff: null, reports: null, detail: null, summary: null, directory: null, directoryFormId: null, directoryQuery: '', directoryPage: 1, loading: new Set(), denied: new Set(), error: '', notice: '' }
  private client: Client
  private now: () => number
  private listeners = new Set<() => void>()
  private drafts = new Map<string, Map<string, Form>>()
  private epoch = 0
  private serial = 0
  private acceptedIdentity = ''
  private routeSequence = 0
  private reads = new Map<string, AbortController>()
  private readOperations = new Map<string, string>()
  private readSerial = new Map<string, number>()
  private disposed = false

  constructor(options: Options = {}) {
    this.now = options.now ?? Date.now
    this.client = options.client ?? new StoresClient({ apiBase: options.apiBase, fetchImpl: options.fetchImpl, onInvalidate: () => this.invalidate() })
  }
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener) }
  snapshot = () => this.state.revision
  private emit() { this.state.revision++; for (const listener of this.listeners) listener() }
  private cancelReads() { for (const controller of this.reads.values()) controller.abort(); this.reads.clear(); this.readOperations.clear(); this.state.loading.clear() }
  private clearServer() { this.state.stores = null; this.state.staff = null; this.state.reports = null; this.state.detail = null; this.state.summary = null; this.state.directory = null; this.state.directoryFormId = null; this.state.storeChoices = null }
  private saveDrafts() {
    if (!this.acceptedIdentity) return
    const saved = this.drafts.get(this.acceptedIdentity) ?? new Map<string, Form>()
    for (const form of this.state.forms) saved.set(form.key, form)
    this.drafts.set(this.acceptedIdentity, saved)
  }
  private invalidate() {
    this.saveDrafts(); this.epoch++; this.routeSequence++; this.cancelReads(); this.clearServer()
    this.state.active = false; this.state.props = null; this.state.forms = []; this.state.error = ''; this.state.notice = ''
    this.emit()
  }
  setContext(props: Props) {
    if (this.disposed) return
    this.saveDrafts()
    const previousIdentity = this.acceptedIdentity
    const previousRoute = this.state.routeKey
    let replaced: boolean
    try { replaced = this.client.setContext(props) } catch { this.invalidate(); return }
    const current = this.client.current()
    if (!current || !validContext(current, this.now())) { this.invalidate(); return }
    const identity = identityKey(current), routeKey = current.routePath
    const routeChanged = routeKey !== previousRoute
    if (replaced || identity !== previousIdentity) {
      this.epoch++; this.routeSequence++; this.cancelReads(); this.clearServer(); this.state.denied.clear()
      this.state.forms = []; this.state.error = ''; this.state.notice = ''
    } else if (routeChanged) {
      this.routeSequence++; this.cancelReads(); this.clearServer(); this.state.forms = []; this.state.error = ''; this.state.notice = ''
    }
    this.acceptedIdentity = identity
    this.state.active = true; this.state.props = current; this.state.routeKey = routeKey; this.state.route = routeView(routeKey)
    if (replaced || identity !== previousIdentity || routeChanged || this.state.forms.length === 0) {
      this.prepareForms(replaced)
      this.emit()
      void this.enterRoute()
    } else this.emit() // Theme/callback-only changes keep both requests and form identities.
  }
  navigate(path: string) { const props = this.client.current(); if (props && canRoute(props, path)) props.onNavigate(path) }
  allowed(operation: Operation, row?: Row | null) { return this.state.active && !this.state.denied.has(operationKey(operation)) && can(this.client.current(), operation, row ?? undefined) }
  routeAllowed() { const props = this.client.current(); return !!props && canRoute(props, this.state.routeKey) }
  form(kind: FormKind) { return this.state.forms.find(form => form.kind === kind) }
  private live(formId: string): Form | null {
    const form = this.state.forms.find(candidate => candidate.id === formId), props = this.client.current()
    return form && props && this.state.active && validContext(props, this.now()) && form.identity === identityKey(props) && form.identity === this.acceptedIdentity && form.route === this.state.routeKey && props.routePath === form.route && form.epoch === this.epoch && this.routeAllowed() ? form : null
  }
  private addForm(kind: FormKind, fields: Fields, target: Row | null = null, renewed = false): Form {
    const key = `${this.state.routeKey}|${kind}|${target?.id ?? ''}`
    const saved = this.drafts.get(this.acceptedIdentity)?.get(key)
    const form: Form = {
      id: `stores-form-${++this.serial}`, key, kind, identity: this.acceptedIdentity, route: this.state.routeKey, epoch: this.epoch,
      fields: saved ? { ...saved.fields } : fields, target: renewed ? null : target, version: renewed ? null : target?.version ?? null,
      latest: null, dirty: saved?.dirty ?? false, awaitingRead: kind.endsWith('-edit') || kind === 'staff-unassign',
      pending: saved?.sent?.inFlight ?? false, error: '', conflict: saved?.conflict ?? false, sent: saved?.sent ?? null, readback: null,
    }
    this.state.forms.push(form)
    return form
  }
  private prepareForms(renewed: boolean) {
    const today = shanghaiToday(this.now())
    switch (this.state.route.kind) {
      case 'stores': this.addForm('store-query', { q: '', businessStatus: '', page: '1' }); break
      case 'store-create': this.addForm('store-create', { name: '', businessStatus: 'OPEN', businessContact: '', managerUserId: '' }); break
      case 'store-detail': this.addForm('store-edit', { name: '', businessStatus: 'OPEN', businessContact: '', managerUserId: '' }, null, renewed); break
      case 'staff':
        this.addForm('staff-query', { storeId: '', page: '1' })
        // A removed link is absent from the next list; recovery must come from its original event, not rows.
        for (const saved of this.drafts.get(this.acceptedIdentity)?.values() ?? []) {
          if (saved.route !== this.state.routeKey || saved.kind !== 'staff-unassign' || !saved.sent?.target) continue
          const recovery = this.addForm('staff-unassign', { ...saved.fields }, saved.sent.target, renewed)
          recovery.awaitingRead = true
        }
        break
      case 'staff-assign': this.addForm('staff-assign', { storeId: '', userId: '' }); break
      case 'reports':
        this.addForm('report-query', { storeId: '', dateFrom: today, dateTo: today, page: '1' })
        this.addForm('summary', { storeId: '', dateFrom: today, dateTo: today }); break
      case 'report-create': this.addForm('report-create', { storeId: '', businessDate: today, amount: '', note: '' }); break
      case 'report-detail': this.addForm('report-edit', { amount: '', note: '' }, null, renewed); break
    }
  }
  patch(formId: string, field: string, value: string) {
    const form = this.live(formId)
    if (!form || form.pending || form.sent || !Object.hasOwn(form.fields, field)) return
    form.fields[field] = value; form.dirty = true; form.error = ''; this.emit()
  }
  private async enterRoute() {
    if (!this.routeAllowed()) return
    const route = this.state.route
    if (route.kind === 'stores') { const form = this.form('store-query'); if (form) await this.submit(form.id) }
    if (route.kind === 'store-detail' || route.kind === 'report-detail') { const form = this.form(route.kind === 'store-detail' ? 'store-edit' : 'report-edit'); if (form) await this.readLatest(form.id) }
  }
  private operationFor(form: Form): Operation {
    switch (form.kind) {
      case 'store-query': return STORE_READ
      case 'store-create': return op('store', 'create')
      case 'store-edit': return op('store', 'update')
      case 'staff-query': return STAFF_READ
      case 'staff-assign': return op('store-staff', 'assign')
      case 'staff-unassign': return op('store-staff', 'unassign')
      case 'report-query': case 'summary': return REPORT_READ
      case 'report-create': return op('store-daily-report', 'create')
      case 'report-edit': return op('store-daily-report', 'update')
    }
  }
  private deny(operation: Operation) {
    this.state.denied.add(operationKey(operation))
    for (const [key, controller] of this.reads) {
      if (this.readOperations.get(key) !== operationKey(operation)) continue
      controller.abort(); this.reads.delete(key); this.readOperations.delete(key); this.state.loading.delete(key)
    }
    if (operation.resource === 'store') { this.state.stores = null; this.state.storeChoices = null; if (this.state.route.kind === 'store-detail') this.state.detail = null }
    if (operation.resource === 'store-staff') this.state.staff = null
    if (operation.resource === 'store-daily-report') { this.state.reports = null; this.state.summary = null; if (this.state.route.kind === 'report-detail') this.state.detail = null }
    this.state.directory = null
    for (const form of this.state.forms) if (this.operationFor(form).resource === operation.resource) { form.latest = null; form.readback = null; form.target = null; form.version = null; if (form.kind.endsWith('-edit') || form.kind === 'staff-unassign') form.awaitingRead = true }
  }
  private error(error: unknown, operation: Operation, form?: Form) {
    if (error instanceof BusinessError && error.status === 403) this.deny(operation)
    if (form) {
      form.error = message(error)
      if (error instanceof BusinessError && error.status === 409) {
        form.conflict = true
        if (error.code === 'VERSION_CONFLICT' && (form.kind.endsWith('-edit') || form.kind === 'staff-unassign')) form.awaitingRead = true
      }
    }
    else this.state.error = message(error)
  }
  private async read<T>(key: string, operation: Operation, request: (signal: AbortSignal) => Promise<T>, apply: (value: T) => void, form?: Form) {
    if (!this.allowed(operation)) { if (form) form.error = '当前授权不允许此项查询。'; this.emit(); return }
    this.reads.get(key)?.abort()
    const controller = new AbortController(), serial = (this.readSerial.get(key) ?? 0) + 1
    this.reads.set(key, controller); this.readOperations.set(key, operationKey(operation)); this.readSerial.set(key, serial)
    const snapshot = this.client.capture(), sequence = this.routeSequence
    this.state.loading.add(key); this.state.error = ''; if (form) form.error = ''; this.emit()
    const ownsRead = () => this.client.isCurrent(snapshot) && sequence === this.routeSequence && this.readSerial.get(key) === serial
    const current = () => !controller.signal.aborted && ownsRead() && this.allowed(operation)
    try { const result = await request(controller.signal); if (current()) apply(result) }
    catch (error) { if (current()) this.error(error, operation, form) }
    // A denial aborts this read and freezes its operation before finally runs.
    // Its cleared data and error still need to reach the subscribed UI.
    finally { if (ownsRead()) { this.state.loading.delete(key); this.reads.delete(key); this.readOperations.delete(key); this.emit() } }
  }
  private validate(form: Form): string | null {
    const fields = form.fields
    if (['staff-query', 'staff-assign', 'report-query', 'report-create'].includes(form.kind) && !validUUID(fields.storeId)) return '请输入有效的门店 UUID。'
    if (['store-create', 'store-edit'].includes(form.kind)) {
      if (!fields.name.trim() || Array.from(fields.name.trim()).length > 200 || fields.name.includes('\0')) return '门店名称需为 1–200 个字符。'
      if (!['OPEN', 'CLOSED'].includes(fields.businessStatus)) return '请选择营业状态。'
      if (Array.from(fields.businessContact).length > 200 || fields.businessContact.includes('\0')) return '业务联系信息最多 200 个字符。'
      if (fields.managerUserId && !validUUID(fields.managerUserId)) return '负责人用户 ID 必须为有效 UUID，留空可清除负责人。'
    }
    if (form.kind === 'staff-assign' && !validUUID(fields.userId)) return '请选择员工或输入有效的员工用户 UUID。'
    if (['report-create', 'report-edit'].includes(form.kind)) {
      if (!validAmount(fields.amount)) return '营业金额需为非负十进制文本，最多 16 位整数和 2 位小数。'
      if (Array.from(fields.note).length > 8192 || fields.note.includes('\0')) return '备注最多 8192 个字符且不能包含空字符。'
      if (form.kind === 'report-create' && !validBusinessDate(fields.businessDate)) return '请确认有效的营业日期。'
    }
    if (['report-query', 'summary'].includes(form.kind)) {
      if (!validBusinessDate(fields.dateFrom) || !validBusinessDate(fields.dateTo) || fields.dateFrom > fields.dateTo) return '请选择有效的起止日期，开始日期不能晚于结束日期。'
      if (form.kind === 'summary' && fields.storeId && !validUUID(fields.storeId)) return '汇总门店 ID 应为有效 UUID，也可留空汇总当前授权内全部日报。'
    }
    if ((form.kind.endsWith('-edit') || form.kind === 'staff-unassign') && (form.awaitingRead || !form.version || !form.target)) return '请先读取记录并使用最新版本，再保存原输入。'
    return null
  }
  async submit(formId: string) {
    const form = this.live(formId)
    if (!form || form.pending || form.sent) return
    const operation = this.operationFor(form)
    if (!this.allowed(operation, form.target)) { form.error = '当前授权不允许此项操作。'; this.emit(); return }
    const validation = this.validate(form)
    if (validation) { form.error = validation; this.emit(); return }
    const fields = { ...form.fields }, page = Math.max(1, Number(fields.page || '1'))
    switch (form.kind) {
      case 'store-query':
        this.state.stores = null
        return this.read('stores', operation, signal => this.client.listStores({ q: fields.q, businessStatus: fields.businessStatus ? fields.businessStatus as 'OPEN' | 'CLOSED' : undefined, page, size: 20 }, { signal }), result => { this.state.stores = result }, form)
      case 'staff-query':
        this.saveDrafts(); this.state.staff = null; this.state.forms = this.state.forms.filter(row => row.kind !== 'staff-unassign' || !!row.sent)
        return this.read('staff', operation, signal => this.client.listStaff(fields.storeId, { page, size: 20 }, { signal }), result => {
          this.state.staff = result
          for (const link of result.records) {
            const existing = this.state.forms.find(candidate => candidate.kind === 'staff-unassign' && candidate.key.endsWith(`|${link.id}`))
            if (!existing) { const editor = this.addForm('staff-unassign', { storeId: fields.storeId }, link); editor.awaitingRead = false }
          }
        }, form)
      case 'report-query':
        this.state.reports = null
        return this.read('reports', operation, signal => this.client.listDailyReports(fields.storeId, { page, size: 20, dateFrom: fields.dateFrom, dateTo: fields.dateTo }, { signal }), result => { this.state.reports = result }, form)
      case 'summary':
        this.state.summary = null
        return this.read('summary', operation, signal => this.client.getSummary({ dateFrom: fields.dateFrom, dateTo: fields.dateTo, ...(fields.storeId ? { storeId: fields.storeId } : {}) }, { signal }), result => { this.state.summary = result }, form)
    }
    // Freeze the original body before touching the transport. No future event can pair it with a different identity.
    const sent: SentWrite = { operation, fields, target: form.target ? { ...form.target } : null, version: form.version, identity: form.identity, outcome: 'pending', inFlight: true, checked: false }
    const snapshot = this.client.capture()
    form.sent = sent; form.pending = true; form.error = ''; form.conflict = false; this.state.notice = ''; this.emit()
    try {
      let result: Store | StaffLink | DailyReport | { ok: true }
      const storeBody = { name: fields.name, businessStatus: fields.businessStatus as 'OPEN' | 'CLOSED', businessContact: fields.businessContact, managerUserId: fields.managerUserId || null }
      switch (form.kind) {
        case 'store-create': result = await this.client.createStore(storeBody); break
        case 'store-edit': result = await this.client.updateStore(sent.target!.id, { version: sent.version!, ...storeBody }); break
        case 'staff-assign': result = await this.client.assignStaff(fields.storeId, { userId: fields.userId }); break
        case 'staff-unassign': result = await this.client.unassignStaff(fields.storeId, sent.target!.id, { version: sent.version! }); break
        case 'report-create': result = await this.client.createDailyReport(fields.storeId, { businessDate: fields.businessDate, amount: fields.amount, note: fields.note }); break
        case 'report-edit': result = await this.client.updateDailyReport(sent.target!.id, { version: sent.version!, amount: fields.amount, note: fields.note }); break
        default: return
      }
      // A successful response from a departed route may settle its draft, but never its visible server view.
      if (!this.client.isCurrent(snapshot)) { sent.outcome = 'unknown'; return }
      form.sent = null; form.pending = false; form.dirty = false; form.conflict = false
      if ('id' in result) { form.target = result; form.version = result.version; form.awaitingRead = false }
      if (this.live(form.id)) {
        this.state.notice = form.kind === 'staff-unassign' ? '员工关联已解除。' : '已保存。'
        if (form.kind.endsWith('-edit') && 'id' in result) this.state.detail = result as Store | DailyReport
        if (form.kind === 'staff-unassign' && this.state.staff) this.state.staff = { ...this.state.staff, records: this.state.staff.records.filter(row => row.id !== sent.target!.id), total: Math.max(0, this.state.staff.total - 1) }
        this.state.props?.onNotify(this.state.notice, 'success')
      }
    } catch (error) {
      // Losing the authoritative context after dispatch cannot prove that COMMIT did not occur.
      const unknown = !this.client.isCurrent(snapshot) || (error instanceof BusinessError && error.unknown) || (!(error instanceof BusinessError) && !(error instanceof TypeError && error.message === 'validation'))
      if (unknown) { sent.outcome = 'unknown'; form.error = '本次写入结果未知。原日期、目标与输入已保留，请读回核对后再决定下一次操作。' }
      else { form.sent = null; if (this.client.isCurrent(snapshot)) this.error(error, operation, form) }
    } finally {
      sent.inFlight = false
      form.pending = false
      // Only transport settlement releases pending. Route returns and renewals may have made several form IDs.
      const successors = new Set([...this.state.forms, ...this.drafts.get(form.identity)?.values() ?? []])
      for (const successor of successors) {
        if (successor === form || successor.key !== form.key || successor.identity !== form.identity || successor.sent !== sent) continue
        successor.pending = false; successor.sent = form.sent; successor.error = form.error
        if (!form.sent) {
          successor.target = form.target; successor.version = form.version; successor.awaitingRead = form.awaitingRead
          successor.dirty = form.dirty; successor.conflict = form.conflict
        }
      }
      this.emit()
    }
  }
  async paginate(formId: string, delta: number) {
    const form = this.live(formId); if (!form || form.pending || form.sent) return
    form.fields.page = String(Math.max(1, Number(form.fields.page || '1') + delta)); this.emit(); await this.submit(formId)
  }
  async readLatest(formId: string) {
    const form = this.live(formId); if (!form || form.pending) return
    const route = this.state.route, operation = form.kind === 'store-edit' ? STORE_READ : form.kind === 'report-edit' ? REPORT_READ : STAFF_READ
    if (form.kind === 'store-edit' && route.kind === 'store-detail') {
      return this.read('detail', operation, signal => this.client.getStore(route.id, { signal }), row => this.acceptLatest(form, row), form)
    }
    if (form.kind === 'report-edit' && route.kind === 'report-detail') {
      return this.read('detail', operation, signal => this.client.getDailyReport(route.id, { signal }), row => this.acceptLatest(form, row), form)
    }
    if (form.kind === 'staff-unassign' && form.target) {
      const targetId = form.target.id
      return this.read(`latest-${form.id}`, operation, signal => this.staffAll(form.fields.storeId, signal), rows => {
        const latest = rows.find(row => row.id === targetId); form.latest = latest ?? null
        if (!latest) form.error = '当前授权内已无此有效关联；请重新查询员工。'
      }, form)
    }
  }
  private acceptLatest(form: Form, row: Store | DailyReport) {
    if (!this.allowed(form.kind === 'store-edit' ? STORE_READ : REPORT_READ, row)) return
    this.state.detail = row; form.latest = row
    if (!form.dirty && !form.conflict && !form.sent) {
      form.fields = 'name' in row ? { name: row.name, businessStatus: row.businessStatus, businessContact: row.businessContact, managerUserId: row.managerUserId ?? '' } : { amount: row.amount, note: row.note }
      form.target = row; form.version = row.version; form.awaitingRead = false; form.latest = null
    }
  }
  useLatest(formId: string) {
    const form = this.live(formId)
    if (!form || form.pending || form.sent || !form.latest) return
    form.target = form.latest; form.version = form.latest.version; form.latest = null; form.awaitingRead = false; form.conflict = false; form.error = ''; this.emit()
  }
  async loadStoreChoices(formId: string, q = '', page = 1) {
    const form = this.live(formId); if (!form) return
    this.state.storeChoices = null
    return this.read('choices', STORE_READ, signal => this.client.listStores({ q, page, size: 20 }, { signal }), result => { this.state.storeChoices = result }, form)
  }
  selectStore(formId: string, id: string) { this.patch(formId, 'storeId', id) }
  async searchMembers(formId: string, q: string, page = 1) {
    const form = this.live(formId), props = this.client.current()
    if (!form || !props || form.pending || form.sent) return
    const purpose: MemberPurpose | null = form.kind === 'store-create' ? op('store', 'create') as MemberPurpose : form.kind === 'store-edit' ? op('store', 'update') as MemberPurpose : form.kind === 'staff-assign' ? op('store-staff', 'assign') as MemberPurpose : null
    const organizationId = form.target?.organizationId ?? props.organization.id
    if (!purpose || !this.allowed(purpose, form.target) || !canMemberLookup(props, purpose, organizationId)) { form.error = '当前授权不允许查询此用途的成员目录。'; this.emit(); return }
    this.state.directory = null; this.state.directoryFormId = form.id; this.state.directoryQuery = q; this.state.directoryPage = page
    return this.read('directory', purpose, signal => this.client.listMembers(purpose, { q, page, size: 10, organizationId }, { signal }), result => { if (this.live(formId)) this.state.directory = result }, form)
  }
  selectMember(formId: string, id: string) {
    const form = this.live(formId)
    if (!form || this.state.directoryFormId !== formId || !this.state.directory?.records.some(row => row.id === id)) return
    this.patch(formId, form.kind === 'staff-assign' ? 'userId' : 'managerUserId', id)
  }
  private async staffAll(storeId: string, signal: AbortSignal) {
    const rows: StaffLink[] = []
    for (let page = 1; ; page++) { const result = await this.client.listStaff(storeId, { page, size: 50 }, { signal }); rows.push(...result.records); if (rows.length >= result.total || !result.records.length) return rows }
  }
  readbackPermission(form: Form) {
    const resource = form.sent?.operation.resource
    return this.allowed(resource === 'store' ? STORE_READ : resource === 'store-staff' ? STAFF_READ : REPORT_READ)
  }
  async readback(formId: string) {
    const form = this.live(formId), sent = form?.sent
    if (!form || !sent || form.pending || sent.inFlight || sent.outcome !== 'unknown' || sent.identity !== this.acceptedIdentity) return
    const fields = sent.fields, resource = sent.operation.resource
    const operation = resource === 'store' ? STORE_READ : resource === 'store-staff' ? STAFF_READ : REPORT_READ
    if (!this.readbackPermission(form)) { form.error = '当前没有对应读取权限，无法核对本次写入。原输入已保留；请取得读取授权后核对，勿直接重复提交。'; this.emit(); return }
    form.readback = null
    return this.read(`readback-${form.id}`, operation, async signal => {
      if (resource === 'store') {
        if (sent.target) return [await this.client.getStore(sent.target.id, { signal })]
        const rows: Store[] = []
        for (let page = 1; ; page++) { const result = await this.client.listStores({ page, size: 50, organizationId: this.client.current()!.organization.id }, { signal }); rows.push(...result.records); if (rows.length >= result.total || !result.records.length) return rows }
      }
      if (resource === 'store-staff') return this.staffAll(fields.storeId, signal)
      if (sent.target) return [await this.client.getDailyReport(sent.target.id, { signal })]
      return (await this.client.listDailyReports(fields.storeId, { dateFrom: fields.businessDate, dateTo: fields.businessDate, page: 1, size: 50 }, { signal })).records
    }, rows => { form.readback = rows; sent.checked = true; form.error = ''; }, form)
  }
  acknowledgeReadback(formId: string) {
    const form = this.live(formId)
    if (!form || !form.sent?.checked || form.pending || form.sent.inFlight || !form.readback) return
    const targetId = form.sent.target?.id
    const latest = targetId ? form.readback.find(row => row.id === targetId) : undefined
    form.sent = null; form.readback = null; form.error = ''; form.conflict = false
    if (form.kind.endsWith('-edit') || form.kind === 'staff-unassign') {
      form.latest = latest ?? null; form.target = null; form.version = null; form.awaitingRead = true
      if (!latest) form.error = '读回未找到目标记录，请重新查询；本次不会自动发送任何写入。'
    }
    this.state.notice = '核对已确认。请根据服务端记录判断是否需要修改；系统不会自动重发。'; this.emit()
  }
  dispose() { if (this.disposed) return; this.saveDrafts(); this.disposed = true; this.invalidate(); this.client.dispose(); this.drafts.clear(); this.listeners.clear() }
}
