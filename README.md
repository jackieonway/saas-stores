# 门店微应用

`saas-stores` 提供门店资料、员工关联和每日营业记录。前端是 React / TypeScript / Vite 构建的 qiankun 子应用，后端是独立 Rust HTTP 服务，数据保存在专用 PostgreSQL 数据库。平台提供身份、菜单、数据授权和受控成员查询。

## 文档入口

| 需要完成的工作 | 文档 |
| --- | --- |
| 从源码启动平台接入与业务服务 | [开发指南](backend/docs/development.md) |
| 理解前端生命周期、公开配置和构建 | [前端说明](frontend/README.md) |
| 配置 API、数据库和授权协议 | [后端说明](backend/README.md) |
| 调用门店、员工、日报和成员接口 | [API 契约](backend/API.md) |
| 理解成员归属校验和跨库时效边界 | [成员契约](backend/docs/member-contract.md) |
| 构建镜像、配置网关并升级版本 | [部署指南](backend/docs/deployment.md) |
| 在 Windows / WSL 运行本机独立网关 | [本机网关](frontend/deploy/local/README.md) |

## 当前架构

平台在 `/micro/stores` 挂载应用，以 HTML 入口加载静态发布包。入口导出 `bootstrap`、`mount`、`update`、`unmount`，平台通过 `props.platform` 传入 SDK `2.0.0`、`MicroProps` 契约 `2.0` 的上下文。子应用使用自己的 React 与 ReactDOM，样式限定在 `[data-stores-root]` 内。路由和组织变更通过生命周期更新传递。

浏览器使用应用 Bearer 请求 `/stores-api`。API 每次向平台重新取得身份与授权，并将租户、组织硬边界和资源数据范围写入 SQL 条件。前端按钮权限只决定界面，服务端独立执行授权。机密客户端 secret 和数据库凭据只属于后端环境。

| 目录 | 内容 |
| --- | --- |
| `frontend/src` | 业务页面、控制器、API 客户端和 qiankun 生命周期 |
| `frontend/deploy` | 静态镜像配置及本机网关工具 |
| `frontend/tests` | 领域、上下文、客户端、构建和发布测试 |
| `backend/src` | Rust 路由、授权、业务与可观测性实现 |
| `backend/migrations` | SQLx 迁移和数据库契约 |
| `backend/manifests` | 应用登记、菜单和授权示例 |
| `.github` | 独立仓库输入检查、测试和镜像构建 |
| `artifacts` | 本机辅助脚本和已有发布产物 |

## qiankun 迁移状态

当前源码和 [登记清单](backend/manifests/bootstrap.manifest.json)采用 qiankun：`entryType=HTML`、`runtimeMode=TRUSTED`，构建输出 classic UMD，没有 iframe 桥接入口或动态 ESM chunk。

已有 `artifacts/micro-apps/stores/versions/1.0.0`、`1.0.1` 是 SDK `1.0.0` 的历史 iframe 产物，保留原始字节用于追溯。当前 `2.0.3-qiankun` 是用 Node `24.22.1` 构建、面向 `http://127.0.0.1:3000` 的本机候选，使用相对 `/stores-api`，尚未激活真实平台版本或替换运行容器。它通过完整发布门禁，记录的 31 项来源输入与当前源码一致。

早期候选 `2.0.0-qiankun` 缺少实际 bundled SDK 的只读契约声明，`2.0.1-qiankun` 尚未采用排他安装完成标记，均不能通过当前已发布版本门禁。`2.0.2-qiankun` 的结构门禁仍可通过，但记录 Node `24.18.0`，包配置及默认构建版本逻辑的来源输入已被更新，因此由 `2.0.3-qiankun` 取代。以上历史目录均保留原字节，不作为当前待激活版本。

本机发布、启动及验证工具要求显式版本，使用 HTML parser 解析属性与实体以拒绝 ESM，检查实际 UMD 四个生命周期及 `bootstrap.sdkContract` 的版本协商行为，拒绝旧 SDK / iframe 和没有 SDK 的生命周期空壳。这是发布兼容性检查，不能替代签名或发布来源信任。

本机发布使用排他目录预留和排他文件复制，最后写入 `release-complete.json`；已有版本即使为空也拒绝覆盖。未完成版本不能启动或通过网关验证，根路径及祖先不允许 symlink / junction。

源码修改不会自动更新平台数据库登记或正在运行的容器。发布后仍需核对实际入口、HTML / TRUSTED 模式及激活记录，再通过浏览器验收，不能用静态产物检查代替部署完成证明。

依赖按 `backend/dependencies.lock.json` 校验，SDK 的来源和文件摘要记录在 `frontend/vendor/micro-sdk/SOURCE.json`。升级 SDK 要同步审阅实际发布字节、类型与契约，再更新依赖记录。

## 工具版本与验证

前端固定 Node `24.22.1`、npm `11.16.0`、React / ReactDOM `19.3.0`、TypeScript `5.9.3`、Vite `8.3.4`；后端使用 Rust `1.97.0`。先确认命令行实际使用的版本，再按开发指南运行检查。

本仓库 CI 只验证和构建自身。数据库专项测试、真实平台授权、浏览器挂载和生产部署需要各自的运行环境；静态测试成功不能替代这些验收。

[版本与安全更新记录](RELEASE.md)。
