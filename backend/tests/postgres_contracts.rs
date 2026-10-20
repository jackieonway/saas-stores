//! Explicit real PostgreSQL contracts, isolated inside the owned CI server.
//! Ordinary test runs ignore the single parent; the CI owner supplies its URL.

use std::{future::Future, sync::Arc, time::Duration};

use sqlx::{postgres::PgPoolOptions, PgPool};
use stores_api::initialize_database;
use tokio::sync::Barrier;
use uuid::Uuid;

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
    assert_eq!(url.host_str(), Some(format!("{owner}-postgres").as_str()));
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
        let changed = sqlx::query("UPDATE stores SET name='Version two',version=version+1,updated_at=now() WHERE id=$1 AND version=1").bind(store).execute(&pool).await.unwrap().rows_affected();
        let stale = sqlx::query("UPDATE stores SET name='stale',version=version+1 WHERE id=$1 AND version=1").bind(store).execute(&pool).await.unwrap().rows_affected();
        assert_eq!((changed, stale), (1, 0));
        check("immutable_ownership_date_and_optimistic_version");

        let link = Uuid::new_v4();
        sqlx::query("INSERT INTO store_staff(id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id) VALUES($1,$2,$3,$4,$5,$5,'Member',$5)").bind(link).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.unwrap();
        let duplicate = sqlx::query("INSERT INTO store_staff(id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id) VALUES($1,$2,$3,$4,$5,$5,'Member',$5)").bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.expect_err("active staff uniqueness");
        assert_eq!(duplicate.as_database_error().and_then(|error| error.constraint()), Some("store_staff_active_member_key"));
        sqlx::query("UPDATE store_staff SET removed_at=now(),updated_at=now(),version=version+1 WHERE id=$1 AND version=1").bind(link).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO store_staff(id,tenant_id,store_id,organization_id,owner_user_id,user_id,display_name,created_by_user_id) VALUES($1,$2,$3,$4,$5,$5,'Member',$5)").bind(Uuid::new_v4()).bind(tenant).bind(store).bind(org).bind(user).execute(&pool).await.unwrap();
        let history: (i64, i64) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE removed_at IS NULL) FROM store_staff").fetch_one(&pool).await.unwrap();
        assert_eq!(history, (2, 1));
        check("staff_active_uniqueness_and_removed_history");

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
