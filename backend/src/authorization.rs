//! Fresh, bounded platform authorization for the independent stores service.
mod client;
mod policy;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::{
    error::{ApiError, ServiceError, ServiceFailure},
    Config,
};

pub(crate) use policy::{Action, Authorized, Resource, Scope};

#[derive(Clone)]
pub(crate) struct Authorizer {
    client: client::PlatformClient,
    concurrent: Arc<Semaphore>,
    timeout: Duration,
}

impl Authorizer {
    pub(crate) fn new(config: &Config) -> Result<Self, ServiceError> {
        let settings = config.platform.as_ref().ok_or_else(|| {
            ServiceError::caused_by(
                ServiceFailure::PlatformConfiguration,
                std::io::Error::other("required platform settings absent"),
            )
        })?;
        Ok(Self {
            client: client::PlatformClient::new(settings)?,
            concurrent: Arc::new(Semaphore::new(settings.max_concurrent)),
            timeout: settings.timeout,
        })
    }
    /// One permit and total budget cover introspection plus the matching app
    /// context and both response bodies. Successful results are never cached.
    pub(crate) async fn authorize(
        &self,
        token: &str,
        resource: Resource,
        action: Action,
        request_id: &str,
    ) -> Result<Authorized, ApiError> {
        let timer = crate::telemetry::registry().timer(operational_telemetry::Stage::Authorization);
        let result = self
            .authorize_inner(token, resource, action, request_id)
            .await;
        timer.finish(crate::telemetry::outcome(&result));
        result
    }

    async fn authorize_inner(
        &self,
        token: &str,
        resource: Resource,
        action: Action,
        request_id: &str,
    ) -> Result<Authorized, ApiError> {
        valid_token(token)?;
        if !resource.supports(action) {
            return Err(ApiError::new(
                403,
                "OPERATION_DENIED",
                "当前用户没有该操作权限",
            ));
        }
        let _permit = self
            .concurrent
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::platform_unavailable())?;
        tokio::time::timeout(self.timeout, async {
            let (intro, context) = self
                .client
                .authorization_pair(token, resource.as_str(), action.as_str(), request_id)
                .await?;
            let identity = policy::validate_intro(&intro, policy::now_millis())?;
            policy::validate_context(&identity, &context, resource, action, policy::now_millis())
        })
        .await
        .map_err(|_| ApiError::platform_unavailable())?
    }
    pub(crate) async fn verify_member(
        &self,
        auth: &Authorized,
        token: &str,
        organization: Uuid,
        user: Uuid,
        request_id: &str,
    ) -> Result<VerifiedMember, ApiError> {
        valid_token(token)?;
        directory_authorized(auth, organization)?;
        let _permit = self
            .concurrent
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::platform_unavailable())?;
        // This absolute witness window starts before HTTP, and includes response
        // transfer plus any later business pool/lock wait. It is never renewed.
        let deadline = Instant::now() + self.timeout.min(Duration::from_secs(3));
        let value = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.client.member(
                token,
                auth.resource(),
                auth.action(),
                organization,
                user,
                request_id,
            ),
        )
        .await
        .map_err(|_| ApiError::platform_unavailable())??;
        let (returned, display_name) = validate_member(&value, organization)?;
        if returned != user {
            return Err(ApiError::platform_unavailable());
        }
        let member = VerifiedMember {
            actor: auth.user_id(),
            tenant: auth.tenant_id(),
            current_organization: auth.organization_id(),
            resource: auth.resource(),
            action: auth.action(),
            organization,
            user,
            display_name,
            deadline,
        };
        member.assert_fresh(auth, organization, user)?;
        Ok(member)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn members(
        &self,
        auth: &Authorized,
        token: &str,
        organization: Uuid,
        page: u32,
        size: u32,
        q: &str,
        request_id: &str,
    ) -> Result<Value, ApiError> {
        valid_token(token)?;
        directory_authorized(auth, organization)?;
        if !(1..=1_000_000).contains(&page)
            || !(1..=50).contains(&size)
            || q.chars().count() > 100
            || q.contains('\0')
        {
            return Err(ApiError::validation());
        }
        let _permit = self
            .concurrent
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::platform_unavailable())?;
        let value = tokio::time::timeout(
            self.timeout,
            self.client.members(
                token,
                auth.resource(),
                auth.action(),
                organization,
                page,
                size,
                q,
                request_id,
            ),
        )
        .await
        .map_err(|_| ApiError::platform_unavailable())??;
        auth.assert_fresh()?;
        let result = validate_page(&value, organization, page, size)?;
        auth.assert_fresh()?;
        Ok(result)
    }
}

fn valid_token(token: &str) -> Result<(), ApiError> {
    if token.is_empty()
        || token.len() > 8185
        || token
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        Err(ApiError::invalid_token())
    } else {
        Ok(())
    }
}

fn directory_authorized(auth: &Authorized, organization: Uuid) -> Result<(), ApiError> {
    if !auth.resource().member_lookup(auth.action()) {
        return Err(ApiError::data_denied());
    }
    auth.require_organization(organization)
}

/// A successfully validated current direct member, bound to the exact operation
/// and actor. Business modules can inspect it but cannot construct/renew it.
pub(crate) struct VerifiedMember {
    actor: Uuid,
    tenant: Uuid,
    current_organization: Uuid,
    resource: Resource,
    action: Action,
    organization: Uuid,
    user: Uuid,
    display_name: String,
    deadline: Instant,
}
impl VerifiedMember {
    pub(crate) fn user_id(&self) -> Uuid {
        self.user
    }
    pub(crate) fn organization_id(&self) -> Uuid {
        self.organization
    }
    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }
    pub(crate) fn assert_fresh(
        &self,
        auth: &Authorized,
        organization: Uuid,
        user: Uuid,
    ) -> Result<(), ApiError> {
        directory_authorized(auth, organization)?;
        if self.actor != auth.user_id()
            || self.tenant != auth.tenant_id()
            || self.current_organization != auth.organization_id()
            || self.resource != auth.resource()
            || self.action != auth.action()
            || self.organization_id() != organization
            || self.user_id() != user
        {
            return Err(ApiError::data_denied());
        }
        self.assert_at(Instant::now())
    }
    fn assert_at(&self, now: Instant) -> Result<(), ApiError> {
        if now >= self.deadline {
            Err(ApiError::new(
                503,
                "MEMBER_VALIDATION_EXPIRED",
                "成员校验已到期，请重新提交请求",
            ))
        } else {
            Ok(())
        }
    }
}

fn validate_member(value: &Value, organization: Uuid) -> Result<(Uuid, String), ApiError> {
    let invalid = ApiError::platform_unavailable;
    let user = value.get("id").and_then(policy::id).ok_or_else(invalid)?;
    if value.get("organizationId").and_then(policy::id) != Some(organization) {
        return Err(invalid());
    }
    let display = value
        .get("displayName")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if display.trim().is_empty() || display.chars().count() > 100 || display.contains('\0') {
        return Err(invalid());
    }
    Ok((user, display.to_owned()))
}

fn validate_page(
    value: &Value,
    organization: Uuid,
    page: u32,
    size: u32,
) -> Result<Value, ApiError> {
    let invalid = ApiError::platform_unavailable;
    let number = |name: &str| value.get(name).and_then(Value::as_u64);
    if number("page") != Some(u64::from(page))
        || number("size") != Some(u64::from(size))
        || number("total").is_none_or(|n| n > 9_007_199_254_740_991)
    {
        return Err(invalid());
    }
    let records = value
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if records.len() > size as usize {
        return Err(invalid());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut safe_records = Vec::with_capacity(records.len());
    for member in records {
        let (user, display_name) = validate_member(member, organization)?;
        if !ids.insert(user) {
            return Err(invalid());
        }
        safe_records.push(
            serde_json::json!({"id":user,"displayName":display_name,"organizationId":organization}),
        );
    }
    Ok(serde_json::json!({"records":safe_records,"total":number("total"),"page":page,"size":size}))
}

impl Authorized {
    /// A full parameterized tenant/grant/owner/hard-ceiling predicate. The caller
    /// has emitted WHERE and adds its own resource-specific active conditions.
    pub(crate) fn append_scope(&self, query: &mut QueryBuilder<'_, Postgres>) {
        query.push("tenant_id=").push_bind(self.tenant_id());
        if let Some(ids) = self.organization_ceiling() {
            query
                .push(" AND organization_id=ANY(")
                .push_bind(ids.to_vec())
                .push("::uuid[])");
        }
        match self.scope() {
            Scope::SelfOnly => {
                query.push(" AND owner_user_id=").push_bind(self.user_id());
                if !self.grant_organization_ids().is_empty() {
                    query
                        .push(" AND organization_id=ANY(")
                        .push_bind(self.grant_organization_ids().to_vec())
                        .push("::uuid[])");
                }
            }
            Scope::Org => {
                query
                    .push(" AND organization_id=")
                    .push_bind(self.organization_id());
            }
            Scope::OrgSubtree => {
                query
                    .push(" AND organization_id=ANY(")
                    .push_bind(self.grant_organization_ids().to_vec())
                    .push("::uuid[])");
            }
            Scope::Tenant => {}
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod witness_tests;
