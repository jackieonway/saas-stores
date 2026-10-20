//! Daily reports use their own resource grant for every read and write.

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, Postgres, QueryBuilder, Row, Transaction};
use uuid::Uuid;

use crate::{
    authorization::{Action, Authorized, Resource},
    domain::{self, Day, Money},
    error::ApiError,
    http::Call,
};

const RECORD_COLUMNS: &str = "id,tenant_id,store_id,organization_id,owner_user_id,\
    business_date::text AS business_day,amount::text AS amount_text,note,version,\
    pg_catalog.to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_iso,\
    pg_catalog.to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_iso";
const MAX_NOTE_CHARACTERS: usize = 8192;

fn database_error(_error: sqlx::Error) -> ApiError {
    ApiError::database_unavailable()
}

fn duplicate_report() -> ApiError {
    ApiError::new(
        409,
        "DAILY_REPORT_EXISTS",
        "该门店营业日已有日报，请查询并编辑现有日报",
    )
}

fn version_conflict() -> ApiError {
    ApiError::new(409, "VERSION_CONFLICT", "日报已被修改，请刷新后重试")
}

fn commit_unknown() -> ApiError {
    ApiError::new(
        503,
        "WRITE_RESULT_UNKNOWN",
        "日报写入结果尚未确认，请查询门店营业日及当前版本核验",
    )
}

fn create_unknown() -> ApiError {
    ApiError::new(
        503,
        "CREATE_RESULT_UNKNOWN",
        "日报创建结果尚未确认，请查询该门店营业日核验",
    )
}

fn create_database_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(driver) = &error {
        if driver.code().as_deref() == Some("23505")
            && driver.constraint() == Some("store_daily_reports_day_key")
        {
            return duplicate_report();
        }
    }
    database_error(error)
}

fn require(call: &Call<'_>, action: Action) -> Result<(), ApiError> {
    if call.auth.resource() != Resource::DailyReport || call.auth.action() != action {
        return Err(ApiError::data_denied());
    }
    call.auth.assert_fresh()
}

#[derive(Debug)]
struct CreateReport {
    business_date: String,
    amount: String,
    note: String,
}

#[derive(Debug)]
struct UpdateReport {
    version: i32,
    amount: Option<String>,
    note: Option<String>,
}

fn body_object(body: Value, allowed: &[&str]) -> Result<Map<String, Value>, ApiError> {
    let Value::Object(fields) = body else {
        return Err(ApiError::validation());
    };
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ApiError::validation());
    }
    Ok(fields)
}

fn string(value: &Value) -> Result<&str, ApiError> {
    value.as_str().ok_or_else(ApiError::validation)
}

fn note(value: &Value) -> Result<String, ApiError> {
    let value = string(value)?;
    if value.contains('\0') || value.chars().count() > MAX_NOTE_CHARACTERS {
        return Err(ApiError::validation());
    }
    Ok(value.to_owned())
}

fn parse_create(body: Value, now: DateTime<Utc>) -> Result<CreateReport, ApiError> {
    let fields = body_object(body, &["businessDate", "amount", "note"])?;
    let amount = Money::parse(string(
        fields.get("amount").ok_or_else(ApiError::validation)?,
    )?)?
    .as_db_string();
    let business_date = match fields.get("businessDate") {
        Some(value) => Day::parse(string(value)?)?,
        None => Day::from_utc(now),
    };
    Ok(CreateReport {
        business_date: business_date.as_str().to_owned(),
        amount,
        note: fields
            .get("note")
            .map(note)
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_update(body: Value) -> Result<UpdateReport, ApiError> {
    let fields = body_object(body, &["version", "amount", "note"])?;
    let version = domain::version(fields.get("version").ok_or_else(ApiError::validation)?)?;
    let amount = fields
        .get("amount")
        .map(|value| Money::parse(string(value)?).map(|money| money.as_db_string()))
        .transpose()?;
    let note = fields.get("note").map(note).transpose()?;
    if amount.is_none() && note.is_none() {
        return Err(ApiError::validation());
    }
    Ok(UpdateReport {
        version,
        amount,
        note,
    })
}

#[derive(Debug)]
struct ReportQuery {
    page: u32,
    size: u32,
    offset: i64,
    date_from: Option<String>,
    date_to: Option<String>,
    store_id: Option<Uuid>,
}

impl ReportQuery {
    fn parse(raw: Option<&str>, summary: bool) -> Result<Self, ApiError> {
        let allowed: &[&str] = if summary {
            &["dateFrom", "dateTo", "storeId"]
        } else {
            &["page", "size", "dateFrom", "dateTo"]
        };
        let fields = domain::query(raw, allowed)?;
        let (page, size) = domain::pagination(&fields)?;
        let date_from = fields
            .get("dateFrom")
            .map(|value| Day::parse(value))
            .transpose()?;
        let date_to = fields
            .get("dateTo")
            .map(|value| Day::parse(value))
            .transpose()?;
        if summary && (date_from.is_none() || date_to.is_none()) {
            return Err(ApiError::validation());
        }
        if let (Some(from), Some(to)) = (&date_from, &date_to) {
            if from.as_str() > to.as_str() {
                return Err(ApiError::validation());
            }
        }
        Ok(Self {
            page,
            size,
            offset: (i64::from(page) - 1) * i64::from(size),
            date_from: date_from.map(|day| day.as_str().to_owned()),
            date_to: date_to.map(|day| day.as_str().to_owned()),
            store_id: fields
                .get("storeId")
                .map(|value| domain::uuid(value))
                .transpose()?,
        })
    }
}

/// PostgreSQL SUM(numeric) has a wider range than an individual NUMERIC(18,2)
/// amount. Validate and pad its text directly without i64 cents or floating point.
fn canonical_aggregate(value: &str) -> Result<String, ApiError> {
    let invalid = ApiError::database_unavailable;
    let mut parts = value.split('.');
    let integral = parts.next().ok_or_else(invalid)?;
    let fractional = parts.next();
    if integral.is_empty()
        || !integral.bytes().all(|byte| byte.is_ascii_digit())
        || parts.next().is_some()
        || fractional.is_some_and(|fraction| {
            fraction.is_empty()
                || fraction.len() > 2
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(invalid());
    }
    let integral = integral.trim_start_matches('0');
    let integral = if integral.is_empty() { "0" } else { integral };
    Ok(match fractional {
        Some(fraction) if fraction.len() == 2 => format!("{integral}.{fraction}"),
        Some(fraction) => format!("{integral}.{fraction}0"),
        None => format!("{integral}.00"),
    })
}

fn record(row: PgRow) -> Result<Value, ApiError> {
    let amount: String = row.try_get("amount_text").map_err(database_error)?;
    Ok(json!({
        "id": row.try_get::<Uuid, _>("id").map_err(database_error)?.to_string(),
        "tenantId": row.try_get::<Uuid, _>("tenant_id").map_err(database_error)?.to_string(),
        "storeId": row.try_get::<Uuid, _>("store_id").map_err(database_error)?.to_string(),
        "organizationId": row.try_get::<Uuid, _>("organization_id").map_err(database_error)?.to_string(),
        "ownerUserId": row.try_get::<Uuid, _>("owner_user_id").map_err(database_error)?.to_string(),
        "businessDate": row.try_get::<String, _>("business_day").map_err(database_error)?,
        "amount": canonical_aggregate(&amount)?,
        "note": row.try_get::<String, _>("note").map_err(database_error)?,
        "version": row.try_get::<i32, _>("version").map_err(database_error)?,
        "createdAt": row.try_get::<String, _>("created_iso").map_err(database_error)?,
        "updatedAt": row.try_get::<String, _>("updated_iso").map_err(database_error)?,
    }))
}

fn filtered<'a>(
    prefix: &str,
    auth: &'a Authorized,
    query: &'a ReportQuery,
) -> QueryBuilder<'a, Postgres> {
    let mut sql = QueryBuilder::new(prefix);
    auth.append_scope(&mut sql);
    if let Some(store_id) = query.store_id {
        sql.push(" AND store_id=").push_bind(store_id);
    }
    if let Some(from) = &query.date_from {
        sql.push(" AND business_date>=")
            .push_bind(from)
            .push("::date");
    }
    if let Some(to) = &query.date_to {
        sql.push(" AND business_date<=")
            .push_bind(to)
            .push("::date");
    }
    sql
}

async fn audit(
    call: &Call<'_>,
    transaction: &mut Transaction<'_, Postgres>,
    entity_id: Uuid,
    organization_id: Uuid,
    action: &'static str,
) -> Result<(), ApiError> {
    let request_id = domain::uuid(call.request_id).map_err(|_| ApiError::database_unavailable())?;
    let inserted = sqlx::query(
        "INSERT INTO public.store_audit_events\
         (id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) \
         VALUES($1,$2,$3,$4,'store-daily-report',$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(call.auth.tenant_id())
    .bind(organization_id)
    .bind(call.auth.user_id())
    .bind(action)
    .bind(entity_id)
    .bind(request_id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if inserted.rows_affected() != 1 {
        return Err(ApiError::database_unavailable());
    }
    call.auth.assert_fresh()
}

pub(crate) async fn list(
    call: &Call<'_>,
    store_id: &str,
    raw: Option<&str>,
) -> Result<Value, ApiError> {
    let timer = crate::telemetry::registry().timer(operational_telemetry::Stage::BusinessSql);
    let result = list_inner(call, store_id, raw).await;
    timer.finish(crate::telemetry::outcome(&result));
    result
}

async fn list_inner(call: &Call<'_>, store_id: &str, raw: Option<&str>) -> Result<Value, ApiError> {
    require(call, Action::Read)?;
    let mut query = ReportQuery::parse(raw, false)?;
    query.store_id = Some(domain::uuid(store_id)?);
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    call.auth.assert_fresh()?;
    let total: i64 = filtered(
        "SELECT count(*)::bigint FROM public.store_daily_reports WHERE ",
        call.auth,
        &query,
    )
    .build_query_scalar()
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    call.auth.assert_fresh()?;
    let rows = filtered(
        &format!("SELECT {RECORD_COLUMNS} FROM public.store_daily_reports WHERE "),
        call.auth,
        &query,
    )
    .push(" ORDER BY business_date DESC,id ASC LIMIT ")
    .push_bind(i64::from(query.size))
    .push(" OFFSET ")
    .push_bind(query.offset)
    .build()
    .fetch_all(&mut *transaction)
    .await
    .map_err(database_error)?;
    let records = rows
        .into_iter()
        .map(record)
        .collect::<Result<Vec<_>, _>>()?;
    call.auth.assert_fresh()?;
    transaction.commit().await.map_err(database_error)?;
    call.auth.assert_fresh()?;
    Ok(json!({"records":records,"total":total,"page":query.page,"size":query.size}))
}

pub(crate) async fn detail(call: &Call<'_>, id: &str) -> Result<Value, ApiError> {
    require(call, Action::Read)?;
    let id = domain::uuid(id)?;
    let mut sql = QueryBuilder::new(format!(
        "SELECT {RECORD_COLUMNS} FROM public.store_daily_reports WHERE "
    ));
    call.auth.append_scope(&mut sql);
    let row = sql
        .push(" AND id=")
        .push_bind(id)
        .build()
        .fetch_optional(call.pool)
        .await
        .map_err(database_error)?;
    call.auth.assert_fresh()?;
    record(row.ok_or_else(ApiError::not_found)?)
}

pub(crate) async fn create(
    call: &Call<'_>,
    store_id: &str,
    body: Value,
) -> Result<Value, ApiError> {
    require(call, Action::Create)?;
    let store_id = domain::uuid(store_id)?;
    let body = parse_create(body, Utc::now())?;
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    call.auth.assert_fresh()?;
    // For SELF, the new report belongs to its authenticated creator. Project
    // that owner while checking the target store; store ownership grants no
    // additional report access and is not a prerequisite for report creation.
    let mut target =
        QueryBuilder::new("SELECT organization_id FROM (SELECT id,tenant_id,organization_id,");
    target
        .push_bind(call.auth.user_id())
        .push("::uuid AS owner_user_id FROM public.stores) AS target WHERE ");
    call.auth.append_scope(&mut target);
    let organization_id: Uuid = target
        .push(" AND id=")
        .push_bind(store_id)
        .build_query_scalar()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(ApiError::not_found)?;
    call.auth.assert_fresh()?;
    call.auth.require_organization(organization_id)?;
    call.auth
        .require_row(organization_id, call.auth.user_id())?;
    let id = Uuid::new_v4();
    let mut insert = QueryBuilder::new(
        "INSERT INTO public.store_daily_reports\
         (id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note) SELECT ",
    );
    insert
        .push_bind(id)
        .push(",tenant_id,")
        .push_bind(store_id)
        .push(",organization_id,owner_user_id,")
        .push_bind(&body.business_date)
        .push("::date,")
        .push_bind(&body.amount)
        .push("::numeric,")
        .push_bind(&body.note)
        .push(" FROM (SELECT id,tenant_id,organization_id,")
        .push_bind(call.auth.user_id())
        .push("::uuid AS owner_user_id FROM public.stores) AS target WHERE ");
    call.auth.append_scope(&mut insert);
    let row = insert
        .push(" AND id=")
        .push_bind(store_id)
        .push(" AND organization_id=")
        .push_bind(organization_id)
        .push(" RETURNING ")
        .push(RECORD_COLUMNS)
        .build()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(create_database_error)?
        .ok_or_else(ApiError::not_found)?;
    call.auth.assert_fresh()?;
    let response = record(row)?;
    audit(call, &mut transaction, id, organization_id, "create").await?;
    call.auth.assert_fresh()?;
    transaction.commit().await.map_err(|_| create_unknown())?;
    call.auth.assert_fresh().map_err(|_| create_unknown())?;
    Ok(response)
}

pub(crate) async fn update(call: &Call<'_>, id: &str, body: Value) -> Result<Value, ApiError> {
    require(call, Action::Update)?;
    let id = domain::uuid(id)?;
    let body = parse_update(body)?;
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    call.auth.assert_fresh()?;
    let mut select = QueryBuilder::new("SELECT organization_id,version,amount::text AS amount_text,note FROM public.store_daily_reports WHERE ");
    call.auth.append_scope(&mut select);
    let current = select
        .push(" AND id=")
        .push_bind(id)
        .push(" FOR UPDATE")
        .build()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(ApiError::not_found)?;
    call.auth.assert_fresh()?;
    let current_version: i32 = current.try_get("version").map_err(database_error)?;
    if current_version != body.version {
        return Err(version_conflict());
    }
    let organization_id: Uuid = current.try_get("organization_id").map_err(database_error)?;
    let amount = match body.amount {
        Some(amount) => amount,
        None => current.try_get("amount_text").map_err(database_error)?,
    };
    let note = match body.note {
        Some(note) => note,
        None => current.try_get("note").map_err(database_error)?,
    };
    let mut sql = QueryBuilder::new("UPDATE public.store_daily_reports SET amount=");
    sql.push_bind(amount)
        .push("::numeric,note=")
        .push_bind(note)
        .push(",version=version+1,updated_at=statement_timestamp() WHERE ");
    call.auth.append_scope(&mut sql);
    let row = sql
        .push(" AND id=")
        .push_bind(id)
        .push(" AND version=")
        .push_bind(body.version)
        .push(" RETURNING ")
        .push(RECORD_COLUMNS)
        .build()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(version_conflict)?;
    call.auth.assert_fresh()?;
    let response = record(row)?;
    audit(call, &mut transaction, id, organization_id, "update").await?;
    call.auth.assert_fresh()?;
    transaction.commit().await.map_err(|_| commit_unknown())?;
    call.auth.assert_fresh().map_err(|_| commit_unknown())?;
    Ok(response)
}

pub(crate) async fn summary(call: &Call<'_>, raw: Option<&str>) -> Result<Value, ApiError> {
    require(call, Action::Read)?;
    let query = ReportQuery::parse(raw, true)?;
    let row = filtered(
        "SELECT count(*)::bigint AS report_count,COALESCE(SUM(amount),0)::text AS total_amount FROM public.store_daily_reports WHERE ",
        call.auth,
        &query,
    ).build().fetch_one(call.pool).await.map_err(database_error)?;
    call.auth.assert_fresh()?;
    let count: i64 = row.try_get("report_count").map_err(database_error)?;
    let amount: String = row.try_get("total_amount").map_err(database_error)?;
    Ok(
        json!({"count":count,"totalAmount":canonical_aggregate(&amount)?,"dateFrom":query.date_from,"dateTo":query.date_to,"storeId":query.store_id.map(|id| id.to_string())}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use std::{borrow::Cow, error::Error, fmt};

    #[derive(Debug)]
    struct DriverFailure {
        state: &'static str,
        constraint: Option<&'static str>,
    }

    impl fmt::Display for DriverFailure {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(
                "postgresql://private-user:private-password@private-host/stores secret-body",
            )
        }
    }

    impl Error for DriverFailure {}

    impl sqlx::error::DatabaseError for DriverFailure {
        fn message(&self) -> &str {
            "secret-body"
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.state))
        }
        fn constraint(&self) -> Option<&str> {
            self.constraint
        }
        fn as_error(&self) -> &(dyn Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    #[test]
    fn only_the_real_daily_unique_constraint_becomes_duplicate_and_all_driver_details_stay_private()
    {
        for (state, constraint, expected_status, expected_code) in [
            (
                "23505",
                Some("store_daily_reports_day_key"),
                409,
                "DAILY_REPORT_EXISTS",
            ),
            (
                "23505",
                Some("store_daily_reports_pkey"),
                503,
                "DATABASE_UNAVAILABLE",
            ),
            ("23505", None, 503, "DATABASE_UNAVAILABLE"),
            (
                "23514",
                Some("store_daily_reports_day_key"),
                503,
                "DATABASE_UNAVAILABLE",
            ),
        ] {
            let error = create_database_error(sqlx::Error::Database(Box::new(DriverFailure {
                state,
                constraint,
            })));
            assert_eq!(
                (error.status(), error.code()),
                (expected_status, expected_code)
            );
            for output in [
                format!("{error}"),
                format!("{error:?}"),
                error.message().to_owned(),
            ] {
                for private in [
                    "private-user",
                    "private-password",
                    "private-host",
                    "secret-body",
                    "store_daily_reports_pkey",
                ] {
                    assert!(!output.contains(private), "{output}");
                }
            }
        }
    }

    #[test]
    fn create_rejects_client_ownership_float_money_and_ambiguous_days() {
        let now = Utc.with_ymd_and_hms(2026, 10, 18, 15, 59, 59).unwrap();
        let parsed = parse_create(json!({"amount":"0012.3"}), now).unwrap();
        assert_eq!(parsed.amount, "12.30");
        assert_eq!(parsed.business_date, "2026-10-18");
        assert_eq!(parsed.note, "");
        let next = Utc.with_ymd_and_hms(2026, 10, 18, 16, 0, 0).unwrap();
        assert_eq!(
            parse_create(json!({"amount":"0"}), next)
                .unwrap()
                .business_date,
            "2026-10-19"
        );
        for body in [
            json!({"amount":12.3}),
            json!({"amount":"1e2"}),
            json!({"amount":"-0.01"}),
            json!({"amount":"1.001"}),
            json!({"amount":"10000000000000000.00"}),
            json!({"amount":"1", "businessDate":"2026-02-29"}),
            json!({"amount":"1", "businessDate":"2026-10-18T00:00:00+08:00"}),
            json!({"amount":"1", "businessDate":null}),
            json!({"amount":"1", "tenantId":"forged"}),
            json!({"amount":"1", "organizationId":"forged"}),
            json!({"amount":"1", "storeId":"forged"}),
            json!({"amount":"1", "ownerUserId":"forged"}),
            json!({"amount":"1", "note":null}),
            json!({"amount":"1", "note":"\0"}),
        ] {
            assert_eq!(
                parse_create(body.clone(), now).unwrap_err().status(),
                400,
                "{body}"
            );
        }
        for date in ["0001-01-01", "2024-02-29", "9999-12-31"] {
            assert_eq!(
                parse_create(json!({"amount":"0.00", "businessDate":date}), now)
                    .unwrap()
                    .business_date,
                date
            );
        }
        assert!(parse_create(json!({"amount":"0", "note":"😀".repeat(8192)}), now).is_ok());
        assert!(parse_create(json!({"amount":"0", "note":"😀".repeat(8193)}), now).is_err());
    }

    #[test]
    fn patch_requires_version_and_content_and_preserves_report_identity() {
        let update =
            parse_update(json!({"version":1,"amount":"9999999999999999.99","note":"已复核"}))
                .unwrap();
        assert_eq!(update.version, 1);
        assert_eq!(update.amount.as_deref(), Some("9999999999999999.99"));
        assert_eq!(update.note.as_deref(), Some("已复核"));
        for body in [
            json!({"amount":"1"}),
            json!({"version":0,"amount":"1"}),
            json!({"version":1.0,"amount":"1"}),
            json!({"version":2147483647,"amount":"1"}),
            json!({"version":1}),
            json!({"version":1,"amount":null}),
            json!({"version":1,"note":null}),
            json!({"version":1,"businessDate":"2026-10-18"}),
            json!({"version":1,"storeId":"forged"}),
            json!({"version":1,"ownerUserId":"forged"}),
            json!({"version":1,"organizationId":"forged"}),
            json!({"version":1,"tenantId":"forged"}),
        ] {
            assert_eq!(
                parse_update(body.clone()).unwrap_err().status(),
                400,
                "{body}"
            );
        }
    }

    #[test]
    fn bounded_queries_reject_duplicate_malformed_and_inverted_filters() {
        let query = ReportQuery::parse(
            Some("page=1000000&size=50&dateFrom=0001-01-01&dateTo=9999-12-31"),
            false,
        )
        .unwrap();
        assert_eq!(
            (query.page, query.size, query.offset),
            (1_000_000, 50, 49_999_950)
        );
        let query = ReportQuery::parse(Some("dateFrom=2026-10-01&dateTo=2026-10-31&storeId=00112233-4455-6677-8899-aabbccddeeff"), true).unwrap();
        assert!(query.store_id.is_some());
        for raw in [
            "dateFrom=2026-10-19&dateTo=2026-10-18",
            "dateFrom=2026-02-29",
            "dateTo=2026-1-01",
            "dateFrom=2026-01-01&%64ateFrom=2026-01-02",
            "page=0",
            "page=1000001",
            "size=51",
            "size=-1",
            "size=1e1",
            "ownerUserId=forged",
            "tenantId=forged",
            "q=%FF",
            "dateFrom=%00",
            "dateFrom=%",
            "dateFrom=%G0",
            "=x",
        ] {
            assert_eq!(
                ReportQuery::parse(Some(raw), false).unwrap_err().status(),
                400,
                "{raw}"
            );
        }
        for raw in [
            None,
            Some("dateFrom=2026-10-01"),
            Some("dateTo=2026-10-31"),
            Some("dateFrom=2026-10-01&dateTo=2026-10-31&page=1"),
        ] {
            assert!(ReportQuery::parse(raw, true).is_err(), "{raw:?}");
        }
        assert!(
            ReportQuery::parse(Some("storeId=00112233-4455-6677-8899-aabbccddeeff"), false)
                .is_err()
        );
    }

    #[test]
    fn aggregate_numeric_text_never_passes_through_cents_or_float() {
        assert_eq!(canonical_aggregate("0").unwrap(), "0.00");
        assert_eq!(
            canonical_aggregate("123456789012345678901234567890.01").unwrap(),
            "123456789012345678901234567890.01"
        );
        assert_eq!(
            canonical_aggregate("1000000000000000000000000000000.0").unwrap(),
            "1000000000000000000000000000000.00"
        );
        for value in ["-1.00", "NaN", "1e30", "1.001", ".01", "1.", ""] {
            assert_eq!(
                canonical_aggregate(value).unwrap_err().status(),
                503,
                "{value}"
            );
        }
    }

    #[test]
    fn report_failures_offer_safe_specific_recovery() {
        assert_eq!(
            (duplicate_report().status(), duplicate_report().code()),
            (409, "DAILY_REPORT_EXISTS")
        );
        assert_eq!(
            (version_conflict().status(), version_conflict().code()),
            (409, "VERSION_CONFLICT")
        );
        assert_eq!(
            (commit_unknown().status(), commit_unknown().code()),
            (503, "WRITE_RESULT_UNKNOWN")
        );
        assert_eq!(
            (create_unknown().status(), create_unknown().code()),
            (503, "CREATE_RESULT_UNKNOWN")
        );
    }
}
