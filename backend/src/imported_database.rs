//! Read-only admission of a database authenticated by the UUID import receipt.

use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use url::Url;
use uuid::{Uuid, Variant};

use crate::{
    error::{ConfigError, ServiceError, ServiceFailure},
    migrations,
};

#[derive(Clone)]
pub(crate) enum DatabaseInitialization {
    Initialize,
    Imported(ImportReceipt),
}

#[derive(Clone)]
pub(crate) struct ImportReceipt {
    database: String,
    login: String,
    marker: String,
    schema_sha256: String,
}

impl DatabaseInitialization {
    pub(crate) fn from_lookup(
        get: &impl Fn(&str) -> Option<String>,
        database_url: &str,
    ) -> Result<Self, ConfigError> {
        let mode = get("STORES_DATABASE_INIT_MODE");
        let expected = get("STORES_EXPECTED_DATABASE");
        let marker = get("STORES_IMPORT_MARKER");
        let schema_sha256 = get("STORES_EXPECTED_SCHEMA_SHA256");
        match mode.as_deref() {
            None | Some("seed") => {
                if expected.is_some() || marker.is_some() || schema_sha256.is_some() {
                    return Err(ConfigError::ImportReceipt);
                }
                Ok(Self::Initialize)
            }
            Some("imported") => {
                let database = expected.ok_or(ConfigError::ImportReceipt)?;
                let marker = marker.ok_or(ConfigError::ImportReceipt)?;
                let schema_sha256 = schema_sha256.ok_or(ConfigError::ImportReceipt)?;
                if database.is_empty()
                    || database.chars().any(char::is_control)
                    || database_url.chars().any(char::is_control)
                    || !valid_marker(&marker)
                    || !lower_sha256(&schema_sha256)
                {
                    return Err(ConfigError::ImportReceipt);
                }
                let url = Url::parse(database_url.trim()).map_err(|_| ConfigError::DatabaseUrl)?;
                let decoded_database = decode_component(url.path().trim_start_matches('/'))?;
                let login = decode_component(url.username())?;
                if database != decoded_database || login.is_empty() {
                    return Err(ConfigError::ImportReceipt);
                }
                Ok(Self::Imported(ImportReceipt {
                    database,
                    login,
                    marker,
                    schema_sha256,
                }))
            }
            // Import is an offline migration command, never an API startup mode.
            _ => Err(ConfigError::DatabaseInitMode),
        }
    }
}

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_marker(marker: &str) -> bool {
    let Some((batch, digest)) = marker
        .strip_prefix("saas-uuid-import/1/")
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    let Ok(batch_id) = Uuid::parse_str(batch) else {
        return false;
    };
    batch_id.get_version_num() == 7
        && batch_id.get_variant() == Variant::RFC4122
        && batch_id.hyphenated().to_string() == batch
        && lower_sha256(digest)
}

fn decode_component(raw: &str) -> Result<String, ConfigError> {
    let mut decoded = Vec::with_capacity(raw.len());
    let mut bytes = raw.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|byte| char::from(byte).to_digit(16));
            let low = bytes.next().and_then(|byte| char::from(byte).to_digit(16));
            let (Some(high), Some(low)) = (high, low) else {
                return Err(ConfigError::DatabaseUrl);
            };
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| ConfigError::DatabaseUrl)?;
    if decoded.chars().any(char::is_control) {
        return Err(ConfigError::DatabaseUrl);
    }
    Ok(decoded)
}

fn failed(error: impl std::error::Error + Send + Sync + 'static) -> ServiceError {
    ServiceError::caused_by(ServiceFailure::ImportedDatabaseRejected, error)
}

// pg_has_role handles both inherited privileges and transitive SET permission.
// The owner set includes every user schema and its objects, so an alternate
// schema cannot be a path around the runtime-role fence. CREATE also checks
// roles reachable only with SET, whose privileges are not currently inherited.
// An accessible membership administrator can grant itself SET/INHERIT later,
// so any ADMIN edge held by an accessible member role is also rejected.
const ADMISSION_SQL: &str = r#"
WITH user_schemas AS (
    SELECT oid,nspowner FROM pg_catalog.pg_namespace
    WHERE nspname <> 'information_schema' AND pg_catalog.left(nspname::text,3) <> 'pg_'
), owners AS (
    SELECT datdba AS oid FROM pg_catalog.pg_database WHERE datname=current_database()
    UNION SELECT nspowner FROM user_schemas
    UNION SELECT c.relowner FROM pg_catalog.pg_class c JOIN user_schemas n ON n.oid=c.relnamespace
    UNION SELECT p.proowner FROM pg_catalog.pg_proc p JOIN user_schemas n ON n.oid=p.pronamespace
    UNION SELECT t.typowner FROM pg_catalog.pg_type t JOIN user_schemas n ON n.oid=t.typnamespace
), reachable_roles AS (
    SELECT r.* FROM pg_catalog.pg_roles r
    WHERE pg_catalog.pg_has_role(session_user,r.oid,'USAGE'::text)
       OR pg_catalog.pg_has_role(session_user,r.oid,'SET'::text)
)
SELECT current_database()::text AS database,
       pg_catalog.shobj_description(d.oid,'pg_database'::pg_catalog.name) AS marker,
       current_user::text AS effective_login, session_user::text AS session_login,
       EXISTS (SELECT 1 FROM pg_catalog.pg_roles r
               WHERE r.rolname=session_user AND r.rolcanlogin)
       AND EXISTS (SELECT 1 FROM pg_catalog.pg_roles dbowner
                   WHERE dbowner.oid=d.datdba AND NOT dbowner.rolcanlogin AND NOT dbowner.rolsuper)
       AND NOT EXISTS (SELECT 1 FROM reachable_roles r
                       WHERE r.rolsuper OR r.rolcreatedb OR r.rolcreaterole
                          OR r.rolreplication OR r.rolbypassrls)
       AND NOT EXISTS (SELECT 1 FROM owners o JOIN reachable_roles r USING(oid))
       AND NOT EXISTS (SELECT 1 FROM reachable_roles r
                       WHERE pg_catalog.has_database_privilege(r.oid,d.oid,'CREATE'::text))
       AND NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_namespace n CROSS JOIN reachable_roles r
           WHERE pg_catalog.left(n.nspname::text,8) <> 'pg_temp_'
             AND pg_catalog.left(n.nspname::text,14) <> 'pg_toast_temp_'
             AND pg_catalog.has_schema_privilege(r.oid,n.oid,'CREATE'::text))
       AND pg_catalog.current_setting('transaction_read_only'::text)='on'
       AND pg_catalog.current_setting('transaction_isolation'::text)='repeatable read' AS runtime_role_valid,
       EXISTS (SELECT 1 FROM reachable_roles r
               WHERE pg_catalog.has_database_privilege(r.oid,d.oid,'TEMPORARY'::text)) AS temporary_privilege,
       EXISTS (
           SELECT 1 FROM pg_catalog.pg_auth_members m CROSS JOIN pg_catalog.pg_roles actor
           WHERE actor.rolname=current_user AND m.admin_option
             AND (pg_catalog.pg_has_role(actor.oid,m.member,'SET'::text)
                  OR pg_catalog.pg_has_role(actor.oid,m.member,'USAGE'::text))) AS role_administration
FROM pg_catalog.pg_database d WHERE d.datname=current_database()
"#;

#[derive(Clone, sqlx::FromRow)]
struct Admission {
    database: String,
    marker: Option<String>,
    effective_login: String,
    session_login: String,
    runtime_role_valid: bool,
    temporary_privilege: bool,
    role_administration: bool,
}

fn admitted(receipt: &ImportReceipt, evidence: &Admission) -> bool {
    evidence.database == receipt.database
        && evidence.marker.as_deref() == Some(receipt.marker.as_str())
        && evidence.effective_login == receipt.login
        && evidence.session_login == receipt.login
        && evidence.runtime_role_valid
        && !evidence.temporary_privilege
        && !evidence.role_administration
}

fn schema_matches(receipt: &ImportReceipt, catalog: &Value) -> Result<bool, ServiceError> {
    if !catalog.is_array() {
        return Ok(false);
    }
    let canonical = serde_json::to_vec(catalog).map_err(failed)?;
    Ok(format!("{:x}", Sha256::digest(canonical)) == receipt.schema_sha256)
}

const CATALOG_SETTINGS: [&str; 3] = [
    "SET LOCAL search_path=pg_catalog",
    "SET LOCAL timezone='UTC'",
    "SET LOCAL row_security=off",
];
const RUNTIME_SEARCH_PATH: &str = "SET LOCAL search_path=pg_catalog,public,pg_temp";

pub(crate) async fn verify(pool: &PgPool, receipt: &ImportReceipt) -> Result<(), ServiceError> {
    let mut transaction = pool.begin().await.map_err(failed)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(failed)?;
    // Deparser output is visibility-sensitive. Match the authenticated import
    // context before reading schema material, including composite-array types.
    for statement in CATALOG_SETTINGS {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .map_err(failed)?;
    }
    let evidence: Admission = sqlx::query_as(ADMISSION_SQL)
        .fetch_one(&mut *transaction)
        .await
        .map_err(failed)?;
    if !admitted(receipt, &evidence) {
        return Err(failed(std::io::Error::other("import admission mismatch")));
    }
    let catalog: Value = sqlx::query_scalar(include_str!("imported_schema.sql"))
        .fetch_one(&mut *transaction)
        .await
        .map_err(failed)?;
    if !schema_matches(receipt, &catalog)? {
        return Err(failed(std::io::Error::other("import schema mismatch")));
    }
    // Existing fingerprints were established with public visible. Preserve
    // their historical deparser context without changing any stored baseline.
    sqlx::query(RUNTIME_SEARCH_PATH)
        .execute(&mut *transaction)
        .await
        .map_err(failed)?;
    migrations::verify_connection(&mut transaction).await?;
    transaction.commit().await.map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = "saas-uuid-import/1/019a1158-67b0-7cce-83d2-123456789abc/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn receipt() -> ImportReceipt {
        ImportReceipt {
            database: "stores imported".into(),
            login: "stores runtime".into(),
            marker: MARKER.into(),
            schema_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
                .into(),
        }
    }

    #[test]
    fn marker_requires_exact_version_canonical_v7_variant_and_digest() {
        assert!(valid_marker(MARKER));
        for invalid in [
            MARKER.replace("/1/", "/2/"),
            MARKER.replace("7cce", "4cce"),
            MARKER.replace("83d2", "c3d2"),
            MARKER.replace("7cce", "7CCE"),
            format!("{MARKER}/extra"),
            format!(" {MARKER}"),
            MARKER.replace("aaaaaaaa", "AAAAaaaa"),
            MARKER[..MARKER.len() - 1].to_owned(),
        ] {
            assert!(!valid_marker(&invalid));
        }
    }

    #[test]
    fn url_identity_decoding_is_strict_without_form_encoding() {
        assert_eq!(
            decode_component("stores%20runtime+").unwrap(),
            "stores runtime+"
        );
        assert_eq!(decode_component("%e9%97%a8%e5%ba%97").unwrap(), "门店");
        for invalid in ["%", "%0", "%GG", "%ff", "%00", "%0A", "%7F", "%c2%85"] {
            assert_eq!(decode_component(invalid), Err(ConfigError::DatabaseUrl));
        }
    }

    #[test]
    fn admission_requires_each_exact_identity_and_runtime_permission_fence() {
        let receipt = receipt();
        let evidence = Admission {
            database: receipt.database.clone(),
            marker: Some(receipt.marker.clone()),
            effective_login: receipt.login.clone(),
            session_login: receipt.login.clone(),
            runtime_role_valid: true,
            temporary_privilege: false,
            role_administration: false,
        };
        assert!(admitted(&receipt, &evidence));
        for rejected in [
            Admission {
                database: "another".into(),
                ..evidence.clone()
            },
            Admission {
                marker: None,
                ..evidence.clone()
            },
            Admission {
                marker: Some(format!("{}\n", receipt.marker)),
                ..evidence.clone()
            },
            Admission {
                effective_login: "owner".into(),
                ..evidence.clone()
            },
            Admission {
                session_login: "owner".into(),
                ..evidence.clone()
            },
            Admission {
                runtime_role_valid: false,
                ..evidence.clone()
            },
            Admission {
                temporary_privilege: true,
                ..evidence.clone()
            },
            Admission {
                role_administration: true,
                ..evidence.clone()
            },
        ] {
            assert!(!admitted(&receipt, &rejected));
        }
    }

    #[test]
    fn runtime_owner_contract_requires_a_non_login_non_superuser_database_owner() {
        let sql = ADMISSION_SQL
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(sql.contains(
            "EXISTS (SELECT 1 FROM pg_catalog.pg_roles dbowner WHERE dbowner.oid=d.datdba AND NOT dbowner.rolcanlogin AND NOT dbowner.rolsuper)"
        ));
    }

    #[test]
    fn schema_digest_authenticates_the_canonical_ordered_json_array() {
        let receipt = receipt();
        assert!(schema_matches(&receipt, &serde_json::json!([])).unwrap());
        for rejected in [
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!(["changed"]),
        ] {
            assert!(!schema_matches(&receipt, &rejected).unwrap());
        }
        let mut receipt = receipt;
        receipt.schema_sha256 =
            "db1ae46d919a4bf5eb460249076679903197cf01faf6dbd1ecd3d849ff7c8b45".into();
        let catalog: Value = serde_json::from_str(r#"[{"b":2,"a":1},{"b":4,"a":3}]"#).unwrap();
        assert!(schema_matches(&receipt, &catalog).unwrap());
        let mut reordered = catalog.as_array().unwrap().clone();
        reordered.reverse();
        assert!(!schema_matches(&receipt, &Value::Array(reordered)).unwrap());
    }

    #[test]
    fn schema_digest_binds_composite_array_type_visibility() {
        let mut receipt = receipt();
        receipt.schema_sha256 =
            "346dc44734d6168a1a7a036118b00386001539cbe0b2585dff1dc6118c67c108".into();
        let mut catalog = serde_json::json!([{"kind":"column","type":"public.contract_row[]"}]);
        assert!(schema_matches(&receipt, &catalog).unwrap());
        // public on search_path makes format_type omit this qualification.
        // That otherwise identical type must not silently match the receipt.
        catalog[0]["type"] = serde_json::json!("contract_row[]");
        assert!(!schema_matches(&receipt, &catalog).unwrap());
    }
}
