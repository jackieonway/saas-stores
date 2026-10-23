# 门店管理发布日志

## 2026-10-23 · Node.js 安全更新

- 前端包 `stores-frontend`：`1.0.0` → `1.0.1`。
- Node.js 基线由 `24.18.0` 升级至官方安全修复版本 `24.22.1`。前端 `.nvmrc`、`.node-version`、package/lock engines、Docker 构建镜像、CI 发布校验和开发/部署文档统一使用 `24.22.1`；默认构建版本取自 `package.json`。
- 修复官方 2026 年 10 月安全公告中影响 Node.js 24.x 的 8 项漏洞：CVE-2026-105482、CVE-2026-105483、CVE-2026-106069、CVE-2026-111497、CVE-2026-105481、CVE-2026-106072、CVE-2026-106074、CVE-2026-106073。
- Rust 服务版本、SDK 契约与既有静态发布产物保持原版本和内容；新产物需使用独立版本 URL 构建、验证后再部署。
- 新建本机候选 `2.0.3-qiankun`，使用 Node `24.22.1`、准确宿主 `http://127.0.0.1:3000` 和相对 API `/stores-api`；完整发布门禁和当前源码 31 项输入摘要校验通过。旧 `2.0.2-qiankun` 保留原字节，由于仍记录旧 Node 和已变化的来源输入，不作为当前待激活候选。候选尚未变更真实平台激活记录或运行容器。

依据：[Node.js 官方安全公告](https://nodejs.org/en/blog/vulnerability/october-2026-security-releases)、[Node.js v24.22.1 发布说明](https://nodejs.org/en/blog/release/v24.22.1)。

### 验证

- Node.js `24.22.1` + 临时安装的 npm `11.16.0`：前端 `npm test` 共 115 项通过，无失败或跳过，包含空缓存 `npm ci`、TypeScript 检查、真实 UMD 构建与静态交付验证。
- 前端使用默认包版本构建到独立临时目录，再执行 `node deploy/verify-release.mjs <输出目录> --version 1.0.1`，通过 qiankun / SDK / 资源门禁。
- `node .github/scripts/check-inputs.mjs` 与 `node --test .github/tests/repository-structure.mjs` 通过（结构测试 2 项）；package/lock 版本与 engines 一致。
- 本次未运行 Docker 镜像构建、真实浏览器及 PostgreSQL 集成验收；未发布或激活部署。
