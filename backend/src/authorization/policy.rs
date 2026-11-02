use std::collections::BTreeSet;

use serde::Deserialize;
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
            ) | (Self::Staff, Action::Read)
        )
    }
    pub(crate) const fn member_lookup(self, action: Action) -> bool {
        matches!(
            (self, action),
            (Self::Store, Action::Create | Action::Update) | (Self::Staff, Action::Read)
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
    context_version: String,
    organization_deadline: Option<std::time::Instant>,
    authority: Option<AuthorityBinding>,
}

#[derive(PartialEq)]
struct AuthorityBinding {
    issuer: String,
    token_use: String,
    client: String,
    source_application: Option<Uuid>,
    target_application: Uuid,
    membership: Uuid,
    account_version: i64,
    identity_version: i64,
    scope: String,
}

impl Authorized {
    pub(crate) fn same_authority(&self, current: &Self) -> bool {
        self.authority == current.authority
            && self.user_id == current.user_id
            && self.tenant_id == current.tenant_id
            && self.organization_id == current.organization_id
            && self.context_version == current.context_version
            && self.resource == current.resource
            && self.action == current.action
            && self.scope == current.scope
            && self.grant_organization_ids == current.grant_organization_ids
    }
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
    pub(crate) fn context_version(&self) -> &str {
        &self.context_version
    }
    pub(crate) fn allows_organization_metadata(&self, org: Uuid) -> bool {
        self.within_ceiling(org)
            && match self.scope {
                Scope::SelfOnly => {
                    self.grant_organization_ids.is_empty()
                        || self.grant_organization_ids.contains(&org)
                }
                Scope::Org => org == self.organization_id,
                Scope::OrgSubtree => self.grant_organization_ids.contains(&org),
                Scope::Tenant => true,
            }
    }
    pub(crate) fn bind_store_organizations(
        &mut self,
        organizations: Vec<Uuid>,
        deadline: std::time::Instant,
    ) -> Result<(), ApiError> {
        self.assert_fresh()?;
        if organizations
            .iter()
            .any(|id| !self.allows_organization_metadata(*id))
        {
            return Err(ApiError::platform_unavailable());
        }
        self.organization_ceiling = Some(organizations);
        self.organization_deadline = Some(deadline);
        self.assert_fresh()
    }
    pub(crate) fn assert_fresh(&self) -> Result<(), ApiError> {
        self.assert_at(now_millis(), std::time::Instant::now())
    }
    pub(super) fn assert_at(&self, now: i64, instant: std::time::Instant) -> Result<(), ApiError> {
        if self
            .organization_deadline
            .is_some_and(|deadline| instant >= deadline)
        {
            Err(ApiError::new(
                503,
                "ORGANIZATION_VALIDATION_EXPIRED",
                "平台门店组织校验已到期，请重新查询或提交",
            ))
        } else if self.expires_at <= now {
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
    // An invalid platform identity invalidates its entire enclosing response.
    crate::domain::uuid(value.as_str()?).ok()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserPrincipal {
    user_id: String,
    membership_id: String,
    member_type: String,
    identity_version: String,
    account_version: String,
    tenant_id: String,
    organization_id: String,
}

fn positive_version(raw: &str) -> Option<i64> {
    if raw.is_empty() || raw.starts_with('0') || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok().filter(|value| *value > 0)
}

pub(super) fn validate_authorization(
    reply: &Value,
    resource: Resource,
    action: Action,
    now: i64,
    identity_issuer: Option<&str>,
    required_scope: Option<&str>,
) -> Result<Authorized, ApiError> {
    let invalid = ApiError::invalid_token;
    if reply["contractVersion"] != "resource-authorization/1" {
        return Err(invalid());
    }
    match reply["subjectMode"].as_str() {
        Some("USER") => {}
        Some("MACHINE") => {
            return Err(ApiError::new(
                403,
                "SUBJECT_MODE_DENIED",
                "门店业务需要当前员工和 STORE 组织证明，不接受机器身份",
            ))
        }
        _ => return Err(invalid()),
    }
    let issuer = reply["issuer"].as_str().ok_or_else(invalid)?;
    let token_use = reply["tokenUse"].as_str().ok_or_else(invalid)?;
    let intro = &reply["introspection"];
    let context = &reply["context"];
    let target_application = reply["targetApplicationId"]
        .as_str()
        .and_then(|raw| crate::domain::uuid(raw).ok())
        .ok_or_else(invalid)?;
    if reply["resource"] != "stores"
        || reply["audience"] != "stores"
        || reply["action"] != action.as_str()
        || intro["active"] != true
        || intro["token_type"] != "Bearer"
        || intro["aud"] != "stores"
        || intro["iss"] != issuer
        || intro["token_use"] != token_use
        || context.pointer("/application/id").and_then(id) != Some(target_application)
        || reply.get("delegation").is_some()
        || reply.get("credentialVersion").is_some()
    {
        return Err(invalid());
    }
    let client = intro["client_id"]
        .as_str()
        .filter(|raw| !raw.is_empty())
        .ok_or_else(invalid)?;
    let scope = intro["scope"]
        .as_str()
        .filter(|raw| {
            !raw.is_empty()
                && raw.len() <= 8192
                && raw
                    .bytes()
                    .all(|byte| byte == b' ' || (0x21..=0x7e).contains(&byte))
        })
        .ok_or_else(invalid)?;
    let scopes = scope.split(' ').collect::<BTreeSet<_>>();
    if scopes
        .iter()
        .any(|value| value.is_empty() || value.len() > 128)
        || scopes.len() != scope.split(' ').count()
    {
        return Err(invalid());
    }
    let source_application = match token_use {
        "app_session"
            if issuer == "urn:saas:platform:internal-capabilities"
                && client == "stores"
                && scope == "app:access"
                && reply.get("clientId").is_none()
                && reply.get("sourceApplicationId").is_none() =>
        {
            None
        }
        "access_token" if identity_issuer == Some(issuer) => {
            if !required_scope.is_some_and(|required| scopes.contains(required)) {
                return Err(invalid());
            }
            let caller = reply["clientId"]
                .as_str()
                .and_then(|raw| crate::domain::uuid(raw).ok())
                .ok_or_else(invalid)?;
            if intro["client_id"].as_str() != Some(caller.to_string().as_str())
                || intro["sub"].as_str().is_none_or(|raw| {
                    raw.is_empty() || raw.len() > 1024 || raw.chars().any(char::is_control)
                })
            {
                return Err(invalid());
            }
            Some(
                reply["sourceApplicationId"]
                    .as_str()
                    .and_then(|raw| crate::domain::uuid(raw).ok())
                    .ok_or_else(invalid)?,
            )
        }
        _ => return Err(invalid()),
    };
    let principal: UserPrincipal =
        serde_json::from_value(reply["principal"].clone()).map_err(|_| invalid())?;
    if principal.member_type != "EMPLOYEE" {
        return Err(ApiError::new(
            403,
            "SUBJECT_MODE_DENIED",
            "门店业务仅接受当前员工身份",
        ));
    }
    let membership = crate::domain::uuid(&principal.membership_id).map_err(|_| invalid())?;
    let account_version = positive_version(&principal.account_version).ok_or_else(invalid)?;
    let identity_version = positive_version(&principal.identity_version).ok_or_else(invalid)?;
    let reply_expiry = reply["expiresAt"]
        .as_str()
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|date| date.timestamp_millis())
        .ok_or_else(invalid)?;
    let token_expiry = intro["exp"]
        .as_i64()
        .and_then(|seconds| seconds.checked_mul(1000))
        .ok_or_else(invalid)?;
    if reply_expiry <= now
        || reply_expiry > now.saturating_add(30_000)
        || token_expiry <= now
        || reply_expiry > token_expiry.saturating_add(999)
    {
        return Err(invalid());
    }
    let identity = TokenIdentity {
        user: crate::domain::uuid(&principal.user_id).map_err(|_| invalid())?,
        tenant: crate::domain::uuid(&principal.tenant_id).map_err(|_| invalid())?,
        organization: crate::domain::uuid(&principal.organization_id).map_err(|_| invalid())?,
        expires_at: i128::from(reply_expiry.min(token_expiry.saturating_add(999))),
    };
    let mut authorized = validate_context(&identity, context, resource, action, now)?;
    // The context's advertised deadline cannot extend its enclosing authority.
    authorized.expires_at = authorized.expires_at.min(reply_expiry).min(token_expiry);
    authorized.authority = Some(AuthorityBinding {
        issuer: issuer.into(),
        token_use: token_use.into(),
        client: client.into(),
        source_application,
        target_application,
        membership,
        account_version,
        identity_version,
        scope: scope.into(),
    });
    authorized.assert_fresh()?;
    Ok(authorized)
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
        context_version: hash.to_owned(),
        organization_deadline: None,
        authority: None,
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
