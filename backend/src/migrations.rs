//! Embedded SQLx migrations and readiness verification for the independent database.

use sqlx::{Connection, PgPool};

use crate::error::{ServiceError, ServiceFailure};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn failed(error: impl std::error::Error + Send + Sync + 'static) -> ServiceError {
    ServiceError::caused_by(ServiceFailure::InitializationFailed, error)
}

pub(crate) async fn initialize(pool: &PgPool) -> Result<(), ServiceError> {
    // SQLx uses session advisory locks, including on early migration failures.
    // A detached connection cannot return such a lock or a changed search_path
    // to the request pool, and cancellation drops its physical socket as well.
    let mut connection = pool.acquire().await.map_err(failed)?.detach();
    let result = async {
        // Runtime queries use a catalog-first path. CREATE must target public,
        // including SQLx's unqualified migration history table, under a normal
        // non-superuser business role.
        sqlx::query(
            "SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',false)",
        )
        .execute(&mut connection)
        .await
        .map_err(failed)?;
        // SQLx exposes run_direct specifically to avoid the Acquire/HRTB
        // limitation for a borrowed connection in a Send startup future.
        MIGRATOR.run_direct(&mut connection).await.map_err(failed)
    }
    .await;
    let closed = connection.close().await.map_err(failed);
    result?;
    closed?;
    verify(pool).await
}

pub(crate) async fn verify(pool: &PgPool) -> Result<(), ServiceError> {
    let applied: Vec<(i64, bool, Vec<u8>)> = sqlx::query_as(
        "SELECT version,success,checksum FROM public._sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .map_err(failed)?;
    let expected: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .collect();
    if applied.len() != expected.len()
        || applied
            .iter()
            .zip(expected)
            .any(|((version, success, checksum), migration)| {
                *version != migration.version
                    || !success
                    || checksum.as_slice() != migration.checksum.as_ref()
            })
    {
        return Err(failed(std::io::Error::other(
            "business migration history mismatch",
        )));
    }
    let verified: bool = sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(fingerprint=public.stores_schema_fingerprint()),false) FROM public._stores_schema_contract WHERE singleton")
        .fetch_one(pool).await.map_err(failed)?;
    if !verified {
        return Err(failed(std::io::Error::other(
            "business schema contract mismatch",
        )));
    }
    Ok(())
}
