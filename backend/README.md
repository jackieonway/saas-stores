# 门店 API 服务

`stores-api` 使用 Rust `1.97.0`、Axum、SQLx 和 PostgreSQL，默认监听 `0.0.0.0:3031`，业务前缀 `/stores-api`。服务拥有自己的数据库与账号，默认启动执行并核验内嵌迁移；已完成 UUID 导入的目标使用下述 `imported` 只读启动校验。平台身份、有效 STORE 组织投影和直接成员通过 HTTP 获取。平台维护组织身份及成员，服务只维护营业资料、营业联系人和日报。

## 启动和源码

程序只读取启动环境，不自动加载 `.env` 或 `config.example`。完整示例见 [开发指南](docs/development.md)，生产运行见 [部署指南](docs/deployment.md)。注入配置后，在本目录执行 `cargo run --locked`；本目录也是 `docker build` 的后端上下文。

独立迁移命令为 `cargo run --locked --bin stores-migrate`，导入前初始化可使用 `cargo run --locked --bin stores-migrate -- --import`。两种方式都只读取 `STORES_DATABASE_URL`，连接已创建的数据库、执行并核验当前全新基线，然后关闭连接退出；不启动 HTTP，不要求平台或 CORS 配置，不创建数据库或角色，`--import` 不导入数据。门店没有 seed，`--seed` 会被拒绝；`--seed --import` 明确互斥，未知或重复参数也会在连接前失败。

迁移命令拒绝 `PG*` 环境变量，避免驱动继承 URL 之外的端口、密码、证书或会话设置；数据库 TLS 仍通过 `STORES_DATABASE_URL` 的受支持参数配置。连接获取限时 5 秒，迁移限时 5 分钟，关闭连接限时 10 秒。先停用业务进程，由导入流程保持独占；该命令不提供数据导入互斥锁。默认 `cargo run --locked` 仍启动 `stores-api`。

迁移命令不读取默认 pgpass 文件。新库 owner 使用 `NOLOGIN` 时，可由本机 postgres peer 会话通过 URL 显式指定 owner，例如 `postgresql://postgres@localhost:5432/new_stores_db?host=/run/postgresql&options[role]=new_stores_owner`。`host` 必须是规范绝对 Unix socket 目录，`options[role]` 仅接受 1–63 字节的小写字母、数字、下划线标识符且不能以数字开头；该委托形式要求 `postgres@localhost`、无密码并显式给出 socket。其他启动 options、未知或重复 URL 参数均拒绝。正常 API 的 URL 参数合同不变。

API 的 `STORES_DATABASE_INIT_MODE` 缺省或为 `seed` 时保持原初始化行为（门店没有 seed 数据）。`import` 只能用于上述独立迁移 CLI；普通 API 会在连接前拒绝该值。`imported` 要求同时设置 `STORES_EXPECTED_DATABASE`、`STORES_IMPORT_MARKER` 和 `STORES_EXPECTED_SCHEMA_SHA256`；其他模式携带任一字段，即使为空，也会在连接前失败。数据库名必须与 URL 路径按 UTF-8 百分号解码后的名称完全相同，拒绝控制字符。marker 必须完整匹配 `saas-uuid-import/1/<规范小写 RFC UUIDv7>/<64 位小写十六进制摘要>`，schema 摘要也必须为 64 位小写十六进制；这些值由已完成导入的认证回执提供，API 不生成或更新它们。

`imported` 启动跳过所有迁移与初始化写入。在同一个 `READ ONLY`、`REPEATABLE READ` 事务中，依次核对实际库名、数据库 COMMENT 与 marker、`current_user` / `session_user` 与 URL LOGIN 账号、运行角色权限、纯 catalog 有序 JSON 的 SHA-256，以及现有完整 SQLx migration history / v3 fingerprint 链。schema 摘要与 UUID 导入使用相同的 canonical JSON 字节合同；校验不扫描业务数据行。运行账号必须独立于数据库、用户 schema 和对象 owner，不能是超级用户或具有建库、建角色、replication、bypass RLS 权限，也不能具有数据库 CREATE / TEMPORARY、非临时 schema CREATE 权限，或通过继承和 `SET ROLE` 到达 owner / 高权角色；CREATE / TEMPORARY 检查同时覆盖可通过 `SET ROLE` 到达的角色。部署时 owner 保持 `NOLOGIN`，数据库仅为 API LOGIN 授予 CONNECT，并撤销 PUBLIC 的默认 TEMPORARY；schema / 表 / 函数只授予业务所需 USAGE、DML 与只读校验的 SELECT / EXECUTE。全部检查通过后才绑定 HTTP；失败仅输出固定分类，不输出 URL、账号、库名、marker 或数据库驱动细节。

| 模块 | 责任 |
| --- | --- |
| `config.rs`、`lib.rs`、`main.rs` | 配置验证、连接池、迁移、进程启动及限时关闭 |
| `http.rs` | 路由、Bearer、JSON、CORS 和请求标识 |
| `authorization/` | 平台 HTTP 协议、身份与数据授权校验 |
| `stores.rs`、`staff.rs`、`reports.rs` | 营业档案、平台成员只读视图、日报和汇总 |
| `directory.rs` | 受控成员目录用途及搜索 / 分页校验 |
| `domain.rs` | 金额、日期、UUID、版本、查询与分页规则 |
| `migrations.rs`、`migrations/` | 迁移历史和数据库结构契约 |
| `imported_database.rs`、`imported_schema.sql` | 导入回执、运行角色与 catalog 摘要的只读启动校验 |
| `telemetry.rs` | 受保护指标及固定分类诊断 |

## 必需环境与来源

| 变量 | 约束 |
| --- | --- |
| `STORES_DATABASE_URL` | 独立 PostgreSQL URL，必须指定主机、账号和数据库；不会回退到 `DATABASE_URL` |
| `PLATFORM_API_ORIGIN` | 平台准确 HTTPS origin；非生产仅回环地址允许 HTTP |
| `PLATFORM_CLIENT_ID` | 当前应用的机密客户端 ID，1–100 位字母、数字、点、下划线或连字符 |
| `PLATFORM_CLIENT_SECRET` | 后端机密，1–1024 字符，不含换行 |
| `STORES_ALLOWED_ORIGINS` | 必填，逗号分隔的准确平台 origin；开发与生产均无隐式默认值，拒绝空值、`null` 和通配符 |

运行完整 HTTP 应用必须提供平台三项配置。`NODE_ENV=production` 启用生产 origin 和端口约束；生产仅 HTTPS，开发额外允许准确回环 HTTP。CORS 不使用 Cookie credentials，网关保留 Origin 时同源写入也必须匹配配置。没有 Origin 的服务间健康请求可用。

PostgreSQL URI 只接受 `sslmode` / `ssl-mode`、根证书 `sslrootcert` / `ssl-root-cert` / `ssl-ca`、客户端证书 `sslcert` / `ssl-cert`、客户端私钥 `sslkey` / `ssl-key` 和 `application_name`；同类参数不能重复。TLS 默认 `prefer`，可显式选择 `disable`、`allow`、`require`、`verify-ca` 或 `verify-full`。数据库 TLS 与平台 HTTPS 独立配置。

## 可选环境与预算

数值必须是无空格的十进制整数，所有时长单位为毫秒。

| 变量 | 默认 | 范围 / 说明 |
| --- | --- | --- |
| `STORES_HOST` | `0.0.0.0` | 数字 IPv4 / IPv6，不接受域名 |
| `STORES_PORT` | `3031` | 生产 1–65535，非生产允许 0 |
| `STORES_DB_MAX_CONNECTIONS` | `10` | 1–30 |
| `STORES_DATABASE_INIT_MODE` | `seed` | 缺省 / `seed` 执行原初始化；`imported` 只读验证导入回执；API 拒绝 `import` |
| `STORES_EXPECTED_DATABASE` | 不设置 | 仅 `imported` 必填，完全匹配 URL 解码后的数据库名 |
| `STORES_IMPORT_MARKER` | 不设置 | 仅 `imported` 必填，完整匹配数据库 COMMENT 的认证导入 marker |
| `STORES_EXPECTED_SCHEMA_SHA256` | 不设置 | 仅 `imported` 必填，认证回执中的 64 位小写 catalog SHA-256 |
| `STORES_DB_ACQUIRE_TIMEOUT_MS` | `5000` | 100–30000 |
| `STORES_DB_STATEMENT_TIMEOUT_MS` | `5000` | 100–30000 |
| `STORES_DB_LOCK_TIMEOUT_MS` | `5000` | 100–30000 |
| `STORES_STARTUP_TIMEOUT_MS` | `30000` | 1000–300000 |
| `STORES_SHUTDOWN_TIMEOUT_MS` | `10000` | 100–60000 |
| `STORES_HEALTH_TIMEOUT_MS` | `2000` | 100–10000 |
| `PLATFORM_AUTH_TIMEOUT_MS` | `3000` | 100–10000，覆盖一次完整授权 |
| `PLATFORM_AUTH_MAX_CONCURRENT` | `32` | 1–128，额度耗尽时直接失败 |
| `PLATFORM_AUTHORIZATION_MODE` | `legacy` | 仅 `legacy` 或 `combined` |
| `PLATFORM_CA_CERT_FILE` | 不设置 | 额外 PEM CA 路径，文件上限 1 MiB；显式空值无效 |
| `STORES_METRICS_TOKEN` | 不设置 | 启用私有指标；16–1024 位可见 ASCII，不能等于平台 secret |

## 平台服务端契约

每个受保护请求重新获取平台授权，成功结果不缓存。`legacy` 依次调用 `POST /oauth/introspect` 和 `GET /api/apps/stores/context`；`combined` 调用 `POST /api/apps/stores/authorize`，由机密客户端认证并提交 token、resource、action。失败不会自动切换协议。

这两种方式消费的服务端授权契约仍为 `1.0`；前端 SDK 上下文契约为 `2.0`，两者版本不能混用。后端要求身份、租户、当前组织、app code、contextVersion、有效期和对应资源动作一致，并叠加组织硬边界与数据授权。

营业联系人验证及平台成员读取使用 `/api/apps/stores/members` 和 `/members/{user}`，可查用途必须先在平台策略中登记，详见 [成员契约](docs/member-contract.md)。有效 STORE 查询使用 `/api/apps/stores/organizations` 和 `/organizations/{id}`，七种用途必须登记为 `organizationLookups`。每个投影绑定原授权 contextVersion，窗口最长三秒，所有业务 SQL 与允许的 STORE 集合相交。公开应用与菜单示例在 `manifests/bootstrap.manifest.json`，数据授权示例在 `manifests/grants.manifest.example.json`，平台数据策略示例在 `policy.example.json`。

## 数据和运维边界

数据库包含营业档案、历史员工关联、日报和审计事件。当前每个 tenant / STORE organization 只能有一份营业档案，同一门店同一营业日只能有一份日报。平台名称、编码、地址和层级不接受业务写入；名称响应来自平台实时投影，成员列表来自平台直接成员。旧 staff 写路由返回 410，不再写关联表。更新采用整数 `version` 乐观并发；金额以十进制字符串传输。详情、分页、汇总和写入都受相同授权范围约束，完整输入和恢复规则见 [API](API.md)。

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --lib
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

真实 PostgreSQL 专项测试需要 `tests/postgres_contracts.rs` 要求的独占 CI 数据库所有权环境，普通测试不会执行。不可对共享、开发业务或生产库直接启用该测试。

Windows 本机若限制 Rust 测试可执行文件的回环入站，可按 [开发指南](docs/development.md)通过测试专用 `STORES_TEST_NODE` 选择 Node 监听 fixture，保留生产客户端与所有原测试断言。该变量不影响生产 API，也不调整系统防火墙。


## 全新数据库基线

仅使用 `migrations/0001_stores.sql` 初始化空库，不再兼容旧升级链或历史迁移记录。SQLx 只记录版本 1，并核对当前结构指纹；漂移或 checksum 不符拒绝启动。不会通过改写迁移历史或旧快照签认现状。UUIDv7、当前身份/不可变约束、幂等与业务审计在首次建表时全部建立。

系统与其他服务的数据归属见[全新安装](../../saas-plateform/backend/docs/fresh-installation.md)。
