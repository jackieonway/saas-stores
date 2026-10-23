//! Fresh platform STORE projections. Business row ownership is checked by SQL.
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use uuid::Uuid;

use super::{policy, Action, Authorized, Authorizer, Resource};
use crate::{domain, error::ApiError};

const MAX_ORGANIZATIONS: usize = 10_000;

pub(crate) struct OrganizationSet {
    records: BTreeMap<Uuid, Value>,
    deadline: Instant,
}

impl OrganizationSet {
    pub(crate) fn ids(&self) -> Vec<Uuid> {
        self.records.keys().copied().collect()
    }
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(crate) fn get(&self, id: Uuid) -> Result<&Value, ApiError> {
        self.records.get(&id).ok_or_else(ApiError::not_found)
    }
    pub(crate) fn matching_names(&self, search: &str) -> Vec<Uuid> {
        let search = search.to_lowercase();
        self.records
            .iter()
            .filter_map(|(id, record)| {
                ["name", "code"]
                    .iter()
                    .any(|key| {
                        record[*key]
                            .as_str()
                            .is_some_and(|text| text.to_lowercase().contains(&search))
                    })
                    .then_some(*id)
            })
            .collect()
    }
}

pub(crate) struct OrganizationQuery {
    pub resource: Resource,
    pub action: Action,
    pub page: u32,
    pub size: u32,
    pub q: String,
}

pub(crate) fn organization_query(
    raw: Option<&str>,
    single: bool,
) -> Result<OrganizationQuery, ApiError> {
    let fields = domain::query(
        raw,
        if single {
            &["resource", "action"]
        } else {
            &["resource", "action", "page", "size", "q"]
        },
    )?;
    let resource = fields
        .get("resource")
        .and_then(|raw| Resource::parse(raw))
        .ok_or_else(ApiError::validation)?;
    let action = fields
        .get("action")
        .and_then(|raw| Action::parse(raw))
        .ok_or_else(ApiError::validation)?;
    if !resource.supports(action) {
        return Err(ApiError::validation());
    }
    let (page, size) = domain::pagination(&fields)?;
    let q = fields.get("q").cloned().unwrap_or_default();
    if q.chars().count() > 100 {
        return Err(ApiError::validation());
    }
    Ok(OrganizationQuery {
        resource,
        action,
        page,
        size,
        q: q.trim().to_owned(),
    })
}

fn expiry(value: &Value) -> Result<(i64, Instant), ApiError> {
    let now = policy::now_millis();
    let expires = value
        .get("expiresAt")
        .and_then(Value::as_str)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|date| date.timestamp_millis())
        .ok_or_else(ApiError::platform_unavailable)?;
    if expires <= now || expires > now.saturating_add(3000) {
        return Err(ApiError::platform_unavailable());
    }
    Ok((
        expires,
        Instant::now() + Duration::from_millis((expires - now) as u64),
    ))
}

fn validate_record(value: &Value, auth: &Authorized) -> Result<(Uuid, Value, Instant), ApiError> {
    let invalid = ApiError::platform_unavailable;
    let id = value.get("id").and_then(policy::id).ok_or_else(invalid)?;
    if value.get("contractVersion").and_then(Value::as_str) != Some("1.0")
        || value.get("contextVersion").and_then(Value::as_str) != Some(auth.context_version())
        || value.get("tenantId").and_then(policy::id) != Some(auth.tenant_id())
        || value.get("organizationType").and_then(Value::as_str) != Some("STORE")
        || value.get("status").and_then(Value::as_str) != Some("ENABLED")
        || !auth.allows_organization_metadata(id)
    {
        return Err(invalid());
    }
    let revision = value
        .get("revision")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let (_, deadline) = expiry(value)?;
    let mut safe = serde_json::Map::new();
    for key in [
        "contractVersion",
        "contextVersion",
        "id",
        "tenantId",
        "organizationType",
        "status",
        "revision",
        "expiresAt",
    ] {
        safe.insert(key.into(), value.get(key).cloned().ok_or_else(invalid)?);
    }
    match value.get("parentId") {
        Some(Value::Null) => {
            safe.insert("parentId".into(), Value::Null);
        }
        Some(parent)
            if parent
                .as_str()
                .and_then(|raw| domain::uuid(raw).ok())
                .is_some() =>
        {
            safe.insert("parentId".into(), parent.clone());
        }
        _ => return Err(invalid()),
    }
    for key in ["name", "code", "path"] {
        let text = value.get(key).and_then(Value::as_str).ok_or_else(invalid)?;
        if text.contains('\0')
            || text.chars().count() > 2048
            || (key != "code" && text.trim().is_empty())
        {
            return Err(invalid());
        }
        safe.insert(key.into(), Value::String(text.to_owned()));
    }
    for key in [
        "province",
        "city",
        "district",
        "township",
        "village",
        "address",
        "contactName",
        "contactPhone",
        "contactEmail",
    ] {
        let field = value.get(key).ok_or_else(invalid)?;
        if !field
            .as_str()
            .is_some_and(|text| !text.contains('\0') && text.chars().count() <= 8192)
        {
            return Err(invalid());
        }
        safe.insert(key.into(), field.clone());
    }
    Ok((id, Value::Object(safe), deadline))
}

fn validate_page(
    value: &Value,
    auth: &Authorized,
    page: u32,
    size: u32,
) -> Result<(Value, BTreeMap<Uuid, Value>, Instant, usize), ApiError> {
    let invalid = ApiError::platform_unavailable;
    let total = value
        .get("total")
        .and_then(Value::as_u64)
        .filter(|total| *total <= MAX_ORGANIZATIONS as u64)
        .ok_or_else(invalid)? as usize;
    if value.get("contractVersion").and_then(Value::as_str) != Some("1.0")
        || value.get("contextVersion").and_then(Value::as_str) != Some(auth.context_version())
        || value.get("page").and_then(Value::as_u64) != Some(u64::from(page))
        || value.get("size").and_then(Value::as_u64) != Some(u64::from(size))
    {
        return Err(invalid());
    }
    let (_, mut deadline) = expiry(value)?;
    let rows = value
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let offset = u64::from(page - 1) * u64::from(size);
    let expected = total.saturating_sub(offset as usize).min(size as usize);
    if rows.len() != expected {
        return Err(invalid());
    }
    let mut records = BTreeMap::new();
    let mut safe = Vec::new();
    for row in rows {
        let (id, record, expires) = validate_record(row, auth)?;
        deadline = deadline.min(expires);
        if records.insert(id, record.clone()).is_some() {
            return Err(invalid());
        }
        safe.push(record);
    }
    Ok((
        json!({"contractVersion":"1.0", "contextVersion":auth.context_version(), "expiresAt":value["expiresAt"], "records":safe, "total":total, "page":page, "size":size}),
        records,
        deadline,
        total,
    ))
}

impl Authorizer {
    pub(crate) async fn store_organizations(
        &self,
        auth: &Authorized,
        token: &str,
        self_targets: Option<&[Uuid]>,
        request_id: &str,
    ) -> Result<OrganizationSet, ApiError> {
        let _permit = self
            .concurrent
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::platform_unavailable())?;
        let started_deadline = Instant::now() + self.timeout.min(Duration::from_secs(3));
        tokio::time::timeout_at(tokio::time::Instant::from_std(started_deadline), async {
            let mut all = BTreeMap::new();
            let mut deadline = started_deadline;
            if let Some(targets) = self_targets {
                if targets.len() > MAX_ORGANIZATIONS {
                    return Err(ApiError::platform_unavailable());
                }
                for target in targets {
                    auth.assert_fresh()?;
                    if !auth.allows_organization_metadata(*target) {
                        return Err(ApiError::data_denied());
                    }
                    let raw = match self
                        .client
                        .organization(token, auth.resource(), auth.action(), *target, request_id)
                        .await
                    {
                        Ok(raw) => raw,
                        Err(error) if error.status() == 404 => continue,
                        Err(error) => return Err(error),
                    };
                    let (id, safe, expires) = validate_record(&raw, auth)?;
                    if id != *target {
                        return Err(ApiError::platform_unavailable());
                    }
                    deadline = deadline.min(expires);
                    if Instant::now() >= deadline {
                        return Err(ApiError::platform_unavailable());
                    }
                    all.insert(id, safe);
                }
                return Ok(OrganizationSet {
                    records: all,
                    deadline,
                });
            }
            let mut total_expected = None;
            for page in 1..=201 {
                auth.assert_fresh()?;
                let raw = self
                    .client
                    .organizations(
                        token,
                        auth.resource(),
                        auth.action(),
                        page,
                        50,
                        "",
                        request_id,
                    )
                    .await?;
                let (_, records, expires, total) = validate_page(&raw, auth, page, 50)?;
                if total_expected
                    .replace(total)
                    .is_some_and(|previous| previous != total)
                {
                    return Err(ApiError::platform_unavailable());
                }
                deadline = deadline.min(expires);
                if Instant::now() >= deadline {
                    return Err(ApiError::platform_unavailable());
                }
                for (id, record) in records {
                    if all.insert(id, record).is_some() {
                        return Err(ApiError::platform_unavailable());
                    }
                }
                if all.len() == total {
                    return Ok(OrganizationSet {
                        records: all,
                        deadline,
                    });
                }
            }
            Err(ApiError::platform_unavailable())
        })
        .await
        .map_err(|_| ApiError::platform_unavailable())?
    }

    pub(crate) async fn organization_directory(
        &self,
        auth: &Authorized,
        token: &str,
        query: &OrganizationQuery,
        id: Option<Uuid>,
        request_id: &str,
    ) -> Result<Value, ApiError> {
        let _permit = self
            .concurrent
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::platform_unavailable())?;
        let deadline = Instant::now() + self.timeout.min(Duration::from_secs(3));
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            auth.assert_fresh()?;
            let (safe, expires) = if let Some(id) = id {
                let raw = self
                    .client
                    .organization(token, query.resource, query.action, id, request_id)
                    .await?;
                let (returned, safe, expires) = validate_record(&raw, auth)?;
                if returned != id {
                    return Err(ApiError::platform_unavailable());
                }
                (safe, expires)
            } else {
                let raw = self
                    .client
                    .organizations(
                        token,
                        query.resource,
                        query.action,
                        query.page,
                        query.size,
                        &query.q,
                        request_id,
                    )
                    .await?;
                let (safe, records, expires, _) =
                    validate_page(&raw, auth, query.page, query.size)?;
                for id in records.keys() {
                    auth.require_organization(*id)
                        .map_err(|_| ApiError::platform_unavailable())?;
                }
                (safe, expires)
            };
            auth.assert_fresh()?;
            if Instant::now() >= deadline.min(expires) {
                return Err(ApiError::platform_unavailable());
            }
            Ok(safe)
        })
        .await
        .map_err(|_| ApiError::platform_unavailable())?
    }
}

#[cfg(test)]
#[path = "organizations_tests.rs"]
mod tests;
