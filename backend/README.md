# stores-api Rust 后端

独立 Rust 2021 crate，使用 Axum／Tokio／SQLx／PostgreSQL，二进制为 `stores-api`，自己的 [Cargo.toml](Cargo.toml)和 `Cargo.lock`。薄 `main.rs` 只装配配置、日志、启动与退出；`lib.rs` 导出配置、业务库初始化、路由及运行入口，业务和平台适配留在私有模块。不依赖平台或工单实现 crate，不建立共享后台 crate。

当前接口覆盖门店资料、有效员工关联、日报和精确汇总，详见 [API.md](API.md)。Phase26 只实施后端与必要验证；没有门店前端、发布 manifest、Dockerfile、镜像发行或生产切流。本文件记录源码行为，实际检查是否通过由台账与本阶段证据决定。

## 构建与启动

在仓库根执行：

```powershell
cargo build --locked --manifest-path business-apps/stores/backend/Cargo.toml
cargo test --locked --manifest-path business-apps/stores/backend/Cargo.toml
cargo run --locked --manifest-path business-apps/stores/backend/Cargo.toml
```

`run` 要求事先在当前进程环境注入下表必需值。服务不读取 `.env` 或 `config.example`；不要直接把含空占位的模板当作可运行配置。监听前完成平台客户端配置校验、专属库连接、SQLx 迁移及 schema 契约核对，失败退出非0。默认绑定 `0.0.0.0:3031`，本机调试可显式设 `STORES_HOST=127.0.0.1`。

<a id="configuration"></a>
## 配置

[config.example](config.example) 不含可用凭据或虚构服务地址。必需值留空，采用时由运行环境／秘密管理器填入真实值；不得把私有配置注入前端、日志或公开产物。

| 名称 | 默认值／范围 | 含义 |
| --- | --- | --- |
| `STORES_DATABASE_URL` | 必需，无默认 | 门店专属 PostgreSQL URI；必须有host、user和单一database路径。`DATABASE_URL`／`WORK_ORDERS_DATABASE_URL`不能替代 |
| `STORES_HOST` | `0.0.0.0` | 数字IPv4／IPv6地址；不接受主机名或URL，启动不为绑定地址做DNS查询 |
| `STORES_PORT` | `3031`；生产1–65535，非生产0–65535 | 非生产0用于隔离测试动态端口 |
| `NODE_ENV` | 仅精确 `production` 启用生产限制 | 生产平台origin及额外CORS origin要求HTTPS，端口不能0 |
| `STORES_ALLOWED_ORIGINS` | 字面字符串 `null` | 逗号分隔的精确origin；`null`可与HTTPS origin并列，去重；无通配符、path、query、fragment或尾斜线 |
| `STORES_DB_MAX_CONNECTIONS` | `10`；1–30 | 门店池最大连接数 |
| `STORES_DB_ACQUIRE_TIMEOUT_MS` | `5000`；100–30000 | 取得业务连接预算 |
| `STORES_DB_STATEMENT_TIMEOUT_MS` | `5000`；100–30000 | 每条业务SQL执行预算 |
| `STORES_DB_LOCK_TIMEOUT_MS` | `5000`；100–30000 | 数据库锁等待预算 |
| `STORES_STARTUP_TIMEOUT_MS` | `30000`；1000–300000 | 客户端配置／连接／迁移整体启动预算 |
| `STORES_SHUTDOWN_TIMEOUT_MS` | `10000`；100–60000 | SIGINT／SIGTERM后服务与池关闭预算 |
| `STORES_HEALTH_TIMEOUT_MS` | `2000`；100–10000 | readiness数据库／schema检查预算 |
| `PLATFORM_API_ORIGIN` | 实际运行必需 | 精确规范HTTPS origin；禁止用户名、口令、path、query、fragment或显式默认端口。非生产才允许localhost／127.0.0.1／IPv6回环HTTP |
| `PLATFORM_CLIENT_ID` | 实际运行必需 | 平台生成并绑定stores应用的机密客户端ID，1–100字节，只含ASCII字母、数字、点、下划线或连字符；不是应用code或示例名称 |
| `PLATFORM_CLIENT_SECRET` | 实际运行必需 | 当前机密客户端secret，非空、最多1024字节且不能含CR／LF，只在服务端使用 |
| `PLATFORM_AUTH_TIMEOUT_MS` | `3000`；100–10000 | 一次合并授权（legacy 为内省＋context）及响应读取的总预算；成员查询另在此预算内受控 |
| `PLATFORM_AUTHORIZATION_MODE` | `legacy`；combined／legacy | 受控合并授权；legacy 显式回滚到 Basic 内省＋Bearer context，失败不自动回退 |
| `PLATFORM_AUTH_MAX_CONCURRENT` | `32`；1–128 | 进程内平台HTTP并发许可，饱和立即503，不无限排队 |
| `PLATFORM_CA_CERT_FILE` | 可省略 | 追加的私有根CA PEM文件，最多1MiB；保留系统根和正常证书／主机名验证 |

数字参数只接受不带空格的十进制整数字符串。`Config` 可在纯配置测试中不带平台设置，但真实 `run`／`application` 必须具备完整平台配置，缺失时在监听前拒绝；设置任一 `PLATFORM_*` 项也不能留下不完整的origin／client／secret。

PostgreSQL TLS与平台HTTPS是独立策略。DSN保留显式 `sslmode`，支持driver的disable／allow／prefer／require／verify-ca／verify-full；未设置固定为prefer，不受外部 `PGSSLMODE` 改写。需要完整数据库证书／主机名验证时由部署明确选择verify-full及CA。允许的URI query仅为sslmode及其ssl-mode别名、根CA／证书／私钥参数的受支持别名、application_name；未知、空值或重复类别拒绝。连接的application_name固定为stores-api，不记录SQL语句；配置错误与Debug不回显DSN或平台secret。

## 数据库与初始化

平台、工单、门店使用三个独立库和三个专属账号，可同一PG实例。门店账号只用于门店库，能够在该库的自有schema内执行初始化与业务事务；不得授予平台／工单库对象读取权或超级用户权限。身份、组织、成员和角色由平台API持有，门店只存业务UUID及经平台验证的名称快照。

[migrations/0001_stores.sql](migrations/0001_stores.sql)创建 `stores`、`store_staff`、`store_daily_reports` 三张业务表及追加式 `store_audit_events`，并保存schema契约；SQLx维护自己的迁移记录。门店tenant／organization唯一，员工有效关联tenant／store／user唯一，日报tenant／store／businessDate唯一。复合外键保证staff／report引用同tenant、同organization的store；触发器拒绝更改identity／owner／创建时间、日报营业日及关联user，并要求版本精确递增。

每次业务变更和只含actor／resource／action／entity／requestId等安全元数据的审计同事务提交。审计失败回滚业务，不把正文、token、DSN或成员凭据写入审计。再次启动及readiness核对迁移版本／checksum与已保存catalog契约；不能删除迁移历史、修改已执行checksum，或把随意存在的表登记成已验证schema。

## 授权与成员边界

每个受保护请求从 `Authorization: Bearer`取得应用token，以stores机密客户端Basic调用平台 `/oauth/introspect`，再用同一应用Bearer调用固定 `/api/apps/stores/context`。校验aud／appCode为stores、用户／tenant／organization一致、context1.0及有效期、精确 `resource:action` permission和该直接membership的data grant。没有正向授权缓存；Cookie和浏览器props不参与身份判断，应用token不能跨工单与门店使用。

平台客户端禁止redirect、环境代理和透明自动重试，每次连接预算不超过1秒且仍受总授权预算限制；上游JSON响应最多256KiB，错误／过大／畸形响应安全拒绝。CA文件只追加信任根，不跳过TLS或主机名验证。

| 资源 | 真实动作 | 所有者与访问边界 |
| --- | --- | --- |
| `store` | read／create／update | owner固定创建人；manager是单独可改的已验证成员，不扩大SELF |
| `store-staff` | read／assign／unassign | owner固定被关联员工，createdByUserId是操作者；staff的SELF与store owner无关 |
| `store-daily-report` | read／create／update | owner固定录入人；summary使用read，没有另造summary动作 |

tenant与context组织硬上限始终进入SQL。SELF按该资源owner及适用组织限制，ORG按当前组织，ORG_SUBTREE按平台给出的组织集合，TENANT仍与硬上限取交集；SELF目录／新关联目标限当前组织。list、count、detail、mutation和summary不能先取全集再在内存筛选。员工／日报动作不隐含要求store/read，但仍按其自身resource grant对目标store的tenant／organization进行SQL校验。不可见记录返回404；另一个资源的授权、负责人身份或员工身份不会补足权限／grant。

[policy.example.json](policy.example.json)是待管理员合并的 `DATA_GRANT_POLICY` 片段，含9个capability pair和3个memberLookups pair：store/create、store/update、store-staff/assign。必须替换实际stores应用UUID、裁剪批准的scopes并保留现有应用／managementScopes；文件中的占位UUID故意不能通过生产策略校验。它不创建应用、客户端、菜单、角色或grant；相关应用permission仍需通过正式平台管理流程登记并授予。普通业务用户不需要 `system:data-grant:read/write` 或通用用户目录管理权限。`managementScopes:[]`不批准任何新的grant管理范围。

新负责人和新员工用当前应用Bearer调用既有成员窄HTTP，验证目标组织中的有效直接成员，再取得业务锁。成员见证从HTTP开始以单调时钟计算 `min(PLATFORM_AUTH_TIMEOUT_MS,3000ms)` 绝对截止，绑定操作者、tenant、当前组织、resource／action、目标组织和目标user；响应读取、池与行锁等待都消耗原窗口，锁后不续期。锁内复核业务identity／version／旧manager与见证，过期503 `MEMBER_VALIDATION_EXPIRED`回滚；平台HTTP超时503 `PLATFORM_UNAVAILABLE`。省略或保持同一manager不重查旧目标，清除历史关联不要求失效旧员工重新通过成员验证。

最后一次平台检查到业务COMMIT之间仍可能发生停用成员或撤权；两个数据库没有共同事务，不承诺跨库强一致或自动撤销已保存关联。下一新请求会重新校验身份／权限；本地写入成功不是持续拥有平台授权的证明。

## 日期、金额、版本及恢复

日报金额必需十进制字符串，0至9999999999999999.99，最多16位整数和2位小数；拒绝负数、JSON number、指数、逗号、空格或需舍入的精度。写入用检查过的整数分转换成参数化numeric值，返回固定两位小数。SUM由PG numeric完成，其字符串结果可超过单笔上限，不经浮点或局部i64累计。

营业日严格有效 `YYYY-MM-DD`，年份0001–9999。省略时按服务器UTC时刻＋08:00的Asia/Shanghai民用日计算；客户端不能提交timestamp／offset，也不人为拒绝有效历史／未来日期。相同门店同日重复或并发录入返回409 `DAILY_REPORT_EXISTS`，不同正文也不覆盖已有日报；应查询现有日报后显式带version修改。

PATCH及员工解除关联必需当前正整数version；冲突409保留调用方草稿并重新读取。日报日期／store／tenant／org／owner及门店owner均不能编辑。本服务无Idempotency-Key／receipt恢复协议；503 `CREATE_RESULT_UNKNOWN`／`WRITE_RESULT_UNKNOWN`、网络断开或调用取消不能证明未提交。先读回当前组织门店、员工关联或门店／营业日日报及version核对，再由操作者决定后续请求；不自动重发POST／PATCH／DELETE，不套用工单的创建键契约。

## 健康、HTTP与验证边界

- `GET /stores-api/health/live`只证明进程可响应；`GET /stores-api/health/ready`及`/stores-api/health`检查门店库迁移／schema，不等同于平台实时授权链可用。
- 所有响应使用 `X-Request-Id`、no-store及安全错误 `{code,message}`；合法请求UUID沿用，否则新建。写入只接受application/json，正文最多64KiB，未知写字段拒绝。
- CORS允许配置中的精确来源，默认 `Origin:null`，不发送Access-Control-Allow-Credentials；OPTIONS只放行声明方法与Authorization／Content-Type／X-Request-Id，不查询平台或业务库。`null`不是身份，实际请求仍需Bearer与授权。

独立build／test、空库／重复启动／坏schema、真实平台普通用户、三个库与专属非超级账号、双业务ELF交叉token、scope／硬上限、成员失效／期限、日报并发／精度／日期边界及安全日志都需要本阶段实际验证。这里列出源码契约与命令，不预记任何测试PASS；最终状态以[台账](../../../docs/architecture-optimization-ledger.md#phase-26)和本阶段原始证据为准。
