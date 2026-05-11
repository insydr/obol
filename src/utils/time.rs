use chrono::Utc;

/// Get the current UTC timestamp as an ISO 8601 string.
pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Get the current UTC timestamp as a Unix epoch in seconds.
pub fn now_epoch_secs() -> i64 {
    Utc::now().timestamp()
}

/// Get the current UTC timestamp as a Unix epoch in milliseconds.
pub fn now_epoch_millis() -> i64 {
    Utc::now().timestamp_millis()
}

/// Format a duration in milliseconds to a human-readable string.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 3_600_000 {
        format!("{:.1}m", ms as f64 / 60_000.0)
    } else {
        format!("{:.1}h", ms as f64 / 3_600_000.0)
    }
}
