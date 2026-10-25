# 门店微应用

`saas-stores` 是平台的 qiankun 门店业务子应用。门店是平台的 STORE 组织：平台统一维护组织身份、层级、地址与成员，门店应用维护营业状态、营业联系信息、营业联系人、日报和统计。前端使用 React / TypeScript / Vite，后端是独立 Rust HTTP 服务，业务数据保存在专属 PostgreSQL 数据库。

## 架构与职责

宿主在 `/micro/stores` 以 HTML / TRUSTED 模式挂载应用。静态入口是 classic UMD，导出 bootstrap、mount、update、unmount；SDK 3.0.0 使用 MicroProps 2.0，经 props.platform 传递身份、组织、路由、主题和权限。源码不再提供 iframe 桥接入口。React / ReactDOM 随应用独立打包，样式限定在 `[data-stores-root]`。

浏览器使用 stores 应用 Bearer 请求 `/stores-api`。API 每次向平台重新获取身份、当前动作授权和有效 STORE 投影，将租户、组织硬边界、数据范围及业务 owner 写入 SQL 条件。前端按钮权限只控制界面；服务端独立执行业务授权。数据库和机密客户端凭据只注入后端。

一个 STORE 对应一份营业档案，唯一键为 tenantId / organizationId。营业档案技术 ID 和历史日报引用保持稳定。名称、编码、层级和地址只读使用平台实时投影；成员页只读显示平台直接成员。旧成员分配 / 解除接口返回 410，旧映射、旧名称和关联历史保留，不自动改类型或删除。

## 文档

| 工作 | 文档 |
| --- | --- |
| 从源码登记应用并启动联调 | [开发指南](backend/docs/development.md) |
| 生命周期、前端业务和构建 | [前端说明](frontend/README.md) |
| API 配置、数据库和平台协议 | [后端说明](backend/README.md) |
| 组织、营业资料、成员、日报和汇总接口 | [API 契约](backend/API.md) |
| 平台组织 / 成员权威、SELF 和事务时效 | [组织与成员契约](backend/docs/member-contract.md) |
| 镜像、HTTPS 网关、迁移和版本升级 | [部署指南](backend/docs/deployment.md) |
| 本机独立网关与不可变静态版本 | [本机网关](frontend/deploy/local/README.md) |
| 当前变更与已有发布历史 | [发布记录](RELEASE.md) |

## 目录

| 目录 | 内容 |
| --- | --- |
| frontend/src、frontend/tests | 页面、API 客户端、控制器、生命周期和回归 |
| frontend/deploy | 构建、静态发布、门禁与本机网关 |
| backend/src | HTTP、授权、实时组织 / 成员投影、营业业务和指标 |
| backend/migrations | SQLx 迁移、历史结构及 v2 校验 |
| backend/manifests | 当前应用、九个菜单、七个业务权限与授权模板 |
| .github | 本仓库检查与镜像构建 |
| artifacts | 公开辅助材料及不可变历史发布 |

平台策略样例在 backend/policy.example.json。它包含七个 capability、七个 STORE organizationLookups 和三个 memberLookups；文件修改不会自动改变平台数据库，正式登记必须经平台 API 或管理界面完成。

## 构建与发布

前端固定 Node 24.22.1、npm 11.16.0、React / ReactDOM 19.3.0、TypeScript 5.9.3、Vite 8.3.4，后端使用 Rust 1.97.0。先核对实际 PATH 命中的版本，再按开发指南执行检查。

新构建必须使用新版本 URL。发布门禁核对 HTML、资源摘要、实际 UMD 四个生命周期以及冻结的 bootstrap.sdkContract 版本协商；SDK3 元数据不能掩盖旧 SDK 或空生命周期壳。安装使用排他目录和文件复制，最后写 release-complete.json，已有版本即使为空也不能覆盖，未完成版本不能启动。

artifacts 中 1.0.0、1.0.1 为旧 iframe 发布；后续 qiankun 候选也按历史字节保留。当前源码改变后需重新构建新版本，并核对来源输入摘要；过去的测试、候选版本或截图不能证明当前平台激活状态。记录实际登记的 HTML / TRUSTED 入口与激活版本，再做浏览器挂载和业务验收。

依赖记录位于 backend/dependencies.lock.json，SDK 发布来源与文件摘要位于 frontend/vendor/micro-sdk/SOURCE.json。升级时同步审阅源字节、类型、契约和依赖记录。

CI 验证本仓库代码与交付包；真实平台普通用户授权、PostgreSQL 升级、浏览器组织切换和部署激活各有独立验收，不能由静态构建结果代替。
