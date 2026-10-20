//! Independent store business service.
//!
//! Identity and authorization are obtained through the platform's bounded HTTP
//! protocol. This crate owns only its separate business database and does not
//! depend on platform or work-order implementation crates. Startup initializes
//! and verifies the schema before binding a listener. Public errors contain
//! fixed safe categories, and no environment file is loaded automatically.

mod authorization;
mod config;
mod directory;
mod domain;
mod error;
mod http;
mod migrations;
mod reports;
mod staff;
mod stores;

use std::{future::IntoFuture, time::Duration};

use sqlx::{postgres::PgPoolOptions, PgPool};
use tokio::{net::TcpListener, sync::oneshot};

use error::ServiceFailure;

pub use config::Config;
pub use error::{ConfigError, ServiceError};
pub use http::{application, router};

/// Initialize or verify the independent store schema transactionally.
///
/// Only the business pool is used. The caller must complete this operation
/// before accepting requests; inconsistent migrations or schema are rejected.
pub async fn initialize_database(pool: &PgPool) -> Result<(), ServiceError> {
    migrations::initialize(pool).await
}

/// Connect only to the explicitly configured business database.
///
/// Acquisition, statements, locks and idle transactions have separate finite
/// budgets. This does not execute migrations or claim application readiness.
pub async fn connect_database(config: &Config) -> Result<PgPool, ServiceError> {
    let statement = format!("{}ms", config.statement_timeout.as_millis());
    let lock = format!("{}ms", config.lock_timeout.as_millis());
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .idle_timeout(Some(Duration::from_secs(30)))
        .after_connect(move |connection, _| {
            let statement = statement.clone();
            let lock = lock.clone();
            Box::pin(async move {
                sqlx::query(
                    "SELECT pg_catalog.set_config('statement_timeout',$1,false), \
                     pg_catalog.set_config('lock_timeout',$2,false), \
                     pg_catalog.set_config('idle_in_transaction_session_timeout','15000ms',false), \
                     pg_catalog.set_config('search_path','pg_catalog,public,pg_temp',false)",
                )
                .bind(statement)
                .bind(lock)
                .execute(connection)
                .await?;
                Ok(())
            })
        })
        .connect_with(config.database.clone())
        .await
        .map_err(|error| ServiceError::caused_by(ServiceFailure::DatabaseUnavailable, error))
}

/// Run the independently deployable API with bounded startup and shutdown.
///
/// The schema and platform client settings are checked before listening. SIGINT
/// or SIGTERM closes the listener and pool within the configured shutdown budget.
pub async fn run(config: Config) -> Result<(), ServiceError> {
    let (pool, authorizer) = tokio::time::timeout(config.startup_timeout, async {
        let authorizer = authorization::Authorizer::new(&config)?;
        let pool = connect_database(&config).await?;
        if let Err(error) = initialize_database(&pool).await {
            pool.close().await;
            return Err(error);
        }
        Ok((pool, authorizer))
    })
    .await
    .map_err(|error| ServiceError::caused_by(ServiceFailure::StartupTimedOut, error))??;
    let listener = match TcpListener::bind(config.bind).await {
        Ok(listener) => listener,
        Err(error) => {
            close_pool(&pool, config.shutdown_timeout).await?;
            return Err(ServiceError::caused_by(
                ServiceFailure::ListenerFailed,
                error,
            ));
        }
    };
    let address = listener
        .local_addr()
        .map_err(|error| ServiceError::caused_by(ServiceFailure::ListenerFailed, error))?;
    tracing::info!(event = "stores_listening", host = %address.ip(), port = address.port());
    let app = http::configured_metrics_router(
        pool.clone(),
        config.health_timeout,
        Some(authorizer),
        config.allowed_origins.clone(),
        config.metrics_token.clone(),
    );
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .into_future();
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => {
            close_pool(&pool, config.shutdown_timeout).await?;
            result.map_err(|error| ServiceError::caused_by(ServiceFailure::ListenerFailed, error))
        }
        signal = shutdown_signal() => {
            let _ = shutdown_tx.send(());
            tokio::time::timeout(config.shutdown_timeout, async {
                let result = server.await;
                pool.close().await;
                signal?;
                result.map_err(|error| ServiceError::caused_by(ServiceFailure::ListenerFailed, error))
            }).await.map_err(|error| ServiceError::caused_by(ServiceFailure::ShutdownTimedOut, error))?
        }
    }
}

async fn close_pool(pool: &PgPool, timeout: Duration) -> Result<(), ServiceError> {
    tokio::time::timeout(timeout, pool.close())
        .await
        .map_err(|error| ServiceError::caused_by(ServiceFailure::ShutdownTimedOut, error))
}

async fn shutdown_signal() -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate())
            .map_err(|error| ServiceError::caused_by(ServiceFailure::SignalFailed, error))?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.map_err(|error| ServiceError::caused_by(ServiceFailure::SignalFailed, error)),
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| ServiceError::caused_by(ServiceFailure::SignalFailed, error))
}

mod telemetry;
