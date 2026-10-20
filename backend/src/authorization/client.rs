use std::{io::Read, sync::Arc, time::Duration};

use reqwest::{
    header::{ACCEPT, CONTENT_TYPE},
    Client, RequestBuilder, StatusCode,
};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    config::PlatformConfig,
    error::{ApiError, ConfigError, ServiceError, ServiceFailure},
};

use super::{Action, Resource};

const MAX_BODY: usize = 256 * 1024;
const MAX_CA: u64 = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResponseKind {
    Authorization,
    General,
    SingleMember,
}

#[derive(Clone)]
pub(super) struct PlatformClient {
    http: Client,
    settings: Arc<PlatformConfig>,
}

impl PlatformClient {
    pub(super) fn new(settings: &PlatformConfig) -> Result<Self, ServiceError> {
        let build = || -> Result<Client, Box<dyn std::error::Error + Send + Sync>> {
            let mut builder = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                // A replay could outlive the caller's current authorization or
                // consume a member witness's original, absolute deadline.
                .retry(reqwest::retry::never())
                .connect_timeout(settings.timeout.min(Duration::from_secs(1)))
                .timeout(settings.timeout)
                .pool_max_idle_per_host(10)
                .pool_idle_timeout(Duration::from_secs(30))
                .no_proxy();
            if let Some(path) = &settings.ca_cert_file {
                let mut bytes = Vec::new();
                std::fs::File::open(path)?
                    .take(MAX_CA + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_CA {
                    return Err(Box::new(ConfigError::PlatformCa));
                }
                let certificates = reqwest::Certificate::from_pem_bundle(&bytes)?;
                if certificates.is_empty() {
                    return Err(Box::new(ConfigError::PlatformCa));
                }
                for certificate in certificates {
                    builder = builder.add_root_certificate(certificate);
                }
            }
            Ok(builder.build()?)
        };
        let http = build().map_err(|source| {
            ServiceError::caused_by(
                ServiceFailure::PlatformClient,
                SanitizedClientSource(source),
            )
        })?;
        Ok(Self {
            http,
            settings: Arc::new(settings.clone()),
        })
    }

    pub(super) async fn authorization_pair(
        &self,
        token: &str,
        resource: &str,
        action: &str,
        request_id: &str,
    ) -> Result<(Value, Value), ApiError> {
        if self.settings.legacy_authorization {
            let intro = self.introspect(token, request_id).await?;
            super::policy::validate_intro(&intro, super::policy::now_millis())?;
            return Ok((intro, self.context(token, request_id).await?));
        }
        let url = self
            .settings
            .origin
            .join("/api/apps/stores/authorize")
            .map_err(|_| ApiError::platform_unavailable())?;
        let value = self
            .json(
                self.http
                    .post(url)
                    .basic_auth(&self.settings.client_id, Some(&self.settings.client_secret))
                    .form(&[("token", token), ("resource", resource), ("action", action)]),
                request_id,
                ResponseKind::Authorization,
            )
            .await?;
        if value.get("contractVersion").and_then(Value::as_str) != Some("1.0")
            || !value["introspection"].is_object()
            || !value["context"].is_object()
        {
            return Err(ApiError::platform_unavailable());
        }
        Ok((value["introspection"].clone(), value["context"].clone()))
    }

    pub(super) async fn introspect(
        &self,
        token: &str,
        request_id: &str,
    ) -> Result<Value, ApiError> {
        let url = self
            .settings
            .origin
            .join("/oauth/introspect")
            .map_err(|_| ApiError::platform_unavailable())?;
        self.json(
            self.http
                .post(url)
                .basic_auth(&self.settings.client_id, Some(&self.settings.client_secret))
                .form(&[("token", token)]),
            request_id,
            ResponseKind::General,
        )
        .await
    }

    pub(super) async fn context(&self, token: &str, request_id: &str) -> Result<Value, ApiError> {
        let url = self
            .settings
            .origin
            .join("/api/apps/stores/context")
            .map_err(|_| ApiError::platform_unavailable())?;
        self.json(
            self.http.get(url).bearer_auth(token),
            request_id,
            ResponseKind::General,
        )
        .await
    }

    pub(super) async fn member(
        &self,
        token: &str,
        resource: Resource,
        action: Action,
        organization: Uuid,
        user: Uuid,
        request_id: &str,
    ) -> Result<Value, ApiError> {
        let url = self
            .settings
            .origin
            .join(&format!("/api/apps/stores/members/{user}"))
            .map_err(|_| ApiError::platform_unavailable())?;
        self.json(
            self.http.get(url).bearer_auth(token).query(&[
                ("resource", resource.as_str().to_owned()),
                ("action", action.as_str().to_owned()),
                ("organizationId", organization.to_string()),
            ]),
            request_id,
            ResponseKind::SingleMember,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn members(
        &self,
        token: &str,
        resource: Resource,
        action: Action,
        organization: Uuid,
        page: u32,
        size: u32,
        q: &str,
        request_id: &str,
    ) -> Result<Value, ApiError> {
        let url = self
            .settings
            .origin
            .join("/api/apps/stores/members")
            .map_err(|_| ApiError::platform_unavailable())?;
        self.json(
            self.http.get(url).bearer_auth(token).query(&[
                ("resource", resource.as_str().to_owned()),
                ("action", action.as_str().to_owned()),
                ("organizationId", organization.to_string()),
                ("page", page.to_string()),
                ("size", size.to_string()),
                ("q", q.to_owned()),
            ]),
            request_id,
            ResponseKind::General,
        )
        .await
    }

    async fn json(
        &self,
        request: RequestBuilder,
        request_id: &str,
        kind: ResponseKind,
    ) -> Result<Value, ApiError> {
        let timer = crate::telemetry::registry().timer(operational_telemetry::Stage::PlatformHttp);
        let result = self.json_inner(request, request_id, kind).await;
        timer.finish(crate::telemetry::outcome(&result));
        result
    }

    async fn json_inner(
        &self,
        mut request: RequestBuilder,
        request_id: &str,
        kind: ResponseKind,
    ) -> Result<Value, ApiError> {
        // Reflect only a canonical UUID, never arbitrary client text or control
        // characters, into upstream metadata or request-building diagnostics.
        if request_id.len() == 36 {
            if let Ok(id) = Uuid::parse_str(request_id) {
                request = request.header("x-request-id", id.to_string());
            }
        }
        let mut response = request
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| ApiError::platform_unavailable())?;
        let authorization_denial =
            response.status() == StatusCode::FORBIDDEN && kind == ResponseKind::Authorization;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED => return Err(ApiError::invalid_token()),
            StatusCode::FORBIDDEN if !authorization_denial => return Err(ApiError::data_denied()),
            StatusCode::FORBIDDEN => {}
            StatusCode::NOT_FOUND if kind == ResponseKind::SingleMember => {
                return Err(ApiError::new(
                    404,
                    "MEMBER_NOT_FOUND",
                    "用户不存在或不属于该组织的有效成员",
                ));
            }
            StatusCode::BAD_REQUEST => return Err(ApiError::validation()),
            _ => return Err(ApiError::platform_unavailable()),
        }
        if !response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.split(';').next())
            .is_some_and(|media| media.eq_ignore_ascii_case("application/json"))
            || response
                .content_length()
                .is_some_and(|length| length > MAX_BODY as u64)
        {
            return Err(ApiError::platform_unavailable());
        }
        let mut body = Vec::with_capacity(4096);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiError::platform_unavailable())?
        {
            if chunk.len() > MAX_BODY - body.len() {
                return Err(ApiError::platform_unavailable());
            }
            body.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&body).map_err(|_| ApiError::platform_unavailable())?;
        if authorization_denial {
            return Err(match value["code"].as_str() {
                Some("OPERATION_DENIED") => super::policy::operation_denied(),
                Some("FORBIDDEN") => ApiError::data_denied(),
                _ => ApiError::platform_unavailable(),
            });
        }
        Ok(value)
    }
}

// Keep private causes for controlled diagnostics. Normal formatting cannot
// expose a CA path, upstream URL, confidential client secret, or bearer token.
struct SanitizedClientSource(Box<dyn std::error::Error + Send + Sync>);

impl std::fmt::Debug for SanitizedClientSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("platform client setup error")
    }
}

impl std::fmt::Display for SanitizedClientSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("platform client setup error")
    }
}

impl std::error::Error for SanitizedClientSource {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
