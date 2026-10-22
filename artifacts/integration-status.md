# 门店本机平台接入

已完成浏览器接入与真实业务联调。入口：http://127.0.0.1:3000/micro/stores 。

## 当前部署

- 应用：门店管理 / stores，ID 2ffcdfa4-5e2b-4856-bba1-d44f6823749a，HTML / 隔离 IFRAME。
- development 当前激活版本：1.0.1，预检 5/5 通过。
- 静态文件：saas-stores/artifacts/micro-apps/stores/versions/1.0.1，由独立网关只读提供；历史 1.0.0 同样保留在该目录。
- 登记入口：/sub-apps/stores/versions/1.0.1/index.html。
- 1.0.0 保留为历史版本；其绝对 HTTP API 配置会被生产版前端客户端拒绝。
- 1.0.1 使用 /stores-api。曾为联调加入的平台 Vite 门店代理、OPTIONS 条件、静态目录默认值修改均已撤回。
- 独立 Nginx 网关 codex-stores-gateway-local 监听 127.0.0.1:3000；门店 API 转发到 3031，其余流量转发给 3002 的平台 Vite。平台仅用 CLI 覆盖开发端口，不修改源码。
- 配置、启动和验证位于 frontend/deploy/local；正式 HTTPS 部署继续参阅 backend/docs/deployment.md。这是 URL 静态发布加本机 Docker 后端，尚未验证平台受管镜像部署或生产网关。

## 后端与密钥

- WSL Docker 容器 codex-stores-api-local，镜像 codex-stores-api:local。
- 专用 PostgreSQL 数据库 stores_codex_local，迁移已完成。
- API 只监听本机 127.0.0.1:3031；现有 Windows/WSL 端口转发可用，不需要额外 Node 桥。
- 专用 OAuth 客户端 stores-client-vxvzqi：机密、client_credentials、profile、300 秒令牌，仅关联 stores。
- 创建接口漏存 applicationId，已通过浏览器编辑补存并确认关联。
- 表单强制要求回调，填本机门店入口；未启用授权码模式。
- 用户已确认创建和保存密钥。密钥与数据库 URL 只保存在 %USERPROFILE%\.codex\local-secrets\saas-stores\runtime.json 及同目录生成的 docker.env，不得提交或输出。
- 启动：运行本目录 start-local-api.ps1，保留终端以保持 WSL 会话；运行中无需重复启动。
- 停止：wsl --exec docker stop codex-stores-api-local。
- 健康检查：http://127.0.0.1:3000/stores-api/health/live 和 /stores-api/health/ready。

## 权限配置

- 平台管理租户、总公司已授权应用及全部 11 项菜单/按钮。
- 当前超级管理员的 9 项数据授权均为 ORG，硬边界仅总公司，不包含下级组织。
- 平台 .env 的 DATA_GRANT_POLICY 已加入门店能力与精确的总公司管理范围，原配置备份位于用户私有配置目录。
- 已验证无令牌请求返回 401；未验证普通用户、跨租户和跨组织账号的完整权限矩阵。

## 浏览器真实业务验证

- 平台令牌注入和 iframe 挂载成功，页面显示运行中。
- 门店创建、列表、详情、更新成功，版本 v1 → v2。
- 受控成员目录查询成功；仅将当前管理员关联到测试门店，并通过员工查询读回。
- 创建 2026-10-22 日报 123.45 元，查询、详情、备注更新成功，版本 v1 → v2。
- 服务端汇总返回 1 份日报、123.45 元。
- 独立网关替换后，浏览器重新验证了令牌注入、运行中、原门店列表、日报 v2 和 123.45 元汇总；未新增业务记录。
- 未解除员工关联，未删除测试门店或日报，保留供人工查看。
- 观察到运行时上下文同步后查询结果可能清空，需要重新点击查询；本次未修改上下文同步机制。

测试记录：

- 门店：本机接入验证门店，66f905ce-9210-4679-8641-b8b7964c6f93。
- 日报：8b28f45f-4370-4d39-ae98-166b9fe15d1a。
- 当前管理员员工关联：5a0ef34b-4668-4a23-9390-9d04c1e2708e。

## 测试与检查

- 前端测试 115 通过；1.0.1 公开运行配置重新构建成功。
- 后端 Windows 编译成功，库测试 46 通过、10 项 HTTP loopback 测试失败。
- 相同后端 Linux Docker 构建环境库测试 56 全通过。
- 平台健康检查、门店 readiness 成功；独立网关将 OPTIONS 交给门店 API，返回 204。
- 独立网关验证 11/11 通过：平台与业务健康、无认证 401、合法预检 204、非法来源 403、私有路径 404、静态 MIME/CORS/缓存与缺资源 404；直连平台 3002 不再提供门店版本或 JSON API。
- 切换时曾出现 iframe 握手超时；独立的无业务令牌握手检查通过，重试后平台挂载和查询通过。没有修改平台握手超时或关闭 iframe 隔离。

## WebSocket 清理

- 聊天服务 3007/3008 已停止。
- 聊天应用及级联菜单/授权已按确认永久删除。
- 孤立客户端 websocket-psd6xx 及其令牌/设备码记录已按本次确认永久删除。
- 旧静态入口返回 404；历史静态文件移至 websocket 示例 artifacts/retired-release-1.0.0。

截图：stores-running.jpg、reports-verified.jpg、staff-verified.jpg、oauth-configured.jpg。
独立网关截图：stores-independent-gateway.jpg、reports-independent-gateway.jpg。
