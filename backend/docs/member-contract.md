# 平台 STORE 组织与成员契约

门店是平台组织的一种，`organizationType=STORE`。平台统一维护组织名称、编码、层级、地址、组织联系人、启用状态和直接成员。门店 API 维护营业档案、营业联系人、日报与业务审计，通过 HTTP 消费平台投影，不直接连接平台数据库。

## 数据归属

| 平台权威数据 | 门店业务数据 |
| --- | --- |
| STORE 的组织身份、名称、编码、父组织和路径 | 稳定的 profile 技术 ID，唯一的 tenant/organization 映射 |
| 行政地址、组织联系信息、状态、有效期 | OPEN/CLOSED 营业状态、营业联系说明 |
| 组织成员和成员显示名 | 经当前成员资格复核的营业联系人 |
| 角色、权限、组织硬边界和数据授权 | 日报、乐观版本、业务 owner 和审计历史 |

managerUserId 表示营业联系人，不是平台组织管理者。门店 owner 为营业档案创建人，日报 owner 为录入人，联系人变化不会改变 owner。已存在的 profile ID、日报 storeId 引用、旧名称和旧 store_staff 历史均保留，旧映射不会自动转成 STORE 或删除。

新增迁移 `0002_platform_owned_store_identity.sql` 在确认原 v1 fingerprint 后冻结旧 stores.name 的实际变更，并阻止 store_staff 新写入。v1 SQL、校验函数和历史 baseline 不变；v2 fingerprint 同时覆盖继承结构和新增约束。

## 平台受控组织投影

平台提供 `GET /api/apps/stores/organizations` 与 `/organizations/{organizationId}`。每次调用都携带当前 stores 应用 Bearer、准确 resource/action、请求 ID。允许的 organizationLookups 为 store/read|create|update、store-staff/read、store-daily-report/read|create|update，均限定 organizationType=STORE。

平台必须验证当前租户、应用授权、组织和祖先有效状态、功能权限与数据范围；单项不能返回失效、非 STORE 或不可见目标。投影及分页的 contextVersion 必须与业务请求首次取得的授权 contextVersion 相同，revision 为组织修订摘要。响应仅保留[API 字段表](../API.md)列出的公开信息。

非 SELF 业务读取按平台分页加载允许的 STORE 组织集合。SELF 先在业务库按原 tenant、硬边界、grant 和 owner 条件取得已知 organizationId，释放连接后逐个单项解析。平台 404 目标从当前结果排除，授权版本变化或协议异常使请求整体失败。业务 SQL 最终继续保留 owner=actor，单项元数据不能扩大业务行权限。

公开初始化目录与成员选择仍执行 require_organization：SELF / ORG 仅当前组织，ORG_SUBTREE 为 grant 与硬边界交集，TENANT 受硬边界限制。SELF 能解析本人旧子组织业务行的元数据，不等于能在该子组织创建新绑定或查成员。

## 平台成员用途

| memberLookups | 用途 |
| --- | --- |
| store/create | 为已有 STORE 初始化营业资料时选择营业联系人 |
| store/update | 每次保存非空营业联系人时重新证明当前成员资格 |
| store-staff/read | 只读查看该 STORE 当前平台直接成员 |

浏览器经门店 API 的 `/members` 查询候选；后端调用平台 `/api/apps/stores/members` 与 `/members/{user}`。成员响应仅保留 id、displayName、organizationId，拒绝重复成员、错误组织与异常分页。候选不构成后续写入的永久资格证明。

员工页直接显示平台成员，不再建立独立成员关系。旧 POST/DELETE staff 路由认证后返回 410 PLATFORM_MEMBERSHIP_MANAGED；新 bootstrap、默认权限及策略不包含 assign/unassign。

每次营业资料更新都校验最终非空 managerUserId，包括只改营业状态而保留联系人。已离职成员或 SELF 子 STORE 的范围外联系人返回可纠正的 MEMBER_NOT_FOUND，用户可明确清空后保存，或切换到获授权上下文。不会自动清空联系人，也不会扩大成员目录。

## 时效、连接与事务

授权每次重新取得，成功结果不缓存。组织和单成员校验各使用从 HTTP 发起前开始的绝对短期证据，窗口不超过 min(PLATFORM_AUTH_TIMEOUT_MS, 3000ms)，并受平台 expiresAt、令牌及授权到期约束。加载多个组织时取最早期限，不因后续页面或响应完成而重置。数据库操作仍使用各自的连接池、锁和语句超时预算；每次等待结束后复核证据，到期事务不能继续进入 COMMIT。

任何平台 HTTP 都不占用业务连接或行锁。已存在档案更新先完成 scoped 预读并释放连接，再查成员，最后短事务锁定行，重新核对版本、组织、owner、旧联系人，并在审计和 COMMIT 前检查全部证据。读取、分页、汇总同样在返回前检查组织及授权时效。

提交前证据到期时回滚；COMMIT 已确认后又到期，或提交结果无法确认，返回 CREATE_RESULT_UNKNOWN / WRITE_RESULT_UNKNOWN。客户端必须查询原始目标和版本，不自动重发。平台组织 / 成员撤销和业务提交之间无分布式事务，短窗口减少竞态但不承诺跨库原子性。

## 验证范围

验收需覆盖：七个组织用途、三个成员用途、非 STORE 与禁用组织历史排除、contextVersion 降权竞态、短窗口耗尽、SELF 子组织本人行与非本人行、保留无效联系人拒绝与显式清空成功、锁后版本变化、提交后结果未知。qiankun update 改变身份、组织或授权时，前端取消旧请求并隔离旧页面、候选和表单。

真实 PostgreSQL 合同还验证有效组织投影在行锁等待中到期：先记录平台实际返回的 expiresAt，通过 pg_stat_activity 与 pg_blocking_pids 证明 API 的 FOR UPDATE 正在等待测试持有的行锁，且此时投影仍有效；到期后释放锁，要求返回 ORGANIZATION_VALIDATION_EXPIRED，营业字段、version、updated_at 和全部审计记录逐值保持不变。该测试复用隔离数据库，不依赖猜测性延迟推断锁竞争。
