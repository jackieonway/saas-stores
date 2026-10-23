# 门店 qiankun 前端

本目录是可独立安装、测试和构建的 React 应用，也是前端 Docker 构建上下文。平台登记应用 code 为 `stores`，路由基址 `/micro/stores`，入口类型 `HTML`，运行模式 `TRUSTED`。源码入口为 `src/qiankun-entry.ts`。

## 本地开发

使用 Node `24.22.1`、npm `11.16.0`。在本目录运行：

```powershell
node --version
npm --version
npm ci
$env:STORES_ALLOWED_HOST_ORIGINS = 'http://127.0.0.1:3000'
$env:STORES_API_BASE = 'http://127.0.0.1:3031/stores-api'
npm run dev
```

按 [开发指南](../backend/docs/development.md)配置平台和 API，再将 `http://127.0.0.1:3014/index.html` 登记为应用入口。默认端口 `3014` 严格占用，已有进程时启动失败。

`dev` 先校验类型和输出目录，生成生产 UMD，再启动 watch 构建与静态预览。修改源码后刷新平台页面生效；修改公开配置后重新启动命令。直接访问 HTML 只加载资源，业务上下文由平台挂载时提供。

## 生命周期与上下文

UMD 全局名称是 `StoresMicroApp`。对 qiankun 导出的四个方法如下：

| 方法 | 行为 |
| --- | --- |
| `bootstrap()` | 初始化阶段，无业务副作用 |
| `mount(props)` | 检查宿主 origin、有效 signal 和挂载容器，创建业务控制器及 React root |
| `update(props)` | 更新身份、组织、权限、主题和 `routePath` |
| `unmount()` | 取消请求、释放控制器、销毁 React root；支持后续重新挂载 |

宿主传入 `container` 和 `platform`；`platform` 是 SDK `2.0.0` 的 `MicroProps`，契约版本 `2.0`。只有该嵌套对象进入业务 SDK，qiankun 的 `name`、`container`、`services` 不进入严格的上下文结构。容器内必须包含 `[data-stores-mount]`。

`bootstrap.sdkContract` 是不可写、不可配置的冻结对象，直接引用随 UMD 打包的 SDK 版本与 `assertSupportedSdkVersion`。它仅提供公开版本信息和版本协商，不包含身份、令牌或凭据，不增加 UMD 顶层导出。

生命周期按队列串行执行。已取消的上下文不会挂载或更新；切身份、组织或授权替换会取消旧请求并清除旧视图。合法续期仍需重新读取编辑版本。路由来自 `platform.routePath`，导航通过 `platform.onNavigate` 交给宿主。所有业务样式以 `[data-stores-root]` 为边界，主题写在应用节点上。

## 公开配置

| 环境变量 | 构建参数 | 默认值及约束 |
| --- | --- | --- |
| `STORES_VERSION` | `--version` | `package.json` 的版本（当前 `1.0.1`）；1–64 位字母、数字、点、下划线或连字符，不能含 `..` |
| `STORES_ALLOWED_HOST_ORIGINS` | 可重复 `--allowed-host-origin` | 必填，环境值以逗号分隔，必须是准确 origin，不能带路径或尾斜杠 |
| `STORES_API_BASE` | `--api-base` | `/stores-api`；根路径或明确的 HTTP(S) API 地址 |
| 无 | `--out-dir` | 本目录 `dist`；自定义输出仅允许在系统临时目录内 |

远程宿主和 API 必须使用 HTTPS；仅 `localhost`、`127.0.0.1`、`[::1]` 可用 HTTP 开发。配置编译进发布包，不在运行时读取后端环境。`VITE_*` 不会自动注入，secret、数据库 URL 和平台令牌不得作为前端配置。

qiankun 子应用运行在平台页面中。跨域业务 API 看到的 Origin 是平台宿主，API 的 `STORES_ALLOWED_ORIGINS` 必须匹配该来源。使用相对 `/stores-api` 时，需要宿主公开网关转发该前缀；API 对实际携带的 Origin 仍执行校验。静态入口与 JS/CSS 跨域加载也需要 CORS。

## 测试和发布

```powershell
npm run typecheck
npm test
npm run build -- --version 2.0.0-local --allowed-host-origin https://platform.example.com --api-base /stores-api
npm run verify:release -- dist --version 2.0.0-local
npm run preview
```

`preview` 读取同一份公开环境配置。`test` 包含真实构建、UMD 导出验证、上下文取消、权限与业务恢复，以及复制工程后从空 npm 缓存安装依赖的检查，需要网络和临时目录空间。构建会先通过 TypeScript 检查，再替换自身拥有的输出目录；未知目录、符号链接或不属于本应用的旧输出会被拒绝覆盖。

发布整个 `dist/`：HTML、单一 `assets/micro-app.umd.js`、CSS、许可证、`asset-manifest.json` 和 `build-info.json`。React / ReactDOM 随 UMD 打包，宿主无需共享框架；输出没有原生 ESM 入口或动态 chunk。`build-info.json` 记录 `runtime=qiankun`、版本、SDK、契约、工具版本及输入摘要。

每次发布使用新版本 URL；HTML 重新校验，版本资源可以长期缓存，缺失资源必须返回真实 `404`。当前源码构建不会改写 `artifacts` 内的历史 iframe 发布。新发布、平台登记和回退规则见 [部署指南](../backend/docs/deployment.md)，本机演练见 [独立网关](deploy/local/README.md)。写入冲突和结果未知的处理见 [API 契约](../backend/API.md)。

`verify:release` 需要目录和显式版本，检查运行时 / SDK / 契约、HTML 与清单中的相对资源，以及实际 UMD 的四个生命周期导出。HTML 使用固定版本的 parse5 解析大小写、引号和实体，拒绝所有 ESM type 形式；实际 UMD 必须携带只读冻结的 `bootstrap.sdkContract`，接受 SDK `2.0.0` 并拒绝旧版本及未知版本。元数据写着 SDK 2 的无 SDK 空壳也不能通过。门禁验证兼容性，不提供签名或供应链真实性证明。本机 `release:local` 使用同一构建器生成并验证未激活候选，拒绝覆盖已有版本；CI 保存通过该门禁的完整静态产物。
