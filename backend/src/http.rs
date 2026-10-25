use std::time::Duration;

use axum::{
    body::to_bytes,
    extract::{rejection::PathRejection, DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    authorization::{
        organization_query, Action, Authorized, Authorizer, OrganizationSet, Resource, Scope,
    },
    error::{ApiError, ServiceError},
    Config,
};

const BODY_LIMIT: usize = 65_536;

/// A request-local business call. Credentials never enter persistent state or
/// formatting implementations. Authorization is freshly obtained per request.
pub(crate) struct Call<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) auth: &'a Authorized,
    pub(crate) authorizer: &'a Authorizer,
    pub(crate) token: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) organizations: &'a OrganizationSet,
}

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    health_timeout: Duration,
    authorizer: Option<Authorizer>,
    allowed_origins: Vec<String>,
    metrics_token: Option<String>,
}

#[derive(Clone)]
struct RequestId(String);

/// Construct the health surface for an already initialized independent pool.
///
/// This compatibility surface has no platform credentials. Business operations
/// return a fixed unavailable response; use [`application`] to serve them.
pub fn router(pool: PgPool, health_timeout: Duration) -> Router {
    configured_router(pool, health_timeout, None, vec![])
}

/// Construct the authenticated store/staff/report API.
///
/// First initialize the database through [`crate::initialize_database`]. Each
/// protected request independently introspects its Bearer and loads the stores
/// context. A caller's Cookie is never used as an authentication credential.
pub fn application(config: &Config, pool: PgPool) -> Result<Router, ServiceError> {
    let authorizer = Authorizer::new(config)?;
    Ok(configured_metrics_router(
        pool,
        config.health_timeout,
        Some(authorizer),
        config.allowed_origins.clone(),
        config.metrics_token.clone(),
    ))
}

pub(crate) fn configured_router(
    pool: PgPool,
    health_timeout: Duration,
    authorizer: Option<Authorizer>,
    allowed_origins: Vec<String>,
) -> Router {
    configured_metrics_router(pool, health_timeout, authorizer, allowed_origins, None)
}

pub(crate) fn configured_metrics_router(
    pool: PgPool,
    health_timeout: Duration,
    authorizer: Option<Authorizer>,
    allowed_origins: Vec<String>,
    metrics_token: Option<String>,
) -> Router {
    let state = AppState {
        metrics_token,
        pool,
        health_timeout,
        authorizer,
        allowed_origins,
    };
    Router::new()
        .route("/stores-api/internal/metrics", get(private_metrics))
        .route("/stores-api/health/live", get(live))
        .route("/stores-api/health/ready", get(ready))
        .route("/stores-api/health", get(ready))
        .route("/stores-api/stores", get(store_list).post(store_create))
        .route(
            "/stores-api/stores/{id}",
            get(store_detail).patch(store_update),
        )
        .route(
            "/stores-api/stores/{id}/staff",
            get(staff_list).post(staff_assign),
        )
        .route(
            "/stores-api/stores/{store}/staff/{link}",
            axum::routing::delete(staff_unassign),
        )
        .route(
            "/stores-api/stores/{id}/daily-reports",
            get(report_list).post(report_create),
        )
        .route("/stores-api/daily-reports/summary", get(report_summary))
        .route(
            "/stores-api/daily-reports/{id}",
            get(report_detail).patch(report_update),
        )
        .route("/stores-api/members", get(member_list))
        .route("/stores-api/organizations", get(organization_list))
        .route("/stores-api/organizations/{id}", get(organization_detail))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state.clone())
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state, request_metadata))
}

enum Endpoint {
    StoreList,
    StoreCreate,
    StoreDetail(String),
    StoreUpdate(String),
    StaffList(String),
    StaffAssign,
    StaffUnassign,
    ReportList(String),
    ReportCreate(String),
    ReportDetail(String),
    ReportUpdate(String),
    ReportSummary,
    Members,
    Organizations,
    Organization(String),
}

impl Endpoint {
    fn authorization(&self, query: Option<&str>) -> Result<(Resource, Action), ApiError> {
        Ok(match self {
            Self::StoreList | Self::StoreDetail(_) => (Resource::Store, Action::Read),
            Self::StoreCreate => (Resource::Store, Action::Create),
            Self::StoreUpdate(_) => (Resource::Store, Action::Update),
            Self::StaffList(_) => (Resource::Staff, Action::Read),
            Self::StaffAssign | Self::StaffUnassign => (Resource::Staff, Action::Read),
            Self::ReportList(_) | Self::ReportDetail(_) | Self::ReportSummary => {
                (Resource::DailyReport, Action::Read)
            }
            Self::ReportCreate(_) => (Resource::DailyReport, Action::Create),
            Self::ReportUpdate(_) => (Resource::DailyReport, Action::Update),
            Self::Members => member_pair(query)?,
            Self::Organizations | Self::Organization(_) => {
                let query = organization_query(query, matches!(self, Self::Organization(_)))?;
                (query.resource, query.action)
            }
        })
    }

    fn writes(&self) -> bool {
        matches!(
            self,
            Self::StoreCreate
                | Self::StoreUpdate(_)
                | Self::ReportCreate(_)
                | Self::ReportUpdate(_)
        )
    }

    fn creates(&self) -> bool {
        matches!(self, Self::StoreCreate | Self::ReportCreate(_))
    }
}

fn member_pair(query: Option<&str>) -> Result<(Resource, Action), ApiError> {
    crate::directory::purpose(query)
}

async fn dispatch(
    state: AppState,
    request: Request,
    endpoint: Endpoint,
) -> Result<Response, ApiError> {
    let token = bearer(request.headers())?.to_owned();
    let authorizer = state
        .authorizer
        .as_ref()
        .ok_or_else(|| ApiError::new(503, "AUTHORIZATION_UNAVAILABLE", "应用授权配置暂不可用"))?;
    let query = request.uri().query().map(str::to_owned);
    let (resource, action) = endpoint.authorization(query.as_deref())?;
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|id| id.0.clone())
        .unwrap_or_else(|| crate::domain::new_id().to_string());
    // Authorize before parsing business input or acquiring a business connection.
    let mut auth = authorizer
        .authorize(&token, resource, action, &request_id)
        .await?;
    // Retired routes authenticate against the remaining member-view permission,
    // then explain the platform-owned workflow without reading legacy links.
    if matches!(endpoint, Endpoint::StaffAssign | Endpoint::StaffUnassign) {
        return Err(ApiError::new(
            410,
            "PLATFORM_MEMBERSHIP_MANAGED",
            "门店成员由平台组织管理统一维护，请在平台调整组织成员",
        ));
    }
    if matches!(
        endpoint,
        Endpoint::Organizations | Endpoint::Organization(_)
    ) {
        let parsed = organization_query(
            query.as_deref(),
            matches!(endpoint, Endpoint::Organization(_)),
        )?;
        let id = if let Endpoint::Organization(id) = &endpoint {
            let id = crate::domain::uuid(id)?;
            // This public directory is used to choose initialization/member
            // targets. Internal existing-row metadata resolution is separate.
            auth.require_organization(id)?;
            Some(id)
        } else {
            None
        };
        let result = authorizer
            .organization_directory(&auth, &token, &parsed, id, &request_id)
            .await?;
        return Ok(Json(result).into_response());
    }
    let body = if endpoint.writes() {
        let mut types = request.headers().get_all("content-type").iter();
        let content_type = types.next().and_then(|value| value.to_str().ok());
        if types.next().is_some()
            || content_type
                .and_then(|value| value.split(';').next())
                .map(str::trim)
                != Some("application/json")
        {
            return Err(ApiError::new(415, "JSON_REQUIRED", "写入请求必须使用JSON"));
        }
        let bytes = to_bytes(request.into_body(), BODY_LIMIT)
            .await
            .map_err(|_| ApiError::new(413, "BODY_TOO_LARGE", "请求正文过大或不完整"))?;
        serde_json::from_slice(&bytes).map_err(|_| ApiError::validation())?
    } else {
        Value::Null
    };
    // SELF metadata resolution follows existing authorized business rows. The
    // platform collection deliberately lists only the selected organization.
    // Finish this read before any HTTP so no pool connection/lock spans it.
    let self_targets = if auth.scope() == Scope::SelfOnly {
        if matches!(
            endpoint,
            Endpoint::StoreCreate
                | Endpoint::ReportCreate(_)
                | Endpoint::StaffList(_)
                | Endpoint::Members
        ) {
            Some(vec![auth.organization_id()])
        } else {
            let table = if resource == Resource::DailyReport {
                "public.store_daily_reports"
            } else {
                "public.stores"
            };
            let mut sql = sqlx::QueryBuilder::<sqlx::Postgres>::new(format!(
                "SELECT DISTINCT organization_id FROM {table} WHERE "
            ));
            auth.append_scope(&mut sql);
            let ids = sql
                .build_query_scalar::<Uuid>()
                .fetch_all(&state.pool)
                .await
                .map_err(|_| ApiError::database_unavailable())?;
            auth.assert_fresh()?;
            Some(ids)
        }
    } else {
        None
    };
    let organizations = authorizer
        .store_organizations(&auth, &token, self_targets.as_deref(), &request_id)
        .await?;
    auth.bind_store_organizations(organizations.ids(), organizations.deadline())?;
    let call = Call {
        pool: &state.pool,
        auth: &auth,
        authorizer,
        token: &token,
        request_id: &request_id,
        organizations: &organizations,
    };
    let status = if endpoint.creates() {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let value = match endpoint {
        Endpoint::StoreList => crate::stores::list(&call, query.as_deref()).await?,
        Endpoint::StoreCreate => crate::stores::create(&call, body).await?,
        Endpoint::StoreDetail(id) => crate::stores::detail(&call, &id).await?,
        Endpoint::StoreUpdate(id) => crate::stores::update(&call, &id, body).await?,
        Endpoint::StaffList(id) => crate::staff::list(&call, &id, query.as_deref()).await?,
        Endpoint::StaffAssign
        | Endpoint::StaffUnassign
        | Endpoint::Organizations
        | Endpoint::Organization(_) => unreachable!("handled before business dispatch"),
        Endpoint::ReportList(id) => crate::reports::list(&call, &id, query.as_deref()).await?,
        Endpoint::ReportCreate(id) => crate::reports::create(&call, &id, body).await?,
        Endpoint::ReportDetail(id) => crate::reports::detail(&call, &id).await?,
        Endpoint::ReportUpdate(id) => crate::reports::update(&call, &id, body).await?,
        Endpoint::ReportSummary => crate::reports::summary(&call, query.as_deref()).await?,
        Endpoint::Members => crate::directory::list(&call, query.as_deref()).await?,
    };
    // Each business function checks freshness after reading/before committing.
    // Do not misclassify an acknowledged commit by reauthorizing here.
    Ok((status, Json(value)).into_response())
}

macro_rules! no_path {
    ($name:ident, $endpoint:ident) => {
        async fn $name(
            State(state): State<AppState>,
            request: Request,
        ) -> Result<Response, ApiError> {
            dispatch(state, request, Endpoint::$endpoint).await
        }
    };
}
macro_rules! one_path {
    ($name:ident, $endpoint:ident) => {
        async fn $name(
            State(state): State<AppState>,
            path: Result<Path<String>, PathRejection>,
            request: Request,
        ) -> Result<Response, ApiError> {
            let Path(id) = path.map_err(|_| ApiError::validation())?;
            dispatch(state, request, Endpoint::$endpoint(id)).await
        }
    };
}

no_path!(store_list, StoreList);
no_path!(store_create, StoreCreate);
no_path!(report_summary, ReportSummary);
no_path!(member_list, Members);
no_path!(organization_list, Organizations);
one_path!(organization_detail, Organization);
one_path!(store_detail, StoreDetail);
one_path!(store_update, StoreUpdate);
one_path!(staff_list, StaffList);
one_path!(report_list, ReportList);
one_path!(report_create, ReportCreate);
one_path!(report_detail, ReportDetail);
one_path!(report_update, ReportUpdate);

async fn staff_unassign(
    State(state): State<AppState>,
    path: Result<Path<(String, String)>, PathRejection>,
    request: Request,
) -> Result<Response, ApiError> {
    let Path((_store, _link)) = path.map_err(|_| ApiError::validation())?;
    dispatch(state, request, Endpoint::StaffUnassign).await
}

async fn staff_assign(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Result<Response, ApiError> {
    let Path(_store) = path.map_err(|_| ApiError::validation())?;
    dispatch(state, request, Endpoint::StaffAssign).await
}

fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    let mut values = headers.get_all("authorization").iter();
    let value = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::invalid_token)?;
    if values.next().is_some() || value.len() > 8192 {
        return Err(ApiError::invalid_token());
    }
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty() && !token.chars().any(char::is_whitespace))
        .ok_or_else(ApiError::invalid_token)
}

async fn live() -> Json<Value> {
    Json(json!({"status":"ok","service":"stores-api","scope":"process"}))
}

async fn ready(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    tokio::time::timeout(state.health_timeout, crate::migrations::verify(&state.pool))
        .await
        .map_err(|_| ApiError::database_unavailable())?
        .map_err(|_| ApiError::database_unavailable())?;
    Ok(Json(
        json!({"status":"ok","service":"stores-api","scope":"business-database-schema"}),
    ))
}

async fn not_found() -> ApiError {
    ApiError::new(404, "ROUTE_NOT_FOUND", "接口不存在")
}

async fn method_not_allowed() -> ApiError {
    ApiError::new(405, "METHOD_NOT_ALLOWED", "请求方法不支持")
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            Json(json!({"code":self.code(),"message":self.message()})),
        )
            .into_response()
    }
}

fn canonical_request_id(headers: &HeaderMap) -> String {
    let mut values = headers.get_all("x-request-id").iter();
    let first = values.next().and_then(|value| value.to_str().ok());
    if values.next().is_none() {
        if let Some(id) = first.and_then(|raw| crate::domain::uuid(raw).ok()) {
            return id.to_string();
        }
    }
    crate::domain::new_id().to_string()
}

async fn request_metadata(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_timer = crate::telemetry::registry().timer(operational_telemetry::Stage::Request);
    let request_id = canonical_request_id(request.headers());
    let mut origin_values = request.headers().get_all("origin").iter();
    let first_origin = origin_values.next();
    let origin = first_origin
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let origin_valid = origin_values.next().is_none()
        && (first_origin.is_none()
            || origin
                .as_ref()
                .is_some_and(|value| state.allowed_origins.contains(value)));
    let allowed = origin_valid.then_some(origin.as_deref()).flatten();
    let mut response = if !origin_valid {
        ApiError::new(403, "ORIGIN_DENIED", "请求来源不在允许范围").into_response()
    } else if request.method() == Method::OPTIONS {
        let headers = request.headers();
        let methods: Vec<_> = headers
            .get_all("access-control-request-method")
            .iter()
            .collect();
        let methods_valid = methods.len() == 1
            && methods[0]
                .to_str()
                .is_ok_and(|value| matches!(value, "GET" | "HEAD" | "POST" | "PATCH" | "DELETE"));
        let requested: Vec<_> = headers
            .get_all("access-control-request-headers")
            .iter()
            .collect();
        let headers_valid = requested.len() <= 1
            && requested.first().is_none_or(|value| {
                value.to_str().is_ok_and(|text| {
                    text.split(',').all(|name| {
                        matches!(
                            name.trim().to_ascii_lowercase().as_str(),
                            "authorization" | "content-type" | "x-request-id"
                        )
                    })
                })
            });
        if allowed.is_some() && methods_valid && headers_valid {
            let mut response = StatusCode::NO_CONTENT.into_response();
            response.headers_mut().insert(
                "access-control-allow-methods",
                HeaderValue::from_static("GET, HEAD, POST, PATCH, DELETE, OPTIONS"),
            );
            response.headers_mut().insert(
                "access-control-allow-headers",
                HeaderValue::from_static("Authorization, Content-Type, X-Request-Id"),
            );
            response
        } else {
            ApiError::new(403, "ORIGIN_DENIED", "预检请求不在允许范围").into_response()
        }
    } else {
        request
            .extensions_mut()
            .insert(RequestId(request_id.clone()));
        next.run(request).await
    };
    let headers = response.headers_mut();
    // UUID formatting always produces a legal header value.
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        headers.insert("x-request-id", value);
    }
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "vary",
        HeaderValue::from_static(
            "Origin, Access-Control-Request-Method, Access-Control-Request-Headers",
        ),
    );
    if let Some(origin) = allowed {
        if let Ok(value) = HeaderValue::from_str(origin) {
            headers.insert("access-control-allow-origin", value);
            headers.insert(
                "access-control-expose-headers",
                HeaderValue::from_static("X-Request-Id"),
            );
        }
    }
    request_timer.finish(operational_telemetry::Outcome::http(
        response.status().as_u16(),
    ));
    response
}

async fn private_metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    crate::telemetry::metrics(&state.pool, state.metrics_token.as_deref(), &headers).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn app() -> Router {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(25))
            .connect_lazy("postgresql://stores_unit:synthetic_password@127.0.0.1:1/stores_unit")
            .unwrap();
        configured_router(
            pool,
            Duration::from_millis(20),
            None,
            vec!["https://platform.example.test".into()],
        )
    }

    #[tokio::test]
    async fn liveness_is_process_only_and_readiness_fails_closed() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/stores-api/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let response = tokio::time::timeout(
            Duration::from_secs(1),
            app().oneshot(
                Request::builder()
                    .uri("/stores-api/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("synthetic_password"));
    }

    #[tokio::test]
    async fn qiankun_host_preflight_has_metadata_without_credentials() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/stores-api/stores")
                    .header("origin", "https://platform.example.test")
                    .header("access-control-request-method", "POST")
                    .header(
                        "access-control-request-headers",
                        "authorization,content-type,x-request-id",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "https://platform.example.test"
        );
        assert!(!response
            .headers()
            .contains_key("access-control-allow-credentials"));
        assert!(crate::domain::uuid(response.headers()["x-request-id"].to_str().unwrap()).is_ok());
    }

    #[tokio::test]
    async fn qiankun_host_write_reaches_bearer_authentication_while_opaque_origin_is_denied() {
        for (origin, status, code) in [
            ("https://platform.example.test", 401, "INVALID_APP_TOKEN"),
            ("null", 403, "ORIGIN_DENIED"),
            ("https://outside.invalid", 403, "ORIGIN_DENIED"),
        ] {
            let response = app()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/stores-api/stores")
                        .header("origin", origin)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(
                response
                    .headers()
                    .get("access-control-allow-origin")
                    .is_some(),
                status == 401
            );
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap()["code"],
                code
            );
        }
    }

    #[tokio::test]
    async fn rejected_origin_and_unknown_route_keep_safe_metadata() {
        for (uri, origin, status) in [
            ("/stores-api/stores", "https://outside.invalid", 403),
            ("/stores-api/stores", "null", 403),
            ("/outside", "https://platform.example.test", 404),
        ] {
            let response = app()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("origin", origin)
                        .header("x-request-id", "not-a-uuid-synthetic-secret")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_ne!(
                response.headers()["x-request-id"],
                "not-a-uuid-synthetic-secret"
            );
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-secret"));
        }
    }

    #[test]
    fn cookie_is_not_a_bearer_and_ambiguous_credentials_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_static("access_token=synthetic-token"),
        );
        assert_eq!(bearer(&headers).unwrap_err().status(), 401);
        for value in [
            "",
            "Bearer ",
            "bearer value",
            "Bearer two values",
            "Bearer\tvalue",
        ] {
            headers.insert("authorization", HeaderValue::from_str(value).unwrap());
            assert_eq!(bearer(&headers).unwrap_err().status(), 401);
        }
        headers.insert("authorization", HeaderValue::from_static("Bearer one"));
        headers.append("authorization", HeaderValue::from_static("Bearer two"));
        assert_eq!(bearer(&headers).unwrap_err().status(), 401);
    }

    #[test]
    fn request_identifiers_are_non_nil_and_not_ambiguous() {
        let valid = "8c35987c-d35a-73fd-9d0b-54ba6bf09801";
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static(valid));
        assert_eq!(canonical_request_id(&headers), valid);
        headers.append("x-request-id", HeaderValue::from_static(valid));
        assert_ne!(canonical_request_id(&headers), valid);
        for value in [
            "00000000-0000-0000-0000-000000000000",
            "8c35987c-d35a-43fd-9d0b-54ba6bf09801", // Intentional legacy v4 metadata.
            "8c35987c-d35a-73fd-cd0b-54ba6bf09801", // Wrong variant despite v7.
            "8c35987cd35a43fd9d0b54ba6bf09801",
            "unsafe-input",
        ] {
            headers.clear();
            headers.insert("x-request-id", HeaderValue::from_str(value).unwrap());
            let actual = canonical_request_id(&headers);
            assert_ne!(actual, value);
            assert!(crate::domain::uuid(&actual).is_ok());
        }
    }

    #[test]
    fn member_routes_only_authorize_registered_server_pairs() {
        assert!(member_pair(Some("resource=store&action=create")).is_ok());
        assert!(member_pair(Some("resource=store-staff&action=read")).is_ok());
        for query in [
            "resource=store&action=read",
            "resource=store-staff&action=assign",
            "resource=store-staff&action=unassign",
            "resource=store&resource=store&action=create",
            "resource=store&action=update&action=create",
        ] {
            assert_eq!(member_pair(Some(query)).unwrap_err().status(), 400);
        }
    }
}
