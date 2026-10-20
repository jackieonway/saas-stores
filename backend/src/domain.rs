//! Pure request invariants. No HTTP client, database driver or identity lookup.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::error::ApiError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_exact_and_never_rounds_or_accepts_exponents() {
        for (raw, expected) in [
            ("0", "0.00"),
            ("0.01", "0.01"),
            ("12.3", "12.30"),
            ("9999999999999999.99", "9999999999999999.99"),
        ] {
            assert_eq!(Money::parse(raw).unwrap().as_db_string(), expected);
        }
        for raw in [
            "",
            "-1",
            "+1",
            ".01",
            "1.",
            "1.001",
            "1e2",
            " 1",
            "1 ",
            "10000000000000000",
            "１２",
            "1,000",
        ] {
            assert!(Money::parse(raw).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn business_day_uses_shanghai_civil_midnight_and_real_dates() {
        let before = DateTime::parse_from_rfc3339("2026-10-18T15:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let after = DateTime::parse_from_rfc3339("2026-10-18T16:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(Day::from_utc(before).as_str(), "2026-10-18");
        assert_eq!(Day::from_utc(after).as_str(), "2026-10-19");
        for raw in ["0001-01-01", "9999-12-31", "2024-02-29", "2099-01-01"] {
            assert!(Day::parse(raw).is_ok());
        }
        for raw in [
            "0000-01-01",
            "2026-02-29",
            "2026-2-01",
            "2026-10-18T00:00:00+08:00",
            "10000-01-01",
        ] {
            assert!(Day::parse(raw).is_err());
        }
    }

    #[test]
    fn query_rejects_ambiguous_encoding_duplicates_unknowns_and_invalid_pages() {
        for raw in [
            "page=1&page=2",
            "other=1",
            "page=%",
            "page=%GG",
            "page=%FF",
            "page=0",
            "size=51",
            "page=1.0",
        ] {
            let result = query(Some(raw), &["page", "size"]).and_then(|q| pagination(&q));
            assert!(result.is_err(), "accepted {raw}");
        }
        assert_eq!(
            pagination(&query(None, &["page", "size"]).unwrap()).unwrap(),
            (1, 20)
        );
        assert_eq!(query(Some("q=a%2Bb+c"), &["q"]).unwrap()["q"], "a+b c");
    }

    #[test]
    fn version_and_identity_require_canonical_input() {
        assert_eq!(version(&serde_json::json!(1)).unwrap(), 1);
        for value in [
            serde_json::json!(0),
            serde_json::json!(1.5),
            serde_json::json!("1"),
            Value::Null,
            serde_json::json!(2147483647),
        ] {
            assert!(version(&value).is_err());
        }
        assert!(uuid("00000000-0000-4000-8000-000000000001").is_ok());
        assert!(uuid("00000000000040008000000000000001").is_err());
        assert!(uuid("00000000-0000-0000-0000-000000000000").is_err());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Money(i64);

impl Money {
    pub(crate) fn parse(raw: &str) -> Result<Self, ApiError> {
        let mut parts = raw.split('.');
        let integral = parts.next().unwrap_or_default();
        let decimal = parts.next();
        if parts.next().is_some()
            || integral.is_empty()
            || integral.len() > 16
            || !integral.bytes().all(|b| b.is_ascii_digit())
            || decimal.is_some_and(|d| {
                d.is_empty() || d.len() > 2 || !d.bytes().all(|b| b.is_ascii_digit())
            })
        {
            return Err(ApiError::validation());
        }
        let units: i64 = integral.parse().map_err(|_| ApiError::validation())?;
        let fraction = match decimal {
            None => 0,
            Some(d) => {
                d.parse::<i64>().map_err(|_| ApiError::validation())?
                    * if d.len() == 1 { 10 } else { 1 }
            }
        };
        units
            .checked_mul(100)
            .and_then(|v| v.checked_add(fraction))
            .map(Self)
            .ok_or_else(ApiError::validation)
    }

    pub(crate) fn as_db_string(&self) -> String {
        format!("{}.{:02}", self.0 / 100, self.0 % 100)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Day(String);

impl Day {
    pub(crate) fn parse(raw: &str) -> Result<Self, ApiError> {
        if raw.len() != 10
            || raw.as_bytes()[4] != b'-'
            || raw.as_bytes()[7] != b'-'
            || !raw
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
        {
            return Err(ApiError::validation());
        }
        let day = NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| ApiError::validation())?;
        if !(1..=9999).contains(&day.year()) {
            return Err(ApiError::validation());
        }
        Ok(Self(raw.to_owned()))
    }

    pub(crate) fn from_utc(now: DateTime<Utc>) -> Self {
        // Shanghai has a fixed +08:00 offset for the example's contemporary business dates.
        let local = now + chrono::Duration::hours(8);
        Self(local.format("%Y-%m-%d").to_string())
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn uuid(raw: &str) -> Result<Uuid, ApiError> {
    let parsed = Uuid::parse_str(raw).map_err(|_| ApiError::validation())?;
    if raw.len() != 36
        || parsed.is_nil()
        || !parsed.hyphenated().to_string().eq_ignore_ascii_case(raw)
    {
        return Err(ApiError::validation());
    }
    Ok(parsed)
}

pub(crate) fn version(value: &Value) -> Result<i32, ApiError> {
    value
        .as_i64()
        .filter(|n| (1..=i64::from(i32::MAX - 1)).contains(n))
        .map(|n| n as i32)
        .ok_or_else(ApiError::validation)
}

fn decode(raw: &str) -> Result<String, ApiError> {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut input = raw.bytes();
    while let Some(b) = input.next() {
        match b {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hi = input
                    .next()
                    .and_then(|v| (v as char).to_digit(16))
                    .ok_or_else(ApiError::validation)?;
                let lo = input
                    .next()
                    .and_then(|v| (v as char).to_digit(16))
                    .ok_or_else(ApiError::validation)?;
                bytes.push((hi * 16 + lo) as u8);
            }
            _ => bytes.push(b),
        }
    }
    String::from_utf8(bytes).map_err(|_| ApiError::validation())
}

pub(crate) fn query(
    raw: Option<&str>,
    allowed: &[&str],
) -> Result<BTreeMap<String, String>, ApiError> {
    let raw = raw.unwrap_or_default();
    if raw.len() > 8192 {
        return Err(ApiError::validation());
    }
    let mut result = BTreeMap::new();
    for pair in raw.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode(key)?;
        let value = decode(value)?;
        if !allowed.contains(&key.as_str())
            || key.contains('\0')
            || value.contains('\0')
            || result.insert(key, value).is_some()
        {
            return Err(ApiError::validation());
        }
    }
    Ok(result)
}

pub(crate) fn pagination(query: &BTreeMap<String, String>) -> Result<(u32, u32), ApiError> {
    let number = |key, default, max| -> Result<u32, ApiError> {
        let Some(raw) = query.get(key) else {
            return Ok(default);
        };
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ApiError::validation());
        }
        raw.parse::<u32>()
            .ok()
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(ApiError::validation)
    };
    Ok((number("page", 1, 1_000_000)?, number("size", 20, 50)?))
}
