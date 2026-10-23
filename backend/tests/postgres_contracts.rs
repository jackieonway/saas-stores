//! Explicit real PostgreSQL contracts, isolated inside the owned CI server.
//! Ordinary test runs ignore database cases; the CI owner supplies its URL.

use std::{
    borrow::Cow,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use sqlx::{postgres::PgPoolOptions, Connection, PgPool};
use stores_api::initialize_database;
use tokio::sync::Barrier;
use uuid::Uuid;

#[path = "../src/authorization/test_http.rs"]
mod test_http;

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
    let name = format!("phase26_stores_sql_{}", Uuid::new_v4().simple());
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

async fn initialize_bounded(pool: &PgPool) -> Result<(), stores_api::ServiceError> {
    tokio::time::timeout(Duration::from_secs(3), initialize_database(pool))
        .await
        .expect("migration verification or recovery exceeded its three-second budget")
}

async fn initialize_v1(pool: &PgPool) {
    let all = sqlx::migrate!("./migrations");
    let v1 = all
        .iter()
        .find(|migration| migration.version == 1)
        .unwrap()
        .clone();
    let migrator = sqlx::migrate::Migrator {
        migrations: Cow::Owned(vec![v1]),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    let mut connection = pool.acquire().await.unwrap().detach();
    sqlx::query("SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',false)")
        .execute(&mut connection)
        .await
        .unwrap();
    migrator.run_direct(&mut connection).await.unwrap();
    connection.close().await.unwrap();
}

async fn business_snapshot(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT pg_catalog.jsonb_build_object('stores',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.stores AS business_row),'staff',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.store_staff AS business_row),'reports',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.store_daily_reports AS business_row),'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(business_row) ORDER BY id),'[]'::jsonb) FROM public.store_audit_events AS business_row))")
        .fetch_one(pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires the owned CI PostgreSQL driver"]
async fn legacy_upgrade_preserves_data_and_freezes_platform_owned_fields() {
    owned(|pool| async move {
        initialize_v1(&pool).await;
        let tenant = Uuid::new_v4();
        let org = Uuid::new_v4();
        let user = Uuid::new_v4();
        let store = Uuid::new_v4();
        sqlx::query("INSERT INTO stores(id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact,manager_user_id,manager_display_name) VALUES($1,$2,$3,$4,'Legacy store','OPEN','Legacy contact',$4,'Legacy manager')").bind(store).bind(tenant).bind(org).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note) VALUES($1,$2,$3,$4,$5,DATE '2026-10-20','42.33'::numeric,'Legacy note')").bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.unwrap();

        // Exercise the unchanged v1 staff contract before migrating its history.
        let link = Uuid::new_v4();
        sqlx::query("INSERT INTO store_staff(id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id) VALUES($1,$2,$3,$4,$5,$5,'Member',$5)").bind(link).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.unwrap();
        let duplicate = sqlx::query("INSERT INTO store_staff(id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id) VALUES($1,$2,$3,$4,$5,$5,'Member',$5)").bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.expect_err("active staff uniqueness");
        assert_eq!(duplicate.as_database_error().and_then(|error| error.constraint()), Some("store_staff_active_member_key"));
        sqlx::query("UPDATE store_staff SET removed_at=now(),updated_at=now(),version=version+1 WHERE id=$1 AND version=1").bind(link).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO store_staff(id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id) VALUES($1,$2,$3,$4,$5,$5,'Member',$5)").bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.unwrap();
        let history: (i64, i64) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE removed_at IS NULL) FROM store_staff").fetch_one(&pool).await.unwrap();
        assert_eq!(history, (2, 1));
        sqlx::query("INSERT INTO store_audit_events(id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) VALUES($1,$2,$3,$4,'store-staff','assign',$5,$6)").bind(Uuid::new_v4()).bind(tenant).bind(org).bind(user).bind(link).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
        check("staff_active_uniqueness_and_removed_history");

        let before = business_snapshot(&pool).await;
        let legacy_baseline: String = sqlx::query_scalar("SELECT fingerprint FROM public._stores_schema_contract WHERE singleton").fetch_one(&pool).await.unwrap();
        let legacy_function: String = sqlx::query_scalar("SELECT pg_catalog.pg_get_functiondef('public.stores_schema_fingerprint()'::regprocedure)").fetch_one(&pool).await.unwrap();
        let legacy_checksum: Vec<u8> = sqlx::query_scalar("SELECT checksum FROM public._sqlx_migrations WHERE version=1").fetch_one(&pool).await.unwrap();
        sqlx::query("ALTER TABLE public.stores ADD COLUMN upgrade_unexpected TEXT").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "upgrade must reject existing v1 schema drift");
        let unapplied: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM public._sqlx_migrations WHERE version=2) AND pg_catalog.to_regclass('public._stores_schema_contract_v2') IS NULL").fetch_one(&pool).await.unwrap();
        assert!(unapplied, "failed upgrade must not record a new baseline");
        sqlx::query("ALTER TABLE public.stores DROP COLUMN upgrade_unexpected").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("verified v1 database upgrades");
        initialize_bounded(&pool).await.expect("v2 startup is repeatable");
        assert_eq!(business_snapshot(&pool).await, before, "upgrade preserves every business and audit value");
        let baselines: (String, String) = sqlx::query_as("SELECT v1.fingerprint,v2.legacy_fingerprint FROM public._stores_schema_contract AS v1 JOIN public._stores_schema_contract_v2 AS v2 USING(singleton)").fetch_one(&pool).await.unwrap();
        assert_eq!(baselines, (legacy_baseline.clone(), legacy_baseline));
        let after_function: String = sqlx::query_scalar("SELECT pg_catalog.pg_get_functiondef('public.stores_schema_fingerprint()'::regprocedure)").fetch_one(&pool).await.unwrap();
        assert_eq!(after_function, legacy_function);
        let after_checksum: Vec<u8> = sqlx::query_scalar("SELECT checksum FROM public._sqlx_migrations WHERE version=1").fetch_one(&pool).await.unwrap();
        assert_eq!(after_checksum, legacy_checksum);
        check("v1_upgrade_preserves_data_baseline_and_migration_checksum");

        reject(&pool, "UPDATE stores SET name='old binary rename',version=version+1", "23514").await;
        reject(&pool, "UPDATE store_staff SET removed_at=now(),updated_at=now(),version=version+1 WHERE removed_at IS NULL", "23514").await;
        reject(&pool, "DELETE FROM store_staff", "23514").await;
        reject(&pool, "TRUNCATE store_staff", "23514").await;
        let blocked_insert = sqlx::query("INSERT INTO store_staff(id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id) VALUES($1,$2,$3,$4,$5,$5,'Member',$5)").bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(Uuid::new_v4()).execute(&pool).await.expect_err("old binary cannot assign staff");
        assert_eq!(blocked_insert.as_database_error().and_then(|error| error.code()).as_deref(), Some("23514"));
        assert_eq!(business_snapshot(&pool).await, before, "blocked legacy writes preserve all rows");
        sqlx::query("UPDATE stores SET name=name,business_status='CLOSED',business_contact='Updated contact',version=version+1,updated_at=now() WHERE id=$1 AND version=1").bind(store).execute(&pool).await.unwrap();
        sqlx::query("UPDATE store_daily_reports SET amount='43.33'::numeric,note='Updated note',version=version+1,updated_at=now() WHERE store_id=$1 AND version=1").bind(store).execute(&pool).await.unwrap();
        let editable: (String, String, String, i32) = sqlx::query_as("SELECT stores.name,stores.business_status,reports.amount::text,reports.version FROM stores JOIN store_daily_reports AS reports ON reports.store_id=stores.id WHERE stores.id=$1").bind(store).fetch_one(&pool).await.unwrap();
        assert_eq!(editable, ("Legacy store".to_owned(), "CLOSED".to_owned(), "43.33".to_owned(), 2));
        initialize_bounded(&pool).await.expect("allowed business writes preserve schema contract");
        check("legacy_name_and_staff_writes_blocked_while_business_data_remains_editable");
    }).await;
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

        let tenant = Uuid::new_v4();
        let org = Uuid::new_v4();
        let user = Uuid::new_v4();
        let store = Uuid::new_v4();
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
                    .bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).execute(&mut *tx).await;
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
                .bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).bind(day).execute(&pool).await.unwrap();
        }
        let exact: String = sqlx::query_scalar("SELECT sum(amount)::text FROM store_daily_reports WHERE business_date<>DATE '2026-10-18'").fetch_one(&pool).await.unwrap();
        assert_eq!(exact, "19999999999999999.98");
        let invalid_amount = sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-19','NaN'::numeric)")
            .bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.expect_err("numeric NaN is not a business amount");
        assert_eq!(invalid_amount.as_database_error().and_then(|error| error.code()).as_deref(), Some("23514"));
        check("numeric_precision_and_large_exact_sum");

        reject(&pool, "UPDATE stores SET organization_id='00000000-0000-4000-8000-000000000011',version=version+1", "23514").await;
        reject(&pool, "UPDATE stores SET name='stale write'", "23514").await;
        reject(&pool, "UPDATE store_daily_reports SET business_date=business_date+1,version=version+1 WHERE business_date=DATE '2026-10-18'", "23514").await;
        reject(&pool, "UPDATE store_daily_reports SET owner_user_id='00000000-0000-4000-8000-000000000012',version=version+1", "23514").await;
        reject(&pool, "UPDATE stores SET name='old binary rename',version=version+1", "23514").await;
        let changed = sqlx::query("UPDATE stores SET business_status='CLOSED',version=version+1,updated_at=now() WHERE id=$1 AND version=1").bind(store).execute(&pool).await.unwrap().rows_affected();
        let stale = sqlx::query("UPDATE stores SET business_status='OPEN',version=version+1 WHERE id=$1 AND version=1").bind(store).execute(&pool).await.unwrap().rows_affected();
        assert_eq!((changed, stale), (1, 0));
        check("immutable_ownership_date_and_optimistic_version");

        let bad_tenant = sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-19',0)").bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(store).bind(org).bind(user).execute(&pool).await.expect_err("composite FK forbids cross tenant store");
        assert_eq!(bad_tenant.as_database_error().and_then(|error| error.code()).as_deref(), Some("23503"));
        check("composite_store_tenant_organization_fk");

        let rolled_back = Uuid::new_v4();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-20',1)").bind(rolled_back).bind(tenant).bind(store).bind(org).bind(user).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO store_audit_events(id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) VALUES($1,$2,$3,$4,'store-daily-report','create',$5,$6)").bind(Uuid::new_v4()).bind(tenant).bind(org).bind(user).bind(rolled_back).bind(Uuid::new_v4()).execute(&mut *tx).await.unwrap();
        tx.rollback().await.unwrap();
        let absent: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM store_daily_reports WHERE id=$1) AND NOT EXISTS(SELECT 1 FROM store_audit_events WHERE entity_id=$1)").bind(rolled_back).fetch_one(&pool).await.unwrap();
        assert!(absent);
        sqlx::query("INSERT INTO store_audit_events(id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id) VALUES($1,$2,$3,$4,'store','update',$5,$6)").bind(Uuid::new_v4()).bind(tenant).bind(org).bind(user).bind(store).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
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
        sqlx::query("ALTER TABLE public._stores_schema_contract_v2 ADD COLUMN unexpected TEXT").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "v2 metadata schema drift must be rejected");
        sqlx::query("ALTER TABLE public._stores_schema_contract_v2 DROP COLUMN unexpected").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored v2 metadata schema contract");
        sqlx::query("ALTER FUNCTION public.stores_schema_fingerprint_v2() VOLATILE").execute(&pool).await.unwrap();
        assert!(initialize_bounded(&pool).await.is_err(), "v2 fingerprint definition drift must be rejected");
        sqlx::query("ALTER FUNCTION public.stores_schema_fingerprint_v2() STABLE").execute(&pool).await.unwrap();
        initialize_bounded(&pool).await.expect("restored v2 fingerprint definition");
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

struct PlatformFixture {
    pool: PgPool,
    tenant: Uuid,
    actor: Uuid,
    other: Uuid,
    parent: Uuid,
    active: Uuid,
    excluded: [Uuid; 2],
    self_scope: AtomicBool,
    expired: AtomicBool,
    lease_ms: AtomicU64,
    check_unlocked: AtomicBool,
    projection_ready: Mutex<Option<tokio::sync::oneshot::Sender<i64>>>,
    organization_reads: AtomicUsize,
}

impl PlatformFixture {
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

    if uri.path() == "/api/apps/stores/authorize" {
        assert_eq!(headers["authorization"], "Basic Zml4dHVyZTpmaXh0dXJl");
        let fields: BTreeMap<_, _> = url::form_urlencoded::parse(&body).into_owned().collect();
        assert_eq!(fields.len(), 3);
        let actor = if fields["token"] == "other-token" {
            state.other
        } else {
            state.actor
        };
        let resource = &fields["resource"];
        let action = &fields["action"];
        let scope = if state.self_scope.load(Ordering::SeqCst) {
            "SELF"
        } else {
            "TENANT"
        };
        let now = chrono::Utc::now().timestamp_millis();
        return Json(json!({
            "contractVersion":"1.0",
            "introspection":{"active":true,"aud":"stores","sub":actor,"tenant_id":state.tenant,
                "organization_id":state.parent,"token_type":"Bearer","exp":now/1000+120},
            "context":{"contractVersion":"1.0","appCode":"stores","contextVersion":"a".repeat(64),
                "expiresAt":chrono::DateTime::from_timestamp_millis(now+20_000).unwrap().to_rfc3339(),
                "user":{"id":actor},"tenant":{"id":state.tenant},"organization":{"id":state.parent},
                "application":{"id":"55555555-5555-4555-8555-555555555555"},
                "organizationScopeIds":[state.parent,state.active,state.excluded[0],state.excluded[1]],
                "permissions":[format!("{resource}:{action}")],
                "dataGrants":[{"resource":resource,"action":action,"tenantId":state.tenant,
                    "subjectUserId":actor,"scope":scope,"organizationIds":[]}]}
        })).into_response();
    }
    assert!(headers["authorization"]
        .to_str()
        .unwrap()
        .starts_with("Bearer "));
    if uri.path().starts_with("/api/apps/stores/organizations") {
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
        if uri.path() == "/api/apps/stores/organizations" {
            let fields: BTreeMap<_, _> =
                url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
                    .into_owned()
                    .collect();
            let page: u32 = fields["page"].parse().unwrap();
            let size: u32 = fields["size"].parse().unwrap();
            let records = if page == 1 {
                state.announce_projection(&record);
                vec![record.clone()]
            } else {
                vec![]
            };
            return Json(json!({"contractVersion":"1.0","contextVersion":"a".repeat(64),
                "expiresAt":record["expiresAt"],"records":records,"total":1,"page":page,"size":size})).into_response();
        }
        if uri.path().ends_with(&state.active.to_string()) {
            state.announce_projection(&record);
            return Json(record).into_response();
        }
        // B is non-STORE and C is disabled. Neither can enter a business snapshot.
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    if uri.path() == "/api/apps/stores/members" {
        let fields: BTreeMap<_, _> =
            url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
                .into_owned()
                .collect();
        assert_eq!(fields["resource"], "store-staff");
        assert_eq!(fields["action"], "read");
        assert_eq!(fields["organizationId"], state.active.to_string());
        let page: u32 = fields["page"].parse().unwrap();
        let size: u32 = fields["size"].parse().unwrap();
        return Json(json!({"records":[{"id":state.actor,"organizationId":state.active,"displayName":"Current member","private":"must not escape"}],"total":1,"page":page,"size":size})).into_response();
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
    use axum::{
        routing::{get, post},
        Router,
    };
    let app = Router::new()
        .route("/api/apps/stores/authorize", post(platform_response))
        .route("/api/apps/stores/organizations", get(platform_response))
        .route(
            "/api/apps/stores/organizations/{id}",
            get(platform_response),
        )
        .route("/api/apps/stores/members", get(platform_response))
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
async fn platform_store_snapshot_guards_legacy_http_reads_and_writes() {
    use serde_json::json;
    owned(|pool| async move {
        initialize_v1(&pool).await;
        let api_pool = PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_secs(2))
            .after_connect(|connection, _| Box::pin(async move {
                sqlx::query("SELECT pg_catalog.set_config('search_path','pg_catalog,public,pg_temp',false)")
                    .execute(connection).await?;
                Ok(())
            })).connect_with((*pool.connect_options()).clone()).await.unwrap();
        let state = Arc::new(PlatformFixture {
            pool: api_pool.clone(), tenant: Uuid::new_v4(), actor: Uuid::new_v4(), other: Uuid::new_v4(),
            parent: Uuid::new_v4(), active: Uuid::new_v4(), excluded: [Uuid::new_v4(), Uuid::new_v4()],
            self_scope: AtomicBool::new(false), expired: AtomicBool::new(false), organization_reads: AtomicUsize::new(0),
            lease_ms: AtomicU64::new(2000), check_unlocked: AtomicBool::new(true), projection_ready: Mutex::new(None),
        });
        let profiles = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let reports = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        for (index, organization) in [state.active, state.excluded[0], state.excluded[1]].into_iter().enumerate() {
            sqlx::query("INSERT INTO stores(id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact) VALUES($1,$2,$3,$4,$5,'OPEN','Legacy contact')")
                .bind(profiles[index]).bind(state.tenant).bind(organization).bind(state.actor).bind(format!("Legacy {index}")).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-20',$6::numeric)")
                .bind(reports[index]).bind(state.tenant).bind(profiles[index]).bind(organization).bind(state.actor).bind(format!("{}.00", (index+1)*10)).execute(&pool).await.unwrap();
        }
        let other_report = Uuid::new_v4();
        sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount) VALUES($1,$2,$3,$4,$5,DATE '2026-10-21',40)")
            .bind(other_report).bind(state.tenant).bind(profiles[0]).bind(state.active).bind(state.other).execute(&pool).await.unwrap();
        initialize_database(&pool).await.unwrap();
        let (origin, _server) = platform_fixture(state.clone()).await;
        let config = stores_api::Config::from_pairs([
            ("STORES_DATABASE_URL", "postgresql://fixture:fixture@127.0.0.1/stores"),
            ("STORES_ALLOWED_ORIGINS", "https://platform.example.test"), ("PLATFORM_API_ORIGIN", &origin),
            ("PLATFORM_CLIENT_ID", "fixture"), ("PLATFORM_CLIENT_SECRET", "fixture"),
            ("PLATFORM_AUTHORIZATION_MODE", "combined"),
        ]).unwrap();
        let app = stores_api::application(&config, api_pool.clone()).unwrap();
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
