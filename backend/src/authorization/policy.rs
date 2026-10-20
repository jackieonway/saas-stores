use std::collections::BTreeSet;

use serde_json::Value;
use uuid::Uuid;

use crate::error::ApiError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resource {
    Store,
    Staff,
    DailyReport,
}

impl Resource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Staff => "store-staff",
            Self::DailyReport => "store-daily-report",
        }
    }
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "store" => Some(Self::Store),
            "store-staff" => Some(Self::Staff),
            "store-daily-report" => Some(Self::DailyReport),
            _ => None,
        }
    }
    pub(crate) const fn supports(self, action: Action) -> bool {
        matches!(
            (self, action),
            (
                Self::Store | Self::DailyReport,
                Action::Read | Action::Create | Action::Update
            ) | (
                Self::Staff,
                Action::Read | Action::Assign | Action::Unassign
            )
        )
    }
    pub(crate) const fn member_lookup(self, action: Action) -> bool {
        matches!(
            (self, action),
            (Self::Store, Action::Create | Action::Update) | (Self::Staff, Action::Assign)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Read,
    Create,
    Update,
    Assign,
    Unassign,
}
impl Action {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Create => "create",
            Self::Update => "update",
            Self::Assign => "assign",
            Self::Unassign => "unassign",
        }
    }
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "read" => Some(Self::Read),
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "assign" => Some(Self::Assign),
            "unassign" => Some(Self::Unassign),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    SelfOnly,
    Org,
    OrgSubtree,
    Tenant,
}

/// Constructed only after current client-authenticated introspection and the
/// matching application's fresh context. It never contains credentials.
pub(crate) struct Authorized {
    tenant_id: Uuid,
    user_id: Uuid,
    organization_id: Uuid,
    organization_ceiling: Option<Vec<Uuid>>,
    scope: Scope,
    grant_organization_ids: Vec<Uuid>,
    resource: Resource,
    action: Action,
    expires_at: i64,
}

impl Authorized {
    pub(crate) fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
    pub(crate) fn user_id(&self) -> Uuid {
        self.user_id
    }
    pub(crate) fn organization_id(&self) -> Uuid {
        self.organization_id
    }
    pub(crate) fn organization_ceiling(&self) -> Option<&[Uuid]> {
        self.organization_ceiling.as_deref()
    }
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
    pub(crate) fn resource(&self) -> Resource {
        self.resource
    }
    pub(crate) fn action(&self) -> Action {
        self.action
    }
    pub(crate) fn grant_organization_ids(&self) -> &[Uuid] {
        &self.grant_organization_ids
    }
    pub(crate) fn assert_fresh(&self) -> Result<(), ApiError> {
        if self.expires_at <= now_millis() {
            Err(ApiError::invalid_context())
        } else {
            Ok(())
        }
    }
    fn within_ceiling(&self, org: Uuid) -> bool {
        self.organization_ceiling
            .as_ref()
            .is_none_or(|ids| ids.contains(&org))
    }
    /// Directory/new-association targets are restricted to the selected org for
    /// SELF. Merely owning an existing child record does not widen that target.
    pub(crate) fn require_organization(&self, org: Uuid) -> Result<(), ApiError> {
        self.assert_fresh()?;
        let in_grant = match self.scope {
            Scope::SelfOnly | Scope::Org => org == self.organization_id,
            Scope::OrgSubtree => self.grant_organization_ids.contains(&org),
            Scope::Tenant => true,
        };
        if in_grant && self.within_ceiling(org) {
            Ok(())
        } else {
            Err(ApiError::data_denied())
        }
    }
    pub(crate) fn require_row(&self, org: Uuid, owner: Uuid) -> Result<(), ApiError> {
        self.assert_fresh()?;
        let in_grant = match self.scope {
            Scope::SelfOnly => {
                owner == self.user_id
                    && (self.grant_organization_ids.is_empty()
                        || self.grant_organization_ids.contains(&org))
            }
            Scope::Org => org == self.organization_id,
            Scope::OrgSubtree => self.grant_organization_ids.contains(&org),
            Scope::Tenant => true,
        };
        if in_grant && self.within_ceiling(org) {
            Ok(())
        } else {
            Err(ApiError::data_denied())
        }
    }
}

pub(super) struct TokenIdentity {
    user: Uuid,
    tenant: Uuid,
    organization: Uuid,
    expires_at: i128,
}
pub(super) fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(super) fn id(value: &Value) -> Option<Uuid> {
    let raw = value.as_str()?;
    if raw.len() != 36 {
        return None;
    }
    let parsed = Uuid::parse_str(raw).ok()?;
    (!parsed.is_nil() && parsed.hyphenated().to_string().eq_ignore_ascii_case(raw))
        .then_some(parsed)
}

pub(super) fn validate_intro(intro: &Value, now: i64) -> Result<TokenIdentity, ApiError> {
    let parse = || -> Option<TokenIdentity> {
        let exp = intro.get("exp")?.as_f64()?;
        if !exp.is_finite() || exp.fract() != 0.0 || exp.abs() > 9_007_199_254_740_991.0 {
            return None;
        }
        let expires_at = (exp as i128) * 1000;
        if intro.get("active")?.as_bool() != Some(true)
            || intro.get("aud")?.as_str() != Some("stores")
            || intro.get("token_type")?.as_str() != Some("Bearer")
            || expires_at <= i128::from(now)
        {
            return None;
        }
        Some(TokenIdentity {
            user: id(intro.get("sub")?)?,
            tenant: id(intro.get("tenant_id")?)?,
            organization: id(intro.get("organization_id")?)?,
            expires_at,
        })
    };
    parse().ok_or_else(ApiError::invalid_token)
}

pub(super) fn validate_context(
    identity: &TokenIdentity,
    context: &Value,
    resource: Resource,
    action: Action,
    now: i64,
) -> Result<Authorized, ApiError> {
    if !resource.supports(action) {
        return Err(operation_denied());
    }
    let invalid = ApiError::invalid_context;
    let expires_at = context
        .get("expiresAt")
        .and_then(Value::as_str)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|date| date.timestamp_millis())
        .ok_or_else(invalid)?;
    let hash = context
        .get("contextVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    if context.get("contractVersion").and_then(Value::as_str) != Some("1.0")
        || context.get("appCode").and_then(Value::as_str) != Some("stores")
        || hash.len() != 64
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        || expires_at <= now
        || expires_at > now.saturating_add(30_000)
        || i128::from(expires_at) > identity.expires_at + 999
        || context.pointer("/user/id").and_then(id) != Some(identity.user)
        || context.pointer("/tenant/id").and_then(id) != Some(identity.tenant)
        || context.pointer("/organization/id").and_then(id) != Some(identity.organization)
        || context.pointer("/application/id").and_then(id).is_none()
    {
        return Err(invalid());
    }
    let organization_ceiling = match context.get("organizationScopeIds") {
        None => Some(vec![identity.organization]),
        Some(Value::Null) => None,
        Some(value) => {
            let ids = organization_ids(value).ok_or_else(invalid)?;
            if !ids.contains(&identity.organization) {
                return Err(invalid());
            }
            Some(ids)
        }
    };
    let (scope, grant_organization_ids) =
        grant(context, identity, resource.as_str(), action.as_str())?;
    let effective_expiry = i128::from(expires_at).min(identity.expires_at) as i64;
    if effective_expiry <= now {
        return Err(invalid());
    }
    Ok(Authorized {
        tenant_id: identity.tenant,
        user_id: identity.user,
        organization_id: identity.organization,
        organization_ceiling,
        scope,
        grant_organization_ids,
        resource,
        action,
        expires_at: effective_expiry,
    })
}

fn grant(
    context: &Value,
    identity: &TokenIdentity,
    resource: &str,
    action: &str,
) -> Result<(Scope, Vec<Uuid>), ApiError> {
    let permission = format!("{resource}:{action}");
    if !context
        .get("permissions")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(&permission)))
    {
        return Err(operation_denied());
    }
    let grants = context
        .get("dataGrants")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut matching = grants.iter().filter(|grant| {
        grant.get("resource").and_then(Value::as_str) == Some(resource)
            && grant.get("action").and_then(Value::as_str) == Some(action)
    });
    let grant = matching.next().ok_or_else(ApiError::data_denied)?;
    if matching.next().is_some()
        || grant.get("tenantId").and_then(id) != Some(identity.tenant)
        || grant.get("subjectUserId").and_then(id) != Some(identity.user)
    {
        return Err(ApiError::data_denied());
    }
    let ids = grant
        .get("organizationIds")
        .and_then(organization_ids)
        .ok_or_else(ApiError::data_denied)?;
    let scope = match grant.get("scope").and_then(Value::as_str) {
        Some("SELF") if ids.is_empty() || ids.contains(&identity.organization) => Scope::SelfOnly,
        Some("ORG") if ids == [identity.organization] => Scope::Org,
        Some("ORG_SUBTREE") if ids.contains(&identity.organization) => Scope::OrgSubtree,
        Some("TENANT") if ids.is_empty() => Scope::Tenant,
        _ => return Err(ApiError::data_denied()),
    };
    Ok((scope, ids))
}

fn organization_ids(value: &Value) -> Option<Vec<Uuid>> {
    let values = value.as_array()?;
    if values.len() > 10_000 {
        return None;
    }
    values
        .iter()
        .map(id)
        .collect::<Option<BTreeSet<_>>>()
        .map(|ids| ids.into_iter().collect())
}

pub(super) fn operation_denied() -> ApiError {
    ApiError::new(403, "OPERATION_DENIED", "当前用户没有该操作权限")
}
