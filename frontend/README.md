# 门店微应用前端

`stores` 是独立 React／TypeScript／Vite HTML 微应用，通过登记 SDK `1.0.0` 在 opaque iframe 内消费[门店 Rust API](../backend/API.md)。应用 code 为 `stores`，宿主基路径 `/micro/stores`，API 默认前缀 `/stores-api`，没有独立登录或浏览器端 client secret。本文记录当前源码与命令；实际验收由[Phase27 台账](../../../docs/architecture-optimization-ledger.md#phase-27)和阶段证据维护，命令清单不等于测试通过。镜像发行、公开 v1/v2 路由和联合部署归 Phase28；本阶段不抽公共 UI 框架。

## 独立安装与运行

固定 Node `24.18.0`、npm `11.16.0`；React／React DOM `19.3.0`、TypeScript `5.9.3`、Vite `8.3.4` 精确锁定。本目录使用自己的 `package-lock.json`、`.npmrc` 与本地 `node_modules`。在本目录执行；以下是本机开发示例，宿主 origin 必须与实际宿主完全一致：

```powershell
npm ci
$env:STORES_ALLOWED_HOST_ORIGINS = 'http://localhost:3000'
$env:STORES_API_BASE = '/stores-api'
npm run dev
```

独立 dev／preview 默认监听 `127.0.0.1:3014`，端口占用即失败；`npm run dev -- --port 3009` 可改端口。根 `npm run dev:stores` 使用开发调度器默认门店端口 `3009`。这些命令只启动前端，平台、门店 Rust API 与受信任网关须另行配置。缺少宿主上下文时显示连接状态，不构造身份或模拟业务响应。

```powershell
npm run typecheck
npm test
npm run build -- --version 1.0.0 --allowed-host-origin http://localhost:3000 --api-base /stores-api
npm run preview
```

`preview` 同样需要公开配置环境变量。dev／preview 只用于开发检查；生产静态 Nginx 应提供正确 MIME、匿名静态资源 CORS 与缺资源 404。

## 公开配置与产物

| 构建参数 | 环境变量 | 约束 |
| --- | --- | --- |
| `--version` | `STORES_VERSION` | 默认 `1.0.0`；受限版本字符串，不能含 `..` |
| 可重复 `--allowed-host-origin` | `STORES_ALLOWED_HOST_ORIGINS`，逗号分隔 | 必须显式给出精确 HTTP(S) origin |
| `--api-base` | `STORES_API_BASE` | 默认 `/stores-api`；安全绝对路径或明确 HTTP(S) 地址，不允许用户信息、query、fragment、协议相对地址 |
| `--out-dir` | 无 | 默认本工程 `dist`；自定义输出只能位于系统临时目录 |

远端必须 HTTPS，HTTP 只允许字面 loopback 地址。正式 origin、客户端 UUID 和 API 地址须由管理员核验，本说明不提供虚构生产域名或可误用 UUID。产物公开配置只有 `{version, allowedHostOrigins, apiBase}`；Vite `envPrefix: []` 不自动注入 `VITE_*`。secret、数据库 URL、宿主 session、Bearer 和签名密钥不属于构建输入。

build 先执行 TypeScript 检查，覆盖前核验输出 owner，拒绝未知目录以及符号链接／junction。`base: './'` 使入口、ESM、CSS 和动态业务 chunk 使用版本内相对路径；输出包括：

- `index.html`、`assets/config.js`、内容摘要入口／React 业务 chunk／CSS、React 许可证。
- `asset-manifest.json`：实际入口和 chunk 关联。
- `build-info.json`：应用／版本／SDK／React／TypeScript／Vite／Node 版本、独立 lock SHA256、逐文件及汇总 source SHA256、入口和动态 chunk 清单。

这些文件为 Phase28 镜像化输入，不表示镜像已发布。根 `build:stores:frontend` 转发前端构建；已有 `build:stores` 仍构建 Rust API。

## SDK、路由与授权

`src/sdk.ts` 重导出登记 SDK 运行时和类型，不手写另一份 `MicroProps`；vendor 由根 `sdk:sync` 生成，`sdk:check` 检查漂移。薄 `main.ts` 先同步安装 bridge 并连接宿主，再加载 React chunk；生命周期壳复验取消 signal，unmount 幂等。

业务请求通过 SDK `requestWithContext` 发送当前应用 Bearer，省略 Cookie、限制响应容量并严格校验 DTO。opaque iframe 的 API 请求带字面 `Origin:null`，匿名静态 CORS 不替代身份授权。真实授权替换、到期、401、宿主取消或卸载立即隔离旧请求和服务器视图；纯主题／同授权展示更新保留合法表单和进行中的请求。403 冻结对应 resource/action 并清除该资源旧数据，纯主题不解除拒绝。

[bootstrap manifest](../deploy/bootstrap.manifest.json)登记 11 项节点：8 个 MENU、3 个 BUTTON；九个精确 resource/action capability、四种现有 scope 独立管理。根 MENU 空 permission 只提供入口，不赋予数据读权。

| 路由 | 页面与授权 |
| --- | --- |
| `/micro/stores` | 门店列表；实际读取仍需 `store:read` |
| `/micro/stores/store-create` | 创建门店；`store:create` MENU |
| `/micro/stores/stores/:id` | 资料详情；`store:read` MENU，修改另需 `store:update` |
| `/micro/stores/staff` | 员工关联；`store-staff:read` MENU，解除另需 `store-staff:unassign` |
| `/micro/stores/staff-assign` | 关联员工；`store-staff:assign` MENU |
| `/micro/stores/reports` | 指定店日报和服务端汇总；`store-daily-report:read` MENU |
| `/micro/stores/daily-report-create` | 新日报；`store-daily-report:create` MENU |
| `/micro/stores/daily-reports/:id` | 日报详情；独立详情 MENU 使用 `store-daily-report:read`，修改另需 `store-daily-report:update` |

创建与详情分开，未知路径显示未找到，BUTTON 不能替代 MENU。员工／日报页可直接输入门店 UUID，不暗中调用 `getStore`；另有 `store:read` 时才能使用辅助门店选择。日报列表必需 storeId，汇总可留空 storeId，由服务器汇总当前授权范围内全部日报。

每次业务 HTTP 默认有 15 秒绝对总期限，覆盖 fetch、成功正文流和错误正文检查。读取超时返回安全的 408／`REQUEST_TIMEOUT` 提示；已发送写入超时按结果未知保留原输入并要求显式读回，不自动重发。请求超时只结束本次请求，不撤销当前授权上下文或其他 resource/action。

## 成员与提交恢复

成员目录只接受 `store/create`、`store/update`、`store-staff/assign`。DTO 为 `{id, displayName, organizationId}`；员工提交将选中 `id` 显式映射为 `userId`。最终有效性由 Rust 写入前复验。SELF owner 分别为门店创建人、被关联员工、日报录入人；manager 和 `createdByUserId` 不扩大这些范围。详见[双消费者成员差异](../../../docs/stores-member-contract.md)。

日报初始营业日为 Asia/Shanghai 当天，用户可修改，提交明确发送已确认 `YYYY-MM-DD`。发送后固定原门店／日期／正文，跨上海零点不重算。金额始终为十进制字符串，单笔最多 16 位整数、2 位小数，返回固定两位；不经 `Number` 或浮点求和。汇总显示服务器 `COUNT/SUM(numeric)` 精确结果，可超过单笔上限，不以分页相加代替。

唯一性 409（同组织门店、重复员工关联、同店同日报）与版本冲突分开。编辑冲突保留输入，先读最新记录，再明确采纳最新版本。网络、协议、5xx 或提交取消可能导致结果未知：保留原输入，按原组织／门店／员工／日期／version 显式读回，不自动重发 POST／PATCH／DELETE，不伪造工单 receipt 或 `Idempotency-Key`。只写无读仍可写；未知后缺少对应 read 时提示当前无法核对并保留原输入。

控制器拥有当前表单，事件只传 `formId`；提交重新检查 identity／route／context epoch，复制当前草稿后再取请求快照。真实授权替换使旧处理器失效；同身份合法文字可保留，已有记录版本必须重读。草稿只在当前存活 iframe 内存按 app／tenant／organization／user 分区，不读写宿主 storage，不保存 token。宿主两页共享登录和当前组织；刷新、关闭或销毁 iframe 可能丢失草稿，无持久保存保证。

## 验证边界

本目录测试可用受控 HTTP 检查 client、表单隔离、取消和构建。最终普通用户验收由根 `tests/phase27-stores-browser.mjs` 使用真实平台、门店与工单 Rust 服务、三个隔离 PostgreSQL 库、生产宿主、Nginx 和正常验证 TLS；成员及业务响应不能用 mock 替代。实际命令、source hash、数量、原始失败和清理由 Phase27 证据单独记录。
