use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::json;

use super::*;
use crate::Config;

const ACTOR: Uuid = Uuid::from_u128(0x01900000000070008000000000000000 + 1);
const TENANT: Uuid = Uuid::from_u128(0x01900000000070008000000000000000 + 2);
const PARENT: Uuid = Uuid::from_u128(0x01900000000070008000000000000000 + 3);
const CHILD: Uuid = Uuid::from_u128(0x01900000000070008000000000000000 + 4);
const REQUEST: &str = "11111111-1111-7111-8111-111111111111";

fn auth() -> Authorized {
    let now = policy::now_millis();
    let context = json!({"contractVersion":"1.0","appCode":"stores","contextVersion":"a".repeat(64),
        "expiresAt":chrono::DateTime::from_timestamp_millis(now+20_000).unwrap().to_rfc3339(),
        "user":{"id":ACTOR},"tenant":{"id":TENANT},"organization":{"id":PARENT},
        "application":{"id":Uuid::from_u128(0x01900000000070008000000000000000 + 5)},"organizationScopeIds":[PARENT,CHILD],
        "permissions":["store:read"],"dataGrants":[{"resource":"store","action":"read",
        "tenantId":TENANT,"subjectUserId":ACTOR,"scope":"SELF","organizationIds":[]}]});
    let mut reply = crate::authorization::tests::resource_reply(now, Resource::Store, Action::Read);
    reply["targetApplicationId"] = context["application"]["id"].clone();
    reply["context"] = context;
    reply["principal"]["userId"] = json!(ACTOR);
    reply["principal"]["tenantId"] = json!(TENANT);
    reply["principal"]["organizationId"] = json!(PARENT);
    policy::validate_authorization(&reply, Resource::Store, Action::Read, now, None, None).unwrap()
}

fn organization(id: Uuid) -> Value {
    json!({"contractVersion":"1.0","contextVersion":"a".repeat(64),"id":id,"tenantId":TENANT,
        "parentId":null,"name":"平台实时门店","code":"LIVE","organizationType":"STORE","status":"ENABLED",
        "path":"/root/store","province":"","city":"","district":"","township":"","village":"",
        "address":"平台地址","contactName":"","contactPhone":"","contactEmail":"","revision":"b".repeat(64),
        "expiresAt":chrono::DateTime::from_timestamp_millis(policy::now_millis()+2_000).unwrap().to_rfc3339()})
}

fn page(records: Vec<Value>, number: u32, size: u32) -> Value {
    json!({"contractVersion":"1.0","contextVersion":"a".repeat(64),"expiresAt":organization(PARENT)["expiresAt"],
        "total":records.len(),"page":number,"size":size,"records":records})
}

#[test]
fn projection_is_identity_bound_active_store_metadata_and_strips_upstream_extras() {
    let auth = auth();
    let mut record = organization(CHILD);
    record["privatePlatformField"] = json!("never-forward");
    let (_, safe, _) = validate_record(&record, &auth).unwrap();
    assert!(safe.get("privatePlatformField").is_none());
    assert_eq!(safe["name"], "平台实时门店");
    for (key, value) in [
        ("contractVersion", json!("2.0")),
        ("contextVersion", json!("c".repeat(64))),
        ("tenantId", json!(CHILD)),
        (
            "id",
            json!(Uuid::from_u128(0x01900000000070008000000000000000 + 99)),
        ),
        ("organizationType", json!("DEPARTMENT")),
        ("status", json!("DISABLED")),
        ("revision", json!("1")),
        ("parentId", json!(Uuid::nil())),
        ("address", Value::Null),
        ("contactEmail", json!("x\0y")),
        (
            "expiresAt",
            json!(
                chrono::DateTime::from_timestamp_millis(policy::now_millis() + 30_000)
                    .unwrap()
                    .to_rfc3339()
            ),
        ),
        (
            "expiresAt",
            json!(
                chrono::DateTime::from_timestamp_millis(policy::now_millis() - 1)
                    .unwrap()
                    .to_rfc3339()
            ),
        ),
    ] {
        let mut changed = record.clone();
        changed[key] = value;
        assert_eq!(
            validate_record(&changed, &auth).unwrap_err().code(),
            "PLATFORM_UNAVAILABLE",
            "{key}"
        );
    }
}

#[test]
fn self_metadata_does_not_prove_row_ownership_or_expand_directory_and_expiry_never_renews() {
    let mut auth = auth();
    validate_record(&organization(CHILD), &auth).unwrap();
    assert!(auth.require_organization(CHILD).is_err());
    auth.bind_store_organizations(vec![CHILD], Instant::now() + Duration::from_secs(1))
        .unwrap();
    auth.require_row(CHILD, ACTOR).unwrap();
    assert!(auth.require_row(CHILD, PARENT).is_err());
    assert!(auth.require_row(PARENT, ACTOR).is_err());
    let mut sql = sqlx::QueryBuilder::<sqlx::Postgres>::new("WHERE ");
    auth.append_scope(&mut sql);
    assert!(sql.sql().contains("owner_user_id="));
    assert!(auth
        .bind_store_organizations(vec![CHILD], Instant::now())
        .is_err());
    assert_eq!(
        auth.assert_fresh().unwrap_err().code(),
        "ORGANIZATION_VALIDATION_EXPIRED"
    );
}

#[test]
fn directory_query_and_pages_reject_ambiguous_purposes_counts_and_duplicate_organizations() {
    for pair in [
        "store&action=read",
        "store&action=create",
        "store&action=update",
        "store-staff&action=read",
        "store-daily-report&action=read",
        "store-daily-report&action=create",
        "store-daily-report&action=update",
    ] {
        organization_query(Some(&format!("resource={pair}")), false).unwrap();
    }
    for query in [
        "resource=store-staff&action=assign",
        "resource=store-staff&action=unassign",
        "resource=store&action=read&resource=store",
        "resource=store&action=read&organizationId=x",
        "resource=store&action=read&q=%00",
        "resource=store&action=read&size=51",
    ] {
        assert!(organization_query(Some(query), false).is_err(), "{query}");
    }
    assert!(organization_query(
        Some(&format!(
            "resource=store&action=read&q={}",
            "%20".repeat(101)
        )),
        false
    )
    .is_err());
    let valid = page(vec![organization(PARENT)], 1, 20);
    validate_page(&valid, &auth(), 1, 20).unwrap();
    for (key, value) in [
        ("total", json!(2)),
        ("page", json!(2)),
        ("size", json!(50)),
        ("contextVersion", json!("d".repeat(64))),
    ] {
        let mut changed = valid.clone();
        changed[key] = value;
        assert!(validate_page(&changed, &auth(), 1, 20).is_err());
    }
    assert!(validate_page(
        &page(vec![organization(PARENT), organization(PARENT)], 1, 20),
        &auth(),
        1,
        20
    )
    .is_err());
}

fn private_form(
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
    field_count: usize,
) -> std::collections::BTreeMap<String, String> {
    assert!(
        uri.query().is_none(),
        "directory credentials and parameters stay in the POST form"
    );
    assert_eq!(headers["authorization"], "Basic Y2xpZW50OnNlY3JldA==");
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/x-www-form-urlencoded"));
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(body).into_owned().collect();
    assert_eq!(fields.len(), field_count);
    assert_eq!(fields["token"], "app-token");
    assert_eq!(fields["resource"], "store");
    assert_eq!(fields["action"], "read");
    fields
}

fn directory(data: Value, mode: u8) -> Value {
    let mut envelope = json!({"contractVersion":"resource-directory/1","directory":"organizations","expiresAt":data["expiresAt"],"data":data});
    match mode {
        4 => envelope["directory"] = json!("members"),
        5 => {
            envelope["expiresAt"] = json!(chrono::DateTime::from_timestamp_millis(
                policy::now_millis() + 30_000
            )
            .unwrap()
            .to_rfc3339())
        }
        6 => {
            envelope["expiresAt"] = json!(chrono::DateTime::from_timestamp_millis(
                policy::now_millis() - 1
            )
            .unwrap()
            .to_rfc3339())
        }
        _ => {}
    }
    envelope
}

async fn list_response(
    State(mode): State<Arc<AtomicU8>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Json<Value> {
    let query = private_form(&headers, &uri, &body, 6);
    let id = if mode.load(Ordering::SeqCst) == 1 {
        CHILD
    } else {
        PARENT
    };
    Json(directory(
        page(
            vec![organization(id)],
            query["page"].parse().unwrap(),
            query["size"].parse().unwrap(),
        ),
        mode.load(Ordering::SeqCst),
    ))
}

async fn detail_response(
    State(mode): State<Arc<AtomicU8>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    private_form(&headers, &uri, &body, 3);
    if id == PARENT {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mode = mode.load(Ordering::SeqCst);
    if mode == 3 {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let mut record = organization(id);
    if mode == 2 {
        record["contextVersion"] = json!("f".repeat(64));
    }
    Json(directory(record, mode)).into_response()
}

#[tokio::test]
async fn self_uses_known_single_targets_skips_inactive_history_and_rejects_downgrade_and_timeout() {
    let mode = Arc::new(AtomicU8::new(0));
    let app = Router::new()
        .route(
            "/internal/platform/apps/stores/organizations",
            post(list_response),
        )
        .route(
            "/internal/platform/apps/stores/organizations/{id}",
            post(detail_response),
        )
        .with_state(mode.clone());
    let (origin, server) = if super::super::tests::test_http::NodePeer::enabled() {
        let peer = super::super::tests::test_http::NodePeer::router(app);
        let origin = peer.origin.clone();
        (
            origin,
            tokio::spawn(async move {
                let _peer = peer;
                std::future::pending::<()>().await;
            }),
        )
    } else {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        (
            origin,
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            }),
        )
    };
    let config = Config::from_pairs([
        (
            "STORES_DATABASE_URL",
            "postgresql://worker:secret@127.0.0.1/stores",
        ),
        ("PLATFORM_API_ORIGIN", origin.as_str()),
        ("PLATFORM_CLIENT_ID", "client"),
        ("PLATFORM_CLIENT_SECRET", "secret"),
        ("PLATFORM_IDENTITY_ISSUER", "https://identity.example.test"),
        ("STORES_ALLOWED_ORIGINS", "https://platform.example.test"),
        ("PLATFORM_AUTH_TIMEOUT_MS", "100"),
    ])
    .unwrap();
    let service = Authorizer::new(&config).unwrap();
    let auth = auth();
    let set = service
        .store_organizations(&auth, "app-token", Some(&[PARENT, CHILD]), REQUEST)
        .await
        .unwrap();
    assert_eq!(set.ids(), vec![CHILD]);
    assert_eq!(set.get(CHILD).unwrap()["name"], "平台实时门店");
    let query = organization_query(Some("resource=store&action=read"), false).unwrap();
    service
        .organization_directory(&auth, "app-token", &query, None, REQUEST)
        .await
        .unwrap();
    mode.store(1, Ordering::SeqCst);
    assert!(service
        .organization_directory(&auth, "app-token", &query, None, REQUEST)
        .await
        .is_err());
    mode.store(2, Ordering::SeqCst);
    assert!(service
        .store_organizations(&auth, "app-token", Some(&[CHILD]), REQUEST)
        .await
        .is_err());
    for invalid_envelope in [4, 5, 6] {
        mode.store(invalid_envelope, Ordering::SeqCst);
        assert!(
            service
                .store_organizations(&auth, "app-token", Some(&[CHILD]), REQUEST)
                .await
                .is_err(),
            "wrong directory domain and invalid outer deadlines fail closed"
        );
    }
    mode.store(3, Ordering::SeqCst);
    assert!(service
        .store_organizations(&auth, "app-token", Some(&[CHILD]), REQUEST)
        .await
        .is_err());
    assert_eq!(service.concurrent.available_permits(), 32);
    server.abort();
    let _ = server.await;
}
