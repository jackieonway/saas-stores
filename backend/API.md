# 门店 API 契约

前缀 /stores-api，应用 code 为 stores。路由见 src/http.rs，字段和校验以 src/stores.rs、staff.rs、reports.rs 等实际实现为准。

## 通用协议

当前 stores 应用 Bearer 是受保护请求身份，不信任 Cookie 或浏览器自报 tenant/organization/owner。permission 与对应资源/动作的数据授权都要满足，每个列表、计数、详情和写入 SQL 均叠加租户/组织硬上限；范围不可见按 404。

写入使用 application/json，正文最多 65536 字节且为对象，未知字段拒绝。查询拒绝未知/重复键和非法编码。UUID 为非 nil 标准形式。分页 page 默认 1、size 默认 20，分别限制 1–1000000 与 1–50；列表和计数使用同一只读快照。

响应带 X-Request-Id 和 Cache-Control:no-store，错误仅含 code、message，不回显 SQL、凭据和上游正文。默认 CORS 允许字面 Origin:null，不使用 Cookie credentials。门店没有 Idempotency-Key 创建重放协议。

## 路由与资源

| 方法与路径 | 资源/动作 |
| --- | --- |
| GET /health/live、/health/ready、/health | 存活/数据库就绪，无业务权限证明 |
| GET /stores、/stores/{id} | store/read |
| POST /stores | store/create |
| PATCH /stores/{id} | store/update |
| GET /stores/{id}/staff | store-staff/read |
| POST /stores/{id}/staff | store-staff/assign |
| DELETE /stores/{store}/staff/{link} | store-staff/unassign |
| GET /stores/{id}/daily-reports | store-daily-report/read |
| POST /stores/{id}/daily-reports | store-daily-report/create |
| PATCH /daily-reports/{id} | store-daily-report/update |
| GET /daily-reports/summary | store-daily-report/read |
| GET /members | 登记用途的受控成员查找 |
| GET /internal/metrics | 显式启用后的独立私有指标 |

所有路径加服务前缀。域内字段和允许状态使用实际 DTO，不把前端展示标签当成协议值。

## 数据与并发

门店 owner 是创建人，员工关联 owner 是经验证员工，日报 owner 是录入人；修改负责人不会改 owner 或扩大 SELF。员工和负责人必须满足平台受控成员协议。

日报金额按十进制字符串传输，数据库 NUMERIC(18,2)，不能让浏览器浮点往返改变金额。营业日是明确的日期字符串，前端默认按 Asia/Shanghai 当天，由用户确认；同店同日唯一，冲突返回 409。

更新和撤销按接口提交 version，范围为 JSON 整数 1–2147483646。版本冲突先重新读取，由用户明确选择最新修订。写入结果未知时先核对详情或唯一业务关系，不自动重发 POST/PATCH/DELETE；当前没有按键回放任意创建的接口。

## 成员与期限

成员目录只允许平台登记的资源动作用途，不因 read 或 unassign 权限开放任意用户目录。实际 memberLookups、组织范围和新成员验证由平台与业务后端共同约束。

成员 HTTP 不持业务连接或行锁，短事务内再核对 version、组织和原值。有效窗口从请求开始为 min(PLATFORM_AUTH_TIMEOUT_MS,3000ms)，提交前到期拒绝；提交后无法确认则返回结果未知。最后一次平台检查到 COMMIT 仍有跨库竞态，不承诺强一致，详见 [成员契约](docs/member-contract.md)。

## 运行与维护

后端默认 legacy 授权模式，可显式选择 combined，无失败自动回退。启动严格校验本业务迁移和 catalog；不改历史 SQL 或 fingerprint 绕过失败。环境、镜像与数据库操作见 [开发指南](docs/development.md)和[部署指南](docs/deployment.md)。
