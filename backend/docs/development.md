# 从源码开发门店应用

本指南使用 Windows PowerShell 举例。所有命令在 `saas-stores` 对应目录执行。准备 Rust `1.97.0`、Node `24.22.1`、npm `11.16.0`、独立 PostgreSQL 数据库，以及可访问的 SaaS 平台。首先用 `node --version`、`npm --version`、`cargo --version` 确认 PATH 实际命中的工具。

## 平台准备

1. 根据 [应用清单](../manifests/bootstrap.manifest.json)登记 `stores`，基址 `/micro/stores`，`entryType=HTML`、`runtimeMode=TRUSTED`。开发入口为 `http://127.0.0.1:3014/index.html`。
2. 创建并关联本应用的机密客户端，妥善保存平台生成的客户端 ID 与一次性 secret。清单中的客户端信息是登记参考，不是可运行凭据。
3. 按清单创建菜单与按钮；根据 [授权清单模板](../manifests/grants.manifest.example.json)填写实际应用、菜单和授权目标 ID。模板中的空 targets 不会自动授予任何用户权限。
4. 将 [策略示例](../policy.example.json)中的实际应用 ID、九个资源动作、三个 `memberLookups` 与所需管理范围配置到平台，随后给测试用户授予相应功能及数据范围。

平台应用、菜单、OAuth 绑定和授权存储在平台自己的数据库。更新仓库 JSON 不会自动更新已有登记。日常验收应包含普通用户及越权用户，管理员可访问不能代表权限矩阵正确。

## 启动独立 API

为门店准备单独的库与账号。账号需要在该库内创建、迁移和访问门店结构的权限。进入 `backend`，通过私有启动环境设置：

```powershell
$env:NODE_ENV = 'development'
$env:STORES_DATABASE_URL = 'postgresql://<账号>:<URL编码密码>@127.0.0.1:5432/stores'
$env:STORES_HOST = '127.0.0.1'
$env:STORES_PORT = '3031'
$env:STORES_ALLOWED_ORIGINS = 'http://127.0.0.1:3000'
$env:PLATFORM_API_ORIGIN = 'http://127.0.0.1:3000'
$env:PLATFORM_CLIENT_ID = '<实际机密客户端 ID>'
$env:PLATFORM_CLIENT_SECRET = '<实际私有 secret>'
$env:PLATFORM_AUTHORIZATION_MODE = 'legacy'
cargo run --locked
```

示例中的占位值必须替换，凭据不要提交到仓库。程序不自动加载 `.env`。完整数值预算、TLS 和来源规则见 [后端说明](../README.md)。`legacy` 使用内省加服务端上下文，`combined` 需要平台提供匹配的合并授权接口；两者没有失败回退。

启动会连接专属库、执行内嵌迁移并校验结构。检查 `http://127.0.0.1:3031/stores-api/health/live` 和 `/stores-api/health/ready`。就绪失败时检查数据库连通性、迁移历史及固定错误分类，不能改历史迁移或 fingerprint 绕过。

## 启动 qiankun 前端

另开终端进入 `frontend`：

```powershell
npm ci
$env:STORES_ALLOWED_HOST_ORIGINS = 'http://127.0.0.1:3000'
$env:STORES_API_BASE = 'http://127.0.0.1:3031/stores-api'
npm run dev
```

在平台登录并打开 `http://127.0.0.1:3000/micro/stores`。开发命令提供 `3014` 的 classic UMD 入口，源码变更自动重建，刷新宿主后生效。平台通过 `props.platform` 传入 SDK `2.0.0` 上下文；直接打开 `3014/index.html` 不具备平台身份。

上述模式由浏览器直接请求 `3031`，API 白名单应是宿主 `3000` 的 origin。若宿主网关已提供 `/stores-api` 代理，可将 `STORES_API_BASE` 改为 `/stores-api`。代理保留 Origin 时，API 仍需匹配宿主来源；跨域静态资源也要允许宿主读取。

需要固定版本静态发布时使用 [本机独立网关](../../frontend/deploy/local/README.md)，由公开网关 `3000` 转发平台 `3002`、业务 API `3031` 和版本目录。该模式不修改平台 Vite 代理。

## 验证命令

仓库根目录：

```sh
node .github/scripts/check-inputs.mjs
node --test .github/tests/repository-structure.mjs
```

`frontend` 目录：

```sh
npm run typecheck
npm test
npm run build -- --version 2.0.0-local --allowed-host-origin http://127.0.0.1:3000 --api-base /stores-api
```

`backend` 目录：

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --lib
cargo clippy --locked --all-targets -- -D warnings
```

前端测试含真实发布构建和空缓存依赖安装，需要网络。后端库测试含本机 HTTP loopback 服务。`tests/postgres_contracts.rs` 中的数据库测试默认忽略，显式运行要求 `TEST_DATABASE_URL` 与 `CI_TEST_POSTGRES_OWNER` 对应源码检查的独占 CI PostgreSQL 主机；URL 指向 `/postgres`，主机为 `<ci-owner>-postgres`，测试会创建并删除随机隔离数据库。

对本次专门创建的临时本机 PostgreSQL cluster，可显式设置测试专用 `STORES_TEST_LOCAL_POSTGRES_OWNER=<ci-owner>@<精确loopback主机>:<端口>`，其中 owner 必须与 `CI_TEST_POSTGRES_OWNER` 完全相同。此模式只接受 `127.0.0.1` 或 `[::1]`、显式的非默认端口（1024 以上且不是 5432），仍只新建并删除随机测试库；不能对日常开发或生产 cluster 启用。准备好独占实例后运行 `cargo test --locked --test postgres_contracts -- --include-ignored --nocapture`，核对输出中的 `database_removed=true` 和 `pool_closed=true`。

若 Windows 环境阻止 Rust 测试进程的回环入站连接，可为当前测试进程显式指定 Node 监听器：

```powershell
$env:STORES_TEST_NODE = '<Node 24.22.1 可执行文件的绝对路径>'
cargo test --locked --lib
Remove-Item Env:STORES_TEST_NODE
```

该选项只编译进测试 fixture。生产 reqwest 客户端、Rust Router、原始响应字节、请求计数、大小与超时断言都保持执行，Node 只负责回环监听和管道传输，退出时清理子进程。不配置时使用原 Rust 监听器。Linux CI 验证原监听路径，Windows CI 显式运行 Node 路径；另一个 CI job 为 PostgreSQL contracts 启动自有 PostgreSQL 18 service。不需要修改防火墙或全局代理。

## 联调验收

通过浏览器确认 qiankun 正常挂载、深路由、主题、切组织、身份续期和离开后重新进入。验证无令牌 `401`、非法来源 `403`、数据范围外详情 `404`，并分别测试门店、员工关联、日报和汇总的合法读取与越权写入。

对成员候选、负责人和员工关联分别验证允许组织及成员失效。对版本冲突先读最新版本再明确采用；结果未知时按 [API 恢复规则](../API.md)核对原始业务目标，不自动重发。最后检查静态资源 MIME、CORS、缺资源 `404` 和版本切换。

仓库中保留的 `1.0.0`、`1.0.1` 静态目录属于旧 iframe 发布。新源码必须构建为新版本并更新平台入口；过去的联调状态和截图不作为当前 qiankun 验收结果。
