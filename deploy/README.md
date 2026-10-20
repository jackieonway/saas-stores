# 门店应用平台接入

门店使用既有 Rust [平台管理 CLI](../../../tools/platform-admin/README.md)，稳定应用 code 为 `stores`，宿主路径 `/micro/stores`，业务 API 为 `/stores-api`。本目录只包含可公开审阅的 bootstrap、授权意图及 schema；实际 UUID、clientId、secret 和管理员 Bearer 由部署环境提供。

[bootstrap.manifest.json](bootstrap.manifest.json) 及 [bootstrap.schema.json](bootstrap.schema.json) 与 CLI 编译期 stores 模板逐字节一致。应用固定 HTML + IFRAME、autoCreateMenu=false，初始占位入口 `/sub-apps/stores/unreleased/index.html` 应返回 404；真实内容通过独立前端构建产物和现有平台发布流程接入。Bootstrap 仅管理应用、客户端关联及菜单；Phase28 镜像和固定公开版本路由由独立部署系统处理。

推荐部署路径见[业务镜像交付说明](../../../deploy/microapps/README.md)、[业务 Compose override](../../../deploy/compose.business.yml)、[公开变量模板](../../../deploy/compose.business.env.example)及[版本路由示例](../../../deploy/microapps/gateway.example.json)。门店 Rust API 与前端 v1/v2 静态镜像独立，业务库和机密客户端均使用门店专属配置。只有入口 Nginx 发布 HTTPS，纯镜像无需平台静态产物卷。新路径不扩展本目录的 bootstrap/grant 权限，不自动授权用户，也不表示外部生产发行已经执行。

本阶段受管 API 验收采用冻结 Linux 源码独立 locked Cargo 编译、ELF/source/inputs/platform/SHA-256 校验后 runtime 装箱，报告标为 `verified-prebuilt-ELF`；默认 [Rust Dockerfile](../backend/Dockerfile)的源码 builder 未实际运行时不能记为 PASS。两版前端仍真实按门店自己的 lockfile 从源码 Docker build。registry push/pull、健康拒绝、普通用户 SDK/Rust 授权和浏览器结果分别以当次 Phase28 报告为准，此处不预记通过。

部署方先固定 digest pull/start 并验证 Docker health，然后由 Rust CLI 在所有环境共用的权威持久 inventory 中执行 `artifact-bind --apply` 与 `artifact-verify`，再保留 v1/v2 路由地推进 Nginx 配置，最后用带 artifact/inventory guard 的 URL register/preflight/activate。每项门店发布命令显式使用 `--app-code stores`；release JSON 继续只有既有 environment/version/entryType/entryUrl/healthUrl/resources。CLI/CI journal 负责不可变来源/digest 绑定，平台仍只保存并预检公开 URL 定义。外部静态 origin 必须设置明确绝对业务 HTTPS API base 或自有受控代理，宿主 origin 白名单不改为容器名；opaque iframe 的业务 `Origin:null` 与 Rust 授权/CORS 校验保持。

管理员先在平台 UI 创建名称精确为 `stores-client` 的 CONFIDENTIAL 客户端，仅 client_credentials、openid、空 redirectUris，accessTokenValidity 和 refreshTokenValidity 均为 300 秒。一次性 secret 只保存到 stores Rust 服务 secret store，平台生成的实际 clientId 通过 `PLATFORM_CLIENT_ID` 传给 CLI。CLI 只关联该已创建客户端，不自动创建、旋转、重置或输出 secret；已绑定其它应用或同名/ID冲突时停止。

```text
saas-platform-admin plan --app-code stores
saas-platform-admin inspect --app-code stores
saas-platform-admin bootstrap --app-code stores --apply
saas-platform-admin inspect --app-code stores
```

远程命令默认使用正常验证的 HTTPS。配置 `PLATFORM_API_ORIGIN`、当前管理员 `PLATFORM_ADMIN_BEARER`，必要时提供公开 `PLATFORM_CA_FILE`；不把凭据放入 argv、清单或前端。开发 HTTP 仅在命令显式指定 `--environment development --allow-loopback-http` 且目标为字面回环时允许。

| key | type | path | permission | visible |
| --- | --- | --- | --- | --- |
| root | MENU | /micro/stores | 空 | true |
| store:read | MENU | /micro/stores/stores/:id | store:read | false |
| store:create | MENU | /micro/stores/store-create | store:create | true |
| store:update | BUTTON | 空 | store:update | false |
| store-staff:read | MENU | /micro/stores/staff | store-staff:read | true |
| store-staff:assign | MENU | /micro/stores/staff-assign | store-staff:assign | true |
| store-staff:unassign | BUTTON | 空 | store-staff:unassign | false |
| store-daily-report:read | MENU | /micro/stores/reports | store-daily-report:read | true |
| store-daily-report:detail | MENU | /micro/stores/daily-reports/:id | store-daily-report:read | false |
| store-daily-report:create | MENU | /micro/stores/daily-report-create | store-daily-report:create | true |
| store-daily-report:update | BUTTON | 空 | store-daily-report:update | false |

共十一项，sortOrder 为表格顺序 0–10；root 的 parentKey=null，其余 parentKey=root。MENU 为 MICRO、BUTTON 为 BLANK，linkUrl 均为空。根菜单没有读权限；创建、详情及 staff/report 独立路由不隐含其它资源动作。两个日报读 MENU 共享一项读 permission，授予该 permission 时 CLI 选择两条菜单。菜单 status 声明 ENABLED 是预期可用性；当前平台菜单 API 不提供 status 字段，CLI 不提交或伪造该字段，实际可用性须由正式 grant/业务授权链验证。

资源通过上述 permission 与现有 `DATA_GRANT_POLICY` capabilities 登记。管理员合并后端 [policy.example.json](../backend/policy.example.json)：九项 capabilities 为 store/read/create/update、store-staff/read/assign/unassign、store-daily-report/read/create/update；scopes 仅 SELF、ORG、ORG_SUBTREE、TENANT，并裁剪到批准范围。memberLookups 精确为 store/create、store/update、store-staff/assign 三对。替换实际应用 UUID并保留其它应用及已批准 managementScopes，示例不授予新的管理范围。没有资源登记 API、STORE scope、store/delete、report/delete 或 summary 动作。

复制 [grants.manifest.example.json](grants.manifest.example.json)，以 [grants.schema.json](grants.schema.json) 校验后填入实际应用/十一菜单 UUID 与批准 targets；占位符故意无效，targets 默认为空，不代表给任何用户授权。

```text
saas-platform-admin grants-plan --manifest path/to/approved-stores-grants.json
saas-platform-admin grants-apply --app-code stores --manifest path/to/approved-stores-grants.json --apply
```

grant 命令以清单 applicationCode 为来源；显式 app-code 或 app-id 与清单冲突时在联网前拒绝。新权限和数据 grant 仅接受门店九项动作；功能菜单与数据范围独立。tenant/organization/role/user/membership 全部使用真实身份 API 返回的 UUID，CLI 复核有效直接成员和归属；角色集合读取后取并集，正式数据写入维持 GET → preview → PATCH 同一 opaque version，remove 始终为空。未列出的历史 grant 行完整保留，scope 冲突停止，不自动清洗、删除或扩权。

授权需要独立 data-grant read/write、实际平台根上下文及明确管理策略；普通业务用户不需要这些管理权限。管理 API 之间没有统一事务，角色 PUT 没有 CAS，须在暂停并发管理的窗口执行。失败或结果未知时保留原意图，先查询精确目标、角色、grant 与审计再审阅下一次操作，不自动重试或删除回滚。最终授权效果必须用真实普通用户下一次 stores 受保护请求核验。
