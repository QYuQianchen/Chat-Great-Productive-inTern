use chrono::{DateTime, Days, NaiveDate, Utc};
use std::env;
use std::process::Command;

use crate::constants::AppResult;

pub fn env_or_default(keys: &[&str], default: &str) -> String {
    keys.iter()
        .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| default.to_string())
}

pub fn pick_date(override_date: Option<&str>, env_keys: &[&str], fallback_default: &str) -> String {
    override_date
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| env_or_default(env_keys, fallback_default))
}

pub fn parse_utc_range(start_date: &str, end_date: &str) -> AppResult<(i64, i64)> {
    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .map_err(|_| format!("Invalid start date `{start_date}`. Expected format YYYY-MM-DD."))?;
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .map_err(|_| format!("Invalid end date `{end_date}`. Expected format YYYY-MM-DD."))?;

    if end < start {
        return Err(
            format!("Invalid date range: end date `{end_date}` is before `{start_date}`.").into(),
        );
    }

    let start_dt = start
        .and_hms_opt(0, 0, 0)
        .ok_or("Could not build start datetime at midnight UTC.")?;
    let end_exclusive = end
        .checked_add_days(Days::new(1))
        .ok_or("Could not compute exclusive end datetime.")?
        .and_hms_opt(0, 0, 0)
        .ok_or("Could not build end datetime at midnight UTC.")?;

    Ok((
        start_dt.and_utc().timestamp(),
        end_exclusive.and_utc().timestamp(),
    ))
}

pub fn display_recipient_to_stream_name(value: &serde_json::Value) -> String {
    if let Some(name) = value.as_str() {
        return name.to_string();
    }
    value
        .as_object()
        .and_then(|obj| obj.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or_default()
        .to_string()
}

pub fn timestamp_to_rfc3339(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

pub fn run_command(program: &str, args: &[&str]) -> AppResult<String> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "command failed: {} {}\n{}",
            program,
            args.join(" "),
            stderr.trim()
        )
        .into());
    }

    Ok(String::from_utf8(output.stdout)?)
}

pub fn compact_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
