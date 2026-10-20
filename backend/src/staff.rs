//! Staff associations use their own grants: the linked user owns the row,
//! independently of the store creator and manager.

use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{postgres::PgRow, PgConnection, Postgres, QueryBuilder, Row};
use uuid::Uuid;

use crate::{
    authorization::{Action, Authorized, Resource},
    domain,
    error::ApiError,
    http::Call,
};

const COLUMNS: &str = "id,tenant_id,store_id,organization_id,owner_user_id,user_id,\
    display_name,created_by_user_id,version,\
    pg_catalog.to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_iso,\
    pg_catalog.to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_iso";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Assignment {
    user_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    version: Value,
}

fn parse_assign(body: Value) -> Result<Uuid, ApiError> {
    let body: Assignment = serde_json::from_value(body).map_err(|_| ApiError::validation())?;
    domain::uuid(&body.user_id)
}

fn parse_unassign(body: Value) -> Result<i32, ApiError> {
    let body: Removal = serde_json::from_value(body).map_err(|_| ApiError::validation())?;
    domain::version(&body.version)
}

fn database_error(_error: sqlx::Error) -> ApiError {
    ApiError::database_unavailable()
}

fn duplicate_assignment() -> ApiError {
    ApiError::new(409, "STAFF_ALREADY_ASSIGNED", "该成员已关联此门店")
}

fn version_conflict() -> ApiError {
    ApiError::new(409, "VERSION_CONFLICT", "员工关联已被修改，请刷新后重试")
}

fn write_unknown() -> ApiError {
    ApiError::new(
        503,
        "WRITE_RESULT_UNKNOWN",
        "员工关联写入结果尚未确认，请刷新核验当前关联和版本",
    )
}

fn insertion_error(error: sqlx::Error) -> ApiError {
    if error.as_database_error().is_some_and(|error| {
        error.code().as_deref() == Some("23505")
            && error.constraint() == Some("store_staff_active_member_key")
    }) {
        duplicate_assignment()
    } else {
        database_error(error)
    }
}

fn require_action(auth: &Authorized, action: Action) -> Result<(), ApiError> {
    auth.assert_fresh()?;
    if auth.resource() == Resource::Staff && auth.action() == action {
        Ok(())
    } else {
        Err(ApiError::new(
            403,
            "OPERATION_DENIED",
            "缺少当前员工关联操作权限",
        ))
    }
}

fn scoped<'a>(prefix: &str, auth: &'a Authorized, store: Uuid) -> QueryBuilder<'a, Postgres> {
    let mut sql = QueryBuilder::new(prefix);
    auth.append_scope(&mut sql);
    sql.push(" AND store_id=")
        .push_bind(store)
        .push(" AND removed_at IS NULL");
    sql
}

fn record(row: PgRow) -> Result<Value, ApiError> {
    Ok(json!({
        "id":row.try_get::<Uuid,_>("id").map_err(database_error)?.to_string(),
        "tenantId":row.try_get::<Uuid,_>("tenant_id").map_err(database_error)?.to_string(),
        "storeId":row.try_get::<Uuid,_>("store_id").map_err(database_error)?.to_string(),
        "organizationId":row.try_get::<Uuid,_>("organization_id").map_err(database_error)?.to_string(),
        "ownerUserId":row.try_get::<Uuid,_>("owner_user_id").map_err(database_error)?.to_string(),
        "userId":row.try_get::<Uuid,_>("user_id").map_err(database_error)?.to_string(),
        "displayName":row.try_get::<String,_>("display_name").map_err(database_error)?,
        "createdByUserId":row.try_get::<Uuid,_>("created_by_user_id").map_err(database_error)?.to_string(),
        "version":row.try_get::<i32,_>("version").map_err(database_error)?,
        "createdAt":row.try_get::<String,_>("created_iso").map_err(database_error)?,
        "updatedAt":row.try_get::<String,_>("updated_iso").map_err(database_error)?,
        "removedAt":Value::Null,
    }))
}

/// A staff grant checks the target store's tenant and organization, never its
/// creator's identity. Projecting the actor as owner applies the same complete
/// SQL boundary while keeping staff SELF independent of store ownership.
async fn store_organization(
    call: &Call<'_>,
    connection: &mut PgConnection,
    store: Uuid,
    lock: bool,
) -> Result<Uuid, ApiError> {
    let mut sql = QueryBuilder::<Postgres>::new(
        "SELECT organization_id FROM (SELECT id,tenant_id,organization_id,",
    );
    sql.push_bind(call.auth.user_id())
        .push(" AS owner_user_id FROM public.stores) AS staff_target_store WHERE ");
    call.auth.append_scope(&mut sql);
    sql.push(" AND id=").push_bind(store);
    if lock {
        sql.push(" FOR UPDATE");
    }
    let organization: Option<Uuid> = sql
        .build_query_scalar()
        .fetch_optional(connection)
        .await
        .map_err(database_error)?;
    call.auth.assert_fresh()?;
    organization.ok_or_else(ApiError::not_found)
}

async fn audit(
    call: &Call<'_>,
    connection: &mut PgConnection,
    organization: Uuid,
    entity: Uuid,
    action: &'static str,
) -> Result<(), ApiError> {
    let request = domain::uuid(call.request_id).map_err(|_| ApiError::database_unavailable())?;
    let result = sqlx::query(
        "INSERT INTO public.store_audit_events \
         (id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) \
         VALUES($1,$2,$3,$4,'store-staff',$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(call.auth.tenant_id())
    .bind(organization)
    .bind(call.auth.user_id())
    .bind(action)
    .bind(entity)
    .bind(request)
    .execute(connection)
    .await
    .map_err(database_error)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::database_unavailable());
    }
    Ok(())
}

pub(crate) async fn list(
    call: &Call<'_>,
    raw_store: &str,
    query: Option<&str>,
) -> Result<Value, ApiError> {
    require_action(call.auth, Action::Read)?;
    let store = domain::uuid(raw_store)?;
    let query = domain::query(query, &["page", "size"])?;
    let (page, size) = domain::pagination(&query)?;
    let offset = i64::from(page - 1) * i64::from(size);
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    store_organization(call, &mut transaction, store, false).await?;
    let total: i64 = scoped(
        "SELECT count(*)::bigint FROM public.store_staff WHERE ",
        call.auth,
        store,
    )
    .build_query_scalar()
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    call.auth.assert_fresh()?;
    let rows = scoped(
        &format!("SELECT {COLUMNS} FROM public.store_staff WHERE "),
        call.auth,
        store,
    )
    .push(" ORDER BY created_at DESC,id ASC LIMIT ")
    .push_bind(i64::from(size))
    .push(" OFFSET ")
    .push_bind(offset)
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
    Ok(json!({"records":records,"total":total,"page":page,"size":size}))
}

pub(crate) async fn assign(
    call: &Call<'_>,
    raw_store: &str,
    body: Value,
) -> Result<Value, ApiError> {
    require_action(call.auth, Action::Assign)?;
    let store = domain::uuid(raw_store)?;
    let user = parse_assign(body)?;
    // This connection is dropped before the platform HTTP call. The target is
    // locked and checked again after member verification, using its original
    // non-renewable witness deadline.
    let organization = {
        let mut connection = call.pool.acquire().await.map_err(database_error)?;
        let organization = store_organization(call, &mut connection, store, false).await?;
        call.auth
            .require_organization(organization)
            .map_err(|_| ApiError::not_found())?;
        call.auth.require_row(organization, user)?;
        let duplicate: Option<Uuid> =
            scoped("SELECT id FROM public.store_staff WHERE ", call.auth, store)
                .push(" AND user_id=")
                .push_bind(user)
                .build_query_scalar()
                .fetch_optional(&mut *connection)
                .await
                .map_err(database_error)?;
        call.auth.assert_fresh()?;
        if duplicate.is_some() {
            return Err(duplicate_assignment());
        }
        organization
    };
    let member = call
        .authorizer
        .verify_member(call.auth, call.token, organization, user, call.request_id)
        .await?;
    member.assert_fresh(call.auth, organization, user)?;

    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    member.assert_fresh(call.auth, organization, user)?;
    let locked_organization = store_organization(call, &mut transaction, store, true).await?;
    if locked_organization != organization {
        return Err(ApiError::not_found());
    }
    call.auth.require_organization(organization)?;
    call.auth.require_row(organization, user)?;
    member.assert_fresh(call.auth, organization, user)?;
    let id = Uuid::new_v4();
    // The INSERT SELECT uses the same staff scope with the derived target
    // owner. The store creator never becomes the staff row's owner.
    let mut insert = QueryBuilder::<Postgres>::new(
        "INSERT INTO public.store_staff \
         (id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id,version) SELECT ",
    );
    insert
        .push_bind(id)
        .push(",tenant_id,")
        .push_bind(store)
        .push(",organization_id,owner_user_id,owner_user_id,")
        .push_bind(member.display_name())
        .push(",")
        .push_bind(call.auth.user_id())
        .push(",1 FROM (SELECT tenant_id,organization_id,")
        .push_bind(user)
        .push(" AS owner_user_id FROM public.stores WHERE id=")
        .push_bind(store)
        .push(") AS staff_target_store WHERE ");
    call.auth.append_scope(&mut insert);
    insert.push(" RETURNING ").push(COLUMNS);
    let row = insert
        .build()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(insertion_error)?
        .ok_or_else(ApiError::database_unavailable)?;
    let response = record(row)?;
    member.assert_fresh(call.auth, organization, user)?;
    audit(call, &mut transaction, organization, id, "assign").await?;
    member.assert_fresh(call.auth, organization, user)?;
    transaction.commit().await.map_err(|_| write_unknown())?;
    member
        .assert_fresh(call.auth, organization, user)
        .map_err(|_| write_unknown())?;
    Ok(response)
}

pub(crate) async fn unassign(
    call: &Call<'_>,
    raw_store: &str,
    raw_link: &str,
    body: Value,
) -> Result<Value, ApiError> {
    require_action(call.auth, Action::Unassign)?;
    let store = domain::uuid(raw_store)?;
    let link = domain::uuid(raw_link)?;
    let version = parse_unassign(body)?;
    let mut transaction = crate::telemetry::begin(call.pool)
        .await
        .map_err(database_error)?;
    let row = scoped(
        "SELECT organization_id,owner_user_id,version FROM public.store_staff WHERE ",
        call.auth,
        store,
    )
    .push(" AND id=")
    .push_bind(link)
    .push(" FOR UPDATE")
    .build()
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(ApiError::not_found)?;
    call.auth.assert_fresh()?;
    let organization: Uuid = row.try_get("organization_id").map_err(database_error)?;
    let owner: Uuid = row.try_get("owner_user_id").map_err(database_error)?;
    call.auth.require_row(organization, owner)?;
    let stored_version: i32 = row.try_get("version").map_err(database_error)?;
    if stored_version != version || stored_version == i32::MAX {
        return Err(version_conflict());
    }
    // Removal preserves the historical identity and ownership, and does not
    // need the old target to remain an active platform member.
    let affected = scoped(
        "UPDATE public.store_staff SET removed_at=clock_timestamp(),\
         updated_at=clock_timestamp(),version=version+1 WHERE ",
        call.auth,
        store,
    )
    .push(" AND id=")
    .push_bind(link)
    .push(" AND version=")
    .push_bind(version)
    .build()
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    if affected.rows_affected() != 1 {
        return Err(ApiError::database_unavailable());
    }
    call.auth.assert_fresh()?;
    audit(call, &mut transaction, organization, link, "unassign").await?;
    call.auth.assert_fresh()?;
    transaction.commit().await.map_err(|_| write_unknown())?;
    call.auth.assert_fresh().map_err(|_| write_unknown())?;
    Ok(json!({"ok":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const USER: &str = "8c35987c-d35a-43fd-9d0b-54ba6bf09801";

    #[test]
    fn assign_only_accepts_a_canonical_user_reference() {
        assert_eq!(
            parse_assign(json!({"userId":USER})).unwrap().to_string(),
            USER
        );
        for body in [
            json!({}),
            json!({"userId":null}),
            json!({"userId":12}),
            json!({"userId":"8c35987cd35a43fd9d0b54ba6bf09801"}),
            json!({"userId":" 8c35987c-d35a-43fd-9d0b-54ba6bf09801"}),
            json!({"userId":USER,"tenantId":USER}),
            json!({"userId":USER,"organizationId":USER}),
            json!({"userId":USER,"ownerUserId":USER}),
            json!({"userId":USER,"displayName":"client supplied"}),
            json!({"userId":USER,"version":1}),
        ] {
            assert_eq!(parse_assign(body).unwrap_err().status(), 400);
        }
    }

    #[test]
    fn removal_requires_an_exact_positive_optimistic_version() {
        assert_eq!(parse_unassign(json!({"version":1})).unwrap(), 1);
        for body in [
            json!({}),
            json!({"version":null}),
            json!({"version":0}),
            json!({"version":-1}),
            json!({"version":1.1}),
            json!({"version":"1"}),
            json!({"version":2147483647}),
            json!({"version":2147483648_i64}),
            json!({"version":1,"userId":USER}),
            json!({"version":1,"removedAt":"2026-10-18T00:00:00Z"}),
            json!({"version":1,"ownerUserId":USER}),
        ] {
            assert_eq!(parse_unassign(body).unwrap_err().status(), 400);
        }
    }
}
