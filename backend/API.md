# stores-api HTTP 契约

前缀为 `/stores-api`。本文件描述当前Rust源码的DTO、授权和恢复语义，不是Phase27前端或已完成生产验收的声明。

## 通用规则

受保护请求使用当前stores应用 `Authorization: Bearer`；服务端逐请求Basic内省及stores Bearer context，不信任Cookie或调用方传入的tenant／organization／owner。精确permission与同resource/action的data grant缺一不可；所有SQL与tenant／context组织硬上限交集，scope-hidden记录按404处理。

写入必需 `Content-Type: application/json`，可带charset参数，最多65536 bytes。正文必需JSON object，未知写字段拒绝；query拒绝未知键、重复键、坏percent编码和NUL。UUID是非nil的36字符带连字符形式，输出规范化。分页默认page1／size20，page1–1000000、size1–50；列表返回 `{records,total,page,size}`，count与records使用同一只读快照。写入请求的version是JSON整数1–2147483646，不接受浮点、字符串或0。

响应均带安全UUID `X-Request-Id`、`Cache-Control:no-store`；输入单个有效UUID沿用，否则生成。错误正文只有 `{code,message}`，不回显token／Basic／SQL／DSN或上游正文。允许来源统一处理成功与错误CORS；默认字面 `Origin:null`，不允许Cookie凭据。OPTIONS允许GET／HEAD／POST／PATCH／DELETE与Authorization／Content-Type／X-Request-Id；没有Idempotency-Key协议。

## 路由与动作

| Method | Path | Resource/action | Success |
| --- | --- | --- | --- |
| GET | `/health/live` | 无；进程liveness | 200 |
| GET | `/health/ready`、`/health` | 无；业务库schema readiness | 200或503 |
| GET | `/stores` | store/read | 分页Store |
| POST | `/stores` | store/create | 201 Store |
| GET | `/stores/{id}` | store/read | 200 Store |
| PATCH | `/stores/{id}` | store/update | 200 Store |
| GET | `/stores/{id}/staff` | store-staff/read | 分页StaffLink |
| POST | `/stores/{id}/staff` | store-staff/assign | 201 StaffLink |
| DELETE | `/stores/{id}/staff/{linkId}` | store-staff/unassign | 200 `{ok:true}` |
| GET | `/stores/{id}/daily-reports` | store-daily-report/read | 分页DailyReport |
| POST | `/stores/{id}/daily-reports` | store-daily-report/create | 201 DailyReport |
| GET | `/daily-reports/{id}` | store-daily-report/read | 200 DailyReport |
| PATCH | `/daily-reports/{id}` | store-daily-report/update | 200 DailyReport |
| GET | `/daily-reports/summary` | store-daily-report/read | 200 Summary |
| GET | `/members` | query指定的三种精确成员用途之一 | 分页Member |

表中path均需加 `/stores-api`，例如 `/stores-api/health/live`。没有门店DELETE、日报DELETE、员工通用update或独立summary动作。staff／report按自己的授权校验目标store的tenant／org，不暗加store/read。

## 门店资料

POST正文：

```json
{"name":"示例门店","businessStatus":"OPEN","businessContact":"","managerUserId":null}
```

`name`、`businessStatus`、`businessContact`必需。name首尾trim后非空且最多200字符；businessStatus精确OPEN／CLOSED；contact是保留原空白的字符串，最多200字符，可空，不可null；都拒绝NUL。managerUserId可省略或null，无负责人；非null时必须是目标组织当前有效直接成员UUID，名称来自平台。

tenant／org为当前授权身份，owner为当前创建用户，不接受正文指定。tenant／org唯一：同组织已有门店返回409 `STORE_ALREADY_EXISTS`。负责人不是owner。

PATCH正文必需version，且至少有一个可改字段：name、businessStatus、businessContact、managerUserId。省略manager保留；null清除；不同UUID设置新负责人并预校验。相同UUID保持原成员名称快照，不重验证历史目标。当前版本或锁前旧负责人已改变返回409 `VERSION_CONFLICT`。tenantId／organizationId／ownerUserId／id／createdAt不可修改。

Store字段：

| Field | Type／meaning |
| --- | --- |
| id／tenantId／organizationId／ownerUserId | UUID；owner固定创建人 |
| name／businessContact | 字符串 |
| businessStatus | OPEN或CLOSED |
| managerUserId／managerDisplayName | 已验证名称快照；无负责人时均null |
| version | 正整数，成功更新递增 |
| createdAt／updatedAt | UTC ISO字符串 |

列表可带page、size、q、businessStatus、organizationId。q trim后最多200字符，按名称或业务联系信息作字面子串搜索，`%`／`_`／反斜线不能扩大为SQL通配查询；organizationId只收窄现有SQL scope，不能扩大grant／组织硬上限。

## 员工关联

POST只接受 `{"userId":"<平台已验证目标用户UUID>"}`；尖括号是说明占位，调用时必须换真实值。目标store的组织由服务端SQL取得，目标user通过平台受控member接口验证；`ownerUserId=userId`，`createdByUserId`为操作者，displayName只来自平台。不能传入tenant／org／owner、displayName或version。

有效tenant／store／user关联唯一，重复返回409 `STAFF_ALREADY_ASSIGNED`。SELF按被关联员工的owner限制，不能因为自己创建门店或是manager就关联别人的SELF记录。

DELETE正文严格为 `{"version":1}`，只解除已授权的有效关联并递增版本，200 `{ok:true}`；保留历史行及审计，不删除平台成员。不要求失效历史user重新通过成员验证；旧version返回409，已不可见／无效行404。之后重新关联是新的业务行，不恢复旧行identity。

StaffLink含id、tenantId、storeId、organizationId、ownerUserId、userId、displayName、createdByUserId、version、createdAt、updatedAt；有效列表的removedAt为null。列表只接受page／size，只展示当前授权的有效关联。

## 每日营业记录

POST示例：

```json
{"businessDate":"2026-10-18","amount":"1200.50","note":"当日营业额"}
```

amount必需非负十进制字符串，最多16位整数和2位小数，范围0–9999999999999999.99。`"0"`输出`"0.00"`，`"12.3"`输出`"12.30"`；不接受JSON number、指数、正负号、空格、逗号、空小数或第三位小数，也不自动舍入。写入为 `NUMERIC(18,2)`，不会用浮点计算。

businessDate可省略，按服务端UTC时刻＋08:00取Asia/Shanghai当天；提供时必需严格真实 `YYYY-MM-DD`，年份0001–9999。允许有效历史／未来日期，拒绝null、时间戳、offset、无效闰日或不补零格式。营业日默认切换在UTC16:00:00对应的上海零点，不由客户端timezone决定。

note可省略为空字符串，最多8192字符，拒绝null／NUL。store来自URL，tenant／organization来自已授权目标store，owner固定当前录入用户。tenant／store／businessDate唯一；重复与并发冲突均409 `DAILY_REPORT_EXISTS`，相同正文也没有receipt重放，不自动更新原记录或重复汇总。

PATCH只接受version及至少一个amount／note字段。businessDate、storeId、tenantId、organizationId、ownerUserId、id、createdAt均不可编辑；冲突409 `VERSION_CONFLICT`。DailyReport返回id／tenantId／storeId／organizationId／ownerUserId、businessDate、固定两位amount字符串、note、version、createdAt／updatedAt。

列表可带page／size及可选dateFrom／dateTo，两端inclusive；提供两端时from不能晚于to。详情按自身read SQL scope查询，SELF是录入人，不是负责人或关联员工。

汇总必需dateFrom和dateTo，可另带storeId；不接受page／size。对与列表一致的授权日报行在数据库执行COUNT／SUM(numeric)，返回：

```json
{"count":0,"totalAmount":"0.00","dateFrom":"2026-10-01","dateTo":"2026-10-31","storeId":null}
```

指定storeId时返回该实际UUID；无记录为count0／totalAmount0.00。总额允许超过单条NUMERIC(18,2)金额上限，以PG numeric文本规范化，不用i64或浮点累计，也不只汇总某一分页。

## 受控成员目录

`GET /stores-api/members`必需resource／action，只接受：

| resource | action | purpose |
| --- | --- | --- |
| store | create | 创建时选择负责人 |
| store | update | 修改负责人 |
| store-staff | assign | 新员工关联 |

可带organizationId（默认当前组织）、q、page／size。q在trim前最多100字符；所有query总长最多8192 bytes。服务先授权这个精确pair，再以当前Bearer转发平台 `/api/apps/stores/members`，只投影Member `{id,displayName,organizationId}`及校验过的分页DTO。SELF／ORG目录目标限当前组织，ORG_SUBTREE／TENANT仍与组织硬上限交集。不能用read／unassign／daily-report用途查目录，也不需要通用用户目录管理权。

平台实际策略还需登记相同memberLookups pair；capability、permission或grant不能由这个目录参数临时创建。普通用户不需要grant管理权限。新manager／staff的单成员验证由服务端执行，不能把目录历史结果当作持久成员资格证明。

## 并发、成员期限与结果未知

成员HTTP发起前的单调截止为 `min(PLATFORM_AUTH_TIMEOUT_MS,3000ms)`；返回、等待池／行锁及事务复核都消耗同一窗口，不能锁后续期。更新负责人先非锁scope预读并释放连接，平台HTTP后才持锁复核version／原manager；不一致409。提交前成员结果已过期503 `MEMBER_VALIDATION_EXPIRED`回滚，context过期401；COMMIT错误或提交后无法确认授权期限仍有效时使用相应unknown错误，不将取消当作回滚证明。

平台身份库与门店库没有共同事务，最后member／grant检查至业务COMMIT仍有竞态；没有跨库强一致、自动撤销关联或通用写入幂等保证。网络断开、503 `CREATE_RESULT_UNKNOWN`／`WRITE_RESULT_UNKNOWN`后保持原输入，先查询当前组织门店／有效staff／门店当日日报及version核对；确认状态后人工决定下一请求，不自动重发创建、修改或解除关联。requestId只关联诊断／审计，不是业务幂等键。

## 错误分类

| Status／code | Meaning |
| --- | --- |
| 400 VALIDATION_ERROR | JSON／query／UUID／金额／日期／版本／字段无效 |
| 401 INVALID_APP_TOKEN／INVALID_CONTEXT | token、实时身份或上下文无效／过期／错绑 |
| 403 OPERATION_DENIED／DATA_DENIED | 精确功能permission或独立grant／目标组织不满足 |
| 403 ORIGIN_DENIED | 来源或预检不在声明范围 |
| 404 RESOURCE_NOT_FOUND | 记录不存在或SQL范围不可见 |
| 404 MEMBER_NOT_FOUND | 平台单成员接口确认目标不存在或不是该组织当前有效成员 |
| 409 STORE_ALREADY_EXISTS／STAFF_ALREADY_ASSIGNED／DAILY_REPORT_EXISTS | 业务唯一性冲突，先查询已有记录 |
| 409 VERSION_CONFLICT | 版本或锁前业务快照变化，保留输入并重新读取 |
| 413 BODY_TOO_LARGE／415 JSON_REQUIRED | 正文超限／不完整或Content-Type无效 |
| 503 PLATFORM_UNAVAILABLE／AUTHORIZATION_UNAVAILABLE | 平台HTTP／并发／协议异常或不可用授权装配，拒绝业务操作 |
| 503 MEMBER_VALIDATION_EXPIRED | 成员见证在本地提交前过期，重新校验后由调用方决定提交 |
| 503 DATABASE_UNAVAILABLE | 数据库／readiness不可用，不能外推写事务结果 |
| 503 CREATE_RESULT_UNKNOWN／WRITE_RESULT_UNKNOWN | 写入可能已提交，先读回核对，不自动重发 |

未知路由404 `ROUTE_NOT_FOUND`、不支持方法405 `METHOD_NOT_ALLOWED`也使用固定安全错误和requestId。具体返回顺序以身份／来源先行的服务端校验为准，不以资源存在性探测绕过授权。
