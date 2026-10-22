# 本机独立网关接入

业务 API 和静态版本路由由此部署目录管理。平台源码不需要认识 stores、业务端口或版本目录。适用于 Windows + WSL Docker 的本机联调，正式环境沿用 HTTPS 网关部署文档。

公开入口保持 http://127.0.0.1:3000：

- 门店版本资源 `/sub-apps/stores/versions/<version>/…` 由独立只读版本目录提供。
- `/stores-api` 原样转发到本机 3031；门店 API 处理 CORS 和 OPTIONS，公共网关阻止私有指标路径。
- 其他请求转发到本机 3002 的平台 Vite，包含平台 API 和 WebSocket。

## 启动

1. 按门店后端文档启动 API。本次联调已有 `artifacts/start-local-api.ps1`，密钥只从用户私有配置读取。
2. 在平台 frontend 目录启动原始 Vite，仅通过 CLI 改端口：

   ```powershell
   npm run dev -- --host 127.0.0.1 --port 3002
   ```

3. 将完整 dist 发布到独立目录 `saas-stores/artifacts/micro-apps/stores/versions/1.0.1/`，保留不可变历史版本。已有产物可迁移目录，无需重建或改登记 URL。不要覆盖同版本文件。
4. 在本目录运行 `./start-gateway.ps1`。使用已安装的 WSL Docker、官方 Nginx 镜像，保持终端运行。`-ReleaseRoot` 可指定其他绝对版本存储根；目录结构须为 `stores/versions/<version>/…`。
5. 打开 http://127.0.0.1:3000/micro/stores。应用登记、菜单授权、OAuth 关联仍通过平台管理界面维护。

先启动平台后端，再启动门店 API、平台 Vite 与独立网关。网关和业务 API 仅监听 loopback，不需要修改浏览器白名单、关闭 iframe 隔离或放宽来源检查。

网关健康检查：`/stores-api/health/ready`。无令牌业务请求应返回 401；缺失静态资源和私有指标应返回 404。`Origin: null` 的合法预检应返回 204，非法 origin 应返回 403。

服务启动后执行 `node verify-gateway.mjs`，检查公开入口、CORS、静态资源、私有路径，以及直接访问平台 3002 时不提供门店 API 和版本文件。该检查不需要令牌、不创建业务数据。再通过浏览器验证平台令牌注入、iframe 挂载和已授权业务查询。

停止网关：`wsl --exec docker stop codex-stores-gateway-local`。重启已有容器：`wsl --exec docker start -a codex-stores-gateway-local`。修改网关配置后先运行 `docker exec … nginx -t` 再 reload；更换挂载路径或镜像时需重新创建容器，不改平台源码。
