# 本机独立网关

本目录提供 Windows + WSL Docker 的本机接入工具。公开入口 `http://127.0.0.1:3000` 同时连接平台、门店 API 和不可变版本文件，路由配置保存在本项目，平台 Vite 通过命令行改端口。

| 公开请求 | 实际目标 |
| --- | --- |
| `/sub-apps/stores/versions/<version>/...` | 只读挂载目录 `stores/versions/<version>/...` |
| `/stores-api` 及子路径 | 本机 `127.0.0.1:3031`，保留前缀、Origin 和 OPTIONS |
| `/stores-api/internal` 及子路径、业务 metrics 路径 | `404` |
| 其他请求与 WebSocket | 本机平台 Vite `127.0.0.1:3002` |

## 准备服务

先启动平台后端与门店 API，按 [开发指南](../../../backend/docs/development.md)注入平台凭据和独立数据库配置。门店 API 的 `STORES_ALLOWED_ORIGINS` 应包含 `http://127.0.0.1:3000`。平台自己的 API origin 与公开网关需要按实际可达地址设置。

平台前端目录运行：

```powershell
npm run dev -- --host 127.0.0.1 --port 3002
```

本机 `artifacts/start-local-api.ps1` 是可选辅助启动器，依赖用户私有目录中的 `runtime.json` 和已经构建的 `codex-stores-api:local` 镜像。它不会生成凭据；已有同名容器时会直接启动旧容器，不会更新环境或镜像。修改私有配置后要核对实际容器环境和重建安排，不能仅重新运行脚本就认定新配置生效。

## 检查已有候选

当前候选为 `2.0.3-qiankun`，由 Node `24.22.1` 构建。先在 `frontend` 目录只读检查现有产物：

```powershell
npm run verify:release -- ../artifacts/micro-apps/stores/versions/2.0.3-qiankun --version 2.0.3-qiankun --published
```

这条命令不重建或覆盖候选。该版本已存在，再次执行相同版本的 `release:local` 会按设计失败。

## 生成后续 qiankun 静态包

在 `frontend` 目录构建一个尚未使用的新版本：

```powershell
npm ci
npm run release:local -- --version 2.0.4-local --allowed-host-origin http://127.0.0.1:3000 --api-base /stores-api
```

工具先在系统临时目录构建，校验 qiankun / SDK `2.0.0` / 契约 `2.0`、HTML、清单、CSS、许可证和真实 UMD 生命周期及其只读 SDK 协商声明，再发布到 `artifacts/micro-apps/stores/versions/<version>`。发布根及其祖先不允许 symlink / junction；工具排他创建版本目录并排他复制文件，最后写入 `release-complete.json`。预存空目录、失败留下的未完成目录和并发占用均拒绝覆盖，失败目录保留供检查，重试应选择新版本。

示例 `2.0.4-local` 需要在实际使用前确认尚未存在；重复发布时换用新版本，不删除或覆盖已有目录。早期 `2.0.0-qiankun` 缺少 SDK 声明，`2.0.1-qiankun` 缺少排他发布完成标记，都不能通过当前已发布版本门禁。`2.0.2-qiankun` 仍保留完整产物，但使用旧 Node `24.18.0` 且三项来源输入已与当前源码不同，不作为当前候选。历史 `1.0.0`、`1.0.1` 使用 SDK `1.0.0` iframe，以上历史字节均保留。

通过平台管理界面将选中的应用登记为 HTML / TRUSTED；接入当前已有候选时入口为 `/sub-apps/stores/versions/2.0.3-qiankun/index.html`。若接入刚生成的后续版本，应将登记 URL 和下面的启动、验证版本一起替换。应用菜单、OAuth 关联和用户授权仍由平台管理。只发布文件不会改变激活版本或运行模式。

## 启动与检查

在本目录运行：

```powershell
./start-gateway.ps1 -ReleaseVersion 2.0.3-qiankun
$env:STORES_RELEASE_VERSION = '2.0.3-qiankun'
node verify-gateway.mjs
```

启动器使用已安装的 WSL Docker 和官方非 root Nginx，容器名 `codex-stores-gateway-local`，保持当前终端运行。`-ReleaseRoot` 可以指定其他绝对版本根，布局仍需是 `stores/versions/<version>/...`；`-Image` 可指定批准的 Nginx 镜像。

启动器和验证器均要求显式版本，不默认选择历史目录。启动器在调用 WSL 或 Docker 前校验本地完整产物；`verify-gateway.mjs` 从公开 URL 校验相同元数据、HTML、UMD 生命周期和完成标记。手工检查本机候选可运行 `node ../verify-release.mjs <候选目录> --version <版本> --published`；构建目录的通用验证不带 `--published`。启动器使用 PATH 中的 Node，可通过 `-NodeExecutable <Node 24.22.1 的绝对路径>` 指定运行时。工具不会帮平台激活版本；Nginx 按 URL 提供挂载根下的版本。

验证器只读访问公开平台 HTML / API 健康、门店 readiness、未认证拒绝、合法和非法 CORS 预检、私有路径屏蔽、UMD / CSS 的 MIME / CORS / 缓存及缺资源 `404`。它还检查直连平台 `3002` 时不提供门店静态文件和 JSON API。检查不携带令牌、不创建业务数据。

| 验证器环境变量 | 默认 |
| --- | --- |
| `STORES_GATEWAY_ORIGIN` | `http://127.0.0.1:3000` |
| `PLATFORM_DEV_ORIGIN` | `http://127.0.0.1:3002` |
| `STORES_RELEASE_VERSION` | 必填，无默认值 |
| `STORES_GATEWAY_TIMEOUT_MS` | `10000`，允许 100–60000 |

检查通过后打开 `http://127.0.0.1:3000/micro/stores`，验证平台 Bearer 注入、qiankun 挂载、业务查询、切组织、撤权、离开后重入。健康与静态验证不替代用户权限验收。

## 停止与调整

```powershell
wsl --exec docker stop codex-stores-gateway-local
wsl --exec docker start -a codex-stores-gateway-local
```

启动器发现同名容器会退出，避免无意覆盖现有部署。配置变更先在目标容器执行 `nginx -t` 再 reload；挂载路径、镜像或容器环境变更需要重新创建容器。此目录只服务回环 HTTP，本机验收完成后按 [生产部署指南](../../../backend/docs/deployment.md)配置 HTTPS 和受控网关。
