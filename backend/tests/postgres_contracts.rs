//! Explicit real PostgreSQL contracts, isolated inside the owned CI server.
//! Ordinary test runs ignore database cases; the CI owner supplies its URL.

use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use sqlx::{postgres::PgPoolOptions, PgPool};
use stores_api::initialize_database;
use tokio::sync::Barrier;
use uuid::Uuid;

#[path = "../src/authorization/test_http.rs"]
mod test_http;

fn new_id() -> Uuid {
    Uuid::now_v7()
}

const FIXTURE_RESOURCE_SCOPE_BINDINGS: &str = r#"{
    "store:read":"store:read",
    "store:create":"store:create",
    "store:update":"store:update",
    "store-staff:read":"store-staff:read",
    "store-daily-report:read":"store-daily-report:read",
    "store-daily-report:create":"store-daily-report:create",
    "store-daily-report:update":"store-daily-report:update"
}"#;

fn owned_server(url: &url::Url, owner: &str, local_owner: Option<&str>) -> bool {
    let Some(local_owner) = local_owner else {
        return url.host_str() == Some(format!("{owner}-postgres").as_str());
    };
    let (Some(host), Some(port)) = (url.host_str(), url.port()) else {
        return false;
    };
    matches!(host, "127.0.0.1" | "[::1]")
        && port >= 1024
        && port != 5432
        && local_owner == format!("{owner}@{host}:{port}")
}

#[test]
fn local_database_opt_in_is_bound_to_the_owner_and_non_default_loopback_endpoint() {
    let owner = "ci-owned-contract";
    let parse =
        |host: &str| url::Url::parse(&format!("postgresql://fixture@{host}/postgres")).unwrap();
    assert!(owned_server(
        &parse("ci-owned-contract-postgres"),
        owner,
        None
    ));
    assert!(!owned_server(&parse("127.0.0.1:45432"), owner, None));
    assert!(owned_server(
        &parse("127.0.0.1:45432"),
        owner,
        Some("ci-owned-contract@127.0.0.1:45432")
    ));
    for (host, marker) in [
        ("127.0.0.1:5432", "ci-owned-contract@127.0.0.1:5432"),
        ("127.0.0.1", "ci-owned-contract@127.0.0.1:5432"),
        ("127.0.0.1:45432", "ci-other@127.0.0.1:45432"),
        ("127.0.0.1:45432", "ci-owned-contract@127.0.0.1:45433"),
        ("192.168.1.2:45432", "ci-owned-contract@192.168.1.2:45432"),
        ("localhost:45432", "ci-owned-contract@localhost:45432"),
    ] {
        assert!(!owned_server(&parse(host), owner, Some(marker)));
    }
}

async fn owned<F, Fut>(case: F)
where
    F: FnOnce(PgPool) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let raw = std::env::var("TEST_DATABASE_URL").expect("owned TEST_DATABASE_URL required");
    let owner = std::env::var("CI_TEST_POSTGRES_OWNER").expect("owned CI owner required");
    assert!(
        owner.starts_with("ci-") && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    );
    let mut url = url::Url::parse(&raw).expect("valid owned URL");
    // Never infer local ownership from an ordinary development database URL.
    let local_owner = std::env::var("STORES_TEST_LOCAL_POSTGRES_OWNER").ok();
    assert!(owned_server(&url, &owner, local_owner.as_deref()));
    assert_eq!(url.path(), "/postgres");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&raw)
        .await
        .expect("owned admin connection");
    let name = format!("phase26_stores_sql_{}", new_id().simple());
    sqlx::query(&format!("CREATE DATABASE \"{name}\" TEMPLATE template0"))
        .execute(&admin)
        .await
        .expect("create owned database");
    println!("STORES_FIXTURE_CREATED database={name}");
    url.set_path(&format!("/{name}"));
    let pool_result = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(|connection, _| {
            Box::pin(async move {
            sqlx::query("SELECT pg_catalog.set_config('statement_timeout','10s',false),pg_catalog.set_config('lock_timeout','5s',false),pg_catalog.set_config('search_path','pg_catalog,public,pg_temp',false)")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(url.as_str())
        .await;
    let pool = match pool_result {
        Ok(pool) => pool,
        Err(_) => {
            let removed = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                .execute(&admin)
                .await
                .is_ok();
            admin.close().await;
            println!("STORES_FIXTURE_SETUP_CLEANUP database_removed={removed} pool_closed=true");
            assert!(removed, "owned setup database cleanup failed");
            panic!("owned business pool setup failed");
        }
    };
    let result = tokio::spawn(case(pool.clone())).await;
    pool.close().await;
    let removed = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
        .execute(&admin)
        .await
        .is_ok();
    admin.close().await;
    println!(
        "STORES_FIXTURE_CLEANUP assertions_ok={} database_removed={removed} pool_closed=true",
        result.is_ok()
    );
    assert!(removed, "owned test database cleanup failed");
    if let Err(error) = result {
        std::panic::resume_unwind(error.into_panic());
    }
}

fn check(name: &str) {
    println!("STORES_SQL_CHECK name={name} status=PASS");
}

async fn reject(pool: &PgPool, sql: &str, code: &str) {
    let error = sqlx::query(sql)
        .execute(pool)
        .await
        .expect_err("forbidden mutation must fail");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some(code)
    );
}

async fn business_snapshot(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT pg_catalog.jsonb_build_object('stores',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.stores AS business_row),'staff',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.store_staff AS business_row),'reports',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.store_daily_reports AS business_row),'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.store_audit_events AS business_row))")
        .fetch_one(pool).await.unwrap()
}

async fn initialize_bounded(pool: &PgPool) -> Result<(), stores_api::ServiceError> {
    tokio::time::timeout(Duration::from_secs(3), initialize_database(pool))
        .await
        .expect("migration verification or recovery exceeded its three-second budget")
}

#[tokio::test]
#[ignore = "requires the owned CI PostgreSQL driver"]
async fn postgres_contracts() {
    owned(|pool| async move {
        initialize_database(&pool).await.expect("empty independent database initializes");
        initialize_database(&pool).await.expect("second start verifies immutable migration");
        let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM information_schema.tables WHERE table_schema='public' AND table_name IN ('stores','store_staff','store_daily_reports','store_audit_events')").fetch_one(&pool).await.unwrap();
        assert_eq!(tables, 4);
        check("empty_and_repeat_initialization");

        let tenant = new_id();
        let org = new_id();
        let user = new_id();
        let store = new_id();
        sqlx::query("INSERT INTO stores(id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact) VALUES($1,$2,$3,$4,'Exact store','OPEN','')").bind(store).bind(tenant).bind(org).bind(user).execute(&pool).await.unwrap();

        let gate = Arc::new(Barrier::new(2));
        let mut jobs = Vec::new();
        for _ in 0..2 {
            let pool = pool.clone();
            let gate = gate.clone();
            jobs.push(tokio::spawn(async move {
                gate.wait().await;
                let mut tx = pool.begin().await.unwrap();
                let result = sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note) VALUES($1,$2,$3,$4,$5,DATE '2026-10-18','0.10'::numeric,'')")
                    .bind(new_id()).bind(tenant).bind(store).bind(org).bind(user).execute(&mut *tx).await;
                match result {
                    Ok(_) => { tx.commit().await.unwrap(); true }
                    Err(error) => {
                        assert_eq!(error.as_database_error().and_then(|error| error.constraint()), Some("store_daily_reports_day_key"));
                        tx.rollback().await.unwrap();
                        false
                    }
                }
            }));
        }
        let mut successes = 0;
        for job in jobs { successes += u32::from(job.await.unwrap()); }
        assert_eq!(successes, 1);
        let concurrent: (i64, String) = sqlx::query_as("SELECT count(*),sum(amount)::text FROM store_daily_reports").fetch_one(&pool).await.unwrap();
        assert_eq!(concurrent, (1, "0.10".to_owned()));
        check("concurrent_daily_uniqueness_no_double_summary");

        for day in ["0001-01-01", "9999-12-31"] {
            sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,$6::date,'9999999999999999.99'::numeric)")
                .bind(new_id()).bind(tenant).bind(store).bind(org).bind(user).bind(day).execute(&pool).await.unwrap();
        }
        let exact: String = sqlx::query_scalar("SELECT sum(amount)::text FROM store_daily_reports WHERE business_date<>DATE '2026-10-18'").fetch_one(&pool).await.unwrap();
        assert_eq!(exact, "19999999999999999.98");
        let invalid_amount = sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-19','NaN'::numeric)")
            .bind(new_id()).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.expect_err("numeric NaN is not a business amount");
        assert_eq!(invalid_amount.as_database_error().and_then(|error| error.code()).as_deref(), Some("23514"));
        check("numeric_precision_and_large_exact_sum");

        reject(&pool, "UPDATE stores SET organization_id='00000000-0000-7000-8000-000000000011',version=version+1", "23514").await;
        reject(&pool, "UPDATE stores SET name='stale write'", "23514").await;
        reject(&pool, "UPDATE store_daily_reports SET business_date=business_date+1,version=version+1 WHERE business_date=DATE '2026-10-18'", "23514").await;
        reject(&pool, "UPDATE store_daily_reports SET owner_user_id='00000000-0000-7000-8000-000000000012',version=version+1", "23514").await;
        reject(&pool, "UPDATE stores SET name='old binary rename',version=version+1", "23514").await;
        let changed = sqlx::query("UPDATE stores SET business_status='CLOSED',version=version+1,updated_at=now() WHERE id=$1 AND version=1").bind(store).execute(&pool).await.unwrap().rows_affected();
        let stale = sqlx::query("UPDATE stores SET business_status='OPEN',version=version+1 WHERE id=$1 AND version=1").bind(store).execute(&pool).await.unwrap().rows_affected();
        assert_eq!((changed, stale), (1, 0));
        check("immutable_ownership_date_and_optimistic_version");

        let bad_tenant = sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-19',0)").bind(new_id()).bind(new_id()).bind(store).bind(org).bind(user).execute(&pool).await.expect_err("composite FK forbids cross tenant store");
        assert_eq!(bad_tenant.as_database_error().and_then(|error| error.code()).as_deref(), Some("23503"));
        check("composite_store_tenant_organization_fk");

        let rolled_back = new_id();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-20',1)").bind(rolled_back).bind(tenant).bind(store).bind(org).bind(user).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO store_audit_events(id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) VALUES($1,$2,$3,$4,'store-daily-report','create',$5,$6)").bind(new_id()).bind(tenant).bind(org).bind(user).bind(rolled_back).bind(new_id()).execute(&mut *tx).await.unwrap();
        tx.rollback().await.unwrap();
        let absent: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM store_daily_reports WHERE id=$1) AND NOT EXISTS(SELECT 1 FROM store_audit_events WHERE entity_id=$1)").bind(rolled_back).fetch_one(&pool).await.unwrap();
        assert!(absent);
        sqlx::query("INSERT INTO store_audit_events(id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) VALUES($1,$2,$3,$4,'store','update',$5,$6)").bind(new_id()).bind(tenant).bind(org).bind(user).bind(store).bind(new_id()).execute(&pool).await.unwrap();
        reject(&pool, "DELETE FROM store_audit_events", "23514").await;
        check("atomic_audit_rollback_and_append_only");

        sqlx::query("ALTER TABLE stores ADD COLUMN unexpected TEXT").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "readiness must reject schema drift");
        sqlx::query("ALTER TABLE stores DROP COLUMN unexpected").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored exact schema contract");
        sqlx::query("ALTER TABLE public._stores_schema_contract ADD COLUMN unexpected TEXT").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "metadata schema drift must be rejected");
        sqlx::query("ALTER TABLE public._stores_schema_contract DROP COLUMN unexpected").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored metadata schema contract");
        sqlx::query("ALTER FUNCTION public.stores_schema_fingerprint() VOLATILE").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "fingerprint definition drift must be rejected");
        sqlx::query("ALTER FUNCTION public.stores_schema_fingerprint() STABLE").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored fingerprint definition");
        sqlx::query("ALTER TABLE public.store_staff DISABLE TRIGGER store_staff_legacy_read_only").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "staff write guard drift must be rejected");
        sqlx::query("ALTER TABLE public.store_staff ENABLE TRIGGER store_staff_legacy_read_only").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored staff write guard");
        sqlx::query("ALTER FUNCTION public.stores_platform_name_immutable() STABLE").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "platform name guard definition drift must be rejected");
        sqlx::query("ALTER FUNCTION public.stores_platform_name_immutable() VOLATILE").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored platform name guard definition");
        sqlx::query("CREATE TABLE public.unexpected(id INTEGER)").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "extra public object must be rejected");
        sqlx::query("DROP TABLE public.unexpected").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored exact public inventory");
        let checksum: Vec<u8> = sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=1").fetch_one(&pool).await.unwrap();
        sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=1").bind(vec![0_u8; 48]).execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "migration checksum mismatch must stop startup");
        sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=1").bind(checksum).execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored migration checksum without a leaked advisory lock");
        check("schema_drift_and_checksum_rejected");
    }).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FixtureMemberType {
    Employee,
    Customer,
}

struct FixtureMember {
    user: Uuid,
    membership: Uuid,
    member_type: FixtureMemberType,
    enabled: bool,
    display_name: &'static str,
}

struct PlatformFixture {
    pool: PgPool,
    tenant: Uuid,
    actor: Uuid,
    actor_membership: Uuid,
    other: Uuid,
    other_membership: Uuid,
    client: Uuid,
    source_application: Uuid,
    parent: Uuid,
    active: Uuid,
    excluded: [Uuid; 2],
    self_scope: AtomicBool,
    expired: AtomicBool,
    lease_ms: AtomicU64,
    check_unlocked: AtomicBool,
    projection_ready: Mutex<Option<tokio::sync::oneshot::Sender<i64>>>,
    organization_reads: AtomicUsize,
    members: Mutex<Vec<FixtureMember>>,
    member_reads: Mutex<Vec<(String, String, Option<Uuid>)>>,
    authorization_reads: AtomicUsize,
    authorization_token_reads: Mutex<std::collections::BTreeMap<String, usize>>,
}

impl PlatformFixture {
    fn new(pool: PgPool) -> Self {
        let actor = new_id();
        let actor_membership = new_id();
        Self {
            pool,
            tenant: new_id(),
            actor,
            actor_membership,
            other: new_id(),
            other_membership: new_id(),
            client: new_id(),
            source_application: new_id(),
            parent: new_id(),
            active: new_id(),
            excluded: [new_id(), new_id()],
            self_scope: AtomicBool::new(false),
            expired: AtomicBool::new(false),
            lease_ms: AtomicU64::new(2000),
            check_unlocked: AtomicBool::new(true),
            projection_ready: Mutex::new(None),
            organization_reads: AtomicUsize::new(0),
            members: Mutex::new(vec![FixtureMember {
                user: actor,
                membership: actor_membership,
                member_type: FixtureMemberType::Employee,
                enabled: true,
                display_name: "Current member",
            }]),
            member_reads: Mutex::new(Vec::new()),
            authorization_reads: AtomicUsize::new(0),
            authorization_token_reads: Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    // This controlled HTTP peer models the platform's employee-only directory.
    // Actual membership-type authority is verified by platform PostgreSQL tests;
    // the stores client continues to consume the existing user-ID DTO.
    fn employee_members(&self) -> std::collections::BTreeMap<Uuid, serde_json::Value> {
        self.members
            .lock()
            .unwrap()
            .iter()
            .filter(|member| member.member_type == FixtureMemberType::Employee && member.enabled)
            .map(|member| {
                (
                    member.user,
                    serde_json::json!({
                        "id":member.user,"organizationId":self.active,
                        "displayName":member.display_name,"membershipId":member.membership,
                        "memberType":"EMPLOYEE","private":"must not escape"
                    }),
                )
            })
            .collect()
    }

    fn organization(&self) -> serde_json::Value {
        let now = chrono::Utc::now().timestamp_millis();
        let expires = if self.expired.load(Ordering::SeqCst) {
            now - 1
        } else {
            now + self.lease_ms.load(Ordering::SeqCst) as i64
        };
        serde_json::json!({
            "contractVersion":"1.0", "contextVersion":"a".repeat(64), "id":self.active,
            "tenantId":self.tenant,"parentId":self.parent,"organizationType":"STORE", "status":"ENABLED",
            "revision":"b".repeat(64),"expiresAt":chrono::DateTime::from_timestamp_millis(expires).unwrap().to_rfc3339(),
            "name":"Platform A", "code":"A", "path":"/parent/A", "province":"", "city":"", "district":"",
            "township":"", "village":"", "address":"", "contactName":"", "contactPhone":"", "contactEmail":""
        })
    }

    fn announce_projection(&self, record: &serde_json::Value) {
        if let Some(ready) = self.projection_ready.lock().unwrap().take() {
            let expires =
                chrono::DateTime::parse_from_rfc3339(record["expiresAt"].as_str().unwrap())
                    .unwrap()
                    .timestamp_millis();
            ready.send(expires).expect("projection expiry receiver");
        }
    }
}

async fn platform_response(
    axum::extract::State(state): axum::extract::State<Arc<PlatformFixture>>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::{response::IntoResponse, Json};
    use serde_json::json;
    use std::collections::BTreeMap;

    assert!(
        uri.query().is_none(),
        "private credentials and lookup fields stay in the POST form"
    );
    assert_eq!(headers["authorization"], "Basic Zml4dHVyZTpmaXh0dXJl");
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/x-www-form-urlencoded"));
    let fields: BTreeMap<_, _> = url::form_urlencoded::parse(&body).into_owned().collect();
    if uri.path() == "/api/apps/stores/authorize" {
        assert_eq!(fields.len(), 3);
        state.authorization_reads.fetch_add(1, Ordering::SeqCst);
        let token_read = {
            let mut reads = state.authorization_token_reads.lock().unwrap();
            let count = reads.entry(fields["token"].clone()).or_default();
            *count += 1;
            *count
        };
        if token_read > 1
            && matches!(
                fields["token"].as_str(),
                "before-commit-revoked" | "before-commit-unavailable"
            )
        {
            assert_eq!(
                state.pool.num_idle(),
                0,
                "the final source fence must run while the write transaction is still leased"
            );
        }
        // Same account, different selected membership: the customer context
        // cannot borrow that account's enabled employee identity or grants.
        if fields["token"] == "customer-token" {
            return (
                axum::http::StatusCode::FORBIDDEN,
                Json(json!({"code":"OPERATION_DENIED"})),
            )
                .into_response();
        }
        if fields["token"] == "disabled-member"
            || (fields["token"] == "before-commit-revoked" && token_read > 1)
        {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(json!({"code":"MEMBERSHIP_DISABLED"})),
            )
                .into_response();
        }
        if fields["token"] == "source-unavailable"
            || (fields["token"] == "before-commit-unavailable" && token_read > 1)
        {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"code":"SOURCE_UNAVAILABLE"})),
            )
                .into_response();
        }
        assert!(matches!(
            fields["token"].as_str(),
            "app-token"
                | "other-token"
                | "machine-token"
                | "id-token"
                | "wrong-audience"
                | "wrong-resource"
                | "wrong-scope"
                | "principal-mismatch"
                | "context-mismatch"
                | "organization-mismatch"
                | "before-commit-revoked"
                | "before-commit-unavailable"
        ));
        let actor = if fields["token"] == "other-token" {
            state.other
        } else {
            state.actor
        };
        let membership = if fields["token"] == "other-token" {
            state.other_membership
        } else {
            state.actor_membership
        };
        let resource = &fields["resource"];
        let action = &fields["action"];
        let scope = if state.self_scope.load(Ordering::SeqCst) {
            "SELF"
        } else {
            "TENANT"
        };
        let now = chrono::Utc::now().timestamp_millis();
        let mut reply = json!({
            "contractVersion":"resource-authorization/1","issuer":"https://identity.example.test",
            "tokenUse":"access_token","subjectMode":"USER","resource":"stores","action":action,
            "audience":"stores","targetApplicationId":"55555555-5555-7555-8555-555555555555",
            "clientId":state.client,"sourceApplicationId":state.source_application,
            "expiresAt":chrono::DateTime::from_timestamp_millis(now+20_000).unwrap().to_rfc3339(),
            "principal":{"userId":actor,"membershipId":membership,"memberType":"EMPLOYEE",
                "identityVersion":"1","accountVersion":"1","tenantId":state.tenant,"organizationId":state.parent},
            "introspection":{"active":true,"aud":"stores","sub":"opaque-pairwise-subject-never-a-platform-uuid",
                "scope":format!("stores:{action} {resource}:{action}"),"client_id":state.client,"token_type":"Bearer",
                "token_use":"access_token","iss":"https://identity.example.test","exp":now/1000+120},
            "context":{"contractVersion":"1.0","appCode":"stores","contextVersion":"a".repeat(64),
                "expiresAt":chrono::DateTime::from_timestamp_millis(now+20_000).unwrap().to_rfc3339(),
                "user":{"id":actor},"tenant":{"id":state.tenant},"organization":{"id":state.parent},
                "application":{"id":"55555555-5555-7555-8555-555555555555"},
                "organizationScopeIds":[state.parent,state.active,state.excluded[0],state.excluded[1]],
                "permissions":[format!("{resource}:{action}")],
                "dataGrants":[{"resource":resource,"action":action,"tenantId":state.tenant,
                    "subjectUserId":actor,"scope":scope,"organizationIds":[]}]}
        });
        match fields["token"].as_str() {
            "machine-token" => {
                reply["subjectMode"] = json!("MACHINE");
                reply.as_object_mut().unwrap().remove("principal");
                reply.as_object_mut().unwrap().remove("context");
                reply.as_object_mut().unwrap().remove("introspection");
                reply["credentialVersion"] = json!("1");
                reply["scope"] = json!([format!("stores:{action}")]);
                reply["delegation"] = json!({"delegationId":new_id(),"version":"1","targetApplicationId":"55555555-5555-7555-8555-555555555555",
                    "tenantId":state.tenant,"organizationId":state.active,"resource":resource,"action":action,"scope":"ORG","organizationIds":[state.active]});
            }
            "id-token" => {
                reply["tokenUse"] = json!("id_token");
                reply["introspection"]["token_use"] = json!("id_token");
            }
            "wrong-audience" => {
                reply["audience"] = json!("work-orders");
                reply["introspection"]["aud"] = json!("work-orders");
            }
            "wrong-resource" => reply["resource"] = json!("work-orders"),
            "wrong-scope" => reply["introspection"]["scope"] = json!("openid"),
            "principal-mismatch" => reply["principal"]["userId"] = json!(state.other),
            "context-mismatch" => reply["context"]["user"]["id"] = json!(state.other),
            "organization-mismatch" => {
                reply["principal"]["organizationId"] = json!(state.excluded[0])
            }
            _ => {}
        }
        return Json(reply).into_response();
    }
    assert!(matches!(
        fields["token"].as_str(),
        "app-token" | "other-token" | "before-commit-revoked" | "before-commit-unavailable"
    ));
    if uri
        .path()
        .starts_with("/internal/platform/apps/stores/organizations")
    {
        // All organization HTTP must precede acquiring or locking business rows.
        // This is the same one-connection pool used by the API. Acquiring it
        // here would deadlock if the request retained its connection over HTTP.
        // A bounded acquisition permits SQLx's asynchronous return-to-pool work
        // without treating transient idle-counter changes as an active lease.
        let mut connection = tokio::time::timeout(Duration::from_millis(200), state.pool.acquire())
            .await
            .expect("business connection held during organization HTTP")
            .unwrap();
        if state.check_unlocked.load(Ordering::SeqCst) {
            sqlx::query("SELECT id FROM public.stores FOR UPDATE NOWAIT")
                .fetch_all(&mut *connection)
                .await
                .expect("organization HTTP must not wait behind a business row lock");
        }
        // The optional Node bridge drives this handler on a separate runtime.
        // Finish the probe lease explicitly rather than parking SQLx's async
        // return-to-pool work on that runtime between HTTP requests.
        connection.close().await.unwrap();
        state.organization_reads.fetch_add(1, Ordering::SeqCst);
        let record = state.organization();
        if uri.path() == "/internal/platform/apps/stores/organizations" {
            assert_eq!(fields.len(), 6);
            let page: u32 = fields["page"].parse().unwrap();
            let size: u32 = fields["size"].parse().unwrap();
            let records = if page == 1 {
                state.announce_projection(&record);
                vec![record.clone()]
            } else {
                vec![]
            };
            let data = json!({"contractVersion":"1.0","contextVersion":"a".repeat(64),
                "expiresAt":record["expiresAt"],"records":records,"total":1,"page":page,"size":size});
            return Json(json!({"contractVersion":"resource-directory/1","directory":"organizations","expiresAt":record["expiresAt"],"data":data})).into_response();
        }
        if uri.path().ends_with(&state.active.to_string()) {
            assert_eq!(fields.len(), 3);
            state.announce_projection(&record);
            return Json(json!({"contractVersion":"resource-directory/1","directory":"organizations","expiresAt":record["expiresAt"],"data":record})).into_response();
        }
        // B is non-STORE and C is disabled. Neither can enter a business snapshot.
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    if uri
        .path()
        .starts_with("/internal/platform/apps/stores/members")
    {
        assert!(matches!(
            (fields["resource"].as_str(), fields["action"].as_str()),
            ("store", "create" | "update") | ("store-staff", "read")
        ));
        assert_eq!(fields["organizationId"], state.active.to_string());
        let user = uri
            .path()
            .strip_prefix("/internal/platform/apps/stores/members/")
            .map(|raw| Uuid::parse_str(raw).unwrap());
        state.member_reads.lock().unwrap().push((
            fields["resource"].clone(),
            fields["action"].clone(),
            user,
        ));
        let members = state.employee_members();
        if let Some(user) = user {
            assert_eq!(fields.len(), 4);
            return members.get(&user).map_or_else(
                || axum::http::StatusCode::NOT_FOUND.into_response(),
                |record| Json(json!({"contractVersion":"resource-directory/1","directory":"members","expiresAt":state.organization()["expiresAt"],"data":record})).into_response(),
            );
        }
        assert_eq!(fields.len(), 7);
        let page: u32 = fields["page"].parse().unwrap();
        let size: u32 = fields["size"].parse().unwrap();
        let q = fields["q"].to_lowercase();
        let records: Vec<_> = members
            .into_values()
            .filter(|record| {
                record["displayName"]
                    .as_str()
                    .unwrap()
                    .to_lowercase()
                    .contains(&q)
            })
            .collect();
        let total = records.len();
        let records: Vec<_> = records
            .into_iter()
            .skip(((page - 1) * size) as usize)
            .take(size as usize)
            .collect();
        return Json(json!({"contractVersion":"resource-directory/1","directory":"members","expiresAt":state.organization()["expiresAt"],
            "data":{"records":records,"total":total,"page":page,"size":size}}))
            .into_response();
    }
    panic!("unexpected fixture route");
}

struct PlatformServer {
    _node: Option<test_http::NodePeer>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for PlatformServer {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn platform_fixture(state: Arc<PlatformFixture>) -> (String, PlatformServer) {
    use axum::{routing::post, Router};
    let app = Router::new()
        .route("/api/apps/stores/authorize", post(platform_response))
        .route(
            "/internal/platform/apps/stores/organizations",
            post(platform_response),
        )
        .route(
            "/internal/platform/apps/stores/organizations/{id}",
            post(platform_response),
        )
        .route(
            "/internal/platform/apps/stores/members",
            post(platform_response),
        )
        .route(
            "/internal/platform/apps/stores/members/{id}",
            post(platform_response),
        )
        .with_state(state);
    if test_http::NodePeer::enabled() {
        let node = test_http::NodePeer::router(app);
        return (
            node.origin.clone(),
            PlatformServer {
                _node: Some(node),
                task: None,
            },
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (
        origin,
        PlatformServer {
            _node: None,
            task: Some(task),
        },
    )
}

async fn api_call(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    token: &str,
) -> (u16, serde_json::Value) {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("origin", "https://platform.example.test")
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
#[ignore = "requires the owned CI PostgreSQL driver"]
async fn platform_store_snapshot_guards_canonical_http_reads_and_writes() {
    use serde_json::json;
    owned(|pool| async move {
        initialize_database(&pool).await.unwrap();
        let api_pool = PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_secs(2))
            .after_connect(|connection, _| Box::pin(async move {
                sqlx::query("SELECT pg_catalog.set_config('search_path','pg_catalog,public,pg_temp',false)")
                    .execute(connection).await?;
                Ok(())
            })).connect_with((*pool.connect_options()).clone()).await.unwrap();
        let state = Arc::new(PlatformFixture::new(api_pool.clone()));
        let profiles = [new_id(), new_id(), new_id()];
        let reports = [new_id(), new_id(), new_id()];
        for (index, organization) in [state.active, state.excluded[0], state.excluded[1]].into_iter().enumerate() {
            sqlx::query("INSERT INTO stores(id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact) VALUES($1,$2,$3,$4,$5,'OPEN','Legacy contact')")
                .bind(profiles[index]).bind(state.tenant).bind(organization).bind(state.actor).bind(format!("Legacy {index}")).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-20',$6::numeric)")
                .bind(reports[index]).bind(state.tenant).bind(profiles[index]).bind(organization).bind(state.actor).bind(format!("{}.00", (index+1)*10)).execute(&pool).await.unwrap();
        }
        let other_report = new_id();
        sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-21',40)")
            .bind(other_report).bind(state.tenant).bind(profiles[0]).bind(state.active).bind(state.other).execute(&pool).await.unwrap();
        initialize_database(&pool).await.unwrap();
        let (origin, _server) = platform_fixture(state.clone()).await;
        let platform_config=|bindings:&str| stores_api::Config::from_pairs([
            ("STORES_DATABASE_URL", "postgresql://fixture:fixture@127.0.0.1/stores"),
            ("STORES_ALLOWED_ORIGINS", "https://platform.example.test"), ("PLATFORM_API_ORIGIN", &origin),
            ("PLATFORM_CLIENT_ID", "fixture"), ("PLATFORM_CLIENT_SECRET", "fixture"),
            ("PLATFORM_AUTHORIZATION_MODE", "combined"),
            ("PLATFORM_IDENTITY_ISSUER", "https://identity.example.test"),
            ("PLATFORM_RESOURCE_SCOPE_BINDINGS",bindings),
        ]);
        let config=platform_config(FIXTURE_RESOURCE_SCOPE_BINDINGS).unwrap();
        let app = stores_api::application(&config, api_pool.clone()).unwrap();
        let before_rejected = business_snapshot(&pool).await;
        let organization_reads = state.organization_reads.load(Ordering::SeqCst);
        let member_reads = state.member_reads.lock().unwrap().len();
        let authorization_reads = state.authorization_reads.load(Ordering::SeqCst);
        for (token,expected) in [
            ("id-token",401),("wrong-audience",401),("wrong-resource",401),
            ("wrong-scope",401),
            ("principal-mismatch",401),("context-mismatch",401),("organization-mismatch",401),
            ("disabled-member",401),("machine-token",403),("source-unavailable",503),
        ] {
            for (method,path,body) in [
                ("GET","/stores-api/stores".to_owned(),None),
                ("PATCH",format!("/stores-api/stores/{}",profiles[0]),Some(json!({"version":1,"businessContact":"Denied source must not write","managerUserId":null}))),
            ] {
                let (status,error)=api_call(&app,method,&path,body,token).await;
                assert_eq!(status,expected,"{token} {method} {path}: {error}");
                if token=="machine-token" {assert_eq!(error["code"],"SUBJECT_MODE_DENIED");}
            }
        }
        assert_eq!(state.organization_reads.load(Ordering::SeqCst),organization_reads,
            "invalid source and MACHINE credentials must not enter the employee STORE directory");
        assert_eq!(state.member_reads.lock().unwrap().len(),member_reads);
        assert_eq!(state.authorization_reads.load(Ordering::SeqCst),authorization_reads+20);
        assert_eq!(business_snapshot(&pool).await,before_rejected);
        check("canonical_source_token_use_audience_resource_and_principal_matrix_rejects_without_directory_or_business_writes");
        let mut missing_binding:serde_json::Value=serde_json::from_str(FIXTURE_RESOURCE_SCOPE_BINDINGS).unwrap();
        missing_binding.as_object_mut().unwrap().remove("store:update");
        let missing_binding=missing_binding.to_string();
        let missing_config=platform_config(&missing_binding).unwrap();
        let missing_app=stores_api::application(&missing_config,api_pool.clone()).unwrap();
        let organization_reads=state.organization_reads.load(Ordering::SeqCst);
        let member_reads=state.member_reads.lock().unwrap().len();
        let (status,error)=api_call(&missing_app,"PATCH",&format!("/stores-api/stores/{}",profiles[0]),
            Some(json!({"version":1,"businessContact":"Unregistered OAuth scope must not write","managerUserId":null})),"app-token").await;
        assert_eq!(status,401,"missing independent scope binding: {error}");
        assert_eq!(state.organization_reads.load(Ordering::SeqCst),organization_reads);
        assert_eq!(state.member_reads.lock().unwrap().len(),member_reads);
        assert_eq!(business_snapshot(&pool).await,before_rejected);
        check("missing_operation_scope_binding_rejects_before_directory_and_business_writes");
        for (token,expected) in [("before-commit-revoked",401),("before-commit-unavailable",503)] {
            let (status,error)=api_call(&app,"PATCH",&format!("/stores-api/stores/{}",profiles[0]),
                Some(json!({"version":1,"businessContact":"Rollback at current-source fence","managerUserId":null})),token).await;
            assert_eq!(status,expected,"{token}: {error}");
            assert_eq!(state.authorization_token_reads.lock().unwrap().get(token),Some(&2),
                "write source must be checked again before commit");
            assert_eq!(business_snapshot(&pool).await,before_rejected,
                "source failure at commit must roll back business, version and audit rows");
        }
        check("fresh_source_rechecked_after_sql_and_revoke_or_outage_rolls_back_before_commit");
        let before_invalid = business_snapshot(&pool).await;
        for raw in [
            "11111111-1111-4111-8111-111111111111", // Intentional legacy v4 negative fixture.
            "11111111-1111-7111-c111-111111111111", // Wrong variant despite v7.
            "not-a-uuid",
        ] {
            for path in [
                format!("/stores-api/stores/{raw}"),
                format!("/stores-api/daily-reports/{raw}"),
                format!("/stores-api/daily-reports/summary?dateFrom=2026-10-01&dateTo=2026-10-31&storeId={raw}"),
            ] {
                let (status, error) = api_call(&app, "GET", &path, None, "app-token").await;
                assert_eq!(status, 400, "{path}: {error}");
            }
            let (status, error) = api_call(&app, "POST", "/stores-api/stores", Some(json!({"organizationId":raw,"businessStatus":"OPEN","businessContact":""})), "app-token").await;
            assert_eq!(status, 400, "{error}");
            let (status, error) = api_call(&app, "PATCH", &format!("/stores-api/stores/{}", profiles[0]), Some(json!({"version":1,"managerUserId":raw})), "app-token").await;
            assert_eq!(status, 400, "{error}");
        }
        assert_eq!(business_snapshot(&pool).await, before_invalid);
        check("http_paths_dtos_and_optional_id_filters_reject_v4_variant_and_malformed_ids");
        let (status, list) = api_call(&app, "GET", "/stores-api/stores", None, "app-token").await;
        assert_eq!(status, 200, "{list}");
        assert_eq!(list["total"], 1);
        assert_eq!(list["records"][0]["id"], profiles[0].to_string());
        assert_eq!(list["records"][0]["name"], "Platform A");
        let (status, members) = api_call(&app, "GET", &format!("/stores-api/stores/{}/staff?page=1&size=20&q=member", profiles[0]), None, "other-token").await;
        assert_eq!(status, 200, "{members}");
        assert_eq!(members["records"], json!([{"id":state.actor,"organizationId":state.active,"displayName":"Current member"}]));
        let summary_path = "/stores-api/daily-reports/summary?dateFrom=2026-10-01&dateTo=2026-10-31";
        let (status, summary) = api_call(&app, "GET", summary_path, None, "app-token").await;
        assert_eq!(status, 200, "{summary}");
        assert_eq!((summary["count"].clone(), summary["totalAmount"].clone()), (json!(2), json!("50.00")));
        let before = business_snapshot(&pool).await;
        for index in [1, 2] {
            for path in [format!("/stores-api/stores/{}", profiles[index]), format!("/stores-api/daily-reports/{}", reports[index])] {
                assert_eq!(api_call(&app, "GET", &path, None, "app-token").await.0, 404);
            }
            let (status, list) = api_call(&app, "GET", &format!("/stores-api/stores/{}/daily-reports", profiles[index]), None, "app-token").await;
            assert_eq!(status, 200);
            assert_eq!(list["total"], 0);
            assert_eq!(api_call(&app, "PATCH", &format!("/stores-api/stores/{}", profiles[index]), Some(json!({"version":1,"businessStatus":"CLOSED"})), "app-token").await.0, 404);
            assert_eq!(api_call(&app, "POST", &format!("/stores-api/stores/{}/daily-reports", profiles[index]), Some(json!({"amount":"99.00","businessDate":"2026-10-22"})), "app-token").await.0, 404);
            assert_eq!(api_call(&app, "PATCH", &format!("/stores-api/daily-reports/{}", reports[index]), Some(json!({"version":1,"amount":"99.00"})), "app-token").await.0, 404);
        }
        assert_eq!(business_snapshot(&pool).await, before);
        check("http_snapshot_excludes_non_store_disabled_legacy_rows_and_summary");

        state.self_scope.store(true, Ordering::SeqCst);
        let profile_path = format!("/stores-api/stores/{}", profiles[0]);
        assert_eq!(api_call(&app, "GET", &profile_path, None, "app-token").await.0, 200);
        assert_eq!(api_call(&app, "GET", &profile_path, None, "other-token").await.0, 404);
        assert_eq!(api_call(&app, "GET", &format!("/stores-api/daily-reports/{}", reports[0]), None, "app-token").await.0, 200);
        assert_eq!(api_call(&app, "GET", &format!("/stores-api/daily-reports/{other_report}"), None, "app-token").await.0, 404);
        assert_eq!(api_call(&app, "GET", &format!("/stores-api/daily-reports/{other_report}"), None, "other-token").await.0, 200);
        let (_, summary) = api_call(&app, "GET", summary_path, None, "app-token").await;
        assert_eq!((summary["count"].clone(), summary["totalAmount"].clone()), (json!(1), json!("10.00")));
        let (status, updated) = api_call(&app, "PATCH", &profile_path, Some(json!({"version":1,"businessContact":"Self changed"})), "app-token").await;
        assert_eq!(status, 200, "{updated}");
        assert_eq!(updated["version"], 2);
        let before = business_snapshot(&pool).await;
        assert_eq!(api_call(&app, "PATCH", &profile_path, Some(json!({"version":2,"businessStatus":"CLOSED"})), "other-token").await.0, 404);
        state.expired.store(true, Ordering::SeqCst);
        assert_eq!(api_call(&app, "PATCH", &profile_path, Some(json!({"version":2,"businessContact":"Expired write"})), "app-token").await.0, 503);
        state.expired.store(false, Ordering::SeqCst);
        assert_eq!(business_snapshot(&pool).await, before);
        check("self_owned_child_read_write_and_expired_projection_rejection");

        sqlx::query("UPDATE stores SET manager_user_id=$1,manager_display_name='Old manager',version=version+1 WHERE id=$2")
            .bind(state.other).bind(profiles[0]).execute(&pool).await.unwrap();
        let before = business_snapshot(&pool).await;
        let (status, error) = api_call(&app, "PATCH", &profile_path, Some(json!({"version":3,"businessStatus":"CLOSED"})), "app-token").await;
        assert_eq!(status, 404, "{error}");
        assert_eq!(error["code"], "MEMBER_NOT_FOUND");
        assert_eq!(business_snapshot(&pool).await, before);
        let (status, cleared) = api_call(&app, "PATCH", &profile_path, Some(json!({"version":3,"businessStatus":"CLOSED","managerUserId":null})), "app-token").await;
        assert_eq!(status, 200, "{cleared}");
        assert_eq!(cleared["version"], 4);
        assert!(cleared["managerUserId"].is_null());
        assert!(cleared["managerDisplayName"].is_null());
        assert!(state.organization_reads.load(Ordering::SeqCst) >= 20);
        check("self_outside_contact_revalidation_and_organization_http_without_pool_or_row_locks");

        // Hold the real row lock before the API call. Only this contention case
        // skips the NOWAIT probe, since the test deliberately owns that lock;
        // the one-connection-pool acquisition probe continues to run on HTTP.
        let before = business_snapshot(&pool).await;
        let mut blocker = pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *blocker).await.unwrap();
        sqlx::query("SELECT id FROM public.stores WHERE id=$1 FOR UPDATE")
            .bind(profiles[0]).fetch_one(&mut *blocker).await.unwrap();
        state.check_unlocked.store(false, Ordering::SeqCst);
        state.lease_ms.store(1000, Ordering::SeqCst);
        let (ready, projection) = tokio::sync::oneshot::channel();
        *state.projection_ready.lock().unwrap() = Some(ready);
        let waiting_app = app.clone();
        let waiting_path = profile_path.clone();
        let waiting_update = tokio::spawn(async move {
            api_call(&waiting_app, "PATCH", &waiting_path,
                Some(json!({"version":4,"businessStatus":"OPEN","businessContact":"Expired during row lock","managerUserId":null})),
                "app-token").await
        });
        let expires_at = tokio::time::timeout(Duration::from_secs(2), projection)
            .await.expect("platform must issue the controlled projection").unwrap();
        let blocked = tokio::time::timeout(Duration::from_millis(800), async {
            loop {
                let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity AS activity WHERE activity.datname=pg_catalog.current_database() AND activity.state='active' AND activity.wait_event_type='Lock' AND $1=ANY(pg_catalog.pg_blocking_pids(activity.pid)) AND activity.query LIKE '%public.stores%' AND activity.query LIKE '%FOR UPDATE%')")
                    .bind(blocker_pid).fetch_one(&pool).await.unwrap();
                if blocked { break; }
                tokio::task::yield_now().await;
            }
        }).await;
        let observed_at = chrono::Utc::now().timestamp_millis();
        if blocked.is_err() || observed_at >= expires_at {
            blocker.rollback().await.unwrap();
            waiting_update.abort();
            let _ = waiting_update.await;
            panic!("update must be observed waiting on the test row lock while its projection is valid");
        }
        println!("STORES_LOCK_LEASE_PROOF waiter_confirmed=true remaining_lease_ms={}", expires_at-observed_at);
        // Wait for the actual expiry returned by the platform, after proving a
        // live PostgreSQL lock waiter. This is not a delay used to infer a race.
        tokio::time::sleep(Duration::from_millis((expires_at-observed_at) as u64 + 20)).await;
        assert!(chrono::Utc::now().timestamp_millis() >= expires_at);
        blocker.rollback().await.unwrap();
        let (status, error) = tokio::time::timeout(Duration::from_secs(2), waiting_update)
            .await.expect("expired update must finish after releasing the row lock").unwrap();
        assert_eq!(status, 503, "{error}");
        assert_eq!(error["code"], "ORGANIZATION_VALIDATION_EXPIRED");
        assert_eq!(business_snapshot(&pool).await, before,
            "an organization lease expiring during a confirmed SQL lock wait must preserve every business, version and audit value");
        state.check_unlocked.store(true, Ordering::SeqCst);
        state.lease_ms.store(2000, Ordering::SeqCst);
        api_pool.close().await;
        check("organization_lease_expires_during_confirmed_row_lock_and_rolls_back");
    }).await;
}

#[tokio::test]
#[ignore = "requires the owned CI PostgreSQL driver"]
async fn employee_member_candidates_and_manager_writes_preserve_the_user_id_contract() {
    use serde_json::json;
    use std::collections::BTreeSet;

    owned(|pool| async move {
        initialize_database(&pool).await.unwrap();
        let api_pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(2))
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        let state = Arc::new(PlatformFixture::new(api_pool.clone()));
        let dual_user = new_id();
        let customer_only = new_id();
        let disabled_employee = new_id();
        let dual_employee_membership = new_id();
        let customer_membership = new_id();
        let actor_customer_membership = new_id();
        let dual_customer_membership = new_id();
        let disabled_membership = new_id();
        state.members.lock().unwrap().extend([
            FixtureMember {
                user: state.actor,
                membership: actor_customer_membership,
                member_type: FixtureMemberType::Customer,
                enabled: true,
                display_name: "Actor customer",
            },
            FixtureMember {
                user: dual_user,
                membership: dual_employee_membership,
                member_type: FixtureMemberType::Employee,
                enabled: true,
                display_name: "Dual employee",
            },
            FixtureMember {
                user: dual_user,
                membership: dual_customer_membership,
                member_type: FixtureMemberType::Customer,
                enabled: true,
                display_name: "Dual customer",
            },
            FixtureMember {
                user: customer_only,
                membership: customer_membership,
                member_type: FixtureMemberType::Customer,
                enabled: true,
                display_name: "Customer only",
            },
            FixtureMember {
                user: disabled_employee,
                membership: disabled_membership,
                member_type: FixtureMemberType::Employee,
                enabled: false,
                display_name: "Disabled employee",
            },
            FixtureMember {
                user: disabled_employee,
                membership: new_id(),
                member_type: FixtureMemberType::Customer,
                enabled: true,
                display_name: "Disabled employee's enabled customer",
            },
        ]);
        let (origin, _server) = platform_fixture(state.clone()).await;
        let config = stores_api::Config::from_pairs([
            ("STORES_DATABASE_URL", "postgresql://fixture:fixture@127.0.0.1/stores"),
            ("STORES_ALLOWED_ORIGINS", "https://platform.example.test"),
            ("PLATFORM_API_ORIGIN", &origin),
            ("PLATFORM_CLIENT_ID", "fixture"),
            ("PLATFORM_CLIENT_SECRET", "fixture"),
            ("PLATFORM_AUTHORIZATION_MODE", "combined"),
            ("PLATFORM_IDENTITY_ISSUER", "https://identity.example.test"),
            ("PLATFORM_RESOURCE_SCOPE_BINDINGS", FIXTURE_RESOURCE_SCOPE_BINDINGS),
        ]).unwrap();
        let app = stores_api::application(&config, api_pool.clone()).unwrap();

        for (resource, action) in [("store", "create"), ("store", "update"), ("store-staff", "read")] {
            let mut returned = BTreeSet::new();
            for page in 1..=3 {
                let path = format!("/stores-api/members?resource={resource}&action={action}&organizationId={}&page={page}&size=1", state.active);
                let (status, candidates) = api_call(&app, "GET", &path, None, "app-token").await;
                assert_eq!(status, 200, "{path}: {candidates}");
                assert_eq!(candidates["total"], 2, "only enabled employee users count");
                assert_eq!(candidates["page"], page);
                assert_eq!(candidates["size"], 1);
                let records = candidates["records"].as_array().unwrap();
                assert_eq!(records.len(), usize::from(page <= 2));
                for record in records {
                    assert_eq!(record.as_object().unwrap().len(), 3, "keep the legacy member DTO");
                    assert_eq!(record["organizationId"], state.active.to_string());
                    returned.insert(Uuid::parse_str(record["id"].as_str().unwrap()).unwrap());
                }
            }
            assert_eq!(returned, BTreeSet::from([state.actor, dual_user]));
        }
        assert_eq!(state.member_reads.lock().unwrap().len(), 9);
        check("three_member_lookup_purposes_keep_user_ids_and_employee_only_pages_and_totals");

        // Neither an eligible user's membership ID nor a customer's account or
        // membership ID is a user-ID target, even if copied from another API.
        let invalid_targets = [
            customer_only,
            disabled_employee,
            customer_membership,
            actor_customer_membership,
            dual_employee_membership,
            dual_customer_membership,
            disabled_membership,
        ];
        let before = business_snapshot(&pool).await;
        for target in invalid_targets {
            let (status, error) = api_call(&app, "POST", "/stores-api/stores", Some(json!({
                "organizationId":state.active,"businessStatus":"OPEN",
                "businessContact":"Rejected contact","managerUserId":target,
            })), "app-token").await;
            assert_eq!(status, 404, "{target}: {error}");
            assert_eq!(error["code"], "MEMBER_NOT_FOUND");
            assert_eq!(state.member_reads.lock().unwrap().last(), Some(&("store".into(), "create".into(), Some(target))));
        }
        assert_eq!(business_snapshot(&pool).await, before);
        let (status, created) = api_call(&app, "POST", "/stores-api/stores", Some(json!({
            "organizationId":state.active,"businessStatus":"OPEN",
            "businessContact":"Employee contact","managerUserId":dual_user,
        })), "app-token").await;
        assert_eq!(status, 201, "{created}");
        assert_eq!(created["managerUserId"], dual_user.to_string());
        assert_eq!(created["managerDisplayName"], "Dual employee");
        assert_eq!(created["version"], 1);
        let profile = created["id"].as_str().unwrap();
        let profile_path = format!("/stores-api/stores/{profile}");
        let staff_path = format!("{profile_path}/staff");
        let (status, staff) = api_call(&app, "GET", &format!("{staff_path}?page=1&size=20"), None, "app-token").await;
        assert_eq!(status, 200, "{staff}");
        assert_eq!(staff["total"], 2);
        let staff_ids: BTreeSet<_> = staff["records"].as_array().unwrap().iter().map(|record| {
            assert_eq!(record.as_object().unwrap().len(), 3);
            assert_eq!(record["organizationId"], state.active.to_string());
            Uuid::parse_str(record["id"].as_str().unwrap()).unwrap()
        }).collect();
        assert_eq!(staff_ids, BTreeSet::from([state.actor, dual_user]));
        check("manager_create_revalidates_user_target_and_staff_get_consumes_employee_directory");

        let mut blocked_requests = vec![
            ("GET", staff_path.clone(), None),
            ("POST", "/stores-api/stores".into(), Some(json!({
                "organizationId":state.active,"businessStatus":"OPEN","businessContact":"",
                "managerUserId":dual_user,
            }))),
            ("PATCH", profile_path.clone(), Some(json!({"version":1,"managerUserId":dual_user}))),
        ];
        for (resource, action) in [("store", "create"), ("store", "update"), ("store-staff", "read")] {
            blocked_requests.push(("GET", format!("/stores-api/members?resource={resource}&action={action}&organizationId={}", state.active), None));
        }
        let before = business_snapshot(&pool).await;
        let organization_reads = state.organization_reads.load(Ordering::SeqCst);
        let member_reads = state.member_reads.lock().unwrap().len();
        let authorization_reads = state.authorization_reads.load(Ordering::SeqCst);
        let blocked_count = blocked_requests.len();
        for (method, path, body) in blocked_requests {
            let (status, error) = api_call(&app, method, &path, body, "customer-token").await;
            assert_eq!(status, 403, "{method} {path}: {error}");
            assert_eq!(error["code"], "OPERATION_DENIED");
        }
        assert_eq!(state.authorization_reads.load(Ordering::SeqCst), authorization_reads + blocked_count);
        assert_eq!(state.organization_reads.load(Ordering::SeqCst), organization_reads);
        assert_eq!(state.member_reads.lock().unwrap().len(), member_reads);
        assert_eq!(business_snapshot(&pool).await, before);
        check("customer_actor_with_same_account_employee_is_denied_before_directory_or_business_access");

        let before = business_snapshot(&pool).await;
        for target in invalid_targets {
            let (status, error) = api_call(&app, "PATCH", &profile_path, Some(json!({
                "version":1,"managerUserId":target,
            })), "app-token").await;
            assert_eq!(status, 404, "{target}: {error}");
            assert_eq!(error["code"], "MEMBER_NOT_FOUND");
            assert_eq!(state.member_reads.lock().unwrap().last(), Some(&("store".into(), "update".into(), Some(target))));
        }
        assert_eq!(business_snapshot(&pool).await, before);
        let (status, updated) = api_call(&app, "PATCH", &profile_path, Some(json!({
            "version":1,"managerUserId":state.actor,
        })), "app-token").await;
        assert_eq!(status, 200, "{updated}");
        assert_eq!(updated["managerUserId"], state.actor.to_string());
        assert_eq!(updated["managerDisplayName"], "Current member");
        let (status, updated) = api_call(&app, "PATCH", &profile_path, Some(json!({
            "version":2,"managerUserId":dual_user,
        })), "app-token").await;
        assert_eq!(status, 200, "{updated}");
        assert_eq!(updated["managerUserId"], dual_user.to_string());
        assert_eq!(updated["managerDisplayName"], "Dual employee");
        assert_eq!(updated["version"], 3);

        state.members.lock().unwrap().iter_mut()
            .find(|member| member.membership == dual_employee_membership).unwrap().enabled = false;
        let before = business_snapshot(&pool).await;
        for body in [
            json!({"version":3,"businessContact":"Retained stale manager"}),
            json!({"version":3,"managerUserId":dual_user}),
            json!({"version":3,"managerUserId":disabled_employee}),
        ] {
            let (status, error) = api_call(&app, "PATCH", &profile_path, Some(body), "app-token").await;
            assert_eq!(status, 404, "{error}");
            assert_eq!(error["code"], "MEMBER_NOT_FOUND");
        }
        assert_eq!(business_snapshot(&pool).await, before,
            "omitted, same-ID and replacement managers must not bypass current employee qualification");
        for (resource, action) in [("store", "create"), ("store", "update"), ("store-staff", "read")] {
            let path = format!("/stores-api/members?resource={resource}&action={action}&organizationId={}", state.active);
            let (status, candidates) = api_call(&app, "GET", &path, None, "app-token").await;
            assert_eq!(status, 200, "{candidates}");
            assert_eq!(candidates["total"], 1);
            assert_eq!(candidates["records"], json!([{"id":state.actor,"organizationId":state.active,"displayName":"Current member"}]));
        }
        let (status, staff) = api_call(&app, "GET", &staff_path, None, "app-token").await;
        assert_eq!(status, 200, "{staff}");
        assert_eq!(staff["total"], 1);
        assert_eq!(staff["records"], json!([{"id":state.actor,"organizationId":state.active,"displayName":"Current member"}]));
        let (status, cleared) = api_call(&app, "PATCH", &profile_path, Some(json!({
            "version":3,"managerUserId":null,"businessContact":"Explicitly cleared",
        })), "app-token").await;
        assert_eq!(status, 200, "{cleared}");
        assert_eq!(cleared["version"], 4);
        assert!(cleared["managerUserId"].is_null());
        assert!(cleared["managerDisplayName"].is_null());
        check("manager_update_rejects_customer_member_ids_disabled_and_retained_stale_targets_but_allows_clear");

        let before = business_snapshot(&pool).await;
        let organization_reads = state.organization_reads.load(Ordering::SeqCst);
        let member_reads = state.member_reads.lock().unwrap().len();
        let authorization_reads = state.authorization_reads.load(Ordering::SeqCst);
        for (method, path, body) in [
            ("POST", staff_path, Some(json!({"userId":state.actor,"displayName":"No local assignment"}))),
            ("DELETE", format!("{profile_path}/staff/{}", new_id()), None),
        ] {
            let (status, error) = api_call(&app, method, &path, body.clone(), "app-token").await;
            assert_eq!(status, 410, "{method} {path}: {error}");
            assert_eq!(error["code"], "PLATFORM_MEMBERSHIP_MANAGED");
            let (status, error) = api_call(&app, method, &path, body, "customer-token").await;
            assert_eq!(status, 403, "{method} {path}: {error}");
            assert_eq!(error["code"], "OPERATION_DENIED");
        }
        assert_eq!(state.authorization_reads.load(Ordering::SeqCst), authorization_reads + 4);
        assert_eq!(state.organization_reads.load(Ordering::SeqCst), organization_reads);
        assert_eq!(state.member_reads.lock().unwrap().len(), member_reads);
        assert_eq!(business_snapshot(&pool).await, before);
        check("retired_staff_writes_authorize_employee_then_return_410_and_deny_customer_without_writes");
        api_pool.close().await;
    }).await;
}

#[tokio::test]
#[ignore = "requires the owned CI PostgreSQL driver"]
async fn untracked_database_is_not_adopted_as_a_fresh_store_database() {
    owned(|pool| async move {
        sqlx::query("CREATE TABLE public.old_business_data(id bigint PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        assert!(initialize_database(&pool).await.is_err());
        let initialized: bool =
            sqlx::query_scalar("SELECT to_regclass('public.stores') IS NOT NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!initialized);
    })
    .await;
}
