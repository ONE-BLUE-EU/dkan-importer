use chrono::prelude::Local;

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
