# 平台 STORE 组织与成员契约

门店是平台组织的一种，`organizationType=STORE`。平台统一维护组织名称、编码、层级、地址、组织联系人、启用状态和直接成员。门店 API 维护营业档案、营业联系人、日报与业务审计，通过 HTTP 消费平台投影，不直接连接平台数据库。

## 数据归属

| 平台权威数据 | 门店业务数据 |
| --- | --- |
| STORE 的组织身份、名称、编码、父组织和路径 | 稳定的 profile 技术 ID，唯一的 tenant/organization 映射 |
| 行政地址、组织联系信息、状态、有效期 | OPEN/CLOSED 营业状态、营业联系说明 |
| 组织成员和成员显示名 | 经当前成员资格复核的营业联系人 |
| 角色、权限、组织硬边界和数据授权 | 日报、乐观版本、业务 owner 和审计历史 |

managerUserId 表示营业联系人，不是平台组织管理者。门店 owner 为营业档案创建人，日报 owner 为录入人，联系人变化不会改变 owner。profile ID 与日报 storeId 引用由门店业务库维护，组织身份和成员资格以平台当前投影为准。

全新数据库统一执行 `0001_stores.sql`，其中已包含平台受控组织身份约束。`stores_schema_fingerprint()` 与 `_stores_schema_contract` 校验当前完整结构；不再使用历史迁移链或旧版 fingerprint 兼容路径。

## 平台受控组织投影

平台提供私有 `POST /internal/platform/apps/stores/organizations` 与 `/organizations/{organizationId}`。后端使用独立 APP_RESOURCE 服务的 Basic 认证，在 Form body 提交原 token、准确 resource/action、分页和搜索字段；token 不进入 URL。允许的 organizationLookups 为 store/read|create|update、store-staff/read、store-daily-report/read|create|update，均限定 organizationType=STORE。

私有目录外层为 `resource-directory/1`，`directory` 必须准确为 organizations，`expiresAt` 最多三秒；`data` 保留原组织 1.0 DTO。后端同时校验外层与内层截止并取较早值，不因解包、翻页或后续请求延长原证据期限。

平台必须验证当前租户、应用授权、组织和祖先有效状态、功能权限与数据范围；单项不能返回失效、非 STORE 或不可见目标。投影及分页的 contextVersion 必须与业务请求首次取得的授权 contextVersion 相同，revision 为组织修订摘要。响应仅保留[API 字段表](../API.md)列出的公开信息。

非 SELF 业务读取按平台分页加载允许的 STORE 组织集合。SELF 先在业务库按原 tenant、硬边界、grant 和 owner 条件取得已知 organizationId，释放连接后逐个单项解析。平台 404 目标从当前结果排除，授权版本变化或协议异常使请求整体失败。业务 SQL 最终继续保留 owner=actor，单项元数据不能扩大业务行权限。

公开初始化目录与成员选择仍执行 require_organization：SELF / ORG 仅当前组织，ORG_SUBTREE 为 grant 与硬边界交集，TENANT 受硬边界限制。SELF 能解析本人旧子组织业务行的元数据，不等于能在该子组织创建新绑定或查成员。

## 平台成员用途

| memberLookups | 用途 |
| --- | --- |
| store/create | 为已有 STORE 初始化营业资料时选择营业联系人 |
| store/update | 每次保存非空营业联系人时重新证明当前成员资格 |
| store-staff/read | 只读查看该 STORE 当前有效的直接员工成员 |

浏览器经门店 API 的 `/members` 查询候选；后端以 Basic + Form POST 调用平台 `/internal/platform/apps/stores/members` 与 `/members/{user}`。外层必须为 `resource-directory/1`、directory=members，且截止最多三秒。成员响应仅保留 id、displayName、organizationId，拒绝重复成员、错误组织与异常分页。私有 userId 来自 validated principal 或目录，公共不透明 sub 不能替代它。候选不构成后续写入的永久资格证明。

平台对 actor 和 target 分别验证当前有效的员工身份。客户 context 即使与有效员工共用账号，也不能借用该员工的角色或目录；候选、分页总数和单项目标均只接受指定组织的 ENABLED EMPLOYEE。同账号同组织还有 CUSTOMER 行时，有效员工只返回一次；仅客户账号、已停用员工以及误传的 membership ID 均不能作为营业联系人。既有 id 和 managerUserId 继续表示 user ID，门店 API 不根据浏览器传入的类型判断员工资格。

员工页直接显示平台有效员工成员，不再建立独立成员关系。旧 POST/DELETE staff 路由在员工 actor 授权后返回 410 PLATFORM_MEMBERSHIP_MANAGED，客户 actor 在授权阶段拒绝；两种情况都不写本地历史关系或审计。新 bootstrap、默认权限及策略不包含 assign/unassign。

初始化营业资料和每次更新都通过单项成员接口校验最终非空 managerUserId，包括只改营业状态而保留联系人、显式提交相同联系人。已离职、停用、仅有客户身份的成员或 SELF 子 STORE 的范围外联系人返回可纠正的 MEMBER_NOT_FOUND，用户可明确清空后保存，或切换到获授权上下文。不会自动清空联系人，也不会扩大成员目录。

## 时效、连接与事务

授权每次重新取得，成功结果不缓存。组织和单成员校验各使用从 HTTP 发起前开始的绝对短期证据，窗口不超过 min(PLATFORM_AUTH_TIMEOUT_MS, 3000ms)，并受平台 expiresAt、令牌及授权到期约束。加载多个组织时取最早期限，不因后续页面或响应完成而重置。数据库操作仍使用各自的连接池、锁和语句超时预算；每次等待结束后复核证据，到期事务不能继续进入 COMMIT。

组织和成员目录 HTTP 在取得业务连接或行锁前完成。已存在档案更新先完成 scoped 预读并释放连接，再查成员，最后短事务锁定行，重新核对版本、组织、owner、旧联系人。四个资料／日报写入口在审计之后、COMMIT 之前重新在线验证原操作，比较 issuer、凭据用途、原客户端／应用／成员代际、contextVersion、权限和数据范围；撤销或故障使本地业务及审计一起回滚。该来源检查不访问门店业务数据库；不会在持锁期间重新取得组织／成员目录或延长原三秒证据。读取、分页、汇总同样在返回前检查组织及授权时效。

提交前同时检查原授权、原组织／联系人证据及最新在线授权截止；最新回复更早到期时持续使用该较早期限。COMMIT 后也分别检查这些原始与最新期限，已确认写入但证据在提交期间到期时报告结果未知，不能丢弃已知更早的截止或把新回复当作续期。

提交前证据到期时回滚；COMMIT 已确认后又到期，或提交结果无法确认，返回 CREATE_RESULT_UNKNOWN / WRITE_RESULT_UNKNOWN。客户端必须查询原始目标和版本，不自动重发。平台组织 / 成员撤销和业务提交之间无分布式事务，短窗口减少竞态但不承诺跨库原子性。

## 验证范围

验收需覆盖：七个组织用途、三个成员用途、非 STORE 与禁用组织历史排除、contextVersion 降权竞态、短窗口耗尽、SELF 子组织本人行与非本人行、保留无效联系人拒绝与显式清空成功、锁后版本变化、提交后结果未知。qiankun update 改变身份、组织或授权时，前端取消旧请求并隔离旧页面、候选和表单。

生产授权仅允许 combined canonical 端口，原 intro/context 拆分调用只留在测试原始 HTTP 限额和错误消毒的隔离 fixture 中。当前门店业务拒绝 MACHINE，不能为满足旧 owner／人员审计字段而伪造用户；机器业务开放需要另有 STORE 组织类型证明和独立审计模型。

真实 PostgreSQL 合同还验证有效组织投影在行锁等待中到期：先记录平台实际返回的 expiresAt，通过 pg_stat_activity 与 pg_blocking_pids 证明 API 的 FOR UPDATE 正在等待测试持有的行锁，且此时投影仍有效；到期后释放锁，要求返回 ORGANIZATION_VALIDATION_EXPIRED，营业字段、version、updated_at 和全部审计记录逐值保持不变。该测试复用隔离数据库，不依赖猜测性延迟推断锁竞争。

`tests/postgres_contracts.rs` 的成员专项使用真实门店 PostgreSQL 和受控平台 HTTP fixture。fixture 显式模拟员工／客户及独立状态，用于验证三个目录用途、实际 staff GET、联系人创建／更新、旧 DTO 和无写入失败合同；它不构成平台成员类型权威的数据库验收。真实成员类型、同组织双身份、客户 actor 的祖先角色／超管隔离及目标 SQL 过滤，必须由平台自身的真实 PostgreSQL 测试另行通过。
