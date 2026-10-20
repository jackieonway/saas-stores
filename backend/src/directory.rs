//! A narrow proxy for the three registered member-association purposes.
use std::collections::BTreeSet;

use serde_json::Value;
use uuid::Uuid;

use crate::{
    authorization::{Action, Resource},
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

fn decode(raw: &str) -> Result<String, ApiError> {
    let mut output = Vec::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => output.push(b' '),
            b'%' => {
                let pair = bytes
                    .get(index + 1..index + 3)
                    .ok_or_else(ApiError::validation)?;
                let high = (pair[0] as char)
                    .to_digit(16)
                    .ok_or_else(ApiError::validation)?;
                let low = (pair[1] as char)
                    .to_digit(16)
                    .ok_or_else(ApiError::validation)?;
                output.push((high * 16 + low) as u8);
                index += 2;
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    let result = String::from_utf8(output).map_err(|_| ApiError::validation())?;
    if result.contains('\0') {
        return Err(ApiError::validation());
    }
    Ok(result)
}
fn positive(raw: &str, max: u32) -> Result<u32, ApiError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::validation());
    }
    raw.parse::<u32>()
        .ok()
        .filter(|n| (1..=max).contains(n))
        .ok_or_else(ApiError::validation)
}
fn uuid(raw: &str) -> Result<Uuid, ApiError> {
    if raw.len() != 36 {
        return Err(ApiError::validation());
    }
    let id = Uuid::parse_str(raw).map_err(|_| ApiError::validation())?;
    if id.is_nil() || !id.hyphenated().to_string().eq_ignore_ascii_case(raw) {
        return Err(ApiError::validation());
    }
    Ok(id)
}
fn trim(raw: &str) -> &str {
    raw.trim_matches(|ch|matches!(ch,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}'))
}
fn parse(raw: Option<&str>) -> Result<Query, ApiError> {
    let raw = raw.unwrap_or_default();
    if raw.len() > 8192 {
        return Err(ApiError::validation());
    }
    let mut resource = None;
    let mut action = None;
    let mut organization = None;
    let mut page = 1;
    let mut size = 20;
    let mut q = String::new();
    let mut seen = BTreeSet::new();
    for part in raw.split('&').filter(|part| !part.is_empty()) {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        let key = decode(key)?;
        let value = decode(value)?;
        if !seen.insert(key.clone()) {
            return Err(ApiError::validation());
        }
        match key.as_str() {
            "resource" => {
                resource = Some(Resource::parse(&value).ok_or_else(ApiError::validation)?)
            }
            "action" => action = Some(Action::parse(&value).ok_or_else(ApiError::validation)?),
            "organizationId" => organization = Some(uuid(&value)?),
            "page" => page = positive(&value, 1_000_000)?,
            "size" => size = positive(&value, 50)?,
            "q" => {
                if value.chars().count() > 100 {
                    return Err(ApiError::validation());
                }
                q = trim(&value).to_owned();
            }
            _ => return Err(ApiError::validation()),
        }
    }
    let resource = resource.ok_or_else(ApiError::validation)?;
    let action = action.ok_or_else(ApiError::validation)?;
    if !resource.member_lookup(action) {
        return Err(ApiError::validation());
    }
    Ok(Query {
        resource,
        action,
        organization,
        page,
        size,
        q,
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
                "resource=store-staff&action=assign",
                Resource::Staff,
                Action::Assign,
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
            "resource=store-staff&action=assign&page=1000000&size=50&q=%25_%5C",
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
}
