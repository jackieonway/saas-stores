//! Store records own their business state and immutable creation identity.

use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, PgConnection, Postgres, QueryBuilder, Row};
use uuid::Uuid;

use crate::{
    authorization::{Action, Authorized, Resource},
    domain,
    error::ApiError,
    http::Call,
};

const COLUMNS: &str = "id,tenant_id,organization_id,owner_user_id,name,business_status,\
    business_contact,manager_user_id,manager_display_name,version,\
    pg_catalog.to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_iso,\
    pg_catalog.to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_iso";

#[derive(Debug)]
struct Create {
    name: String,
    status: String,
    contact: String,
    manager: Option<Uuid>,
}

#[derive(Debug)]
struct Patch {
    version: i32,
    name: Option<String>,
    status: Option<String>,
    contact: Option<String>,
    manager: Option<Option<Uuid>>,
}

#[derive(Debug)]
struct Filters {
    page: u32,
    size: u32,
    pattern: Option<String>,
    status: Option<String>,
    organization: Option<Uuid>,
}

struct Current {
    organization: Uuid,
    owner: Uuid,
    manager: Option<Uuid>,
    manager_name: Option<String>,
    version: i32,
    name: String,
    status: String,
    contact: String,
}

fn object(body: Value, allowed: &[&str]) -> Result<Map<String, Value>, ApiError> {
    let Value::Object(fields) = body else {
        return Err(ApiError::validation());
    };
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ApiError::validation());
    }
    Ok(fields)
}

fn text(value: &Value, trim: bool, required: bool) -> Result<String, ApiError> {
    let value = value.as_str().ok_or_else(ApiError::validation)?;
    let value = if trim { value.trim() } else { value };
    if value.contains('\0') || value.chars().count() > 200 || (required && value.is_empty()) {
        return Err(ApiError::validation());
    }
    Ok(value.to_owned())
}

fn status(value: &Value) -> Result<String, ApiError> {
    let value = value.as_str().ok_or_else(ApiError::validation)?;
    if !["OPEN", "CLOSED"].contains(&value) {
        return Err(ApiError::validation());
    }
    Ok(value.to_owned())
}

fn manager(value: &Value) -> Result<Option<Uuid>, ApiError> {
    if value.is_null() {
        Ok(None)
    } else {
        domain::uuid(value.as_str().ok_or_else(ApiError::validation)?).map(Some)
    }
}

fn parse_create(body: Value) -> Result<Create, ApiError> {
    let fields = object(
        body,
        &["name", "businessStatus", "businessContact", "managerUserId"],
    )?;
    Ok(Create {
        name: text(
            fields.get("name").ok_or_else(ApiError::validation)?,
            true,
            true,
        )?,
        status: status(
            fields
                .get("businessStatus")
                .ok_or_else(ApiError::validation)?,
        )?,
        contact: text(
            fields
                .get("businessContact")
                .ok_or_else(ApiError::validation)?,
            false,
            false,
        )?,
        manager: fields
            .get("managerUserId")
            .map(manager)
            .transpose()?
            .flatten(),
    })
}

fn parse_update(body: Value) -> Result<Patch, ApiError> {
    let fields = object(
        body,
        &[
            "version",
            "name",
            "businessStatus",
            "businessContact",
            "managerUserId",
        ],
    )?;
    if fields.len() < 2 {
        return Err(ApiError::validation());
    }
    Ok(Patch {
        version: domain::version(fields.get("version").ok_or_else(ApiError::validation)?)?,
        name: fields
            .get("name")
            .map(|value| text(value, true, true))
            .transpose()?,
        status: fields.get("businessStatus").map(status).transpose()?,
        contact: fields
            .get("businessContact")
            .map(|value| text(value, false, false))
            .transpose()?,
        manager: fields.get("managerUserId").map(manager).transpose()?,
    })
}

fn literal_pattern(value: &str) -> String {
    let mut pattern = String::with_capacity(value.len() + 2);
    pattern.push('%');
    for character in value.chars() {
        if matches!(character, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

fn parse_list(raw: Option<&str>) -> Result<Filters, ApiError> {
    let fields = domain::query(
        raw,
        &["page", "size", "q", "businessStatus", "organizationId"],
    )?;
    let (page, size) = domain::pagination(&fields)?;
    let search = fields
        .get("q")
        .map(|value| value.trim())
        .unwrap_or_default();
    if search.chars().count() > 200 {
        return Err(ApiError::validation());
    }
    Ok(Filters {
        page,
        size,
        pattern: (!search.is_empty()).then(|| literal_pattern(search)),
        status: fields
            .get("businessStatus")
            .map(|value| status(&Value::String(value.clone())))
            .transpose()?,
        organization: fields
            .get("organizationId")
            .map(|value| domain::uuid(value))
            .transpose()?,
    })
}

fn database_error(_error: sqlx::Error) -> ApiError {
    ApiError::database_unavailable()
}

fn insertion_error(error: sqlx::Error) -> ApiError {
    if error.as_database_error().is_some_and(|error| {
        error.code().as_deref() == Some("23505")
            && error.constraint() == Some("stores_tenant_id_organization_id_key")
    }) {
        ApiError::new(
            409,
            "STORE_ALREADY_EXISTS",
            "当前组织已有门店，请查询并编辑现有门店",
        )
    } else {
        database_error(error)
    }
}

fn version_conflict() -> ApiError {
    ApiError::new(409, "VERSION_CONFLICT", "门店已被修改，请刷新后重试")
}

fn validate_locked(before: &Current, locked: &Current, version: i32) -> Result<(), ApiError> {
    // A member witness belongs to the exact preflight organization and owner.
    // Recheck both explicitly instead of relying only on the database trigger.
    if locked.version != version
        || locked.manager != before.manager
        || locked.organization != before.organization
        || locked.owner != before.owner
    {
        return Err(version_conflict());
    }
    Ok(())
}

fn write_unknown(create: bool) -> ApiError {
    if create {
        ApiError::new(
            503,
            "CREATE_RESULT_UNKNOWN",
            "门店创建结果尚未确认，请查询当前组织门店核验",
        )
    } else {
        ApiError::new(
            503,
            "WRITE_RESULT_UNKNOWN",
            "门店写入结果尚未确认，请刷新核验当前版本",
        )
    }
}

fn require(auth: &Authorized, action: Action) -> Result<(), ApiError> {
    auth.assert_fresh()?;
    if auth.resource() == Resource::Store && auth.action() == action {
        Ok(())
    } else {
        Err(ApiError::data_denied())
    }
}

fn scoped<'a>(prefix: &str, auth: &'a Authorized) -> QueryBuilder<'a, Postgres> {
    let mut sql = QueryBuilder::new(prefix);
    auth.append_scope(&mut sql);
    sql
}

fn filtered<'a>(
    prefix: &str,
    auth: &'a Authorized,
    filters: &'a Filters,
) -> QueryBuilder<'a, Postgres> {
    let mut sql = scoped(prefix, auth);
    if let Some(organization) = filters.organization {
        sql.push(" AND organization_id=").push_bind(organization);
    }
    if let Some(status) = &filters.status {
        sql.push(" AND business_status=").push_bind(status);
    }
    if let Some(pattern) = &filters.pattern {
        sql.push(" AND (name ILIKE ")
            .push_bind(pattern)
            .push(" ESCAPE CHR(92) OR business_contact ILIKE ")
            .push_bind(pattern)
            .push(" ESCAPE CHR(92))");
    }
    sql
}

fn record(row: PgRow) -> Result<Value, ApiError> {
    Ok(json!({
        "id":row.try_get::<Uuid,_>("id").map_err(database_error)?.to_string(),
        "tenantId":row.try_get::<Uuid,_>("tenant_id").map_err(database_error)?.to_string(),
        "organizationId":row.try_get::<Uuid,_>("organization_id").map_err(database_error)?.to_string(),
        "ownerUserId":row.try_get::<Uuid,_>("owner_user_id").map_err(database_error)?.to_string(),
        "name":row.try_get::<String,_>("name").map_err(database_error)?,
        "businessStatus":row.try_get::<String,_>("business_status").map_err(database_error)?,
        "businessContact":row.try_get::<String,_>("business_contact").map_err(database_error)?,
        "managerUserId":row.try_get::<Option<Uuid>,_>("manager_user_id").map_err(database_error)?.map(|user|user.to_string()),
        "managerDisplayName":row.try_get::<Option<String>,_>("manager_display_name").map_err(database_error)?,
        "version":row.try_get::<i32,_>("version").map_err(database_error)?,
        "createdAt":row.try_get::<String,_>("created_iso").map_err(database_error)?,
        "updatedAt":row.try_get::<String,_>("updated_iso").map_err(database_error)?,
    }))
}

fn current(row: PgRow) -> Result<Current, ApiError> {
    Ok(Current {
        organization: row.try_get("organization_id").map_err(database_error)?,
        owner: row.try_get("owner_user_id").map_err(database_error)?,
        manager: row.try_get("manager_user_id").map_err(database_error)?,
        manager_name: row
            .try_get("manager_display_name")
            .map_err(database_error)?,
        version: row.try_get("version").map_err(database_error)?,
        name: row.try_get("name").map_err(database_error)?,
        status: row.try_get("business_status").map_err(database_error)?,
        contact: row.try_get("business_contact").map_err(database_error)?,
    })
}

async fn load(
    call: &Call<'_>,
    connection: &mut PgConnection,
    id: Uuid,
    lock: bool,
) -> Result<PgRow, ApiError> {
    let mut sql = scoped(
        &format!("SELECT {COLUMNS} FROM public.stores WHERE "),
        call.auth,
    );
    sql.push(" AND id=").push_bind(id);
    if lock {
        sql.push(" FOR UPDATE");
    }
    let row = sql
        .build()
        .fetch_optional(connection)
        .await
        .map_err(database_error)?;
    call.auth.assert_fresh()?;
    row.ok_or_else(ApiError::not_found)
}

async fn audit(
    call: &Call<'_>,
    connection: &mut PgConnection,
    organization: Uuid,
    entity: Uuid,
    action: &'static str,
) -> Result<(), ApiError> {
    let request = domain::uuid(call.request_id).map_err(|_| ApiError::database_unavailable())?;
    let result = sqlx::query("INSERT INTO public.store_audit_events \
        (id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) \
        VALUES($1,$2,$3,$4,'store',$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(call.auth.tenant_id()).bind(organization)
        .bind(call.auth.user_id()).bind(action).bind(entity).bind(request)
        .execute(connection).await.map_err(database_error)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::database_unavailable());
    }
    Ok(())
}

pub(crate) async fn list(call: &Call<'_>, raw_query: Option<&str>) -> Result<Value, ApiError> {
    let timer = crate::telemetry::registry().timer(operational_telemetry::Stage::BusinessSql);
    let result = list_inner(call, raw_query).await;
    timer.finish(crate::telemetry::outcome(&result));
    result
}

async fn list_inner(call: &Call<'_>, raw_query: Option<&str>) -> Result<Value, ApiError> {
    require(call.auth, Action::Read)?;
    let filters = parse_list(raw_query)?;
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    let total: i64 = filtered(
        "SELECT count(*)::bigint FROM public.stores WHERE ",
        call.auth,
        &filters,
    )
    .build_query_scalar()
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    call.auth.assert_fresh()?;
    let rows = filtered(
        &format!("SELECT {COLUMNS} FROM public.stores WHERE "),
        call.auth,
        &filters,
    )
    .push(" ORDER BY created_at DESC,id ASC LIMIT ")
    .push_bind(i64::from(filters.size))
    .push(" OFFSET ")
    .push_bind(i64::from(filters.page - 1) * i64::from(filters.size))
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
    Ok(json!({"records":records,"total":total,"page":filters.page,"size":filters.size}))
}

pub(crate) async fn detail(call: &Call<'_>, raw_id: &str) -> Result<Value, ApiError> {
    require(call.auth, Action::Read)?;
    let id = domain::uuid(raw_id)?;
    let mut connection = call.pool.acquire().await.map_err(database_error)?;
    record(load(call, &mut connection, id, false).await?)
}

pub(crate) async fn create(call: &Call<'_>, body: Value) -> Result<Value, ApiError> {
    require(call.auth, Action::Create)?;
    let body = parse_create(body)?;
    let organization = call.auth.organization_id();
    call.auth.require_organization(organization)?;
    call.auth.require_row(organization, call.auth.user_id())?;
    let member = if let Some(user) = body.manager {
        Some(
            call.authorizer
                .verify_member(call.auth, call.token, organization, user, call.request_id)
                .await?,
        )
    } else {
        None
    };
    let assert_fresh = || {
        call.auth.assert_fresh()?;
        if let (Some(member), Some(user)) = (&member, body.manager) {
            member.assert_fresh(call.auth, organization, user)?;
        }
        Ok::<(), ApiError>(())
    };
    assert_fresh()?;
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    assert_fresh()?;
    let id = Uuid::new_v4();
    let mut insert = QueryBuilder::<Postgres>::new(
        "INSERT INTO public.stores (id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact,manager_user_id,manager_display_name,version) SELECT ",
    );
    insert
        .push_bind(id)
        .push(",tenant_id,organization_id,owner_user_id,")
        .push_bind(&body.name)
        .push(",")
        .push_bind(&body.status)
        .push(",")
        .push_bind(&body.contact)
        .push(",")
        .push_bind(body.manager)
        .push(",")
        .push_bind(member.as_ref().map(|member| member.display_name()))
        .push(",1 FROM (SELECT ")
        .push_bind(call.auth.tenant_id())
        .push(" AS tenant_id,")
        .push_bind(organization)
        .push(" AS organization_id,")
        .push_bind(call.auth.user_id())
        .push(" AS owner_user_id) AS new_store WHERE ");
    call.auth.append_scope(&mut insert);
    insert.push(" RETURNING ").push(COLUMNS);
    let row = insert
        .build()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(insertion_error)?
        .ok_or_else(ApiError::database_unavailable)?;
    let response = record(row)?;
    assert_fresh()?;
    audit(call, &mut transaction, organization, id, "create").await?;
    assert_fresh()?;
    transaction
        .commit()
        .await
        .map_err(|_| write_unknown(true))?;
    assert_fresh().map_err(|_| write_unknown(true))?;
    Ok(response)
}

pub(crate) async fn update(call: &Call<'_>, raw_id: &str, body: Value) -> Result<Value, ApiError> {
    require(call.auth, Action::Update)?;
    let id = domain::uuid(raw_id)?;
    let body = parse_update(body)?;
    let before = {
        let mut connection = call.pool.acquire().await.map_err(database_error)?;
        current(load(call, &mut connection, id, false).await?)?
    };
    if before.version != body.version {
        return Err(version_conflict());
    }
    let desired_manager = body.manager.unwrap_or(before.manager);
    let manager_changed = desired_manager != before.manager;
    // Preserving or clearing the old relation must work even after its member
    // has left the platform. A new nonempty target needs fresh HTTP evidence.
    let member = if manager_changed {
        if let Some(user) = desired_manager {
            call.auth.require_organization(before.organization)?;
            Some(
                call.authorizer
                    .verify_member(
                        call.auth,
                        call.token,
                        before.organization,
                        user,
                        call.request_id,
                    )
                    .await?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let assert_fresh = || {
        call.auth.assert_fresh()?;
        if let (Some(member), Some(user)) = (&member, desired_manager) {
            member.assert_fresh(call.auth, before.organization, user)?;
        }
        Ok::<(), ApiError>(())
    };
    assert_fresh()?;
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    assert_fresh()?;
    let locked = current(load(call, &mut transaction, id, true).await?)?;
    validate_locked(&before, &locked, body.version)?;
    call.auth.require_row(locked.organization, locked.owner)?;
    if manager_changed && desired_manager.is_some() {
        call.auth.require_organization(locked.organization)?;
    }
    assert_fresh()?;
    let manager_name = if !manager_changed {
        locked.manager_name.as_deref()
    } else {
        member.as_ref().map(|member| member.display_name())
    };
    let mut update = QueryBuilder::<Postgres>::new("UPDATE public.stores SET name=");
    update
        .push_bind(body.name.as_deref().unwrap_or(&locked.name))
        .push(",business_status=")
        .push_bind(body.status.as_deref().unwrap_or(&locked.status))
        .push(",business_contact=")
        .push_bind(body.contact.as_deref().unwrap_or(&locked.contact))
        .push(",manager_user_id=")
        .push_bind(desired_manager)
        .push(",manager_display_name=")
        .push_bind(manager_name)
        .push(",updated_at=clock_timestamp(),version=version+1 WHERE ");
    call.auth.append_scope(&mut update);
    update
        .push(" AND id=")
        .push_bind(id)
        .push(" AND version=")
        .push_bind(body.version)
        .push(" RETURNING ")
        .push(COLUMNS);
    let row = update
        .build()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(ApiError::database_unavailable)?;
    let response = record(row)?;
    assert_fresh()?;
    audit(call, &mut transaction, locked.organization, id, "update").await?;
    assert_fresh()?;
    transaction
        .commit()
        .await
        .map_err(|_| write_unknown(false))?;
    assert_fresh().map_err(|_| write_unknown(false))?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const USER: &str = "8c35987c-d35a-43fd-9d0b-54ba6bf09801";

    #[test]
    fn locked_store_must_retain_the_exact_preflight_identity_for_its_member_witness() {
        let org = Uuid::from_u128(1);
        let owner = Uuid::from_u128(2);
        let manager = Some(Uuid::from_u128(3));
        let current = |organization, owner, manager, version| Current {
            organization,
            owner,
            manager,
            manager_name: Some("Verified manager".to_owned()),
            version,
            name: "Store".to_owned(),
            status: "OPEN".to_owned(),
            contact: String::new(),
        };
        let before = current(org, owner, manager, 5);
        assert!(validate_locked(&before, &current(org, owner, manager, 5), 5).is_ok());
        for changed in [
            current(Uuid::from_u128(4), owner, manager, 5),
            current(org, Uuid::from_u128(4), manager, 5),
            current(org, owner, Some(Uuid::from_u128(4)), 5),
            current(org, owner, None, 5),
            current(org, owner, manager, 6),
        ] {
            let error = validate_locked(&before, &changed, 5).unwrap_err();
            assert_eq!((error.status(), error.code()), (409, "VERSION_CONFLICT"));
        }
    }

    #[test]
    fn create_validates_business_fields_and_rejects_client_ownership() {
        let body = parse_create(
            json!({"name":"  Central  ","businessStatus":"OPEN","businessContact":""}),
        )
        .unwrap();
        assert_eq!(body.name, "Central");
        assert!(body.manager.is_none());
        assert!(parse_create(json!({"name":"店","businessStatus":"CLOSED","businessContact":"x","managerUserId":null})).unwrap().manager.is_none());
        for body in [
            json!({"name":" ","businessStatus":"OPEN","businessContact":""}),
            json!({"name":"店","businessStatus":"open","businessContact":""}),
            json!({"name":"店","businessStatus":"OPEN"}),
            json!({"name":"店","businessStatus":"OPEN","businessContact":null}),
            json!({"name":"店","businessStatus":"OPEN","businessContact":"","managerUserId":"invalid"}),
            json!({"name":"店","businessStatus":"OPEN","businessContact":"","ownerUserId":USER}),
            json!({"name":"店","businessStatus":"OPEN","businessContact":"","organizationId":USER}),
            json!({"name":"店","businessStatus":"OPEN","businessContact":"","tenantId":USER}),
            json!({"name":"店","businessStatus":"OPEN","businessContact":"","managerDisplayName":"untrusted"}),
        ] {
            assert_eq!(parse_create(body).unwrap_err().status(), 400);
        }
    }

    #[test]
    fn update_preserves_the_distinction_between_omitted_and_cleared_manager() {
        let keep = parse_update(json!({"version":1,"name":"new"})).unwrap();
        let clear = parse_update(json!({"version":1,"managerUserId":null})).unwrap();
        let set = parse_update(json!({"version":1,"managerUserId":USER})).unwrap();
        assert_eq!(keep.manager, None);
        assert_eq!(clear.manager, Some(None));
        assert_eq!(set.manager, Some(Some(domain::uuid(USER).unwrap())));
        for body in [
            json!({"version":1}),
            json!({"version":0,"name":"new"}),
            json!({"version":1.0,"name":"new"}),
            json!({"version":1,"name":null}),
            json!({"version":1,"businessStatus":null}),
            json!({"version":1,"businessContact":null}),
            json!({"version":1,"managerUserId":false}),
            json!({"version":1,"name":"new","ownerUserId":USER}),
            json!({"version":1,"name":"new","createdAt":"2026-10-18"}),
        ] {
            assert_eq!(parse_update(body).unwrap_err().status(), 400);
        }
    }

    #[test]
    fn text_limits_count_characters_and_never_accept_postgres_nul() {
        assert!(parse_create(json!({"name":"店".repeat(200),"businessStatus":"OPEN","businessContact":"人".repeat(200)})).is_ok());
        for (name, contact) in [
            ("店".repeat(201), String::new()),
            ("店".to_owned(), "人".repeat(201)),
            ("店\0".to_owned(), String::new()),
            ("店".to_owned(), "x\0".to_owned()),
        ] {
            assert!(parse_create(
                json!({"name":name,"businessStatus":"OPEN","businessContact":contact})
            )
            .is_err());
        }
    }

    #[test]
    fn list_filters_require_real_status_and_escape_search_metacharacters() {
        let filters = parse_list(Some("q=a%25b_c%5Cd&businessStatus=OPEN&page=2&size=50")).unwrap();
        assert_eq!(filters.pattern.as_deref(), Some("%a\\%b\\_c\\\\d%"));
        assert_eq!((filters.page, filters.size), (2, 50));
        for raw in [
            "businessStatus=open",
            "businessStatus=",
            "ownerUserId=x",
            "organizationId=x",
            "q=%00",
            "q=a&q=b",
        ] {
            assert!(parse_list(Some(raw)).is_err());
        }
    }
}
