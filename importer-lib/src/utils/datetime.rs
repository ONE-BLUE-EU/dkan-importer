use anyhow::Result;
use chrono::prelude::Local;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat};

pub fn get_utc_iso_datetime() -> String {
    let timestamp = chrono::Utc::now().to_rfc3339();
    return timestamp;
}

// let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
pub fn get_local_iso_datetime() -> String {
    return Local::now().to_rfc3339();
}

pub fn get_local_datetime_with_format(format: &str) -> String {
    return Local::now().format(format).to_string();
}

/// Parse a date string in various formats and return ISO format (YYYY-MM-DD)
pub fn parse_date_string(s: &str) -> Result<String> {
    // Try different date formats
    let formats = vec![
        "%Y-%m-%d", // ISO: 2024-08-28
        "%d/%m/%Y", // European: 28/08/2024
        "%m/%d/%Y", // US: 08/28/2024
        "%Y/%m/%d", // Asian: 2024/08/28
        "%d-%m-%Y", // European with dashes: 28-08-2024
        "%Y.%m.%d", // Dots: 2024.08.28
        "%d.%m.%Y", // European dots: 28.08.2024
    ];

    for format in formats {
        if let Ok(date) = NaiveDate::parse_from_str(s, format) {
            // Always return in ISO format (YYYY-MM-DD)
            return Ok(date.format("%Y-%m-%d").to_string());
        }
    }

    Err(anyhow::anyhow!("Unable to parse date: {}", s))
}

/// Parse a datetime string in various formats and return RFC 3339 format (YYYY-MM-DDTHH:MM:SSZ)
pub fn parse_datetime_string(s: &str) -> Result<String> {
    // Try different datetime formats
    let formats = vec![
        "%Y-%m-%dT%H:%M:%S", // ISO with T: 2024-08-28T15:30:00
        "%Y-%m-%d %H:%M:%S", // ISO with space: 2024-08-28 15:30:00
        "%d/%m/%Y %H:%M:%S", // European: 28/08/2024 15:30:00
        "%d/%m/%Y %H:%M",    // European no seconds: 28/08/2024 15:30
        "%m/%d/%Y %H:%M:%S", // US: 08/28/2024 15:30:00
        "%Y-%m-%d",          // Date only (add time): 2024-08-28
        "%d/%m/%Y",          // Date only: 28/08/2024
    ];

    for format in formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, format) {
            // Convert to UTC DateTime and use built-in RFC 3339 formatting with seconds precision
            return Ok(dt.and_utc().to_rfc3339_opts(SecondsFormat::Secs, true));
        }
    }

    // Try parsing as date only and add default time
    if let Ok(date) = parse_date_string(s) {
        // Parse as NaiveDate, add midnight time, convert to UTC, and format as RFC 3339
        if let Ok(parsed_date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
            let dt = parsed_date.and_hms_opt(0, 0, 0).unwrap();
            return Ok(dt.and_utc().to_rfc3339_opts(SecondsFormat::Secs, true));
        }
    }

    Err(anyhow::anyhow!("Unable to parse datetime: {}", s))
}

/// Parse a time string in various formats and return HH:MM:SS format
pub fn parse_time_string(s: &str) -> Result<String> {
    // Try different time formats
    let formats = vec![
        "%H:%M:%S",    // Full: 15:30:00
        "%H:%M",       // No seconds: 15:30
        "%I:%M:%S %p", // 12-hour with AM/PM: 03:30:00 PM
        "%I:%M %p",    // 12-hour no seconds: 03:30 PM
    ];

    for format in formats {
        if let Ok(time) = NaiveTime::parse_from_str(s, format) {
            // Always return in 24-hour format with seconds
            return Ok(time.format("%H:%M:%S").to_string());
        }
    }

    Err(anyhow::anyhow!("Unable to parse time: {}", s))
}
