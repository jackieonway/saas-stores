use serde_json::{json, Value};
use uuid::Uuid;

use super::*;

#[path = "test_http.rs"]
pub(crate) mod test_http;

const USER: &str = "11111111-1111-7111-8111-111111111111";
const TENANT: &str = "22222222-2222-7222-8222-222222222222";
const ORG: &str = "33333333-3333-7333-8333-333333333333";
const CHILD: &str = "44444444-4444-7444-8444-444444444444";

fn context(now: i64, resource: Resource, action: Action) -> Value {
    json!({"contractVersion":"1.0","appCode":"stores","contextVersion":"a".repeat(64),
        "expiresAt":chrono::DateTime::from_timestamp_millis(now+20_000).unwrap().to_rfc3339(),
        "user":{"id":USER,"username":"clerk","displayName":"员工"},
        "tenant":{"id":TENANT},"organization":{"id":ORG,"name":"门店"},
        "application":{"id":"55555555-5555-7555-8555-555555555555"},
        "permissions":[format!("{}:{}",resource.as_str(),action.as_str())],
        "dataGrants":[{"resource":resource.as_str(),"action":action.as_str(),"tenantId":TENANT,
            "subjectUserId":USER,"scope":"SELF","organizationIds":[]}]})
}

pub(super) fn resource_reply(now: i64, resource: Resource, action: Action) -> Value {
    json!({"contractVersion":"resource-authorization/1","subjectMode":"USER",
        "issuer":"urn:saas:platform:internal-capabilities","tokenUse":"app_session",
        "resource":"stores","action":action.as_str(),"audience":"stores",
        "targetApplicationId":"55555555-5555-7555-8555-555555555555",
        "expiresAt":chrono::DateTime::from_timestamp_millis(now+20_000).unwrap().to_rfc3339(),
        "principal":{"userId":USER,"membershipId":"66666666-6666-7666-8666-666666666666","memberType":"EMPLOYEE",
            "identityVersion":"7","accountVersion":"3","tenantId":TENANT,"organizationId":ORG},
        "introspection":{"active":true,"aud":"stores","token_type":"Bearer","token_use":"app_session",
            "iss":"urn:saas:platform:internal-capabilities","scope":"app:access","client_id":"stores","exp":now/1000+120},
        "context":context(now,resource,action)})
}

#[test]
fn canonical_user_principal_is_separate_from_the_public_opaque_subject() {
    let now = policy::now_millis();
    let mut reply = resource_reply(now, Resource::Store, Action::Read);
    let client = "77777777-7777-7777-8777-777777777777";
    reply["issuer"] = json!("https://identity.example.test");
    reply["tokenUse"] = json!("access_token");
    reply["clientId"] = json!(client);
    reply["sourceApplicationId"] = json!("88888888-8888-7888-8888-888888888888");
    reply["introspection"]["iss"] = reply["issuer"].clone();
    reply["introspection"]["token_use"] = reply["tokenUse"].clone();
    reply["introspection"]["client_id"] = json!(client);
    reply["introspection"]["scope"] = json!("stores:read");
    reply["introspection"]["sub"] = json!("opaque-pairwise-subject-not-a-platform-id");
    let authorized = policy::validate_authorization(
        &reply,
        Resource::Store,
        Action::Read,
        now,
        Some("https://identity.example.test"),
        Some("stores:read"),
    )
    .unwrap();
    assert_eq!(authorized.user_id().to_string(), USER);
    assert!(policy::validate_authorization(
        &reply,
        Resource::Store,
        Action::Read,
        now,
        None,
        Some("stores:read")
    )
    .is_err());
    assert!(policy::validate_authorization(
        &reply,
        Resource::Store,
        Action::Read,
        now,
        Some("https://other-identity.example.test"),
        Some("stores:read")
    )
    .is_err());
    assert!(policy::validate_authorization(
        &reply,
        Resource::Store,
        Action::Read,
        now,
        Some("https://identity.example.test"),
        None
    )
    .is_err());
    let mut wrong_scope = reply.clone();
    wrong_scope["introspection"]["scope"] = json!("openid");
    assert!(policy::validate_authorization(
        &wrong_scope,
        Resource::Store,
        Action::Read,
        now,
        Some("https://identity.example.test"),
        Some("stores:read")
    )
    .is_err());
    reply["principal"]["userId"] = reply["introspection"]["sub"].clone();
    assert!(policy::validate_authorization(
        &reply,
        Resource::Store,
        Action::Read,
        now,
        Some("https://identity.example.test"),
        Some("stores:read")
    )
    .is_err());
}

#[test]
fn resource_reply_rejects_wrong_authority_subject_scope_and_original_binding() {
    let now = policy::now_millis();
    let baseline = resource_reply(now, Resource::Store, Action::Read);
    policy::validate_authorization(&baseline, Resource::Store, Action::Read, now, None, None)
        .unwrap();
    for (pointer, value) in [
        ("/contractVersion", json!("1.0")),
        ("/subjectMode", json!("ACCOUNT_ONLY")),
        ("/issuer", json!("https://identity.example.test")),
        ("/tokenUse", json!("id_token")),
        ("/resource", json!("work-orders")),
        ("/action", json!("update")),
        ("/audience", json!("work-orders")),
        ("/targetApplicationId", json!(CHILD)),
        ("/principal/userId", json!(CHILD)),
        ("/principal/tenantId", json!(CHILD)),
        ("/principal/organizationId", json!(CHILD)),
        ("/principal/membershipId", json!(Uuid::nil())),
        ("/principal/identityVersion", json!("0")),
        ("/principal/accountVersion", json!("03")),
        ("/principal/accountVersion", json!("9223372036854775808")),
        ("/introspection/active", json!(false)),
        ("/introspection/token_use", json!("access_token")),
        ("/introspection/iss", json!("https://identity.example.test")),
        ("/introspection/scope", json!("openid")),
        ("/introspection/client_id", json!("public-client")),
        ("/introspection/exp", json!(now / 1000)),
    ] {
        let mut changed = baseline.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(
            policy::validate_authorization(
                &changed,
                Resource::Store,
                Action::Read,
                now,
                None,
                None
            )
            .is_err(),
            "{pointer}"
        );
    }
    for mode in ["MACHINE", "CUSTOMER"] {
        let mut changed = baseline.clone();
        if mode == "MACHINE" {
            changed["subjectMode"] = json!(mode);
        } else {
            changed["principal"]["memberType"] = json!(mode);
        }
        assert_eq!(
            policy::validate_authorization(
                &changed,
                Resource::Store,
                Action::Read,
                now,
                None,
                None
            )
            .err()
            .unwrap()
            .status(),
            403
        );
    }
    let mut changed = baseline.clone();
    changed["principal"]["untrustedHuman"] = json!(true);
    assert!(policy::validate_authorization(
        &changed,
        Resource::Store,
        Action::Read,
        now,
        None,
        None
    )
    .is_err());
}

#[test]
fn a_fresh_checkpoint_preserves_an_earlier_deadline_for_the_same_original_authority() {
    let now = policy::now_millis();
    let original = resource_reply(now, Resource::Store, Action::Update);
    let first =
        policy::validate_authorization(&original, Resource::Store, Action::Update, now, None, None)
            .unwrap();
    let mut shortened = original.clone();
    let end = chrono::DateTime::from_timestamp_millis(now + 500)
        .unwrap()
        .to_rfc3339();
    shortened["expiresAt"] = json!(end);
    shortened["context"]["expiresAt"] = shortened["expiresAt"].clone();
    let latest = policy::validate_authorization(
        &shortened,
        Resource::Store,
        Action::Update,
        now,
        None,
        None,
    )
    .unwrap();
    assert!(first.same_authority(&latest));
    first.assert_at(now + 500, Instant::now()).unwrap();
    assert!(latest.assert_at(now + 500, Instant::now()).is_err());
}

fn authorized(
    ctx: &Value,
    resource: Resource,
    action: Action,
    now: i64,
) -> Result<Authorized, ApiError> {
    let mut reply = resource_reply(now, resource, action);
    reply["context"] = ctx.clone();
    policy::validate_authorization(&reply, resource, action, now, None, None)
}

#[test]
fn cross_application_tokens_and_malformed_resource_introspection_are_rejected() {
    let now = 1_700_000_000_000;
    for (field, replacement) in [
        ("active", json!(false)),
        ("active", json!("true")),
        ("aud", json!("work-orders")),
        ("aud", json!(["stores"])),
        ("token_type", json!("bearer")),
        ("exp", json!(now / 1000)),
        ("exp", json!(1.25)),
        ("exp", json!(9_007_199_254_740_992_u64)),
        ("exp", json!("9999999999")),
    ] {
        let mut value = resource_reply(now, Resource::Store, Action::Read);
        value["introspection"][field] = replacement;
        assert!(
            policy::validate_authorization(&value, Resource::Store, Action::Read, now, None, None)
                .is_err(),
            "{field}"
        );
    }
}

#[test]
fn every_typed_platform_identity_rejects_legacy_versions_and_wrong_variants() {
    let now = policy::now_millis();
    for raw in [
        "11111111-1111-4111-8111-111111111111", // Intentional legacy v4 negative fixture.
        "11111111-1111-7111-c111-111111111111", // Wrong variant despite version 7.
        "not-a-uuid",
    ] {
        for field in ["userId", "tenantId", "organizationId", "membershipId"] {
            let mut value = resource_reply(now, Resource::Store, Action::Read);
            value["principal"][field] = json!(raw);
            assert!(
                policy::validate_authorization(
                    &value,
                    Resource::Store,
                    Action::Read,
                    now,
                    None,
                    None
                )
                .is_err(),
                "{field}: {raw}"
            );
        }
        for pointer in [
            "/user/id",
            "/tenant/id",
            "/organization/id",
            "/application/id",
            "/dataGrants/0/tenantId",
            "/dataGrants/0/subjectUserId",
        ] {
            let mut value = context(now, Resource::Store, Action::Read);
            *value.pointer_mut(pointer).unwrap() = json!(raw);
            assert!(
                authorized(&value, Resource::Store, Action::Read, now).is_err(),
                "{pointer}: {raw}"
            );
        }
        for field in ["organizationScopeIds", "dataGrants"] {
            let mut value = context(now, Resource::Store, Action::Read);
            if field == "organizationScopeIds" {
                value[field] = json!([ORG, raw]);
            } else {
                value["dataGrants"][0]["organizationIds"] = json!([ORG, raw]);
            }
            assert!(
                authorized(&value, Resource::Store, Action::Read, now).is_err(),
                "{field}: {raw}"
            );
        }
    }
}

#[test]
fn exact_resource_action_permission_and_single_identity_bound_grant_are_required() {
    let now = policy::now_millis();
    for (resource, action) in [
        (Resource::Store, Action::Read),
        (Resource::Store, Action::Create),
        (Resource::Store, Action::Update),
        (Resource::Staff, Action::Read),
        (Resource::DailyReport, Action::Read),
        (Resource::DailyReport, Action::Create),
        (Resource::DailyReport, Action::Update),
    ] {
        let original = context(now, resource, action);
        let auth = authorized(&original, resource, action, now).unwrap();
        assert_eq!((auth.resource(), auth.action()), (resource, action));
        let mut value = original.clone();
        value["permissions"] = json!([]);
        assert_eq!(
            authorized(&value, resource, action, now)
                .err()
                .unwrap()
                .code(),
            "OPERATION_DENIED"
        );
        for (field, replacement) in [
            ("tenantId", json!(CHILD)),
            ("subjectUserId", json!(CHILD)),
            ("resource", json!("ticket")),
            ("scope", json!("STORE")),
            ("organizationIds", json!(null)),
        ] {
            let mut value = original.clone();
            value["dataGrants"][0][field] = replacement;
            assert_eq!(
                authorized(&value, resource, action, now)
                    .err()
                    .unwrap()
                    .code(),
                "DATA_DENIED",
                "{field}"
            );
        }
        let mut value = original.clone();
        value["dataGrants"]
            .as_array_mut()
            .unwrap()
            .push(original["dataGrants"][0].clone());
        assert!(authorized(&value, resource, action, now).is_err());
    }
    let value = context(now, Resource::Staff, Action::Create);
    assert!(authorized(&value, Resource::Staff, Action::Create, now).is_err());
}

#[test]
fn context_identity_expiry_and_hard_ceiling_fail_closed() {
    let now = policy::now_millis();
    for (pointer, replacement) in [
        ("/appCode", json!("work-orders")),
        ("/contractVersion", json!("2.0")),
        ("/contextVersion", json!("z".repeat(64))),
        ("/expiresAt", json!("invalid")),
        ("/user/id", json!(CHILD)),
        ("/tenant/id", json!(CHILD)),
        ("/organization/id", json!(CHILD)),
        ("/application/id", json!(null)),
    ] {
        let mut value = context(now, Resource::Store, Action::Read);
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(
            authorized(&value, Resource::Store, Action::Read, now)
                .err()
                .unwrap()
                .code(),
            if pointer == "/application/id" {
                "INVALID_APP_TOKEN"
            } else {
                "INVALID_CONTEXT"
            },
            "{pointer}"
        );
    }
    for ceiling in [
        json!([]),
        json!([CHILD]),
        json!(ORG),
        json!([ORG, null]),
        json!(vec![ORG; 10_001]),
    ] {
        let mut value = context(now, Resource::Store, Action::Read);
        value["organizationScopeIds"] = ceiling;
        assert!(authorized(&value, Resource::Store, Action::Read, now).is_err());
    }
    let value = context(now, Resource::Store, Action::Read);
    assert_eq!(
        authorized(&value, Resource::Store, Action::Read, now)
            .unwrap()
            .organization_ceiling()
            .unwrap(),
        [Uuid::parse_str(ORG).unwrap()]
    );
}

#[test]
fn row_and_sql_scopes_intersect_owner_grant_and_separate_hard_ceiling() {
    let now = policy::now_millis();
    let current = Uuid::parse_str(ORG).unwrap();
    let child = Uuid::parse_str(CHILD).unwrap();
    let actor = Uuid::parse_str(USER).unwrap();
    let outside = crate::domain::new_id();
    for (scope, ids, own_child) in [
        ("SELF", json!([ORG, CHILD]), true),
        ("ORG", json!([ORG]), false),
        ("ORG_SUBTREE", json!([ORG, CHILD]), true),
        ("TENANT", json!([]), true),
    ] {
        let mut value = context(now, Resource::DailyReport, Action::Read);
        value["organizationScopeIds"] = json!([ORG, CHILD]);
        value["dataGrants"][0]["scope"] = json!(scope);
        value["dataGrants"][0]["organizationIds"] = ids;
        let auth = authorized(&value, Resource::DailyReport, Action::Read, now).unwrap();
        auth.require_row(current, actor).unwrap();
        assert_eq!(auth.require_row(child, actor).is_ok(), own_child);
        assert!(auth.require_row(outside, actor).is_err());
        assert_eq!(auth.require_row(current, outside).is_err(), scope == "SELF");
        assert_eq!(
            auth.require_organization(child).is_ok(),
            scope == "ORG_SUBTREE" || scope == "TENANT"
        );
        let mut query = QueryBuilder::new("SELECT id FROM stores WHERE ");
        auth.append_scope(&mut query);
        let sql = query.sql();
        assert!(sql.contains("tenant_id=$1 AND organization_id=ANY($2::uuid[])"));
        assert!(!sql.contains("deleted_at"));
        for id in [USER, TENANT, ORG, CHILD] {
            assert!(!sql.contains(id));
        }
        if scope == "SELF" {
            assert!(sql.contains("owner_user_id=$3"));
        }
    }
}

#[test]
fn malformed_tokens_and_unknown_resource_actions_never_enter_platform_http() {
    for token in ["", "space token", "newline\r\ntoken", "nul\0token"] {
        assert_eq!(valid_token(token).unwrap_err().code(), "INVALID_APP_TOKEN");
    }
    assert!(valid_token(&"x".repeat(8186)).is_err());
    assert!(valid_token("valid-token").is_ok());
    assert!(Resource::parse("ticket").is_none());
    assert!(Action::parse("delete").is_none());
}

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use std::sync::{
    atomic::{AtomicU8, AtomicUsize, Ordering},
    Mutex,
};
use tokio::net::TcpListener;
const REQUEST_ID: &str = "66666666-6666-7666-8666-666666666666";
struct Fixture {
    mode: AtomicU8,
    calls: AtomicUsize,
    headers: Mutex<Vec<(String, String)>>,
}

async fn fixture_response(
    State(state): State<Arc<Fixture>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> Response {
    state.calls.fetch_add(1, Ordering::SeqCst);
    state.headers.lock().unwrap().push((
        headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        headers
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    ));
    if uri.path() == "/api/apps/stores/authorize" {
        assert_eq!(headers["content-type"], "application/x-www-form-urlencoded");
        let pairs = url::form_urlencoded::parse(&body).collect::<Vec<_>>();
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].0, "token");
        assert!(!pairs[0].1.is_empty());
    } else {
        assert!(body.is_empty());
    }
    let mode = state.mode.load(Ordering::SeqCst);
    if mode == 2 {
        return (StatusCode::FOUND, [("location", "/unexpected")], "").into_response();
    }
    if mode == 3 {
        return ([("content-type", "text/html")], "secret upstream detail").into_response();
    }
    if mode == 4 {
        return ([("content-type", "application/json")], "{broken secret").into_response();
    }
    if mode == 5 {
        return (
            [("content-type", "application/json")],
            " ".repeat(256 * 1024 + 1),
        )
            .into_response();
    }
    if mode == 6 {
        return (StatusCode::UNAUTHORIZED, "upstream secret").into_response();
    }
    if mode == 7 {
        return (StatusCode::FORBIDDEN, Json(json!({"code":"FORBIDDEN"}))).into_response();
    }
    if mode == 8 {
        return (StatusCode::INTERNAL_SERVER_ERROR, "upstream secret").into_response();
    }
    if mode == 9 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    if mode == 10 {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
    let now = policy::now_millis();
    if uri.path() == "/api/apps/stores/authorize" {
        let mut value = resource_reply(now, Resource::Store, Action::Read);
        if mode == 1 {
            value["introspection"]["active"] = json!(false);
        }
        Json(value).into_response()
    } else {
        Json(context(now, Resource::Store, Action::Read)).into_response()
    }
}

async fn fixture(
    timeout: &str,
    max: &str,
) -> (Authorizer, Arc<Fixture>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(Fixture {
        mode: AtomicU8::new(0),
        calls: AtomicUsize::new(0),
        headers: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .route("/api/apps/stores/authorize", post(fixture_response))
        .with_state(state.clone());
    let (origin, server) = if test_http::NodePeer::enabled() {
        let peer = test_http::NodePeer::router(app);
        let origin = peer.origin.clone();
        let server = tokio::spawn(async move {
            let _peer = peer;
            std::future::pending::<()>().await;
        });
        (origin, server)
    } else {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (origin, server)
    };
    let config = Config::from_pairs([
        (
            "STORES_DATABASE_URL",
            "postgresql://worker:secret@127.0.0.1/orders",
        ),
        ("PLATFORM_API_ORIGIN", &origin),
        ("STORES_ALLOWED_ORIGINS", "https://platform.example.test"),
        ("PLATFORM_CLIENT_ID", "client"),
        ("PLATFORM_AUTHORIZATION_MODE", "combined"),
        ("PLATFORM_CLIENT_SECRET", "secret"),
        ("PLATFORM_AUTH_TIMEOUT_MS", timeout),
        ("PLATFORM_AUTH_MAX_CONCURRENT", max),
    ])
    .unwrap();
    (Authorizer::new(&config).unwrap(), state, server)
}

#[tokio::test]
async fn fresh_basic_resource_authorization_is_never_positive_cached() {
    let (auth, state, server) = fixture("3000", "2").await;
    auth.authorize("app-token", Resource::Store, Action::Read, REQUEST_ID)
        .await
        .unwrap();
    let seen = state.headers.lock().unwrap().clone();
    assert_eq!(
        seen,
        [("Basic Y2xpZW50OnNlY3JldA==".into(), REQUEST_ID.into())]
    );
    state.mode.store(1, Ordering::SeqCst);
    assert_eq!(
        auth.authorize("app-token", Resource::Store, Action::Read, REQUEST_ID)
            .await
            .err()
            .unwrap()
            .code(),
        "INVALID_APP_TOKEN"
    );
    assert_eq!(state.calls.load(Ordering::SeqCst), 2);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn upstream_status_body_redirect_json_and_capacity_fail_closed() {
    let (auth, state, server) = fixture("3000", "2").await;
    for (mode, code) in [
        (2, "PLATFORM_UNAVAILABLE"),
        (3, "PLATFORM_UNAVAILABLE"),
        (4, "PLATFORM_UNAVAILABLE"),
        (5, "PLATFORM_UNAVAILABLE"),
        (6, "INVALID_APP_TOKEN"),
        (7, "DATA_DENIED"),
        (8, "PLATFORM_UNAVAILABLE"),
    ] {
        state.mode.store(mode, Ordering::SeqCst);
        let error = auth
            .authorize("app-token", Resource::Store, Action::Read, REQUEST_ID)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code(), code);
        assert!(!format!("{error:?} {error}").contains("secret"));
    }
    assert_eq!(state.calls.load(Ordering::SeqCst), 7);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn shared_authorization_budget_and_concurrency_bound_do_not_queue_requests() {
    let (auth, state, server) = fixture("100", "1").await;
    let permit = auth.concurrent.clone().acquire_owned().await.unwrap();
    assert_eq!(
        auth.authorize("token", Resource::Store, Action::Read, REQUEST_ID)
            .await
            .err()
            .unwrap()
            .code(),
        "PLATFORM_UNAVAILABLE"
    );
    assert_eq!(state.calls.load(Ordering::SeqCst), 0);
    drop(permit);
    state.mode.store(9, Ordering::SeqCst);
    let started = std::time::Instant::now();
    assert_eq!(
        auth.authorize("token", Resource::Store, Action::Read, REQUEST_ID)
            .await
            .err()
            .unwrap()
            .code(),
        "PLATFORM_UNAVAILABLE"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(auth.concurrent.available_permits(), 1);
    state.mode.store(10, Ordering::SeqCst);
    assert_eq!(
        auth.authorize("token", Resource::Store, Action::Read, REQUEST_ID)
            .await
            .err()
            .unwrap()
            .code(),
        "PLATFORM_UNAVAILABLE"
    );
    assert_eq!(auth.concurrent.available_permits(), 1);
    server.abort();
    let _ = server.await;
}

#[test]
fn protected_application_requires_platform_configuration() {
    let config = Config::from_pairs([
        (
            "STORES_DATABASE_URL",
            "postgresql://worker:secret@127.0.0.1/orders",
        ),
        ("STORES_ALLOWED_ORIGINS", "https://platform.example.test"),
    ])
    .unwrap();
    let error = Authorizer::new(&config).err().unwrap();
    assert_eq!(
        error.to_string(),
        "platform authorization configuration is required"
    );
    assert!(!format!("{error:?}").contains("secret"));
}
