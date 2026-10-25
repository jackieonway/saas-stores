//! API mode and receipt failures are rejected before a database or listener can
//! be opened. These subprocess tests do not require PostgreSQL.

use std::process::{Command, Output};

const URL: &str =
    "postgresql://private-runtime-login:private-runtime-password@127.0.0.1:1/private-database";
const MARKER: &str = "saas-uuid-import/1/019a1158-67b0-7cce-83d2-123456789abc/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA256: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stores-api"));
    command.env_clear().env("STORES_DATABASE_URL", URL);
    command
}

fn rejected(output: Output, category: &str) {
    assert!(!output.status.success());
    let rendered = format!(
        "{}{}",
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(rendered.contains(category), "{rendered}");
    assert!(!rendered.contains("stores_listening"), "{rendered}");
    assert!(!rendered.contains("connection unavailable"), "{rendered}");
    for private in ["private-", MARKER, SHA256] {
        assert!(!rendered.contains(private), "{rendered}");
    }
}

#[test]
fn offline_import_mode_cannot_start_the_api() {
    rejected(
        command()
            .env("STORES_DATABASE_INIT_MODE", "import")
            .output()
            .unwrap(),
        "STORES_DATABASE_INIT_MODE must be seed or imported",
    );
}

#[test]
fn orphan_receipts_are_rejected_before_other_http_configuration() {
    for mode in [None, Some("seed")] {
        for name in [
            "STORES_EXPECTED_DATABASE",
            "STORES_IMPORT_MARKER",
            "STORES_EXPECTED_SCHEMA_SHA256",
        ] {
            let mut command = command();
            if let Some(mode) = mode {
                command.env("STORES_DATABASE_INIT_MODE", mode);
            }
            rejected(
                command.env(name, "").output().unwrap(),
                "STORES imported database receipt configuration is invalid",
            );
        }
    }
}

#[test]
fn incomplete_or_malformed_imported_receipts_never_reach_connection() {
    let fields = [
        ("STORES_EXPECTED_DATABASE", "private-database"),
        ("STORES_IMPORT_MARKER", MARKER),
        ("STORES_EXPECTED_SCHEMA_SHA256", SHA256),
    ];
    for omitted in 0..fields.len() {
        let mut command = command();
        command.env("STORES_DATABASE_INIT_MODE", "imported");
        for (index, (name, value)) in fields.iter().enumerate() {
            if index != omitted {
                command.env(name, value);
            }
        }
        rejected(
            command.output().unwrap(),
            "STORES imported database receipt configuration is invalid",
        );
    }
    for (name, invalid) in [
        ("STORES_EXPECTED_DATABASE", "private-wrong-database"),
        ("STORES_IMPORT_MARKER", "private-invalid-marker"),
        ("STORES_EXPECTED_SCHEMA_SHA256", "private-invalid-sha256"),
    ] {
        rejected(
            command()
                .env("STORES_DATABASE_INIT_MODE", "imported")
                .envs(fields)
                .env(name, invalid)
                .output()
                .unwrap(),
            "STORES imported database receipt configuration is invalid",
        );
    }
}

#[cfg(any(windows, unix))]
#[test]
fn non_unicode_mode_and_receipts_cannot_fall_back_to_initialization() {
    #[cfg(windows)]
    let non_unicode = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0xd800])
    };
    #[cfg(unix)]
    let non_unicode = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![0xff])
    };
    for (name, category) in [
        (
            "STORES_DATABASE_INIT_MODE",
            "STORES_DATABASE_INIT_MODE must be seed or imported",
        ),
        (
            "STORES_EXPECTED_DATABASE",
            "STORES imported database receipt configuration is invalid",
        ),
        (
            "STORES_IMPORT_MARKER",
            "STORES imported database receipt configuration is invalid",
        ),
        (
            "STORES_EXPECTED_SCHEMA_SHA256",
            "STORES imported database receipt configuration is invalid",
        ),
    ] {
        rejected(
            command().env(name, &non_unicode).output().unwrap(),
            category,
        );
    }
}
