# 门店管理发布日志

## 2026-10-24 · 平台 STORE 组织与营业业务分工

- 门店身份改为平台 STORE 组织，名称、编码、层级、地址和成员由平台统一维护；营业档案保留技术 ID 与 tenant / organization 唯一关系。
- 新建接口为已有 STORE 完善营业资料，更新只接受营业字段；平台名称作为实时只读别名返回，搜索也使用实时名称 / 编码。
- 员工页改为平台直接成员只读视图，旧分配 / 解除路由返回 410。新清单为九菜单、七权限，增加七个 STORE 组织查询用途，成员用途改为营业联系人及 staff/read。
- 所有列表、详情、日报、汇总与写入增加新鲜 STORE 组织限制，保留原 SELF owner 条件；非 STORE、禁用、到期或无权历史映射排除但不删除。
- 新 0002 迁移先校验 v1，再冻结旧名称实际更新和员工关联表写入，建立 v2 结构校验；0001 与 v1 fingerprint 不变。
- 本次库测试 61 项、真实 PostgreSQL 合同 4 项及严格 clippy 已通过；真实合同覆盖 v1 升级保留、平台 STORE 过滤、SELF owner、营业联系人清空、只读成员、无连接跨 HTTP，以及有效组织投影在实际行锁等待中到期后拒绝更新并回滚，隔离测试库均已清理。静态新候选和平台激活需按本次源码重新验证，旧候选保留原字节。

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
