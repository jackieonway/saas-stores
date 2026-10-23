# 门店 HTTP API

业务前缀为 `/stores-api`。门店是平台 `organizationType=STORE` 的组织，平台维护名称、编码、层级、地址和成员；本服务只维护对应门店的营业资料、营业联系人、日报和审计。门店档案的 `id` 是业务技术 ID，`organizationId` 是平台组织 ID，二者不能混用。

## 身份与数据边界

每次业务请求必须携带唯一的 `Authorization: Bearer <stores 应用令牌>`，服务重新校验平台身份、当前应用、资源动作、功能权限和数据授权，再取当前有效 STORE 组织投影。Cookie 不作为身份。业务正文不能设置租户或 owner。

所有列表、总数、详情、日报、汇总和写入同时受租户、组织硬边界、资源动作数据授权及新鲜 STORE 组织集合限制。禁用、到期、非 STORE、无应用授权或范围外组织的历史数据不参与当前结果。历史映射不会自动改类型或删除。

`SELF` 门店行要求 `ownerUserId=当前操作人`，日报行要求录入人为当前操作人；组织元数据的单项解析不证明业务行所有权。已有本人业务行可位于硬边界与 grant 允许的子组织。新建绑定、组织选择和成员查找的 SELF 目标仍仅为当前组织。`ORG`、`ORG_SUBTREE`、`TENANT` 继续与硬边界相交。成员查看、日报使用各自授权，不额外要求 `store:read`。

成功响应直接返回记录或分页，没有 `data` 包装。创建返回 `201`，读取及更新返回 `200`。错误为 `{"code":"...","message":"..."}`。响应带 `Cache-Control: no-store`、`X-Request-Id`，不回显上游正文、SQL 或凭据。

## 路由

| 方法与路径 | 资源 / 动作 | 结果 |
| --- | --- | --- |
| `GET /health/live` | 公开 | 进程存活 |
| `GET /health/ready`、`GET /health` | 公开 | 数据库和迁移就绪 |
| `GET /organizations`、`GET /organizations/{organizationId}` | query 指定的七种用途之一 | 平台 STORE 目录 / 单项投影 |
| `GET /stores`、`GET /stores/{id}` | `store/read` | 营业档案分页 / 详情 |
| `POST /stores` | `store/create` | 为已有 STORE 完善营业资料 |
| `PATCH /stores/{id}` | `store/update` | 更新营业字段 |
| `GET /stores/{id}/staff` | `store-staff/read` | 平台当前直接成员分页 |
| `POST /stores/{id}/staff`、`DELETE /stores/{store}/staff/{link}` | 认证并校验 `store-staff/read` | `410 PLATFORM_MEMBERSHIP_MANAGED` |
| `GET /members` | `store/create`、`store/update`、`store-staff/read` | 平台直接成员候选 |
| `GET /stores/{id}/daily-reports`、`GET /daily-reports/{id}` | `store-daily-report/read` | 日报分页 / 详情 |
| `POST /stores/{id}/daily-reports` | `store-daily-report/create` | 新日报 |
| `PATCH /daily-reports/{id}` | `store-daily-report/update` | 更新日报金额 / 备注 |
| `GET /daily-reports/summary` | `store-daily-report/read` | 日期范围内精确汇总 |
| `GET /internal/metrics` | 独立运维 Bearer | 私网 Prometheus 指标 |

本服务不提供组织创建、组织身份修改、成员分配、门店删除或日报删除。旧成员写接口不读取或写入本地关联表，调用方应转到平台组织管理。

## 通用输入

UUID 必须为非 nil、带连字符的标准格式。JSON 写入要求 `Content-Type: application/json`，正文上限 65536 字节，拒绝未知字段。更新必须传 JSON 整数 `version`（1–2147483646）。查询拒绝未知键、重复键、错误百分号编码、NUL 和无效 UTF-8。

分页默认 `page=1&size=20`，page 为 1–1000000，size 为 1–50。普通分页为 `{"records":[],"total":0,"page":1,"size":20}`。业务列表的 count 和 records 来自同一只读事务快照。

CORS 精确匹配必填 `STORES_ALLOWED_ORIGINS`。不支持空配置、`null`、通配符或生产 HTTP。开发额外允许准确回环 HTTP。合法预检返回 204，允许 GET、HEAD、POST、PATCH、DELETE、OPTIONS 及 Authorization、Content-Type、X-Request-Id；不开放 Cookie credentials。没有 Origin 的服务间请求可使用 Bearer。

## 平台 STORE 组织目录

集合请求必须传 `resource`、`action`，可带 `page`、`size`、`q`（去空白前最多 100 字符）。七种用途为 `store/read|create|update`、`store-staff/read`、`store-daily-report/read|create|update`，平台必须配置对应 `organizationLookups` 且类型为 STORE。单项路径只接受 resource/action，不接受分页或搜索。

目录页面除普通分页字段外，包含 `contractVersion:"1.0"`、64 位十六进制 `contextVersion`、UTC RFC3339 `expiresAt`。每个组织投影也包含这三项，以及：

| 字段 | 类型与含义 |
| --- | --- |
| `id`、`tenantId` | UUID 字符串 |
| `parentId` | UUID 字符串或 null |
| `name`、`code`、`path` | 平台当前组织名称、编码和路径字符串 |
| `organizationType`、`status` | 固定 `STORE`、`ENABLED` |
| `revision` | 64 位十六进制修订摘要字符串 |
| `province`、`city`、`district`、`township`、`village`、`address` | 必填字符串，可为空 |
| `contactName`、`contactPhone`、`contactEmail` | 平台组织联系人字段，必填字符串，可为空 |

目录用于选择操作目标，SELF 只列出当前组织。组织投影的短期有效窗口约束服务端本次请求，不要求用户在三秒内填完表。提交只发送选中的 organizationId，服务端重新获取组织与权限，不能提交组织名称等投影字段。

## 营业资料

`GET /stores` 接受 page、size、q、businessStatus、organizationId。q 去除两端空白后最多 200 字符，按字面子串搜索实时平台组织名、编码及本地营业联系信息；百分号和下划线不作为 SQL 通配符。

`POST /stores` 示例：

```json
{"organizationId":"00000000-0000-4000-8000-000000000001","businessStatus":"OPEN","businessContact":"营业电话或说明","managerUserId":null}
```

organizationId、businessStatus、businessContact 必填。目标必须是当前允许的有效 STORE。businessStatus 仅 OPEN/CLOSED，businessContact 可为空且最多 200 字符。managerUserId 可省略、为 null 或当前有效直接成员 UUID；它是营业联系人，不代表平台组织负责人，也不改变业务 owner。

同一 `(tenantId, organizationId)` 只允许一份营业档案，重复返回 `409 STORE_ALREADY_EXISTS`。owner 为资料初始化人，初始 version 为 1。

`PATCH /stores/{id}` 必须含 version，并至少含 businessStatus、businessContact、managerUserId 一项。省略 manager 表示保留，null 表示显式清空。不能提交 name、code、address、parentId、organizationId、tenantId、ownerUserId 或 managerDisplayName。

每次保存都会复核最终非空营业联系人，即使只改营业状态、未修改联系人。联系人已离职或当前 SELF 范围无法证明子 STORE 联系人时，返回 `404 MEMBER_NOT_FOUND`；用户可显式清空联系人再保存，或切到获授权组织上下文。服务不会自动清空字段，也不会将此情况误判为整个 store:update 撤权。

营业记录返回 id、tenantId、organizationId、ownerUserId、businessStatus、businessContact、managerUserId、managerDisplayName、version、createdAt、updatedAt。另有 `organization` 完整实时平台投影，兼容字段 `name` 始终取 `organization.name`。数据库旧 name 仅保留历史，不作为显示和搜索权威源。平台组织联系人与营业联系人是不同字段。

## 平台门店成员

`GET /stores/{id}/staff?page=1&size=20&q=姓名` 返回当前平台直接成员，记录仅有 `id`、`displayName`、`organizationId`。q 在去空白前最多 100 字符。目标 profile 的租户、组织和当前 staff/read 范围必须有效，查看成员不依赖 profile 创建人。

`GET /members` 必须传 resource/action，可带 organizationId（默认当前组织）、page、size、q；仅支持 store/create、store/update、store-staff/read。平台须登记相同 memberLookups。候选数据不能替代提交时的成员复核。历史 store_staff 行保留查询审计价值，当前接口不再从其推断成员关系。

## 日报与汇总

日报列表接受 page、size、dateFrom、dateTo，日期范围含两端。创建正文：

```json
{"businessDate":"2026-10-24","amount":"123.45","note":"当日营业记录"}
```

amount 必填，非负十进制字符串，整数最多 16 位、小数最多 2 位，不接受 JSON 数字、指数、符号或空白。数据库 NUMERIC(18,2)，响应金额固定两位小数，不能通过 JavaScript Number 往返。businessDate 可省略，此时按 Asia/Shanghai 当日取值；前端发送用户确认的明确日期。日期为真实 YYYY-MM-DD，年份 0001–9999。note 默认空，最多 8192 字符。

同店同日唯一，重复为 `409 DAILY_REPORT_EXISTS`。日报 owner 为录入人。更新必须传 version，且至少传 amount 或 note，不允许改门店、营业日或身份。响应保留 id、tenantId、storeId、organizationId、ownerUserId、businessDate、amount、note、version、createdAt、updatedAt。

汇总必须同时传 dateFrom、dateTo，可带 storeId，不接受分页。返回 count、totalAmount、dateFrom、dateTo、storeId；未按店筛选时 storeId 为 null，空金额为 "0.00"。汇总同样排除失效或非 STORE 组织。历史 profile 技术 ID 和报告 storeId 引用不变。

## 失败与恢复

| 状态 / code | 处理 |
| --- | --- |
| `400 VALIDATION_ERROR` | 修正结构、字段、编码、金额、日期或版本 |
| `401 INVALID_APP_TOKEN`、`INVALID_CONTEXT` | 身份失效，清理旧视图并等待平台上下文更新 |
| `403 OPERATION_DENIED`、`DATA_DENIED` | 当前资源动作或数据范围不允许 |
| `403 ORIGIN_DENIED` | 核对准确 origin / 预检配置 |
| `404 RESOURCE_NOT_FOUND` | 业务行或有效 STORE 组织不存在 / 不可见 |
| `404 MEMBER_NOT_FOUND` | 清空无效营业联系人或使用正确组织上下文 |
| `409 VERSION_CONFLICT` | 读取最新版本，由用户明确采用后再提交 |
| `409 STORE_ALREADY_EXISTS`、`DAILY_REPORT_EXISTS` | 查询已存在的组织营业资料或同店同日报 |
| `410 PLATFORM_MEMBERSHIP_MANAGED` | 到平台组织管理维护成员 |
| `413 BODY_TOO_LARGE`、`415 JSON_REQUIRED` | 缩小正文或发送正确 JSON |
| `503 PLATFORM_UNAVAILABLE`、`AUTHORIZATION_UNAVAILABLE` | 平台依赖或配置不可用，请求不放行 |
| `503 DATABASE_UNAVAILABLE` | 检查业务库和 readiness |
| `503 ORGANIZATION_VALIDATION_EXPIRED`、`MEMBER_VALIDATION_EXPIRED` | 请求内的短期校验证据到期，不能继续提交 |
| `503 CREATE_RESULT_UNKNOWN`、`WRITE_RESULT_UNKNOWN` | 先查询原目标和版本，不能自动重发 |

本服务无 Idempotency-Key 重放协议。网络中断、超时、取消、无效成功响应及明确结果未知都不能证明未提交。保留原始 organizationId、profileId、营业日和正文，先核对现有档案 / 日报 / 版本；无读权限时交给有权限人员核对。

组织投影与成员证据使用绝对 `min(PLATFORM_AUTH_TIMEOUT_MS, 3000ms)` 窗口，包含网络、连接池和行锁等待；多个组织取最早到期时间，不能逐页延长。数据库仍按连接池、锁和语句各自的超时预算运行；等待结束后重新核验，过期事务不能继续进入 COMMIT。提交前过期回滚，提交后无法确认返回结果未知。平台与业务库无分布式事务，最后一次复核到 COMMIT 仍有短暂撤销竞态，详见[组织与成员契约](docs/member-contract.md)。
