//! A narrow proxy for manager selection and current direct platform members.
use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use crate::{
    authorization::{Action, Resource},
    domain,
    error::ApiError,
    http::Call,
};

struct Query {
    resource: Resource,
    action: Action,
    organization: Option<Uuid>,
    page: u32,
    size: u32,
    q: String,
}

pub(crate) struct MemberSearch {
    pub page: u32,
    pub size: u32,
    pub q: String,
}

/// Shared with the read-only store-member view, including validation before
/// trimming so padding cannot bypass the platform's search budget.
pub(crate) fn member_search(raw: Option<&str>) -> Result<MemberSearch, ApiError> {
    search_fields(&domain::query(raw, &["page", "size", "q"])?)
}

fn search_fields(fields: &BTreeMap<String, String>) -> Result<MemberSearch, ApiError> {
    let (page, size) = domain::pagination(fields)?;
    let q = fields.get("q").map(String::as_str).unwrap_or_default();
    if q.chars().count() > 100 {
        return Err(ApiError::validation());
    }
    Ok(MemberSearch {
        page,
        size,
        q: trim(q).to_owned(),
    })
}

/// Let the HTTP adapter authorize the exact requested operation before it
/// constructs its common Call. Parsing again in list preserves this binding.
pub(crate) fn purpose(raw: Option<&str>) -> Result<(Resource, Action), ApiError> {
    let query = parse(raw)?;
    Ok((query.resource, query.action))
}

pub(crate) async fn list(call: &Call<'_>, raw: Option<&str>) -> Result<Value, ApiError> {
    let query = parse(raw)?;
    if call.auth.resource() != query.resource || call.auth.action() != query.action {
        return Err(ApiError::data_denied());
    }
    let organization = query
        .organization
        .unwrap_or_else(|| call.auth.organization_id());
    call.auth.assert_fresh()?;
    call.organizations.get(organization)?;
    call.auth.require_organization(organization)?;
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

fn trim(raw: &str) -> &str {
    raw.trim_matches(|ch|matches!(ch,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}'))
}
fn parse(raw: Option<&str>) -> Result<Query, ApiError> {
    let fields = domain::query(
        raw,
        &["resource", "action", "organizationId", "page", "size", "q"],
    )?;
    let resource = fields
        .get("resource")
        .and_then(|value| Resource::parse(value))
        .ok_or_else(ApiError::validation)?;
    let action = fields
        .get("action")
        .and_then(|value| Action::parse(value))
        .ok_or_else(ApiError::validation)?;
    if !resource.member_lookup(action) {
        return Err(ApiError::validation());
    }
    let organization = fields
        .get("organizationId")
        .map(|value| domain::uuid(value))
        .transpose()?;
    let search = search_fields(&fields)?;
    Ok(Query {
        resource,
        action,
        organization,
        page: search.page,
        size: search.size,
        q: search.q,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_is_mandatory_exact_and_has_only_three_registered_pairs() {
        for (raw, resource, action) in [
            (
                "resource=store&action=create",
                Resource::Store,
                Action::Create,
            ),
            (
                "resource=store&action=update",
                Resource::Store,
                Action::Update,
            ),
            (
                "resource=store-staff&action=read",
                Resource::Staff,
                Action::Read,
            ),
        ] {
            assert_eq!(purpose(Some(raw)).unwrap(), (resource, action));
        }
        for raw in [
            "",
            "resource=store",
            "action=create",
            "resource=ticket&action=create",
            "resource=store&action=read",
            "resource=store-staff&action=create",
            "resource=store-staff&action=assign",
            "resource=store-staff&action=unassign",
            "resource=store-daily-report&action=create",
            "resource=store&action=create&resource=store",
            "resource=store&action=create&%61ction=update",
            "resource=store&action=create&tenantId=other",
            "resource=store&action=create&page=0",
            "resource=store&action=create&size=51",
            "resource=store&action=create&q=%FF",
            "resource=store&action=create&q=%00",
            "resource=store&action=create&q=%G0",
            "resource=store&action=create&organizationId=00112233445566778899aabbccddeeff",
        ] {
            assert!(purpose(Some(raw)).is_err(), "{raw}");
        }
    }

    #[test]
    fn pagination_and_search_bounds_are_checked_before_trim() {
        let query = parse(Some(
            "resource=store-staff&action=read&page=1000000&size=50&q=%25_%5C",
        ))
        .unwrap();
        assert_eq!(
            (query.page, query.size, query.q.as_str()),
            (1_000_000, 50, "%_\\")
        );
        assert!(parse(Some(&format!(
            "resource=store&action=create&q={}",
            "😀".repeat(100)
        )))
        .is_ok());
        assert!(parse(Some(&format!(
            "resource=store&action=create&q=+{}+",
            "x".repeat(99)
        )))
        .is_err());
        assert!(parse(Some(&format!(
            "resource=store&action=create&{}",
            "x".repeat(8192)
        )))
        .is_err());
        let query = parse(Some(
            "resource=store&action=create&q=%EF%BB%BF+hi+%EF%BB%BF",
        ))
        .unwrap();
        assert_eq!(query.q, "hi");
    }

    #[test]
    fn store_member_search_rejects_authority_fields_and_ambiguous_pagination() {
        let defaults = member_search(None).unwrap();
        assert_eq!(
            (defaults.page, defaults.size, defaults.q.as_str()),
            (1, 20, "")
        );
        let search = member_search(Some("page=2&size=50&q=+%E6%9D%8E+%E5%9B%9B+")).unwrap();
        assert_eq!(
            (search.page, search.size, search.q.as_str()),
            (2, 50, "李 四")
        );
        for raw in [
            "page=1&page=2",
            "page=1&%70age=2",
            "page=0",
            "size=51",
            "q=%FF",
            "q=%00",
            "q=%",
            "resource=store-staff",
            "action=assign",
            "organizationId=11111111-1111-4111-8111-111111111111",
            "tenantId=11111111-1111-4111-8111-111111111111",
            "ownerUserId=11111111-1111-4111-8111-111111111111",
        ] {
            assert!(member_search(Some(raw)).is_err(), "{raw}");
        }
        assert!(member_search(Some(&format!("q={}", "😀".repeat(100)))).is_ok());
        assert!(member_search(Some(&format!("q=+{}+", "x".repeat(99)))).is_err());
    }
}
