# 门店 API 服务

`stores-api` 使用 Rust `1.97.0`、Axum、SQLx 和 PostgreSQL，默认监听 `0.0.0.0:3031`，业务前缀 `/stores-api`。服务拥有自己的数据库与账号，启动时执行并核验内嵌迁移。平台身份和成员资料通过 HTTP 获取。

## 启动和源码

程序只读取启动环境，不自动加载 `.env` 或 `config.example`。完整示例见 [开发指南](docs/development.md)，生产运行见 [部署指南](docs/deployment.md)。注入配置后，在本目录执行 `cargo run --locked`；本目录也是 `docker build` 的后端上下文。

| 模块 | 责任 |
| --- | --- |
| `config.rs`、`lib.rs`、`main.rs` | 配置验证、连接池、迁移、进程启动及限时关闭 |
| `http.rs` | 路由、Bearer、JSON、CORS 和请求标识 |
| `authorization/` | 平台 HTTP 协议、身份与数据授权校验 |
| `stores.rs`、`staff.rs`、`reports.rs` | 门店、员工关联、日报和汇总 |
| `directory.rs` | 受控成员目录用途校验 |
| `domain.rs` | 金额、日期、UUID、版本、查询与分页规则 |
| `migrations.rs`、`migrations/` | 迁移历史和数据库结构契约 |
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

负责人及员工选择使用 `/api/apps/stores/members` 和 `/members/{user}`，可查用途必须先在平台策略中登记，详见 [成员契约](docs/member-contract.md)。公开应用与菜单示例在 `manifests/bootstrap.manifest.json`，数据授权示例在 `manifests/grants.manifest.example.json`，平台数据策略示例在 `policy.example.json`。

## 数据和运维边界

数据库包含门店、员工关联、日报和审计事件。当前每个租户组织只能有一间门店，门店同一营业日只能有一份日报。更新采用整数 `version` 乐观并发；金额以十进制字符串传输。详情、分页、汇总和写入都受相同授权范围约束，完整输入和恢复规则见 [API](API.md)。

迁移历史 checksum 和 schema fingerprint 不匹配时拒绝启动或就绪。通过新增迁移演进结构；不要改写已执行 SQL、迁移历史或 fingerprint 来跳过校验。存活接口只证明进程可响应，就绪接口同时校验业务库结构。

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --lib
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

真实 PostgreSQL 专项测试需要 `tests/postgres_contracts.rs` 要求的独占 CI 数据库所有权环境，普通测试不会执行。不可对共享、开发业务或生产库直接启用该测试。

Windows 本机若限制 Rust 测试可执行文件的回环入站，可按 [开发指南](docs/development.md)通过测试专用 `STORES_TEST_NODE` 选择 Node 监听 fixture，保留生产客户端与所有原测试断言。该变量不影响生产 API，也不调整系统防火墙。
