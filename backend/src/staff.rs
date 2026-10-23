//! Current direct platform members for a store organization. Legacy local staff
//! associations are retained by the database as history and never queried here.

use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

use crate::{
    authorization::{Action, Authorized, Resource},
    directory, domain,
    error::ApiError,
    http::Call,
};

fn require_read(auth: &Authorized) -> Result<(), ApiError> {
    auth.assert_fresh()?;
    if auth.resource() == Resource::Staff && auth.action() == Action::Read {
        Ok(())
    } else {
        Err(ApiError::new(
            403,
            "OPERATION_DENIED",
            "缺少当前门店成员读取权限",
        ))
    }
}

/// A staff grant checks the target store's tenant and organization, independently
/// of its business-profile creator. The actor projection applies the full SELF
/// SQL boundary without inheriting the store creator's identity.
async fn store_organization(call: &Call<'_>, store: Uuid) -> Result<Uuid, ApiError> {
    let mut sql = QueryBuilder::<Postgres>::new(
        "SELECT organization_id FROM (SELECT id,tenant_id,organization_id,",
    );
    sql.push_bind(call.auth.user_id())
        .push(" AS owner_user_id FROM public.stores) AS staff_target_store WHERE ");
    call.auth.append_scope(&mut sql);
    sql.push(" AND id=").push_bind(store);
    let organization: Option<Uuid> = sql
        .build_query_scalar()
        .fetch_optional(call.pool)
        .await
        .map_err(|_| ApiError::database_unavailable())?;
    call.auth.assert_fresh()?;
    let organization = organization.ok_or_else(ApiError::not_found)?;
    call.auth
        .require_organization(organization)
        .map_err(|_| ApiError::not_found())?;
    call.organizations.get(organization)?;
    Ok(organization)
}

pub(crate) async fn list(
    call: &Call<'_>,
    raw_store: &str,
    raw_query: Option<&str>,
) -> Result<Value, ApiError> {
    require_read(call.auth)?;
    let store = domain::uuid(raw_store)?;
    let query = directory::member_search(raw_query)?;
    // No business-pool connection is held while the platform is contacted.
    let organization = store_organization(call, store).await?;
    call.authorizer
        .members(
            call.auth,
            call.token,
            organization,
            query.page,
            query.size,
            &query.q,
            call.request_id,
        )
        .await
}
