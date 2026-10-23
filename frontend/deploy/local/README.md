# 本机独立网关

本目录提供 Windows + WSL Docker 的独立静态版本接入工具。公开入口为 `http://127.0.0.1:3000`，组合平台前端、门店 API 和已完整发布的 qiankun 版本目录。它适用于固定静态包验收；从源码持续开发可直接使用[开发指南](../../../backend/docs/development.md)的 3014 开发服务。

| 公开请求 | 实际目标 |
| --- | --- |
| `/sub-apps/stores/versions/<version>/...` | 只读版本根 `stores/versions/<version>/...` |
| `/stores-api` 及子路径 | `127.0.0.1:3031`，保留前缀、Origin、OPTIONS |
| `/stores-api/internal` 及子路径、业务 metrics 路径 | 404 |
| 其他请求与 WebSocket | 平台 Vite `127.0.0.1:3002` |

## 准备平台与 API

先按开发指南启动平台后端和门店 API，注入本次登记的机密客户端与专属数据库配置。门店 API 的 `STORES_ALLOWED_ORIGINS` 必须包含准确宿主 `http://127.0.0.1:3000`，平台 API origin 使用后端实际可达地址。

平台须支持 STORE 组织和受控组织 / 成员接口，并按当前 bootstrap 配置九个菜单、七个业务权限、七个 STORE organizationLookups 与三个 memberLookups。名称、层级、地址和成员在平台维护，门店应用仅完善营业资料及录入日报。已有历史映射不会自动转为 STORE。

为独立网关模式启动平台前端：

```powershell
npm run dev -- --host 127.0.0.1 --port 3002
```

确认平台后端健康和门店 `/stores-api/health/ready` 均可用，再发布前端。本指南不读取旧运行凭据，也不提供可以直接运行的客户端 secret。

## 发布本次源码的新版本

先选择一个尚未使用的实际版本号。下面的占位值必须替换；整组命令应使用同一个版本。在 `frontend` 执行：

```powershell
$releaseVersion = '<本次未使用的新版本>'
npm ci
npm run release:local -- --version $releaseVersion --allowed-host-origin http://127.0.0.1:3000 --api-base /stores-api
npm run verify:release -- "../artifacts/micro-apps/stores/versions/$releaseVersion" --version $releaseVersion --published
```

运行时固定为 Node 24.22.1 / npm 11.16.0，执行前检查 PATH 实际命中的版本。工具在临时目录构建并校验 qiankun、实际 SDK 2.0.0 / 契约 2.0、HTML、资源摘要、CSS、许可证、UMD 四个生命周期及只读 SDK 协商声明，随后发布到版本目录。

发布根及其祖先禁止 symlink / junction。工具排他创建版本目录与文件，最后写入 `release-complete.json`；已有版本即使为空也拒绝覆盖，失败的未完成目录保留供检查，重试使用新版本。普通开发 dist 没有已发布版本的完成标记，需经 release 流程才能用于此网关。

构建成功后，核对 build-info 中的来源输入与本次源码。将平台应用登记为 `entryType=HTML`、`runtimeMode=TRUSTED`，入口设为 `/sub-apps/stores/versions/<本次实际版本>/index.html`，再按平台流程激活。菜单、OAuth 关联和用户权限也需正式登记；文件发布不会自动改变平台数据库或激活版本。

## 启动与检查

在本目录设置与上一步相同的实际版本：

```powershell
$releaseVersion = '<本次实际发布版本>'
./start-gateway.ps1 -ReleaseVersion $releaseVersion
```

启动器保持当前终端运行，使用 WSL Docker 和批准的非 root Nginx 镜像，容器名为 `codex-stores-gateway-local`。另开终端在本目录执行：

```powershell
$env:STORES_RELEASE_VERSION = '<本次实际发布版本>'
node verify-gateway.mjs
```

`-ReleaseRoot` 可指定其他绝对版本根，布局仍为 stores/versions/version；`-Image` 可指定批准的镜像，`-NodeExecutable` 可指定 Node 24.22.1 的绝对路径。启动器在访问 WSL / Docker 前校验完整本地产物，启动器和验证器均无默认历史版本。同名容器存在时启动器退出，需先核对已有实例。

验证器只读访问公开平台 HTML / API 健康、门店 readiness、未认证拒绝、CORS 预检、私有路径屏蔽，以及 UMD / CSS 的 MIME、CORS、缓存和缺资源 404。它还检查直连平台 3002 时不提供门店文件或业务 JSON API；不会携带令牌或创建业务数据。

| 环境变量 | 默认与限制 |
| --- | --- |
| `STORES_GATEWAY_ORIGIN` | `http://127.0.0.1:3000` |
| `PLATFORM_DEV_ORIGIN` | `http://127.0.0.1:3002` |
| `STORES_RELEASE_VERSION` | 必填，无默认 |
| `STORES_GATEWAY_TIMEOUT_MS` | 10000，范围 100–60000 |

通过后打开 `/micro/stores`，验证实际 Bearer 注入、qiankun 挂载、STORE 目录、营业资料和日报、成员只读、身份续期、切组织、撤权与卸载后重入。组织投影的短期窗口由 API 在每次请求内重新检查，用户填写表单不受三秒时限约束。健康与静态验证不替代普通用户及越权验收。

## 历史包与恢复

`2.0.3-qiankun` 是前一轮 qiankun / Node 升级的历史快照，未包含本次平台 STORE 组织权威、成员只读及新目录业务。更早的 2.0.x 包也按当时来源保留；其中部分不满足当前发布门禁。1.0.0 / 1.0.1 为 SDK 1 的旧 iframe 发布。历史目录保留原始字节用于追溯，不据此推断当前源码、运行服务或激活状态。

只读复核某个历史包可运行 `node ../verify-release.mjs <历史目录> --version <历史版本> --published`；门禁通过只证明包结构符合检查规则，仍需核对来源摘要和业务功能。回退前还须确认后端与已经执行的数据库迁移兼容。

停止和重新附着到已经核对的网关实例：

```powershell
wsl --exec docker stop codex-stores-gateway-local
wsl --exec docker start -a codex-stores-gateway-local
```

配置变更先在目标容器执行 nginx -t，再 reload；镜像、挂载或容器环境变化需要重新创建实例。本目录仅用于回环 HTTP。生产按[部署指南](../../../backend/docs/deployment.md)配置 HTTPS、受控网络和不可变版本存储。
