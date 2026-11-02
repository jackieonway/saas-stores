# 门店发布与运维

后端和前端分别构建、分别发布。前端是 qiankun HTML / TRUSTED 静态包，后端使用专属 PostgreSQL 数据库。平台须提供 STORE 类型及受控组织 / 成员接口，统一维护组织身份和成员；门店应用只维护营业业务。平台、门店 API 和静态服务通过公开 HTTPS 网关组合，机密客户端凭据只注入 API 进程。

## 构建镜像

从本仓库根执行，版本与域名替换为实际值：

```sh
docker build -t saas-stores-api:local backend
docker build --build-arg APP_CODE=stores --build-arg APP_VERSION=2.0.0-local --build-arg HOST_ORIGIN=https://platform.example.com --build-arg API_BASE=/stores-api -t saas-stores-web:local frontend
```

`frontend` 镜像构建要求准确的 HTTPS `HOST_ORIGIN`；`API_BASE` 是根路径或 HTTPS 地址。镜像驱动不接受回环 HTTP，HTTP 本机演练使用 [本机网关](../../frontend/deploy/local/README.md)和普通前端构建。前端构建参数全为公开值，修改来源或 API 地址需要重新构建。

API 镜像使用 Rust `1.97.0` 编译，运行镜像为 Debian；静态镜像使用 Node `24.22.1`、npm `11.16.0` 构建，最终运行非 root Nginx。正式发布记录源码修订、输入摘要和不可变镜像摘要，再将审阅后的镜像放入自己的 registry。CI 会构建镜像，不会自动发布或激活部署。

## Compose 环境与来源覆盖

将 `backend/compose.env.example` 复制到仓库外的私有配置，例如 `/srv/saas/private/stores.env`。填写 `POSTGRES_IMAGE`、`STORES_API_IMAGE`、`STORES_FRONTEND_IMAGE`、`STORES_POSTGRES_PASSWORD`、`STORES_DATABASE_URL`、`STORES_ALLOWED_ORIGINS` 和平台三项配置。正式镜像建议用 `registry/name@sha256:...`。

数据库 URL 指向 `stores-postgres:5432/stores`，账号 `stores`，密码按 URI 规则编码。容器内的 `127.0.0.1` 代表自身，不能用它访问宿主平台。`PLATFORM_API_ORIGIN` 必须是 API 可达且证书可信的平台私有 HTTPS origin，同时提供准确 `POST /api/apps/stores/authorize` 和 `POST /internal/platform/apps/stores/{members,organizations}[/id]`；仅路由公开应用路径的浏览器网关不能完成此后端协议。私有网关只放行这些必要方法和路径，拒绝凭据 query 参数。

`compose.yml` 从私有 env 文件读取必填 `STORES_ALLOWED_ORIGINS`。qiankun 从平台页面发出的跨域请求，以及携带 Origin 的同源写入，均需放行实际平台来源。例如：

```dotenv
STORES_ALLOWED_ORIGINS=https://platform.example.com
```

Compose 与 API 配置都拒绝缺失和空值，API 还拒绝 `null`、通配符、非准确 origin 及生产 HTTP。生产仅使用 `combined`。接受外部 token 时还须配置准确 `PLATFORM_IDENTITY_ISSUER` 和 `PLATFORM_RESOURCE_SCOPE_BINDINGS`，scope 映射与平台、身份服务使用同一审核配置来源，漂移或缺项失败关闭。需要其他超时预算或额外 CA 时，可在私有 override 的 `environment` 中显式透传。额外 CA 文件还需只读挂载到容器内，并将 `PLATFORM_CA_CERT_FILE` 指向容器路径。所有选项见 [后端配置](../README.md)。

`EDGE_NETWORK_NAME` 指定已有的受控 Docker 私网，默认示例名 `platform-edge`。Compose 会创建自己的内部数据库网络，但不会创建外部 edge 网络或启动平台。可选 `STORES_METRICS_TOKEN` 专用于私网采集。

## 启动服务

首次启动 API 前，环境 owner 须使用同版本独立 `stores-migrate` 和 owner 连接在专属空库建立当前 `0001` 基线，并完成 owner/runtime 权限交接；运行账号不获得 DDL 或迁移历史写权限。迁移作业必须显式单独执行，不依赖 API 或 Compose 自动建表。NOLOGIN owner 的本机委托连接方式见[独立迁移命令](../README.md)。以下命令用于启动已经完成基线的服务。

从 `backend` 执行：

```sh
docker compose --env-file /srv/saas/private/stores.env -f compose.yml config --quiet
docker compose --env-file /srv/saas/private/stores.env -f compose.yml up -d
docker compose --env-file /srv/saas/private/stores.env -f compose.yml ps
docker compose --env-file /srv/saas/private/stores.env -f compose.yml logs --tail 100 stores-api
```

`stores-postgres` 通过健康检查后启动 `stores-api`。API 每次启动只读核验现有当前迁移 checksum 与 schema fingerprint；没有基线或结构漂移时拒绝，不能为启动而给运行账号 DDL 权限。前端默认映射宿主回环 `3014:8080`，API 默认回环 `3031:3031`；`API_PORT`、`FRONTEND_PORT` 可在 env 文件调整。共享 edge 网络允许网关访问容器服务名。

当前仅支持 canonical `0001` 全新安装，不使用旧 `0002` 升级、历史 checksum 接管或 UUID 搬迁。已有业务数据或旧环境的处置必须单独核对目标与授权，不通过 API 启动修改或重建；平台明确创建 / 维护 STORE 后再完善营业资料。

API 结构校验失败时停止发布，定位历史或迁移差异；不要删除数据库卷或手改 checksum 解决。静态容器 `/healthz` 只证明静态服务工作；业务 readiness 为 `/stores-api/health/ready`。

## 网关与平台登记

推荐由平台公开网关提供两条路由：

| 公开路径 | 上游与要求 |
| --- | --- |
| `/stores-api`、`/stores-api/...` | `stores-api:3031`，保留业务前缀及合法 Origin / OPTIONS |
| `/sub-apps/stores/versions/<version>/...` | 对应不可变静态版本，路由到静态容器时去掉版本前缀 |

若网关配置有服务白名单，显式加入实际的 `stores-api`、`stores-web` 服务。静态文件通过相对地址加载，正确返回 HTML / JavaScript / CSS MIME，开放宿主读取 CORS。HTML 使用重新校验，版本资源可长期缓存，缺失资源返回 `404`，不能回退到平台 SPA HTML。

宿主需要启用 qiankun 的可信 HTML 加载策略，并使其 CSP 允许实际静态与 API 来源；使用宿主策略生成器时核对 `trustedHtmlEval` 与应用来源白名单。子应用响应头不能替宿主页面放开执行策略。业务私有 `/stores-api/internal`、`/stores-api/internal/...` 及指标兼容路径应在公开网关返回 `404`。

平台应用 code 固定 `stores`、基址 `/micro/stores`，登记 `entryType=HTML`、`runtimeMode=TRUSTED`，入口指向真实新版本的 `index.html`。登记模板在 [bootstrap.manifest.json](../manifests/bootstrap.manifest.json)。当前 bootstrap 是九个菜单、七种业务权限；新策略登记七个 STORE organizationLookups 和 store/create、store/update、store-staff/read 三个 memberLookups，不再给新角色授 assign/unassign。修改文件不会自动迁移平台数据库中的旧应用、版本或激活记录，必须逐项核对。

外部静态域名也可承载完整 `dist`。这时需要静态 CORS、宿主 CSP 及明确的 API 网关，API 白名单仍使用平台页面 origin。

## 版本切换与验收

从前端目录运行 `npm run verify:release -- dist --version <实际版本>`。校验器检查 `build-info.json` 的 `application=stores`、`runtime=qiankun`、`sdkVersion=2.0.0`、契约 `2.0` 和实际版本，再核对 HTML、清单、CSS、许可证及 UMD 的四个生命周期导出。HTML 属性与实体通过 parse5 解析；`bootstrap.sdkContract` 必须冻结、只读，并执行 SDK 2 接受、SDK 1 / 未知版本拒绝的协商检查。旧 iframe / module 入口、metadata SDK2 配旧 SDK 或没有 SDK 的生命周期空壳不能通过。该检查不替代签名或来源信任。CI 在构建后运行相同校验并保存完整 `stores-qiankun-static` 产物。

仓库 `artifacts/micro-apps/stores/versions/1.0.0` 与 `1.0.1` 是历史 iframe 发布，不能用于验收当前 qiankun。生成新版本目录或新镜像，登记新入口并切换激活版本；不可覆盖已使用的同版本字节。单个静态容器只承载一个 dist，不能让所有历史 URL 都指向不断替换的新镜像。

上线前验证公开 HTTPS、证书、静态 MIME / CORS / 缓存 / 404、业务健康、无认证拒绝、普通用户合法与越权操作、非 STORE / 禁用组织历史排除、成员只读、SELF 本人子组织业务行与非本人拒绝、失效营业联系人显式清空，再通过浏览器检查 qiankun 挂载、组织切换、撤权、深路由和卸载后重入。数据写入按 [API 恢复规则](../API.md)处理未知结果。

## 升级与恢复

升级前备份业务数据库并记录平台登记、应用版本、镜像摘要、私有配置版本和未确认写入。前端回退需要保留旧版本服务或独立版本存储，后端回退须确认旧程序兼容已经执行的数据库迁移。

数据库恢复先进入隔离新库校验迁移与业务数据，再切换连接。平台身份库和门店业务库分别恢复，恢复时间点和跨库成员状态需要重新核对。本仓库没有自动完成平台及多个业务库协调恢复的命令。
