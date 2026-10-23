# 门店 HTTP API

所有路径以 `/stores-api` 开头。路由实现位于 `src/http.rs`，请求规则位于 `stores.rs`、`staff.rs`、`reports.rs`、`directory.rs` 和 `domain.rs`。下文中的 `{id}`、`{store}`、`{link}` 均为非 nil、带连字符的标准 UUID。

## 身份、授权与响应

业务请求必须提供唯一的 `Authorization: Bearer <stores 应用令牌>`。服务不从 Cookie 取身份，不接受客户端设置租户、owner 或组织归属。API 重新向平台校验当前身份与对应资源动作；功能 permission、独立数据授权以及租户 / 组织硬边界必须同时满足。

每个资源使用自己的授权。员工与日报操作不隐式要求 `store:read`；目标门店仍须落在当前动作允许的租户和组织范围。`SELF` 分别按门店创建人、员工关联的员工本人、日报录入人判断，负责人变更不会改变 owner。

成功业务响应直接返回记录或分页对象，没有额外 `data` 包装。创建返回 `201`，其他成功操作返回 `200`。分页格式：

```json
{"records":[],"total":0,"page":1,"size":20}
```

常规错误返回 JSON：

```json
{"code":"VERSION_CONFLICT","message":"门店已被修改，请刷新后重试"}
```

响应带 `Cache-Control: no-store` 和 `X-Request-Id`。合法且唯一的非 nil UUID 请求标识会沿用，否则服务生成新标识。错误不回显 SQL、凭据、上游正文或任意输入。私有指标鉴权失败是空正文 `401`，不使用业务 JSON 格式。

## 通用输入与 CORS

写入包括 `POST`、`PATCH` 和员工解除关联的 `DELETE`，都必须使用 `Content-Type: application/json`，正文上限 `65536` 字节。正文必须符合对应对象结构，未知字段拒绝。查询拒绝未知或重复键、错误百分号编码及无效 UTF-8。

分页可选 `page` 默认 `1`，范围 `1–1000000`；`size` 默认 `20`，范围 `1–50`。列表总数与记录在同一只读快照中查询。更新及解除关联必须传 JSON 整数 `version`，范围 `1–2147483646`，不能用字符串、小数或浮点字面量代替。

CORS 对请求携带的 Origin 与必填 `STORES_ALLOWED_ORIGINS` 精确匹配。未配置或空配置拒绝启动，`null` 和通配符不属于支持的 origin；生产要求 HTTPS，开发额外允许准确回环 HTTP。没有 Origin 的服务间请求可正常处理。合法预检返回 `204`，允许 `GET, HEAD, POST, PATCH, DELETE, OPTIONS`，请求头仅允许 `Authorization`、`Content-Type`、`X-Request-Id`。非法、opaque `null` 或重复 Origin 返回 `403 ORIGIN_DENIED`，不开放 Cookie credentials。

## 路由清单

| 方法与相对路径 | 所需资源 / 动作 | 成功结果 |
| --- | --- | --- |
| `GET /health/live` | 公开 | 进程状态 |
| `GET /health/ready`、`GET /health` | 公开 | 数据库与迁移就绪状态 |
| `GET /stores` | `store/read` | 门店分页 |
| `POST /stores` | `store/create` | 新门店记录，201 |
| `GET /stores/{id}` | `store/read` | 门店记录 |
| `PATCH /stores/{id}` | `store/update` | 更新后的门店记录 |
| `GET /stores/{id}/staff` | `store-staff/read` | 有效员工关联分页 |
| `POST /stores/{id}/staff` | `store-staff/assign` | 新关联记录，201 |
| `DELETE /stores/{store}/staff/{link}` | `store-staff/unassign` | `{"ok":true}` |
| `GET /stores/{id}/daily-reports` | `store-daily-report/read` | 日报分页 |
| `POST /stores/{id}/daily-reports` | `store-daily-report/create` | 新日报，201 |
| `GET /daily-reports/{id}` | `store-daily-report/read` | 日报记录 |
| `PATCH /daily-reports/{id}` | `store-daily-report/update` | 更新后的日报 |
| `GET /daily-reports/summary` | `store-daily-report/read` | 日期范围汇总 |
| `GET /members` | 指定的成员查找用途 | 成员分页 |
| `GET /internal/metrics` | 独立运维 Bearer | Prometheus 文本 |

当前没有门店删除、日报删除或批量写入接口。不存在的路由返回 `404 ROUTE_NOT_FOUND`，不支持的方法返回 `405 METHOD_NOT_ALLOWED`。

## 门店

`GET /stores` 接受 `page`、`size`、`q`、`businessStatus`、`organizationId`。`q` 去除两端空白后最多 200 字符，在门店名和联系信息中按字面子串匹配；`%`、`_` 不作为通配符。`organizationId` 只收窄当前授权范围。

创建请求：

```json
{"name":"中心店","businessStatus":"OPEN","businessContact":"工作电话或联系人","managerUserId":null}
```

`name`、`businessStatus`、`businessContact` 必填。名称去除两端空白后非空，名称与联系信息各最多 200 字符；联系信息允许空字符串。状态仅为 `OPEN` 或 `CLOSED`。`managerUserId` 可省略、为 `null` 或有效成员 UUID，显示名由平台查询获得。

创建归属取当前平台组织，owner 为创建人，初始版本为 `1`。每个租户组织只能有一间门店，重复创建返回 `409 STORE_ALREADY_EXISTS`。

`PATCH /stores/{id}` 必须包含 `version`，并至少包含 `name`、`businessStatus`、`businessContact`、`managerUserId` 中一项。省略负责人表示保持原值，传 `null` 表示清空。不能修改租户、组织或 owner。负责人关联按 [成员契约](docs/member-contract.md)校验。

记录包含 `id`、`tenantId`、`organizationId`、`ownerUserId`、上述业务字段、`managerDisplayName`、`version`、`createdAt`、`updatedAt`。负责人 ID 和显示名成对为值或 `null`，时间为 UTC ISO 字符串。

## 员工关联

`GET /stores/{id}/staff` 只接受 `page`、`size`，只返回未解除的关联。关联创建请求为 `{"userId":"<成员 UUID>"}`，服务端核验该用户是目标门店组织的有效直接成员。一个员工可以关联多间门店，同一门店重复有效关联返回 `409 STAFF_ALREADY_ASSIGNED`。

员工关联的 owner 为员工本人，`createdByUserId` 为操作人，两者含义独立。记录包含 `id`、`tenantId`、`storeId`、`organizationId`、`ownerUserId`、`userId`、`displayName`、`createdByUserId`、`version`、时间字段和 `removedAt:null`。

解除关联使用 `DELETE /stores/{store}/staff/{link}`，JSON 正文为 `{"version":1}`。成功后关联不再出现在有效列表，返回 `{"ok":true}`。服务执行逻辑解除并保留业务审计。

## 每日营业记录

`GET /stores/{id}/daily-reports` 接受 `page`、`size`、`dateFrom`、`dateTo`。日期为真实的 `YYYY-MM-DD`，筛选范围含两端；同时提供时必须满足起始日不晚于结束日。

创建请求：

```json
{"businessDate":"2026-10-23","amount":"123.45","note":"当日营业记录"}
```

`amount` 必填，必须为非负十进制字符串；整数最多 16 位，小数最多 2 位，不接受指数、符号、空白或 JSON 数字。数据库为 `NUMERIC(18,2)`，响应始终使用两位小数字符串。金额可大于 JavaScript 安全整数精度，客户端不能通过 `Number` 往返。

`businessDate` 可省略，此时服务按 Asia/Shanghai 的当前营业日取值；官方前端要求用户确认并发送明确日期，避免请求与恢复跨越午夜时改变目标。日期年份范围 `0001–9999`。`note` 可省略，默认空字符串，最多 8192 字符，不含 NUL。

同店同营业日唯一，重复返回 `409 DAILY_REPORT_EXISTS`。owner 为录入人。`PATCH /daily-reports/{id}` 必须传 `version`，并至少包含 `amount` 或 `note`；营业日、门店与归属不能更改。

日报记录包含 `id`、`tenantId`、`storeId`、`organizationId`、`ownerUserId`、`businessDate`、`amount`、`note`、`version`、`createdAt`、`updatedAt`。

`GET /daily-reports/summary` 必须同时提供 `dateFrom`、`dateTo`，可用 `storeId` 收窄范围，不接受分页字段。响应包含 `count`、`totalAmount`、`dateFrom`、`dateTo` 和 `storeId`。未筛选店时 `storeId` 为 `null`，空范围金额为 `"0.00"`。汇总金额可能超过单份日报上限，仍按精确字符串处理。

## 受控成员目录

`GET /members` 必须指定 `resource` 与 `action`，仅支持 `store/create`、`store/update`、`store-staff/assign`。可选 `organizationId` 默认当前组织，`page`、`size` 使用通用分页；`q` 在去空白前最多 100 字符。

调用者需要对应动作的功能与数据授权，平台需登记相同的 `memberLookups`。目录返回当前租户和允许组织内的有效直接成员，不因 `read` 或 `unassign` 权限开放用户目录。分页记录只含 `id`、`displayName`、`organizationId`；候选结果不会替代后续写入时的成员复核。

## 健康与私有指标

存活返回 `{"status":"ok","service":"stores-api","scope":"process"}`。就绪与兼容 `/health` 返回 scope `business-database-schema`，数据库连接、迁移历史或结构校验失败返回 `503 DATABASE_UNAVAILABLE`。这些接口不证明用户有业务权限。

`/internal/metrics` 使用独立的 `STORES_METRICS_TOKEN` Bearer，未配置或认证不符返回 `401`。成功返回 `text/plain; version=0.0.4`。公开网关应屏蔽整个私有路径，采集端通过受控网络访问 API。

## 错误与恢复

| 状态 / code | 含义和处理 |
| --- | --- |
| `400 VALIDATION_ERROR` | 参数、对象结构、日期、金额或版本不符合协议，修正后再提交 |
| `401 INVALID_APP_TOKEN`、`INVALID_CONTEXT` | 当前应用身份失效，清除旧视图并等待平台更新 |
| `403 OPERATION_DENIED`、`DATA_DENIED` | 当前资源动作没有权限或范围，停止该动作并更新授权 |
| `403 ORIGIN_DENIED` | 配置的 Origin 或预检头不匹配 |
| `404 RESOURCE_NOT_FOUND` | 记录不存在或对该资源动作不可见，不泄露跨范围存在性 |
| `404 MEMBER_NOT_FOUND` | 负责人或员工不是当前允许组织中的有效成员 |
| `409 VERSION_CONFLICT` | 重新读取最新版本，由用户明确采用后再修订 |
| `409 STORE_ALREADY_EXISTS`、`STAFF_ALREADY_ASSIGNED`、`DAILY_REPORT_EXISTS` | 核对组织门店、有效关联或门店营业日唯一关系 |
| `413 BODY_TOO_LARGE`、`415 JSON_REQUIRED` | 缩小正文或改用正确 JSON 类型 |
| `503 PLATFORM_UNAVAILABLE`、`AUTHORIZATION_UNAVAILABLE` | 平台依赖或授权配置不可用，不放行业务 |
| `503 DATABASE_UNAVAILABLE` | 业务数据库不可用，检查服务和就绪状态 |
| `503 MEMBER_VALIDATION_EXPIRED` | 成员验证窗口已过，原事务不允许继续提交 |
| `503 CREATE_RESULT_UNKNOWN`、`WRITE_RESULT_UNKNOWN` | 结果无法确认，先查原业务目标与当前版本 |

当前没有 `Idempotency-Key` 创建重放协议。网络中断、请求超时、取消、无效成功响应或明确结果未知，均不能证明服务端没有提交。保留原始目标和正文，先查询当前组织门店、有效员工关联、原门店营业日或具体记录版本；无读取权限时交由有权限的人员核对。不要自动重发 `POST`、`PATCH` 或 `DELETE`。

成员验证从请求开始使用绝对期限 `min(PLATFORM_AUTH_TIMEOUT_MS, 3000ms)`，包含网络、连接池等待和事务执行。提交前过期回滚；提交后无法确认返回未知结果。平台与业务库之间没有分布式事务，最后一次检查到 COMMIT 仍存在短暂撤销竞态，详见 [成员契约](docs/member-contract.md)。
