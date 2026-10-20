# 简单门店业务

独立源码发行目录的导出、干净构建、部署与兼容矩阵见[独立业务交付](../../docs/independent-business-delivery.md)。当前最终验收以[Phase34报告](../../docs/validation/phase-34.md)为准，下文保留各阶段的历史证据边界。

Phase32 增加默认禁用的私有指标端点：显式设置独立 `STORES_METRICS_TOKEN` 后，从私有网络采集 `/stores-api/internal/metrics`。源码镜像从仓库根构建：`docker build -f business-apps/stores/backend/Dockerfile -t stores-api:candidate .`。固定指标协议不共享业务 SQL 或身份模型，指标含义及容量/告警限制见[监控说明](../../deploy/monitoring/README.md)。

`stores` 是工单之外的第二个独立业务消费者。当前目录包含 [React／TypeScript／Vite 前端](frontend/README.md)、[Rust API](backend/README.md)、专属 PostgreSQL 业务库，以及[公开应用／菜单登记 manifest](deploy/bootstrap.manifest.json)和[受控 grant 示例](deploy/grants.manifest.example.json)。[HTTP 契约](backend/API.md)以当前源码为准；Phase27 正式r2的Chromium／Chrome／Edge各21个必需浏览器组通过、零失败／跳过，覆盖真实平台与两业务Rust、三库、独立空缓存构建及普通用户业务操作。阶段状态及完整门禁结果见[任务台账](../../docs/architecture-optimization-ledger.md#phase-27)与[Phase27验证记录](../../docs/validation/phase-27.md)，双消费者的实际成员请求和差异见[成员契约](../../docs/stores-member-contract.md)。

后端只持有门店业务数据，以平台 HTTP 内省、实时 context 和受控成员接口取得身份与授权。平台、工单、门店使用三个独立数据库和三个专属账号，可位于同一 PostgreSQL 实例；门店账号不能查询平台身份表或工单表，也不依赖它们的 Rust 实现 crate。

| 资源 | 动作 | SELF 的 owner |
| --- | --- | --- |
| `store` | `read`、`create`、`update` | 创建门店的当前用户；负责人可变但 owner 不变 |
| `store-staff` | `read`、`assign`、`unassign` | 经平台验证后关联的员工；`createdByUserId` 另记操作者 |
| `store-daily-report` | `read`、`create`、`update` | 录入日报的当前用户；汇总使用同一 `read` 授权 |

九个精确 capability、四种现有 scope、三个成员用途分别登记；没有 STORE scope、门店删除、日报删除或独立汇总动作。一店映射一个组织节点是本示例的业务约束，不改变平台的通用组织模型。资料、员工关联和日报的 list／count／detail／write／summary 都在参数化 SQL 中交集 tenant、组织硬上限和对应 grant，不能以负责人、员工关系或另一资源的读权限替代授权。

金额以非负十进制字符串输入，业务库使用 `NUMERIC(18,2)`，输出固定两位小数；汇总由 PostgreSQL 精确求和，可超过单笔上限，不以分页相加代替。营业日是严格有效的 `YYYY-MM-DD`，后端省略时按 Asia/Shanghai 的 UTC+08:00 当天计算，允许有效历史或未来日期；前端展示可编辑日期，明确提交用户确认值，并冻结发送时的原门店／日期／正文。

同组织门店、有效员工关联和同店同日报分别有唯一性约束，重复返回409。版本409保留输入，重读并明确采纳新版本。写入没有工单创建 receipt 或 Idempotency-Key 协议，网络／协议／5xx或提交取消后未知时按原组织／门店／员工／日期／version显式读回，不自动重发。只写无读仍可写；未知后缺少read权限时保留原输入并说明当前无法核对。

前端每次HTTP默认15秒绝对总期限，覆盖fetch、成功与错误正文读取；读取超时给安全408提示，已发送写入超时按结果未知保留原输入并要求读回。超时只结束本次请求，不撤销当前context或其他动作，不自动重发写入。

收到403响应头即停用精确resource/action，取消同用途并行请求并隐藏相关服务器视图与目录，不等待错误正文；同用途迟到200不能恢复旧视图，纯主题不解除拒绝，其他动作不因此失权。401使当前context完整失效。路由返回或同身份续期恢复的表单共享原写入的inFlight状态，直到原请求结束才允许读回、确认或下一次提交。未知员工解除关联依据原发送事件恢复核对入口，即使后续列表已无原行也不会丢失原目标；空列表仅是当前授权下的观察，不充当历史删除成功回执。

## 前端与登记

前端独立锁定React19.3.0／TypeScript5.9.3／Vite8.3.4，使用Node24.18.0／npm11.16.0、自己的依赖和登记SDK1.0.0；dev／build／preview、相对ESM与动态chunk、公开配置和产物摘要见[前端说明](frontend/README.md)。薄入口先同步连接bridge，再加载React代码；opaque iframe使用Origin:null、应用Bearer和省略Cookie，不读宿主storage。

应用code为stores、basePath为`/micro/stores`、API为`/stores-api`。bootstrap初始entryUrl `/sub-apps/stores/unreleased/index.html`是待实际发布替换的登记值。11项菜单节点包括根入口、资料创建／详情、独立员工查询／关联、日报列表／创建／详情及三个写按钮；根空permission不赋予数据读权。员工和日报可直接输入门店UUID，不暗取门店详情，只有另具store/read时才使用辅助门店选择。

应用／客户端／菜单、角色与直接成员grant分别管理，发布不自动授权。管理员使用[Rust管理CLI](../../tools/platform-admin/README.md)显式选择`--app-code stores`，clientId人工绑定实际stores客户端，manifest中的身份占位替换为实际UUID；不自动创建或打印secret，不用SQL直写角色或grant代替正式API。平台菜单POST只投影实际支持字段，不将manifest的status擅自加入当前API请求。

负责人目录只接受store/create、store/update，员工目录只接受store-staff/assign；store/read、unassign与日报不声明成员用途。门店保留平台Member.id，负责人提交映射到managerUserId，员工提交映射到userId；工单将平台id投影为userId后提交assigneeUserId。正式r2三产品均实际查询两业务成员并完成负责人／员工关联和工单处理人分配，业务跨应用Bearer返回401，平台跨应用成员请求403，未登记store/read成员用途400。短期目录结果不等于持久成员资格，新关联仍经Rust复验。成员HTTP从发起前计算最多3秒的绝对见证窗口，连接／锁等待及提交前复核共用预算，不在锁后续期；平台库与业务库没有共同事务，最后成员／grant检查至COMMIT仍可能撤权或失效，不承诺跨库原子或自动撤销历史关联。与工单的字段／用途／scope／owner差异及实际请求见[成员契约](../../docs/stores-member-contract.md)。

草稿仅在当前iframe内存按app／tenant／organization／user分区；主题和同身份续期保留合法输入，真实身份替换隐藏旧服务器视图和目录，编辑version需重读。旧formId不能用新组织凭据提交旧输入。宿主两页共享登录及当前组织；刷新、关闭或销毁iframe不保证草稿保留。

## 固定镜像与外部 URL 发布

Phase28提供[独立静态／Rust镜像、Compose与发行入口](../../deploy/microapps/README.md)。门店v1／v2各有固定digest静态镜像，Nginx剥离`/sub-apps/stores/versions/<version>/`后代理镜像根目录，无平台产物卷；两版本容器及路由保留，未知版本／缺chunk真实404。前端回滚不隐含门店API或数据库回滚。受管交付驱动从冻结源码在Linux按门店Cargo.lock编译、校验ELF／架构／来源后装入runtime镜像；四静态镜像由两前端源码分别构建，不据此宣称默认API Dockerfile builder已运行。

部署方先提供实际registry／架构、HTTPS来源／CA、门店专属数据库／账号及实际机密客户端，拉固定digest并确认当前容器health，再用所有环境共享的权威持久inventory执行artifact-bind／verify，随后测试并发布保留旧版的路由。inventory固定同stores版本的来源／digest和构建身份，同环境完整URL登记定义不可变；跨环境可使用不同公开origin但须同一产物。临时runner、Actions artifact或cache不是权威历史。

平台应用管理“发布版本”中可“登记已部署 URL”，填写版本、HTML入口、可选health URL及动态资源行；RegisterBody仍只有原六字段，不保存镜像digest。登记得到DRAFT，单独预检必须检查正文PASSED，HTTP200加FAILED仍拒绝激活，激活前服务端再次预检。health URL只检查公开HTTP地址，不读取Docker health；平台不创建路由、拉镜像、启动容器或自动授权。guarded CLI登记另带artifact／inventory核验，UI或旧无绑定登记不能宣称已验证镜像身份。

v1同域入口可用`/stores-api`，批准的外部静态origin提供v2时明确构建绝对平台HTTPS API base或其自有受控代理；相对路径不会自动请求主平台。宿主CSP、MICRO_ALLOWED_ORIGINS和静态frame-ancestors明确批准公开origin，业务请求仍以opaque Origin:null／Bearer／omit Cookie进行Rust授权。GitHub候选、受保护手工镜像publication与Linux目标部署分别管理；模板与源码工作流不表示远端已运行或生产已采用，实际阶段状态另见[Phase28台账](../../docs/architecture-optimization-ledger.md#phase-28)。

## 后端构建与配置

独立构建入口：

```powershell
cargo build --locked --manifest-path business-apps/stores/backend/Cargo.toml
cargo test --locked --manifest-path business-apps/stores/backend/Cargo.toml
```

启动前须按 [后端配置](backend/README.md#configuration)显式注入专属数据库与平台机密客户端配置；仓库不自动加载 `.env`。平台策略的 [示例片段](backend/policy.example.json)包含真实动作和三种成员用途，应用 UUID 必须由管理员用实际 stores 应用值替换，再合并现有策略；它不创建应用、菜单、角色或 grant。

Phase26后端空库、三库／双服务隔离与并发结果保留在原历史证据，不作Phase27或28通过记录。命令可执行、代码存在或说明已更新均不等于测试通过。业务不提供支付、POS、库存、排班、审批或公共框架；Phase28镜像与公开v1／v2路由的实际执行单独验收，生产采用仍须明确配置。


## Phase 28 正式验收（2026-10-19）

工单／门店镜像与统一外部 URL 接入已完成本阶段隔离验收：同一 839 项源码输入下，完整 fast 63 步、1083 项通过，0 失败／跳过；真实六镜像 registry push／候选清除／固定 digest pull 和 Compose 联合检查 14/14 通过，Chromium／Chrome／Edge 各 9/9 通过。来源、实际命令、旧失败及五项原验收映射见[Phase 28 验证报告](../../docs/validation/phase-28.md)。

平台仍只登记、预检与激活公开 URL；digest 不变性由 Rust CLI／CI 权威持久 inventory 负责。两 API 按 Cargo.lock 在隔离 Linux 编译、核验 ELF 后 runtime 装箱；默认 API Dockerfile 的源码 builder 未执行。同一 daemon 的 pull 前确认本轮候选镜像身份已清除，基础层缓存可能仍在；本结果不声称跨机器空缓存。远端 Actions、外部 registry 发行与生产部署未执行。前端回退须保留旧版本路由与资源，API／数据库独立升级。
