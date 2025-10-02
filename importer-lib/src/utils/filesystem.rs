use std::fs::OpenOptions;
use std::io::Write;

use crate::ERRORS_LOG_FILE;
use crate::utils::get_utc_iso_datetime;

/// Centralized function to write error messages to the errors log file
///
/// # Arguments
/// * `error_type` - A description of the error type/category (e.g., "Data Dictionary Duplicate Check Error")
/// * `error_message` - The actual error message content
pub fn write_error_to_log(error_type: &str, error_message: &str) {
    let timestamp = get_utc_iso_datetime();
    let log_entry = format!("\n[{}] {}:\n{}\n", timestamp, error_type, error_message);

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(ERRORS_LOG_FILE)
    {
        let _ = writeln!(file, "{}", log_entry);
    }
}
